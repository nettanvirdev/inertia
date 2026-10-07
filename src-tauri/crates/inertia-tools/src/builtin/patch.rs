//! `patch` - changing several files in one call.
//!
//! A rename that touches nine files is nine `edit` calls, which is nine round
//! trips to the model, nine permission prompts and nine chances to be
//! interrupted halfway through with the codebase in a state that compiles
//! nowhere. The unit of work was always "this refactor", and splitting it into
//! one call per file was an accident of the tool, not a property of the change.
//!
//! So this takes a unified diff - the format every model has already seen a
//! million of - and applies the whole thing or none of it. "None of it" is the
//! important half. A patch that applies four files and fails on the fifth has
//! produced a broken tree and a model that now has to work out what landed, and
//! that is strictly worse than having done nothing.
//!
//! Every rule `edit` enforces is enforced here too, per file: the model must have
//! read the file this session, and it must not have changed since. Nothing about
//! doing nine edits at once makes editing from memory any safer.
//!
//! Locating a hunk reuses the match ladder in `replace.rs` rather than trusting
//! the line numbers in the `@@` header. Those numbers are the model's
//! arithmetic, and its arithmetic is the least reliable thing it produces; the
//! surrounding context lines are what it actually copied, and matching on those
//! is what makes a patch land.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use serde_json::{json, Value};

use super::fence;
use super::read_state::ReadState;
use crate::replace::replace;

/// Written as an escape rather than the character itself, because an invisible
/// BOM in the source is a thing an editor will helpfully strip one day.
const BOM: char = '\u{feff}';

/// Unchanged lines shown either side of a change in the diffs handed to the
/// card.
const CONTEXT: usize = 3;

/// Above this the LCS table is not built and the changed region is reported
/// as "all of the old, then all of the new". A model that rewrote a 50k-line
/// file is not going to read a minimal diff of it anyway.
const MAX_LCS_CELLS: usize = 4_000_000;

/// A failure the model can act on. The registry turns it into an `ok: false`
/// result carrying exactly this text, which is what Electron's `ToolError` did.
fn refuse(message: impl Into<String>) -> Error {
    Error::Other(message.into())
}

// ── parsing ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkLine {
    /// `' '`, `'-'` or `'+'`.
    pub sign: char,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// The `@@` line as written, quoted back when the hunk does not apply.
    pub header: String,
    pub body: Vec<HunkLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// From the `+++` side, which names where the content ends up.
    pub path: String,
    pub creating: bool,
    pub deleting: bool,
    pub hunks: Vec<Hunk>,
}

/// Strips the `a/` and `b/` git puts on a path, and unquotes a quoted one.
/// `None` is `/dev/null`.
fn clean_path(raw: &str) -> Option<String> {
    let mut text = raw.trim();
    // git quotes a path containing a space or a non-ASCII character.
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        text = &text[1..text.len() - 1];
    }
    // A tab separates the path from a timestamp in a diff made by `diff -u`.
    let text = text.split('\t').next().unwrap_or_default().trim();
    if text == "/dev/null" {
        return None;
    }
    let stripped = text
        .strip_prefix("a/")
        .or_else(|| text.strip_prefix("b/"))
        .unwrap_or(text);
    Some(stripped.to_string())
}

/// `@@ -old[,count] +new[,count] @@` -> the two counts. An omitted count means
/// one line, which is what `@@ -3 +3 @@` says.
fn parse_header(line: &str) -> Option<(usize, usize)> {
    let rest = line.trim_start_matches('@').trim_start();
    let mut parts = rest.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    if !parts.next()?.starts_with("@@") {
        return None;
    }
    let count = |range: &str| -> Option<usize> {
        let mut pieces = range.splitn(2, ',');
        pieces.next()?.parse::<usize>().ok()?;
        match pieces.next() {
            Some(n) => n.parse().ok(),
            None => Some(1),
        }
    };
    Some((count(old)?, count(new)?))
}

/// Reads a unified diff into a list of files, each with a list of hunks.
///
/// Hunk bodies are bounded by the counts in the `@@` header rather than by
/// scanning for the next marker. That is not a micro-optimisation: a context
/// line that happens to be blank arrives as an empty string with no leading
/// space, and a scanner that stops at the first line which does not start with
/// a space, plus or minus stops in the middle of half the real diffs there are.
pub fn parse_diff(text: &str) -> std::result::Result<Vec<FileDiff>, String> {
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut files: Vec<FileDiff> = Vec::new();
    let mut i = 0;

    let fail = |line: usize, why: &str| -> String {
        format!("The diff could not be read at line {}: {why}", line + 1)
    };

    while i < lines.len() {
        let line = lines[i];

        if let Some(before) = line.strip_prefix("--- ") {
            let Some(after) = lines.get(i + 1).and_then(|next| next.strip_prefix("+++ ")) else {
                return Err(fail(i, "a `---` line must be followed by a `+++` line."));
            };
            let before = clean_path(before);
            let after = clean_path(after);
            let (Some(path), creating, deleting) = (match (&before, &after) {
                (None, None) => {
                    return Err(fail(
                        i,
                        "both sides are /dev/null, so there is nothing to do.",
                    ))
                }
                // The `+++` side names where the content ends up, which is the
                // file we write. The `---` side only matters for telling a
                // create from a change.
                (None, Some(after)) => (Some(after.clone()), true, false),
                (Some(before), None) => (Some(before.clone()), false, true),
                (Some(_), Some(after)) => (Some(after.clone()), false, false),
            }) else {
                unreachable!("every arm above yields a path");
            };
            files.push(FileDiff {
                path,
                creating,
                deleting,
                hunks: Vec::new(),
            });
            i += 2;
            continue;
        }

        if line.starts_with("@@") {
            let Some(current) = files.last_mut() else {
                return Err(fail(
                    i,
                    "a hunk appeared before any `--- ` / `+++ ` file header.",
                ));
            };
            let Some((old_count, new_count)) = parse_header(line) else {
                return Err(fail(
                    i,
                    &format!("`{}` is not a hunk header.", head(line, 60)),
                ));
            };

            let mut body = Vec::new();
            let mut old_seen = 0;
            let mut new_seen = 0;
            i += 1;
            while i < lines.len() && (old_seen < old_count || new_seen < new_count) {
                let raw = lines[i];
                // "\ No newline at end of file" is a note about the line above,
                // not a line of its own, and it counts towards neither side.
                if raw.starts_with('\\') {
                    i += 1;
                    continue;
                }
                let (sign, content) = match raw.chars().next() {
                    None => (' ', ""),
                    Some(first) => (first, &raw[first.len_utf8()..]),
                };
                match sign {
                    ' ' => {
                        old_seen += 1;
                        new_seen += 1;
                    }
                    '-' => old_seen += 1,
                    '+' => new_seen += 1,
                    _ => {
                        return Err(fail(
                            i,
                            &format!(
                                "`{}` is not a diff line. Every line in a hunk starts with a \
                                 space, - or +.",
                                head(raw, 60)
                            ),
                        ))
                    }
                }
                body.push(HunkLine {
                    sign,
                    content: content.to_string(),
                });
                i += 1;
            }
            if old_seen != old_count || new_seen != new_count {
                return Err(fail(
                    i.saturating_sub(1),
                    &format!(
                        "the hunk header promised {old_count} old and {new_count} new lines but \
                         the body has {old_seen} and {new_seen}. Count the lines again, or leave \
                         the counts off."
                    ),
                ));
            }
            current.hunks.push(Hunk {
                header: line.to_string(),
                body,
            });
            continue;
        }

        // `diff --git`, `index`, mode lines, and whatever else git puts between
        // files. None of it changes any content, so none of it is read.
        i += 1;
    }

    if files.is_empty() {
        return Err(
            "No file headers were found. A unified diff needs `--- a/path` and \
                    `+++ b/path` lines above each set of hunks."
                .into(),
        );
    }
    for file in &files {
        if file.hunks.is_empty() && !file.deleting {
            return Err(format!(
                "{} has a file header but no hunks, so there is nothing to apply to it.",
                file.path
            ));
        }
    }
    Ok(files)
}

/// The first `n` characters, for quoting a line back without pasting a
/// minified bundle into the error.
fn head(line: &str, n: usize) -> String {
    line.chars().take(n).collect()
}

/// The text a hunk expects to find, and the text it puts in its place.
fn hunk_texts(hunk: &Hunk) -> (String, String) {
    let side = |skip: char| -> String {
        hunk.body
            .iter()
            .filter(|line| line.sign != skip)
            .map(|line| line.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    (side('+'), side('-'))
}

// ── applying ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub content: String,
    /// One entry per hunk that changed something: which rung of the ladder
    /// found it, or `created` / `no-op`.
    pub strategies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkFailure {
    /// Zero-based index of the hunk that did not apply.
    pub hunk: usize,
    pub header: String,
    pub why: String,
}

/// Applies every hunk for one file, in memory.
///
/// Hunks are applied top to bottom, each against the result of the last, which
/// is what makes two changes in the same function work: the second hunk's
/// context lines are the ones the first hunk left behind.
pub fn apply_hunks(
    content: &str,
    hunks: &[Hunk],
    creating: bool,
) -> std::result::Result<Applied, HunkFailure> {
    // Creating a file has nothing to search: the hunks are the whole content,
    // and running them through the matcher would refuse them for having no
    // context to anchor to. A trailing newline is added because a source file
    // has one and the diff has no way to express it.
    if creating {
        let text = hunks
            .iter()
            .map(|hunk| hunk_texts(hunk).1)
            .collect::<Vec<_>>()
            .join("\n");
        let content = if text.is_empty() || text.ends_with('\n') {
            text
        } else {
            format!("{text}\n")
        };
        return Ok(Applied {
            content,
            strategies: vec!["created".into()],
        });
    }

    let mut out = content.to_string();
    let mut strategies = Vec::new();

    for (index, hunk) in hunks.iter().enumerate() {
        let (before, after) = hunk_texts(hunk);

        if before == after {
            // A hunk of pure context changes nothing. Some generators emit one,
            // and refusing it would fail a patch that is perfectly correct.
            strategies.push("no-op".into());
            continue;
        }

        if before.is_empty() {
            // Nothing to find, so nothing to anchor to. `replace` refuses this
            // for the same reason, and the model needs to be told what to do
            // instead.
            return Err(HunkFailure {
                hunk: index,
                header: hunk.header.clone(),
                why: "the hunk only adds lines and gives no context, so there is nowhere to put \
                      them. Include the surrounding unchanged lines."
                    .into(),
            });
        }

        match replace(&out, &before, &after, false) {
            Ok(result) => {
                out = result.content;
                strategies.push(result.strategy.as_str().to_string());
            }
            Err(why) => {
                return Err(HunkFailure {
                    hunk: index,
                    header: hunk.header.clone(),
                    why,
                })
            }
        }
    }

    Ok(Applied {
        content: out,
        strategies,
    })
}

// ── the diff handed to the card ─────────────────────────────────────────

fn to_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// The edit script between two line lists, as `(sign, text)` where sign is
/// `' '`, `'-'` or `'+'`.
///
/// Common head and tail are peeled off first. That is not just an
/// optimisation: it is what keeps the table small enough to build for the
/// usual case, where a big file changed in one place.
fn edit_script<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(char, &'a str)> {
    let mut ops = Vec::new();

    let mut head = 0;
    while head < a.len() && head < b.len() && a[head] == b[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < a.len() - head
        && tail < b.len() - head
        && a[a.len() - 1 - tail] == b[b.len() - 1 - tail]
    {
        tail += 1;
    }

    ops.extend(a[..head].iter().map(|line| (' ', *line)));

    let mid_a = &a[head..a.len() - tail];
    let mid_b = &b[head..b.len() - tail];

    if (mid_a.len() + 1) * (mid_b.len() + 1) > MAX_LCS_CELLS {
        ops.extend(mid_a.iter().map(|line| ('-', *line)));
        ops.extend(mid_b.iter().map(|line| ('+', *line)));
    } else {
        let n = mid_a.len();
        let m = mid_b.len();
        let width = m + 1;
        let mut table = vec![0u32; (n + 1) * width];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                table[i * width + j] = if mid_a[i] == mid_b[j] {
                    table[(i + 1) * width + j + 1] + 1
                } else {
                    table[(i + 1) * width + j].max(table[i * width + j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if mid_a[i] == mid_b[j] {
                ops.push((' ', mid_a[i]));
                i += 1;
                j += 1;
            } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
                ops.push(('-', mid_a[i]));
                i += 1;
            } else {
                ops.push(('+', mid_b[j]));
                j += 1;
            }
        }
        ops.extend(mid_a[i..].iter().map(|line| ('-', *line)));
        ops.extend(mid_b[j..].iter().map(|line| ('+', *line)));
    }

    ops.extend(a[a.len() - tail..].iter().map(|line| (' ', *line)));
    ops
}

/// A standard unified diff. Empty when nothing changed, so callers can treat
/// "no diff" as "no change" without comparing the texts again.
pub fn unified_diff(old_text: &str, new_text: &str, file_path: &str) -> String {
    let a = to_lines(old_text);
    let b = to_lines(new_text);
    let ops = edit_script(&a, &b);
    if ops.iter().all(|(sign, _)| *sign == ' ') {
        return String::new();
    }

    // Which ops are near enough to a change to be worth printing.
    let mut keep = vec![false; ops.len()];
    for (i, (sign, _)) in ops.iter().enumerate() {
        if *sign == ' ' {
            continue;
        }
        let from = i.saturating_sub(CONTEXT);
        let to = (i + CONTEXT).min(ops.len() - 1);
        for slot in &mut keep[from..=to] {
            *slot = true;
        }
    }

    let mut out = vec![format!("--- a/{file_path}"), format!("+++ b/{file_path}")];
    let (mut old_line, mut new_line) = (1usize, 1usize);
    let mut i = 0;
    while i < ops.len() {
        if !keep[i] {
            if ops[i].0 != '+' {
                old_line += 1;
            }
            if ops[i].0 != '-' {
                new_line += 1;
            }
            i += 1;
            continue;
        }
        let (start_old, start_new) = (old_line, new_line);
        let mut body = Vec::new();
        let (mut old_count, mut new_count) = (0, 0);
        while i < ops.len() && keep[i] {
            let (sign, text) = ops[i];
            body.push(format!("{sign}{text}"));
            if sign != '+' {
                old_line += 1;
                old_count += 1;
            }
            if sign != '-' {
                new_line += 1;
                new_count += 1;
            }
            i += 1;
        }
        out.push(format!(
            "@@ -{start_old},{old_count} +{start_new},{new_count} @@"
        ));
        out.extend(body);
    }
    out.join("\n")
}

// ── the tool ────────────────────────────────────────────────────────────

const DESCRIPTION: &str = "Apply a unified diff across one or more files in a single call. Use this instead of a run of `edit` calls whenever a change touches more than one file, or more than one place in the same file.

The whole patch applies or none of it does. If any hunk fails, nothing is written and you are told which hunk failed and why.

Format - a standard unified diff:

    --- a/src/one.js
    +++ b/src/one.js
    @@ -10,6 +10,7 @@
     function greet(name) {
    -  return \"hi \" + name;
    +  return `hi ${name}`;
     }
    --- a/src/two.js
    +++ b/src/two.js
    @@ -1,3 +1,3 @@
    -const one = require(\"./one\");
    +const { greet } = require(\"./one\");

Rules:
- Every file needs a `--- a/path` line and a `+++ b/path` line. Paths may be absolute or relative to the working directory. The `a/` and `b/` prefixes are optional.
- Every hunk needs an `@@ -old,count +new,count @@` header. The line numbers are not trusted - hunks are located by their context lines - but the two counts must match the number of lines in the hunk body or the patch is refused.
- Lines in a hunk start with a space (unchanged), `-` (removed) or `+` (added). Include at least two or three unchanged lines either side of a change so the hunk can be found unambiguously.
- To create a file, use `--- /dev/null` as the first header and give one hunk that is all `+` lines.
- Deleting a file is not supported. Use the shell for that.
- Every file you change must have been read in this conversation first, exactly as with `edit`, and must not have changed on disk since.";

fn modified_time(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// Temp-then-rename, so a crash mid-write leaves the old file intact rather
/// than a truncated one. Mirrors the one in `files.rs`, which is private there.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!(
        "{}.{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
        std::process::id()
    ));

    let write = || -> std::io::Result<()> {
        let mut handle = std::fs::File::create(&temp)?;
        handle.write_all(contents.as_bytes())?;
        handle.flush()?;
        let _ = handle.sync_all();
        Ok(())
    };

    if let Err(e) = write() {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

/// One file the patch will write, decided before anything is written.
struct Planned {
    abs: PathBuf,
    /// What was on disk, byte for byte, for putting back if a later write
    /// fails. `None` for a file that did not exist.
    original: Option<String>,
    text: String,
    created: bool,
    diff: String,
    strategies: Vec<String>,
}

#[derive(Debug)]
pub struct PatchTool {
    state: Arc<ReadState>,
    /// Shared with `read` and `edit`: a patch is several edits, and what it
    /// broke is the same question asked once at the end.
    lsp: Arc<inertia_lsp::Lsp>,
}

impl PatchTool {
    pub fn new(state: Arc<ReadState>, lsp: Arc<inertia_lsp::Lsp>) -> Self {
        Self { state, lsp }
    }
}

#[async_trait]
impl Tool for PatchTool {
    fn id(&self) -> &str {
        "patch"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "diff": { "type": "string", "description": "The unified diff to apply." }
            },
            "required": ["diff"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    /// One question for the patch, and then one per file inside `execute`.
    ///
    /// The target here is deliberately `*`: the paths are inside the diff, and
    /// digging them out to build a target would mean parsing it twice and would
    /// still produce a string like "three paths joined by commas" that no rule
    /// anybody writes would ever match. The per-file asks below carry the real
    /// paths, so a rule written about one file still fires, and a session that
    /// remembers "always allow edit" answers all of them at once.
    fn permission(&self, _args: &Value) -> PermissionRequest {
        let any = inertia_core::permission::ANY;
        PermissionRequest::new("edit", any).with_always(any)
    }

    fn render(&self, args: &Value) -> Option<String> {
        let count = args
            .get("diff")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with("+++ "))
            .count();
        Some(if count == 1 {
            "1 file".to_string()
        } else {
            format!("{count} files")
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let diff = args
            .get("diff")
            .and_then(Value::as_str)
            .ok_or_else(|| refuse("diff is required."))?;
        let files = parse_diff(diff).map_err(refuse)?;

        if let Some(deleting) = files.iter().find(|file| file.deleting) {
            return Err(refuse(format!(
                "This patch deletes {}, which patch will not do. Remove that file from the diff \
                 and delete it with the shell if it really should go.",
                deleting.path
            )));
        }

        let mut seen = std::collections::HashSet::new();
        for file in &files {
            if !seen.insert(file.path.as_str()) {
                return Err(refuse(format!(
                    "{} appears twice in the diff. Put all of its hunks under one file header, \
                     in order.",
                    file.path
                )));
            }
        }

        // ── read everything, decide everything, write nothing ───────────

        let mut planned: Vec<Planned> = Vec::new();
        let mut failures: Vec<String> = Vec::new();

        for file in &files {
            // The same resolution `read`, `write` and `edit` use, so a path
            // `read` recorded is the path looked up here.
            let abs = fence::reach(ctx, &file.path).await?;
            let shown = abs.display().to_string();

            // The permission descriptor above asked once about the patch as a
            // whole; this is the question a rule about a particular file is
            // written about.
            let per_file = PermissionRequest::new("edit", shown.clone()).with_always(shown.clone());
            if !ctx.permissions.ask(&per_file).await?.is_allowed() {
                return Err(Error::Denied(format!("Patch {shown}")));
            }

            let stat = std::fs::metadata(&abs).ok();

            if file.creating {
                if stat.is_some() {
                    failures.push(format!(
                        "{shown}: the diff creates this file but it already exists. Patch it instead."
                    ));
                    continue;
                }
            } else {
                let Some(stat) = &stat else {
                    failures.push(format!("{shown}: file not found."));
                    continue;
                };
                if stat.is_dir() {
                    failures.push(format!("{shown} is a directory, not a file."));
                    continue;
                }
                if !self.state.was_read(&ctx.session, &abs) {
                    failures.push(format!(
                        "{shown}: not read in this conversation. Read it first so the patch is \
                         based on what it actually contains."
                    ));
                    continue;
                }
                if self.state.is_stale(&ctx.session, &abs, modified_time(&abs)) {
                    failures.push(format!(
                        "{shown}: changed on disk since it was read. Read it again before patching."
                    ));
                    continue;
                }
            }

            let raw = match &stat {
                Some(_) => match std::fs::read_to_string(&abs) {
                    Ok(text) => text,
                    Err(e) => {
                        failures.push(format!("{shown}: could not be read as text ({e})."));
                        continue;
                    }
                },
                None => String::new(),
            };
            // Same reasoning as `edit`: the ladder in replace.rs compares
            // lines, and a \r on the end of every one of them defeats all of
            // it, because the model was shown LF. Search in LF and put the
            // file's own endings back.
            let had_bom = raw.starts_with(BOM);
            let body = raw.trim_start_matches(BOM);
            let crlf = body.contains("\r\n");
            let content = if crlf {
                body.replace("\r\n", "\n")
            } else {
                body.to_string()
            };

            let applied = match apply_hunks(&content, &file.hunks, file.creating) {
                Ok(applied) => applied,
                Err(failure) => {
                    failures.push(format!(
                        "{shown}: hunk {} of {} (`{}`) did not apply - {}",
                        failure.hunk + 1,
                        file.hunks.len(),
                        failure.header,
                        failure.why
                    ));
                    continue;
                }
            };
            if applied.content == content {
                failures.push(format!(
                    "{shown}: the hunks changed nothing. The file may already be in the state you \
                     wanted."
                ));
                continue;
            }

            let text = if crlf {
                applied.content.replace('\n', "\r\n")
            } else {
                applied.content.clone()
            };
            planned.push(Planned {
                created: stat.is_none(),
                original: stat.as_ref().map(|_| raw.clone()),
                text: if had_bom {
                    format!("{BOM}{text}")
                } else {
                    text
                },
                diff: unified_diff(&content, &applied.content, &shown),
                strategies: applied.strategies,
                abs,
            });
        }

        if !failures.is_empty() {
            return Err(refuse(format!(
                "Nothing was written. {} of {} file(s) in this patch could not be applied:\n\n{}\n\n\
                 Fix the diff and send the whole thing again.",
                failures.len(),
                files.len(),
                failures
                    .iter()
                    .map(|line| format!("- {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )));
        }

        // ── write, and put it back if a write fails ─────────────────────

        let mut written: Vec<&Planned> = Vec::new();
        for entry in &planned {
            if let Err(error) = write_atomically(&entry.abs, &entry.text) {
                // The in-memory pass caught everything a diff can get wrong,
                // so reaching here means the filesystem refused - a read-only
                // file, a full disk. The promise this tool makes is all or
                // nothing, so the ones that landed are put back before the
                // failure is reported.
                let mut restore_failures = Vec::new();
                for done in written.iter().rev() {
                    let undo = match &done.original {
                        None => std::fs::remove_file(&done.abs),
                        Some(original) => write_atomically(&done.abs, original),
                    };
                    if let Err(undo_error) = undo {
                        restore_failures.push(format!("{}: {undo_error}", done.abs.display()));
                    }
                }
                return Err(refuse(format!(
                    "Writing {} failed: {error}.{}",
                    entry.abs.display(),
                    if restore_failures.is_empty() {
                        " Nothing was left changed.".to_string()
                    } else {
                        format!(
                            " The earlier files could not all be put back, so the tree is now \
                             partly patched: {}",
                            restore_failures.join("; ")
                        )
                    }
                )));
            }
            written.push(entry);
        }

        // ── after ───────────────────────────────────────────────────────

        // The files are newer than the timestamps recorded at read time, and a
        // stale mark here would refuse the model's own next edit.
        for entry in &planned {
            self.state
                .mark_read(&ctx.session, &entry.abs, modified_time(&entry.abs));
        }

        let mut lines = vec![format!("Applied {} file(s):", planned.len())];
        lines.extend(planned.iter().map(|entry| {
            format!(
                "- {} ({})",
                entry.abs.display(),
                if entry.created {
                    "created".to_string()
                } else {
                    format!("{} hunk(s)", entry.strategies.len())
                }
            )
        }));

        // What the patch broke. A patch touches several files at once, which
        // is exactly when the compiler's opinion is worth having before the
        // model moves on - and exactly when it is least likely to think of
        // asking.
        for entry in &planned {
            let broke = self.lsp.report(&entry.abs, Some(&ctx.root), 0).await;
            if !broke.is_empty() {
                lines.push(broke);
            }
        }

        let title = if planned.len() == 1 {
            planned[0]
                .abs
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| planned[0].abs.display().to_string())
        } else {
            format!("{} files", planned.len())
        };

        Ok(ToolOutcome {
            title: Some(title),
            output: lines.join("\n"),
            metadata: Some(json!({
                "files": planned.iter().map(|entry| json!({
                    "path": entry.abs.display().to_string(),
                    "created": entry.created,
                    "hunks": entry.strategies.len(),
                    "diff": entry.diff,
                })).collect::<Vec<_>>(),
            })),
            images: Vec::new(),
        })
    }
}

/// The tool, sharing the read-tracker with `read`, `write` and `edit`.
pub fn patch_tool(state: Arc<ReadState>, lsp: Arc<inertia_lsp::Lsp>) -> Arc<dyn Tool> {
    Arc::new(PatchTool::new(state, lsp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::files::ReadTool;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_mock::MockGate;

    struct Bench {
        dir: tempfile::TempDir,
        state: Arc<ReadState>,
        gate: Arc<MockGate>,
    }

    impl Bench {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().expect("a temp dir"),
                state: Arc::new(ReadState::new()),
                gate: Arc::new(MockGate::allow_all()),
            }
        }

        fn ctx(&self) -> ToolContext {
            ToolContext {
                root: self.dir.path().to_path_buf(),
                session: SessionId::from_existing("test"),
                call_id: ToolCallId::from_existing("tc_1"),
                permissions: self.gate.clone(),
            }
        }

        fn tool(&self) -> PatchTool {
            PatchTool::new(
                self.state.clone(),
                Arc::new(inertia_lsp::Lsp::with_launcher(
                    inertia_lsp::testing::fake_launcher(),
                )),
            )
        }

        /// Writes a file and reads it, the way a session would.
        ///
        /// The read goes through the real tool rather than poking at the
        /// state directly, because that is the only thing that registers the
        /// file under the same key patch itself looks up.
        async fn make(&self, name: &str, contents: &str) -> PathBuf {
            let file = self.dir.path().join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, contents).unwrap();
            ReadTool::new(
                self.state.clone(),
                Arc::new(inertia_lsp::Lsp::with_launcher(
                    inertia_lsp::testing::fake_launcher(),
                )),
            )
            .execute(
                json!({ "filePath": file.display().to_string() }),
                &self.ctx(),
            )
            .await
            .expect("the read ran");
            file
        }

        async fn patch(&self, lines: &[String]) -> Result<ToolOutcome> {
            self.tool()
                .execute(json!({ "diff": lines.join("\n") }), &self.ctx())
                .await
        }
    }

    fn lines(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn header(path: &Path) -> [String; 2] {
        [
            format!("--- a/{}", path.display()),
            format!("+++ b/{}", path.display()),
        ]
    }

    // ── parsing ─────────────────────────────────────────────────────────

    #[test]
    fn reads_a_two_file_diff() {
        let files = parse_diff(
            &[
                "diff --git a/one.js b/one.js",
                "index 1234567..89abcde 100644",
                "--- a/one.js",
                "+++ b/one.js",
                "@@ -1,3 +1,3 @@",
                " const a = 1;",
                "-const b = 2;",
                "+const b = 3;",
                " const c = 4;",
                "--- a/two.js",
                "+++ b/two.js",
                "@@ -1,2 +1,2 @@",
                "-old",
                "+new",
                " tail",
            ]
            .join("\n"),
        )
        .unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "one.js");
        assert_eq!(files[0].hunks.len(), 1);
        assert_eq!(files[1].path, "two.js");
    }

    #[test]
    fn keeps_a_blank_context_line_rather_than_stopping_at_it() {
        let files = parse_diff(
            &[
                "--- a/x.js",
                "+++ b/x.js",
                "@@ -1,4 +1,4 @@",
                " one",
                "",
                "-two",
                "+TWO",
                "",
            ]
            .join("\n"),
        )
        .unwrap();
        // Four old lines and four new, spread over five body lines: the blank
        // line at index 1 is a context line, counted on both sides.
        let body = &files[0].hunks[0].body;
        assert_eq!(body.len(), 5);
        assert_eq!(
            body[1],
            HunkLine {
                sign: ' ',
                content: String::new()
            }
        );
    }

    #[test]
    fn treats_a_missing_count_as_one_line() {
        let files =
            parse_diff(&["--- a/x", "+++ b/x", "@@ -3 +3 @@", "-a", "+b"].join("\n")).unwrap();
        assert_eq!(files[0].hunks[0].body.len(), 2);
    }

    #[test]
    fn notices_dev_null_as_a_create() {
        let files =
            parse_diff(&["--- /dev/null", "+++ b/new.js", "@@ -0,0 +1,1 @@", "+hello"].join("\n"))
                .unwrap();
        assert!(files[0].creating);
        assert_eq!(files[0].path, "new.js");
    }

    #[test]
    fn refuses_a_diff_with_no_file_headers() {
        let err = parse_diff("@@ -1,1 +1,1 @@\n-a\n+b").unwrap_err();
        assert!(err.contains("file header"), "{err}");
    }

    #[test]
    fn refuses_a_hunk_whose_counts_do_not_match_its_body() {
        let err =
            parse_diff(&["--- a/x", "+++ b/x", "@@ -1,9 +1,9 @@", " a"].join("\n")).unwrap_err();
        assert!(err.contains("promised"), "{err}");
    }

    #[test]
    fn a_quoted_path_with_a_timestamp_is_cleaned() {
        assert_eq!(clean_path("\"a/my file.js\""), Some("my file.js".into()));
        assert_eq!(
            clean_path("b/x.js\t2026-01-01 00:00:00"),
            Some("x.js".into())
        );
        assert_eq!(clean_path("/dev/null"), None);
    }

    // ── applying in memory ──────────────────────────────────────────────

    const FILE: &str = "one\ntwo\nthree\nfour\n";

    fn hunks(diff: &[&str]) -> Vec<Hunk> {
        parse_diff(&diff.join("\n")).unwrap().remove(0).hunks
    }

    #[test]
    fn applies_a_hunk_found_by_its_context() {
        let parsed = hunks(&[
            "--- a/x",
            "+++ b/x",
            "@@ -1,3 +1,3 @@",
            " one",
            "-two",
            "+TWO",
            " three",
        ]);
        let result = apply_hunks(FILE, &parsed, false).unwrap();
        assert_eq!(result.content, "one\nTWO\nthree\nfour\n");
    }

    #[test]
    fn ignores_the_line_numbers_in_the_header() {
        // The header claims line 400. The context says otherwise, and the
        // context is the part the model actually copied.
        let parsed = hunks(&[
            "--- a/x",
            "+++ b/x",
            "@@ -400,3 +400,3 @@",
            " one",
            "-two",
            "+TWO",
            " three",
        ]);
        assert_eq!(
            apply_hunks(FILE, &parsed, false).unwrap().content,
            "one\nTWO\nthree\nfour\n"
        );
    }

    #[test]
    fn applies_two_hunks_in_order() {
        let parsed = hunks(&[
            "--- a/x",
            "+++ b/x",
            "@@ -1,2 +1,2 @@",
            " one",
            "-two",
            "+TWO",
            "@@ -3,2 +3,2 @@",
            " three",
            "-four",
            "+FOUR",
        ]);
        let result = apply_hunks(FILE, &parsed, false).unwrap();
        assert_eq!(result.content, "one\nTWO\nthree\nFOUR\n");
        assert_eq!(result.strategies.len(), 2);
    }

    #[test]
    fn reports_which_hunk_failed() {
        let parsed = hunks(&[
            "--- a/x",
            "+++ b/x",
            "@@ -1,2 +1,2 @@",
            " one",
            "-two",
            "+TWO",
            "@@ -3,2 +3,2 @@",
            " nothing",
            "-like",
            "+this",
        ]);
        let failure = apply_hunks(FILE, &parsed, false).unwrap_err();
        assert_eq!(failure.hunk, 1);
        assert!(failure.why.contains("Could not find"), "{}", failure.why);
    }

    #[test]
    fn refuses_a_hunk_with_no_context_to_anchor_to() {
        let parsed = hunks(&["--- a/x", "+++ b/x", "@@ -1,0 +1,1 @@", "+floating"]);
        let failure = apply_hunks(FILE, &parsed, false).unwrap_err();
        assert!(failure.why.contains("no context"), "{}", failure.why);
    }

    #[test]
    fn builds_the_whole_content_when_creating_a_file() {
        let parsed = hunks(&[
            "--- /dev/null",
            "+++ b/new.js",
            "@@ -0,0 +1,2 @@",
            "+one",
            "+two",
        ]);
        let result = apply_hunks("", &parsed, true).unwrap();
        assert_eq!(result.content, "one\ntwo\n");
        assert_eq!(result.strategies, vec!["created".to_string()]);
    }

    // ── the diff for the card ───────────────────────────────────────────

    #[test]
    fn a_unified_diff_has_a_header_the_card_can_parse() {
        let diff = unified_diff("a\nb\nc\n", "a\nB\nc\nd\n", "x.txt");
        let header = diff.lines().find(|line| line.starts_with("@@")).unwrap();
        assert_eq!(header, "@@ -1,3 +1,4 @@");
        assert!(diff.contains("--- a/x.txt"));
        assert!(diff.contains("+++ b/x.txt"));
        assert!(diff.contains("\n-b\n"));
        assert!(diff.contains("\n+B\n"));
        assert!(diff.contains("\n+d"));
        assert!(diff.contains("\n a\n"));
    }

    #[test]
    fn a_unified_diff_is_empty_when_nothing_changed() {
        assert_eq!(unified_diff("same\n", "same\n", "x.txt"), "");
    }

    #[test]
    fn far_apart_changes_become_separate_hunks() {
        let old: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        let new = old
            .replace("line 2\n", "LINE 2\n")
            .replace("line 19\n", "LINE 19\n");
        let diff = unified_diff(&old, &new, "x");
        assert_eq!(
            diff.lines().filter(|line| line.starts_with("@@")).count(),
            2
        );
    }

    // ── the tool ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn changes_several_files_in_one_call() {
        let bench = Bench::new();
        let one = bench
            .make("multi/one.js", "const a = 1;\nconst b = 2;\nconst c = 3;\n")
            .await;
        let two = bench
            .make("multi/two.js", "let x = 10;\nlet y = 20;\n")
            .await;

        let [one_a, one_b] = header(&one);
        let [two_a, two_b] = header(&two);
        let out = bench
            .patch(&lines(&[
                &one_a,
                &one_b,
                "@@ -1,3 +1,3 @@",
                " const a = 1;",
                "-const b = 2;",
                "+const b = 22;",
                " const c = 3;",
                &two_a,
                &two_b,
                "@@ -1,2 +1,2 @@",
                " let x = 10;",
                "-let y = 20;",
                "+let y = 200;",
            ]))
            .await
            .expect("the patch applied");

        assert_eq!(
            std::fs::read_to_string(&one).unwrap(),
            "const a = 1;\nconst b = 22;\nconst c = 3;\n"
        );
        assert_eq!(
            std::fs::read_to_string(&two).unwrap(),
            "let x = 10;\nlet y = 200;\n"
        );

        assert_eq!(out.title.as_deref(), Some("2 files"));
        assert!(
            out.output.starts_with("Applied 2 file(s):\n- "),
            "{}",
            out.output
        );
        assert!(out.output.contains("(1 hunk(s))"), "{}", out.output);

        // The shape the card and the thread's file list read.
        let files = out.metadata.unwrap()["files"].as_array().unwrap().clone();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["path"], json!(one.display().to_string()));
        assert_eq!(files[0]["created"], json!(false));
        assert_eq!(files[0]["hunks"], json!(1));
        let diff = files[0]["diff"].as_str().unwrap();
        assert!(diff.contains("-const b = 2;"), "{diff}");
        assert!(diff.contains("+const b = 22;"), "{diff}");
    }

    #[tokio::test]
    async fn two_hunks_in_one_file_land_together() {
        let bench = Bench::new();
        let file = bench.make("both.txt", FILE).await;
        let [a, b] = header(&file);
        let out = bench
            .patch(&lines(&[
                &a,
                &b,
                "@@ -1,2 +1,2 @@",
                " one",
                "-two",
                "+TWO",
                "@@ -3,2 +3,2 @@",
                " three",
                "-four",
                "+FOUR",
            ]))
            .await
            .expect("the patch applied");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "one\nTWO\nthree\nFOUR\n"
        );
        assert_eq!(out.title.as_deref(), Some("both.txt"));
        assert!(out.output.contains("(2 hunk(s))"), "{}", out.output);
    }

    #[tokio::test]
    async fn writes_nothing_at_all_when_one_hunk_in_one_file_fails() {
        let bench = Bench::new();
        let one = bench.make("atomic/one.js", "alpha\nbravo\ncharlie\n").await;
        let two = bench.make("atomic/two.js", "delta\necho\n").await;

        let [one_a, one_b] = header(&one);
        let [two_a, two_b] = header(&two);
        let err = bench
            .patch(&lines(&[
                &one_a,
                &one_b,
                "@@ -1,3 +1,3 @@",
                " alpha",
                "-bravo",
                "+BRAVO",
                " charlie",
                &two_a,
                &two_b,
                "@@ -1,2 +1,2 @@",
                " nowhere",
                "-in the file",
                "+at all",
            ]))
            .await
            .expect_err("one hunk is stale");

        let text = err.to_string();
        assert!(
            text.starts_with("Nothing was written. 1 of 2 file(s)"),
            "{text}"
        );
        assert!(text.contains(&two.display().to_string()), "{text}");
        assert!(
            text.contains("hunk 1 of 1 (`@@ -1,2 +1,2 @@`) did not apply - Could not find"),
            "{text}"
        );
        assert!(
            text.ends_with("Fix the diff and send the whole thing again."),
            "{text}"
        );
        // The file whose hunks were perfectly fine is untouched too. That is
        // the whole promise of the tool.
        assert_eq!(
            std::fs::read_to_string(&one).unwrap(),
            "alpha\nbravo\ncharlie\n"
        );
        assert_eq!(std::fs::read_to_string(&two).unwrap(), "delta\necho\n");
    }

    #[tokio::test]
    async fn creates_a_file_from_a_dev_null_diff() {
        let bench = Bench::new();
        let target = bench.dir.path().join("created").join("fresh.js");

        let out = bench
            .patch(&lines(&[
                "--- /dev/null",
                &format!("+++ b/{}", target.display()),
                "@@ -0,0 +1,3 @@",
                "+export function hello() {",
                "+  return \"hi\";",
                "+}",
            ]))
            .await
            .expect("the file was created");

        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "export function hello() {\n  return \"hi\";\n}\n"
        );
        assert!(out.output.contains("(created)"), "{}", out.output);
        assert_eq!(out.metadata.unwrap()["files"][0]["created"], json!(true));
        // And it is editable straight away, as a file `write` made is.
        assert!(bench.state.was_read(&bench.ctx().session, &target));
    }

    #[tokio::test]
    async fn refuses_to_create_a_file_that_already_exists() {
        let bench = Bench::new();
        let file = bench.make("taken.js", "one\n").await;
        let err = bench
            .patch(&lines(&[
                "--- /dev/null",
                &format!("+++ b/{}", file.display()),
                "@@ -0,0 +1,1 @@",
                "+two",
            ]))
            .await
            .expect_err("it exists");
        assert!(
            err.to_string()
                .contains("already exists. Patch it instead."),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\n");
    }

    #[tokio::test]
    async fn refuses_to_patch_a_file_that_was_never_read() {
        let bench = Bench::new();
        let file = bench.dir.path().join("unread.js");
        std::fs::write(&file, "one\ntwo\n").unwrap();

        let [a, b] = header(&file);
        let err = bench
            .patch(&lines(&[&a, &b, "@@ -1,2 +1,2 @@", " one", "-two", "+TWO"]))
            .await
            .expect_err("never read");
        assert!(
            err.to_string().contains("not read in this conversation"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\n");
    }

    #[tokio::test]
    async fn refuses_to_patch_a_file_that_changed_since_it_was_read() {
        let bench = Bench::new();
        let file = bench.make("stale.js", "one\ntwo\n").await;
        std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
        // Filesystems differ on mtime resolution, and a write in the same
        // millisecond as the read would not read as newer. This stands in for
        // somebody else's editor saving a second later.
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();

        let [a, b] = header(&file);
        let err = bench
            .patch(&lines(&[&a, &b, "@@ -1,2 +1,2 @@", " one", "-two", "+TWO"]))
            .await
            .expect_err("changed on disk");
        assert!(err.to_string().contains("changed on disk"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\nthree\n");
    }

    #[tokio::test]
    async fn refuses_a_diff_that_deletes_a_file() {
        let bench = Bench::new();
        let file = bench.make("doomed.js", "one\n").await;
        let err = bench
            .patch(&lines(&[
                &format!("--- a/{}", file.display()),
                "+++ /dev/null",
                "@@ -1,1 +0,0 @@",
                "-one",
            ]))
            .await
            .expect_err("deleting is refused");
        assert!(err.to_string().contains("will not do"), "{err}");
        assert!(file.exists());
    }

    #[tokio::test]
    async fn refuses_a_file_that_is_listed_twice() {
        let bench = Bench::new();
        let file = bench.make("twice.js", "one\ntwo\n").await;
        let [a, b] = header(&file);
        let once = lines(&[&a, &b, "@@ -1,1 +1,1 @@", "-one", "+ONE"]);
        let mut twice = once.clone();
        twice.extend(once);
        let err = bench.patch(&twice).await.expect_err("listed twice");
        assert!(err.to_string().contains("appears twice"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\n");
    }

    #[tokio::test]
    async fn a_patch_that_changes_nothing_is_refused_rather_than_reported_as_applied() {
        let bench = Bench::new();
        let file = bench.make("noop.js", "one\ntwo\n").await;
        let [a, b] = header(&file);
        let err = bench
            .patch(&lines(&[&a, &b, "@@ -1,2 +1,2 @@", " one", " two"]))
            .await
            .expect_err("nothing changed");
        assert!(
            err.to_string().contains("the hunks changed nothing"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn keeps_crlf_line_endings_and_a_bom() {
        let bench = Bench::new();
        let file = bench
            .make("windows.js", "\u{feff}one\r\ntwo\r\nthree\r\n")
            .await;
        let [a, b] = header(&file);
        bench
            .patch(&lines(&[
                &a,
                &b,
                "@@ -1,3 +1,3 @@",
                " one",
                "-two",
                "+TWO",
                " three",
            ]))
            .await
            .expect("the patch applied");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "\u{feff}one\r\nTWO\r\nthree\r\n"
        );
    }

    #[tokio::test]
    async fn can_patch_again_straight_after_its_own_patch() {
        let bench = Bench::new();
        let file = bench.make("twice.txt", "one\ntwo\n").await;
        let [a, b] = header(&file);
        bench
            .patch(&lines(&[&a, &b, "@@ -1,2 +1,2 @@", "-one", "+1", " two"]))
            .await
            .expect("first");
        bench
            .patch(&lines(&[&a, &b, "@@ -1,2 +1,2 @@", " 1", "-two", "+2"]))
            .await
            .expect("the file is not stale to the tool that just wrote it");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "1\n2\n");
    }

    #[tokio::test]
    async fn asks_about_each_file_under_the_edit_key() {
        let bench = Bench::new();
        let file = bench.make("asked.js", "one\ntwo\n").await;
        let [a, b] = header(&file);
        bench
            .patch(&lines(&[&a, &b, "@@ -1,2 +1,2 @@", " one", "-two", "+TWO"]))
            .await
            .expect("the patch applied");

        let asked = bench.gate.asked();
        // Every ask, including the read that seeded the file, is under a key
        // the settings pane knows, and the patch's own ask names the file.
        assert!(asked.iter().all(|q| q.key == "edit" || q.key == "read"));
        let shown = file.display().to_string();
        assert!(asked.iter().any(|q| q.key == "edit"
            && q.target == shown
            && q.always.as_deref() == Some(shown.as_str())));
    }

    #[tokio::test]
    async fn a_refused_file_stops_the_patch_before_anything_is_written() {
        let bench = Bench::new();
        let file = bench.make("kept.js", "one\ntwo\n").await;
        let gate = Arc::new(MockGate::deny_all());
        let ctx = ToolContext {
            permissions: gate,
            ..bench.ctx()
        };
        let [a, b] = header(&file);
        let err = bench
            .tool()
            .execute(
                json!({ "diff": lines(&[&a, &b, "@@ -1,2 +1,2 @@", " one", "-two", "+TWO"]).join("\n") }),
                &ctx,
            )
            .await
            .expect_err("the person said no");
        assert!(matches!(err, Error::Denied(_)), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn the_descriptor_matches_the_electron_tool() {
        let tool = PatchTool::new(
            Arc::new(ReadState::new()),
            Arc::new(inertia_lsp::Lsp::with_launcher(
                inertia_lsp::testing::fake_launcher(),
            )),
        );
        assert_eq!(tool.id(), "patch");
        let permission = tool.permission(&json!({ "diff": "" }));
        assert_eq!(
            (permission.key.as_str(), permission.target.as_str()),
            ("edit", "*")
        );
        assert_eq!(permission.always.as_deref(), Some("*"));
        assert_eq!(
            tool.render(&json!({ "diff": "+++ b/x\n+++ b/y" }))
                .as_deref(),
            Some("2 files")
        );
        assert_eq!(
            tool.render(&json!({ "diff": "+++ b/x" })).as_deref(),
            Some("1 file")
        );
        assert_eq!(tool.parameters()["required"], json!(["diff"]));
    }
}

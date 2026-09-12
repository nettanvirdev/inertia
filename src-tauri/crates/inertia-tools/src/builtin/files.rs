//! `read`, `write` and `edit`.
//!
//! The three that matter most: almost every task the agent does routes through
//! them, and they are the ones that can destroy work rather than merely waste
//! a turn. The care is concentrated in three places - refusing to read what it
//! cannot usefully read, writing atomically, and refusing to edit a file it
//! has not looked at.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use serde_json::{json, Value};

use super::read_state::ReadState;
use crate::replace::replace;

/// Lines returned when the caller does not say.
const DEFAULT_LIMIT: usize = 2_000;
/// Per-line character cap. A minified bundle on one line must not become the
/// entire response.
const MAX_LINE_LENGTH: usize = 2_000;
/// Overall cap on a read, in bytes.
const MAX_BYTES: usize = 50_000;
/// Images beyond this are refused rather than encoded; the base64 alone would
/// dominate the context window.
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Extensions that are never worth reading as text. Content sniffing catches
/// the rest; this just avoids loading a 2GB archive to discover that.
const BINARY_EXTENSIONS: &[&str] = &[
    "zip", "tar", "gz", "exe", "dll", "so", "class", "jar", "7z", "doc", "docx", "xls", "xlsx",
    "ppt", "pptx", "pdf", "bin", "dat", "o", "a", "lib", "wasm", "pyc", "mp3", "mp4", "mov",
    "woff", "woff2", "ttf",
];

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Resolves a caller-supplied path against the workspace.
///
/// Absolute paths are honoured - the agent legitimately works on projects
/// outside the workspace folder - so this is resolution, not confinement. The
/// permission layer is what decides whether a given path may be touched.
fn resolve(root: &Path, supplied: &str) -> PathBuf {
    let path = Path::new(supplied);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Whether bytes look like something other than text.
///
/// A NUL byte is decisive. Beyond that, a high proportion of control
/// characters means the file may decode as UTF-8 but is not text anyone wants
/// numbered and quoted back at them.
fn looks_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(4_096)];
    if sample.is_empty() {
        return false;
    }
    if sample.contains(&0) {
        return true;
    }
    let control = sample
        .iter()
        .filter(|b| **b < 9 || (**b > 13 && **b < 32))
        .count();
    control * 100 / sample.len() > 30
}

/// Lists near-miss filenames for a path that does not exist.
///
/// A typo costs a whole round trip otherwise, and the model has no way to
/// discover the real name short of listing the directory itself.
fn suggestions(path: &Path) -> Vec<String> {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let wanted = name.to_string_lossy().to_ascii_lowercase();

    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_string_lossy().to_string();
            let lower = name.to_ascii_lowercase();
            (lower.contains(&wanted) || wanted.contains(&lower)).then_some(name)
        })
        .take(3)
        .collect()
}

fn not_found(path: &Path) -> Error {
    let mut message = format!("{} does not exist.", path.display());
    let near = suggestions(path);
    if !near.is_empty() {
        message.push_str(&format!(" Did you mean one of these? {}", near.join(", ")));
    }
    Error::not_found("file", message)
}

fn modified_time(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

// ── read ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ReadTool {
    state: Arc<ReadState>,
    /// The language servers, for the half of this tool that is not about the
    /// file it was given: what the change did to everything else.
    lsp: Arc<inertia_lsp::Lsp>,
}

impl ReadTool {
    pub fn new(state: Arc<ReadState>, lsp: Arc<inertia_lsp::Lsp>) -> Self {
        Self { state, lsp }
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn id(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read a file from disk with line numbers, or list a directory. Reads up to \
         2000 lines at a time; use offset and limit to page through a longer file. \
         Always read a file before editing it. An image file - png, jpg, gif, webp - \
         is shown to you as a picture: this is how to look at a screenshot, a design \
         or a photo on disk. Do not open an image in a browser or on a computer to \
         see it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filePath": { "type": "string", "description": "Path to the file or directory." },
                "offset": { "type": "integer", "description": "First line to read, 1-based." },
                "limit": { "type": "integer", "description": "How many lines to read." }
            },
            "required": ["filePath"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("filePath")
            .and_then(Value::as_str)
            .unwrap_or(inertia_core::permission::ANY);
        // A read grant is remembered as "read anything": reading is
        // low-consequence, and a per-path prompt for every file makes the
        // agent unusable without teaching the user anything.
        PermissionRequest::new("read", target).with_always(inertia_core::permission::ANY)
    }

    fn render(&self, args: &Value) -> Option<String> {
        args.get("filePath")
            .and_then(Value::as_str)
            .map(|p| format!("Reading {p}"))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let supplied = args
            .get("filePath")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("filePath is required.".into()))?;
        let path = resolve(&ctx.root, supplied);

        let metadata = std::fs::metadata(&path).map_err(|_| not_found(&path))?;

        if metadata.is_dir() {
            return read_directory(&path, &args);
        }

        let ext = extension(&path);

        if IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            if metadata.len() > MAX_IMAGE_BYTES {
                return Err(Error::InvalidInput(format!(
                    "{} is {:.1}MB, too large to look at (the limit is 5MB).",
                    path.display(),
                    metadata.len() as f64 / 1_048_576.0
                )));
            }
            let bytes = std::fs::read(&path)?;
            let mime = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                _ => "image/png",
            };
            let encoded = base64_encode(&bytes);
            return Ok(ToolOutcome {
                title: Some(path.display().to_string()),
                output: format!("Showing {} as an image.", path.display()),
                images: vec![format!("data:{mime};base64,{encoded}")],
                metadata: Some(json!({ "path": path.display().to_string() })),
            });
        }

        if BINARY_EXTENSIONS.contains(&ext.as_str()) {
            return Err(Error::InvalidInput(format!(
                "{} is a binary file and cannot be read as text.",
                path.display()
            )));
        }

        let bytes = std::fs::read(&path)?;
        if looks_binary(&bytes) {
            return Err(Error::InvalidInput(format!(
                "{} looks like a binary file and cannot be read as text.",
                path.display()
            )));
        }

        let text = String::from_utf8_lossy(&bytes);
        if text.is_empty() {
            self.state.mark_read(&ctx.session, &path, modified_time(&path));
            return Ok(ToolOutcome {
                title: Some(path.display().to_string()),
                output: "(Empty file)".into(),
                ..Default::default()
            });
        }

        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();

        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        if offset > total {
            return Err(Error::InvalidInput(format!(
                "Offset {offset} is out of range for this file ({total} lines)"
            )));
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_LIMIT as u64) as usize;

        let mut rendered = String::new();
        let mut used = 0;
        let mut shown = 0;

        for (index, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
            let body = if line.chars().count() > MAX_LINE_LENGTH {
                let cut: String = line.chars().take(MAX_LINE_LENGTH).collect();
                format!("{cut}... (line truncated to {MAX_LINE_LENGTH} chars)")
            } else {
                (*line).to_string()
            };
            let numbered = format!("{}: {}\n", index + 1, body);

            // Stop *before* exceeding the cap, so the promise is about what
            // came back rather than what was skipped.
            if used + numbered.len() > MAX_BYTES {
                break;
            }
            used += numbered.len();
            shown += 1;
            rendered.push_str(&numbered);
        }

        let last = offset + shown - 1;
        if last < total {
            rendered.push_str(&format!(
                "\n[showing lines {offset}-{last} of {total}; use offset to read further]\n"
            ));
        }

        // The only thing that makes a file eligible for `edit`.
        self.state.mark_read(&ctx.session, &path, modified_time(&path));

        // Start the language server for this file now, in the background. The
        // first edit is otherwise spent waiting for a server to index the
        // project, and reading a file is the reliable signal that one is about
        // to be edited.
        self.lsp.warm(&path, Some(&ctx.root));

        Ok(ToolOutcome {
            title: Some(path.display().to_string()),
            output: rendered,
            metadata: Some(json!({
                "path": path.display().to_string(),
                "lines": total,
            })),
            images: Vec::new(),
        })
    }
}

fn read_directory(path: &Path, args: &Value) -> Result<ToolOutcome> {
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LIMIT as u64) as usize;

    let mut entries: Vec<String> = std::fs::read_dir(path)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            // A trailing slash so the model can tell a folder from a file
            // without a second call.
            let is_dir = entry.file_type().ok()?.is_dir();
            Some(if is_dir { format!("{name}/") } else { name })
        })
        .collect();
    entries.sort();

    let total = entries.len();
    let shown: Vec<String> = entries.into_iter().skip(offset - 1).take(limit).collect();

    let mut output = format!("{} is a directory containing {total} entries:\n", path.display());
    for entry in &shown {
        output.push_str(&format!("  {entry}\n"));
    }

    Ok(ToolOutcome {
        title: Some(path.display().to_string()),
        output,
        metadata: Some(json!({ "path": path.display().to_string(), "entries": total })),
        images: Vec::new(),
    })
}

/// Minimal base64, to avoid a dependency for one call site.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }

    out
}

// ── write ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct WriteTool {
    state: Arc<ReadState>,
    /// The language servers, for the half of this tool that is not about the
    /// file it was given: what the change did to everything else.
    lsp: Arc<inertia_lsp::Lsp>,
}

impl WriteTool {
    pub fn new(state: Arc<ReadState>, lsp: Arc<inertia_lsp::Lsp>) -> Self {
        Self { state, lsp }
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn id(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write a file, creating it and any missing parent directories, replacing \
         anything already there. Prefer the edit tool for changing part of an \
         existing file."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filePath": { "type": "string", "description": "Path to write." },
                "content": { "type": "string", "description": "The complete new contents." }
            },
            "required": ["filePath", "content"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("filePath")
            .and_then(Value::as_str)
            .unwrap_or(inertia_core::permission::ANY);
        // Shares the `edit` key with the other file-changing tools: someone who
        // decided this agent may change files meant all the ways it does so.
        // Remembered per exact path, not as a wildcard - writing is not read.
        PermissionRequest::new("edit", target).with_always(target)
    }

    fn render(&self, args: &Value) -> Option<String> {
        args.get("filePath")
            .and_then(Value::as_str)
            .map(|p| format!("Writing {p}"))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let supplied = args
            .get("filePath")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("filePath is required.".into()))?;
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("content is required.".into()))?;

        let path = resolve(&ctx.root, supplied);
        let existed = path.exists();

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Temp-then-rename, so a crash mid-write leaves the old file intact
        // rather than a truncated one.
        write_atomically(&path, content)?;

        // A file just written is known-current, so `edit`'s read-before-write
        // gate is satisfied without a redundant read.
        self.state.mark_read(&ctx.session, &path, modified_time(&path));

        let lines = content.lines().count();
        // What the write broke, here and in the files that depend on this one.
        // A whole-file replace routinely breaks its importers, and that is the
        // one case the model has no other way to find out about.
        let broke = self
            .lsp
            .report(&path, Some(&ctx.root), inertia_lsp::MAX_OTHER_FILES)
            .await;
        Ok(ToolOutcome {
            title: Some(path.display().to_string()),
            output: format!(
                "{}{broke}",
                if existed {
                    format!("Wrote {lines} lines to {}", path.display())
                } else {
                    format!("Created {} with {lines} lines", path.display())
                }
            ),
            metadata: Some(json!({
                "path": path.display().to_string(),
                "existed": existed,
                "lines": lines,
            })),
            images: Vec::new(),
        })
    }
}

fn write_atomically(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;

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
        return Err(e.into());
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(e.into());
    }
    Ok(())
}

// ── edit ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct EditTool {
    state: Arc<ReadState>,
    /// The language servers, for the half of this tool that is not about the
    /// file it was given: what the change did to everything else.
    lsp: Arc<inertia_lsp::Lsp>,
}

impl EditTool {
    pub fn new(state: Arc<ReadState>, lsp: Arc<inertia_lsp::Lsp>) -> Self {
        Self { state, lsp }
    }
}

#[async_trait]
impl Tool for EditTool {
    fn id(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Replace exact text in a file. The file must have been read in this \
         conversation first. oldString must match what is in the file, including \
         indentation, and must identify one place unless replaceAll is set."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filePath": { "type": "string", "description": "Path to the file to change." },
                "oldString": { "type": "string", "description": "The exact text to replace." },
                "newString": { "type": "string", "description": "What to replace it with." },
                "replaceAll": {
                    "type": "boolean",
                    "default": false,
                    "description": "Replace every occurrence instead of requiring exactly one."
                }
            },
            "required": ["filePath", "oldString", "newString"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("filePath")
            .and_then(Value::as_str)
            .unwrap_or(inertia_core::permission::ANY);
        PermissionRequest::new("edit", target).with_always(target)
    }

    fn render(&self, args: &Value) -> Option<String> {
        args.get("filePath")
            .and_then(Value::as_str)
            .map(|p| format!("Editing {p}"))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let supplied = args
            .get("filePath")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("filePath is required.".into()))?;
        let old = args
            .get("oldString")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("oldString is required.".into()))?;
        let new = args
            .get("newString")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidInput("newString is required.".into()))?;
        let replace_all = args
            .get("replaceAll")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let path = resolve(&ctx.root, supplied);

        let metadata = std::fs::metadata(&path).map_err(|_| not_found(&path))?;
        if metadata.is_dir() {
            return Err(Error::InvalidInput(format!(
                "{} is a directory, not a file.",
                path.display()
            )));
        }

        // An edit is a claim about what the file contains. A model that has not
        // read it is reconstructing from memory, and a plausible edit applied
        // to the wrong contents is the worst outcome this tool has.
        if !self.state.was_read(&ctx.session, &path) {
            return Err(Error::InvalidInput(format!(
                "File {} has not been read in this conversation. Use the read tool \
                 first so the edit is based on what the file actually contains.",
                path.display()
            )));
        }
        if self.state.is_stale(&ctx.session, &path, modified_time(&path)) {
            return Err(Error::InvalidInput(format!(
                "File {} has changed on disk since it was read. Read it again before \
                 editing.",
                path.display()
            )));
        }

        let raw = std::fs::read_to_string(&path)?;

        // The match ladder reasons in LF and without a BOM; both are restored
        // before writing so the file's own conventions survive the edit.
        let had_bom = raw.starts_with('\u{feff}');
        let body = raw.trim_start_matches('\u{feff}');
        let was_crlf = body.contains("\r\n");
        let normalized = if was_crlf {
            body.replace("\r\n", "\n")
        } else {
            body.to_string()
        };

        let old = old.replace("\r\n", "\n");
        let new = new.replace("\r\n", "\n");

        // The ladder's messages are already written for the model to act on.
        let result = replace(&normalized, &old, &new, replace_all)
            .map_err(Error::InvalidInput)?;

        let mut updated = result.content;
        if was_crlf {
            updated = updated.replace('\n', "\r\n");
        }
        if had_bom {
            updated.insert(0, '\u{feff}');
        }

        write_atomically(&path, &updated)?;

        // Re-marked with the post-write mtime, so the model's next edit to the
        // same file is not rejected as stale by its own change.
        self.state.mark_read(&ctx.session, &path, modified_time(&path));

        // What the edit broke. Asked of the language server rather than
        // guessed, and empty when there is no server for this language.
        let broke = self.lsp.report(&path, Some(&ctx.root), 0).await;

        Ok(ToolOutcome {
            title: Some(path.display().to_string()),
            output: format!(
                "Applied edit to {} ({} match, {} replacement{}){broke}",
                path.display(),
                result.strategy.as_str(),
                result.count,
                if result.count == 1 { "" } else { "s" }
            ),
            metadata: Some(json!({
                "path": path.display().to_string(),
                "strategy": result.strategy.as_str(),
                "replacements": result.count,
            })),
            images: Vec::new(),
        })
    }
}

/// The three file tools, sharing one read-tracker and one set of language
/// servers.
pub fn file_tools(state: Arc<ReadState>, lsp: Arc<inertia_lsp::Lsp>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadTool::new(state.clone(), lsp.clone())),
        Arc::new(WriteTool::new(state.clone(), lsp.clone())),
        Arc::new(EditTool::new(state, lsp)),
    ]
}

//! Reading a shell command well enough to ask a good question about it.
//!
//! The shell tool is the one that can do real damage, so the permission it
//! asks for has to be about the *command*, not about "the terminal". "May this
//! agent run `git push *`" is a question a person can answer once and mean it;
//! "may this agent use the shell" is a question nobody can answer honestly.
//!
//! opencode gets this by parsing the command with a real tree-sitter bash
//! grammar. That is a dependency we are not adding, so this is a deliberately
//! modest tokeniser instead. Be clear about what that means: it understands
//! quoting, escapes and top-level operators, and nothing else. It does not know
//! about subshells, process substitution, here-documents, variable expansion,
//! aliases, functions, or the difference between PowerShell and bash beyond
//! the characters they happen to share. Every consumer here is therefore
//! advisory - it shapes the question the user is asked and the warning they
//! see, and never decides on its own that something is safe. A tokeniser that
//! is wrong about `rm -rf /` costs a scary-looking prompt; one that is trusted
//! to *allow* things would cost a filesystem. The one exception is [`shape`],
//! which a rule's allow depends on, and which therefore reads anything it is
//! unsure of as something no rule can see into.
//!
//! Pure and I/O-free on purpose, so all of it is unit-testable without a shell.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use inertia_core::permission::Shape;
use inertia_store::fsx::is_within;

// ── tokenising ─────────────────────────────────────────────────────────────

/// Characters a backslash is allowed to escape when we are not inside single
/// quotes.
///
/// This set is the whole Windows compromise. On POSIX a backslash escapes the
/// next character whatever it is, so `C:\Users\me` would tokenise to
/// `C:Usersme` and every path in every prompt this app shows on Windows would
/// be mangled. So a backslash only escapes when the next character is one that
/// could plausibly have needed escaping; before an ordinary letter it stays
/// literal and the path survives.
const ESCAPABLE: &[char] = &[
    '\'', '"', '\\', ' ', '\t', '&', '|', ';', '<', '>', '(', ')', '$', '`',
];

/// The tokens of a command line, and for each whether any of its characters
/// arrived inside quotes.
///
/// Callers need `quoted`: a quoted argument is one the user meant literally,
/// so `rm "my file.txt"` is one path and not two, and a token that arrived
/// quoted is not a flag however it starts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tokens {
    pub tokens: Vec<String>,
    pub quoted: Vec<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Bare,
    Single,
    Double,
}

/// Split `command` into tokens, respecting single quotes, double quotes and
/// backslash escapes.
pub fn tokenize(command: &str) -> Tokens {
    let chars: Vec<char> = command.chars().collect();
    let mut out = Tokens::default();

    let mut current = String::new();
    let mut started = false;
    let mut was_quoted = false;
    let mut mode = Mode::Bare;

    let flush =
        |current: &mut String, started: &mut bool, was_quoted: &mut bool, out: &mut Tokens| {
            if !*started {
                return;
            }
            out.tokens.push(std::mem::take(current));
            out.quoted.push(*was_quoted);
            *started = false;
            *was_quoted = false;
        };

    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        match mode {
            Mode::Single => {
                // Single quotes are literal all the way through, backslashes
                // included.
                if c == '\'' {
                    mode = Mode::Bare;
                } else {
                    current.push(c);
                    started = true;
                }
            }
            Mode::Double => {
                if c == '\\' && next.is_some_and(|n| ESCAPABLE.contains(&n)) {
                    current.push(chars[i + 1]);
                    started = true;
                    i += 1;
                } else if c == '"' {
                    mode = Mode::Bare;
                } else {
                    current.push(c);
                    started = true;
                }
            }
            Mode::Bare => match c {
                '\'' => {
                    mode = Mode::Single;
                    started = true;
                    was_quoted = true;
                }
                '"' => {
                    mode = Mode::Double;
                    started = true;
                    was_quoted = true;
                }
                '\\' => {
                    if next.is_some_and(|n| ESCAPABLE.contains(&n)) {
                        current.push(chars[i + 1]);
                        i += 1;
                    } else {
                        // A lone backslash before something ordinary. Windows
                        // path. Keep it.
                        current.push(c);
                    }
                    started = true;
                }
                ' ' | '\t' | '\n' | '\r' => {
                    flush(&mut current, &mut started, &mut was_quoted, &mut out);
                }
                _ => {
                    current.push(c);
                    started = true;
                }
            },
        }
        i += 1;
    }

    flush(&mut current, &mut started, &mut was_quoted, &mut out);
    out
}

// ── splitting ──────────────────────────────────────────────────────────────

/// Top-level operators, longest first so `&&` is never read as two `&`.
const OPERATORS: &[&str] = &["&&", "||", ";", "|", "&"];

/// One segment of a chained command, and the operator that FOLLOWS it.
///
/// `a && b` is `[{ a, "&&" }, { b, None }]`. Trailing on purpose: it reads
/// as "run a, and then", which is the order the line is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub operator: Option<&'static str>,
}

/// Split a command line on its top-level operators.
///
/// This is what lets `npm test && git push` be asked about as two things the
/// user can recognise rather than one opaque string that matches no rule they
/// would ever have written. Operators inside quotes are not operators.
pub fn split(command: &str) -> Vec<Segment> {
    scan(command, true).0
}

/// The segments of a command line, and whether anything in it is hidden from
/// a rule written about those segments.
///
/// `posix` decides what a backslash means. In bash it escapes the character
/// after it, quotes included; in PowerShell it is an ordinary character - the
/// path separator - and a string ends at the next quote whatever comes before
/// it. Reading a PowerShell line the bash way would take `"C:\dir\" ; rm x` as
/// one quoted string, and the `rm` would never be seen.
fn scan(command: &str, posix: bool) -> (Vec<Segment>, bool) {
    let chars: Vec<char> = command.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut mode = Mode::Bare;
    let mut hidden = false;

    let push = |current: &mut String, operator: Option<&'static str>, parts: &mut Vec<Segment>| {
        let text = current.trim().to_string();
        if !text.is_empty() {
            parts.push(Segment { text, operator });
        }
        current.clear();
    };

    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        // Looked for whatever the quoting, because both shells expand `$(`
        // and backticks inside double quotes, and a literal `$(` in single
        // quotes is rare enough that asking about one costs nothing.
        if matches!(c, '`' | '\n' | '\r')
            || (matches!(c, '$' | '@' | '<' | '>') && next == Some('('))
        {
            hidden = true;
        }
        match mode {
            Mode::Single => {
                current.push(c);
                if c == '\'' {
                    mode = Mode::Bare;
                }
            }
            Mode::Double => {
                current.push(c);
                if posix && c == '\\' && next.is_some() {
                    current.push(chars[i + 1]);
                    i += 1;
                } else if c == '"' {
                    mode = Mode::Bare;
                }
            }
            Mode::Bare => {
                if c == '\'' {
                    mode = Mode::Single;
                    current.push(c);
                } else if c == '"' {
                    mode = Mode::Double;
                    current.push(c);
                } else if posix && c == '\\' && next.is_some() {
                    // Carry the escape through untouched; splitting is not the
                    // place to decide what a backslash meant.
                    current.push(c);
                    current.push(chars[i + 1]);
                    i += 1;
                } else if let Some(op) = OPERATORS
                    .iter()
                    .find(|op| starts_with_at(&chars, i, op) && !is_redirection(&chars, i, op))
                {
                    push(&mut current, Some(op), &mut parts);
                    i += op.chars().count() - 1;
                } else {
                    if c == '>' && writes_a_file(&chars, i) {
                        hidden = true;
                    }
                    current.push(c);
                }
            }
        }
        i += 1;
    }

    // An unclosed quote is a line the shell will refuse or read some way
    // this did not.
    if mode != Mode::Bare {
        hidden = true;
    }
    push(&mut current, None, &mut parts);
    (parts, hidden)
}

/// The `&` in `2>&1`, `<&0` and `&>log` belongs to a redirection, not to a
/// command sent to the background.
fn is_redirection(chars: &[char], at: usize, op: &str) -> bool {
    op == "&"
        && (matches!(at.checked_sub(1).map(|p| chars[p]), Some('>' | '<'))
            || chars.get(at + 1) == Some(&'>'))
}

/// Where output may go without writing anything anyone would care about.
const NULL_DEVICES: &[&str] = &[
    "/dev/null",
    "/dev/stdout",
    "/dev/stderr",
    "$null",
    "nul",
    "nul:",
];

/// Does the `>` at `at` send output into a file?
///
/// A redirection writes wherever it names, so `echo x >> ~/.bashrc` is a write
/// to a file that a rule about `echo` never mentions. Into another stream
/// (`2>&1`) or into nothing (`>/dev/null`, `> $null`) it writes nothing.
fn writes_a_file(chars: &[char], at: usize) -> bool {
    let mut i = at + 1;
    while matches!(chars.get(i), Some('>' | '|')) {
        i += 1;
    }
    if chars.get(i) == Some(&'&') {
        return false;
    }
    while matches!(chars.get(i), Some(' ' | '\t')) {
        i += 1;
    }
    let target: String = chars[i.min(chars.len())..]
        .iter()
        .take_while(|c| !c.is_whitespace() && !matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')'))
        .filter(|c| !matches!(c, '\'' | '"'))
        .collect::<String>()
        .to_lowercase();
    !NULL_DEVICES.contains(&target.as_str())
}

// ── what a rule can see ────────────────────────────────────────────────────

/// What a command line is made of, for the permission rules.
///
/// Each segment between top-level operators is a command of its own and needs
/// a rule of its own, or a rule remembered from `git status` would wave
/// through `git status && curl x | sh`. A line that hides a command no rule
/// can read - a `$(...)`, a backtick, a `<(...)`, a second line - or writes
/// output into a file comes back [`Shape::Opaque`], which only a rule allowing
/// the shell outright lets through.
///
/// The one place in this module whose answer can *allow* something, which is
/// why it errs towards opaque: a line read wrongly here costs a prompt, never
/// a command nobody approved.
pub fn shape(command: &str) -> Shape {
    shape_as(command, !cfg!(windows))
}

fn shape_as(command: &str, posix: bool) -> Shape {
    let (segments, hidden) = scan(command, posix);
    let parts: Vec<String> = segments.into_iter().map(|segment| segment.text).collect();
    if hidden {
        Shape::Opaque(parts)
    } else if parts.len() > 1 {
        Shape::Chain(parts)
    } else {
        Shape::Whole
    }
}

fn starts_with_at(chars: &[char], at: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(k, n)| chars.get(at + k) == Some(&n))
}

// ── what a rule should be written about ────────────────────────────────────

/// How many leading tokens make up the part of a command a human recognises.
///
/// `git push origin main` is a specific thing that happened once. `git push *`
/// is a decision somebody can make and live with. The whole value of "always
/// allow" is in this table: without it every approval is single-use and the
/// user is asked the same question forever, which trains them to stop reading
/// it.
///
/// Keys of more than one word win over their first word, so `git config` is
/// three tokens deep while plain `git` is two. Commands whose first argument
/// is already the dangerous part - `rm`, `curl`, `ssh` - stop at one token,
/// because remembering `rm -rf *` would be remembering the wrong half.
const ARITY: &[(&str, usize)] = &[
    ("git", 2),
    ("git config", 3),
    ("git remote", 3),
    ("npm", 2),
    ("npm run", 3),
    ("pnpm", 2),
    ("pnpm run", 3),
    ("yarn", 2),
    ("bun", 2),
    ("bun run", 3),
    ("npx", 2),
    ("docker", 2),
    ("docker compose", 3),
    ("kubectl", 2),
    ("cargo", 2),
    ("go", 2),
    ("dotnet", 2),
    ("pip", 2),
    ("python", 1),
    ("node", 1),
    ("terraform", 2),
    ("aws", 2),
    ("gcloud", 3),
    ("gh", 3),
    ("make", 2),
    ("systemctl", 2),
    ("brew", 2),
    ("apt", 2),
    ("apt-get", 2),
    ("choco", 2),
    ("winget", 2),
    // One token each: everything after the name is the argument, and the
    // argument is the part worth being asked about again next time.
    ("rm", 1),
    ("cat", 1),
    ("ls", 1),
    ("cp", 1),
    ("mv", 1),
    ("mkdir", 1),
    ("touch", 1),
    ("curl", 1),
    ("wget", 1),
    ("chmod", 1),
    ("echo", 1),
    ("grep", 1),
    ("find", 1),
    ("sed", 1),
    ("awk", 1),
    ("tar", 1),
    ("zip", 1),
    ("unzip", 1),
    ("ssh", 1),
    ("scp", 1),
    ("ps", 1),
    ("kill", 1),
];

/// The longest key in ARITY, in words. Bounds the prefix search.
const MAX_ARITY_WORDS: usize = 3;

/// The pattern to remember when the user answers "always allow".
///
/// Longest matching prefix in ARITY, that many tokens, then ` *`. An unknown
/// command falls back to its own name, which is the most specific thing we can
/// honestly claim to understand about it.
pub fn always_pattern(command: &str) -> String {
    let Tokens { tokens, .. } = tokenize(command);
    if tokens.is_empty() {
        return "*".to_string();
    }

    let mut words = MAX_ARITY_WORDS.min(tokens.len());
    while words >= 1 {
        let key = tokens[..words].join(" ");
        if let Some((_, arity)) = ARITY.iter().find(|(k, _)| *k == key) {
            let keep = (*arity).min(tokens.len());
            return format!("{} *", tokens[..keep].join(" "));
        }
        words -= 1;
    }

    format!("{} *", tokens[0])
}

// ── danger ─────────────────────────────────────────────────────────────────

/// Glob matching, `*` and `?` only.
///
/// The same rules as the renderer's `permission.js`: two wildcards, nothing
/// else, and the semantics are frozen.
fn glob_matches(pattern: &str, value: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return pattern == value;
    }
    let mut regex = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            other => regex.push_str(&regex::escape(&other.to_string())),
        }
    }
    regex.push('$');
    regex::RegexBuilder::new(&regex)
        .dot_matches_new_line(true)
        .build()
        .map(|r| r.is_match(value))
        .unwrap_or(false)
}

/// One pattern that deserves a warning, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Danger {
    pub pattern: &'static str,
    pub why: &'static str,
}

/// Commands that deserve a loud warning even when a rule would let them
/// through.
///
/// ADVISORY, and that word is doing real work: `dangers()` never blocks
/// anything. It feeds metadata into the result so the UI can shout, and the
/// user still decides. A list like this is guaranteed to be incomplete and
/// occasionally wrong, and a guess that silently refuses work is a far worse
/// failure than a guess that asks twice.
const DANGEROUS: &[Danger] = &[
    Danger {
        pattern: "rm -rf /",
        why: "Deletes everything on the filesystem.",
    },
    Danger {
        pattern: "rm -fr /",
        why: "Deletes everything on the filesystem.",
    },
    Danger {
        pattern: "rm -rf /*",
        why: "Deletes everything under the filesystem root.",
    },
    Danger {
        pattern: "rm -rf --no-preserve-root*",
        why: "Deliberately disables the root guard.",
    },
    Danger {
        pattern: "rm -rf ~",
        why: "Deletes the entire home directory.",
    },
    Danger {
        pattern: "rm -rf ~/*",
        why: "Deletes everything in the home directory.",
    },
    Danger {
        pattern: "mkfs*",
        why: "Formats a filesystem, destroying whatever was on it.",
    },
    Danger {
        pattern: "dd if=* of=/dev/*",
        why: "Writes raw data straight to a device.",
    },
    Danger {
        pattern: ":(){*};:",
        why: "Fork bomb. Hangs the machine until it is rebooted.",
    },
    Danger {
        pattern: "chmod -R 777 /",
        why: "Makes the whole filesystem world-writable.",
    },
    Danger {
        pattern: "chmod -R 777 /*",
        why: "Makes the whole filesystem world-writable.",
    },
    Danger {
        pattern: "curl * | sh",
        why: "Runs a script off the internet without reading it.",
    },
    Danger {
        pattern: "curl * | bash",
        why: "Runs a script off the internet without reading it.",
    },
    Danger {
        pattern: "wget * | sh",
        why: "Runs a script off the internet without reading it.",
    },
    Danger {
        pattern: "wget * | bash",
        why: "Runs a script off the internet without reading it.",
    },
    Danger {
        pattern: "git push --force*main*",
        why: "Force-pushes over a protected branch.",
    },
    Danger {
        pattern: "git push --force*master*",
        why: "Force-pushes over a protected branch.",
    },
    Danger {
        pattern: "git push -f *main*",
        why: "Force-pushes over a protected branch.",
    },
    Danger {
        pattern: "git push -f *master*",
        why: "Force-pushes over a protected branch.",
    },
    Danger {
        pattern: "*drop database*",
        why: "Drops a database.",
    },
    Danger {
        pattern: "format c:*",
        why: "Formats the system drive.",
    },
    Danger {
        pattern: "del /f /s /q c:\\*",
        why: "Recursively force-deletes the system drive.",
    },
];

/// Whitespace collapsed to single spaces, lowercased, trimmed.
///
/// `DROP DATABASE` and `drop   database` are the same intent, and a warning
/// that only fires on one spelling is a warning that mostly does not fire.
fn normalized(command: &str) -> String {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Which of the dangerous patterns this command looks like.
pub fn dangers(command: &str) -> Vec<Danger> {
    let text = normalized(command);
    if text.is_empty() {
        return Vec::new();
    }
    DANGEROUS
        .iter()
        .filter(|entry| glob_matches(&entry.pattern.to_lowercase(), &text))
        .cloned()
        .collect()
}

/// A sweep found in one part of a command, and the part it was found in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sweep {
    pub pattern: &'static str,
    pub why: &'static str,
    pub command: String,
}

/// Emptying a folder, wherever that folder is.
///
/// Separate from `DANGEROUS` because it is a different kind of claim. Those
/// patterns are about the filesystem root and the home directory - places an
/// agent has no business in at all. These are about the working folder, where
/// an agent is supposed to have a free hand: it may write, rewrite, move and
/// delete whatever it likes in there without being asked, and that is the
/// point.
///
/// Deleting all of it is the one exception. An agent that decides a clean
/// slate is the fastest route to a passing build is one keystroke from taking
/// a day's work with it, and unlike every other edit there is nothing left to
/// read afterwards to find out what happened. So this always asks, and cannot
/// be turned off with an "always allow".
const MASS_DELETE: &[Danger] = &[
    // rm, in every spelling of recursive-and-forced against a whole directory.
    Danger {
        pattern: "rm -r* *",
        why: "Deletes a directory and everything in it.",
    },
    Danger {
        pattern: "rm -fr* *",
        why: "Deletes a directory and everything in it.",
    },
    // PowerShell.
    Danger {
        pattern: "remove-item * -recurse*",
        why: "Deletes a directory and everything in it.",
    },
    Danger {
        pattern: "remove-item -recurse*",
        why: "Deletes a directory and everything in it.",
    },
    Danger {
        pattern: "ri * -recurse*",
        why: "Deletes a directory and everything in it.",
    },
    // cmd.
    Danger {
        pattern: "rd /s*",
        why: "Deletes a directory tree.",
    },
    Danger {
        pattern: "rmdir /s*",
        why: "Deletes a directory tree.",
    },
    Danger {
        pattern: "del /s*",
        why: "Deletes files recursively.",
    },
    Danger {
        pattern: "del /q*",
        why: "Deletes files without confirming.",
    },
    // Everything that is not checked in, which is usually more than people think.
    Danger {
        pattern: "git clean -*d*f*",
        why: "Deletes every untracked file, including ones never saved.",
    },
    Danger {
        pattern: "git clean -*f*d*",
        why: "Deletes every untracked file, including ones never saved.",
    },
    Danger {
        pattern: "git reset --hard*",
        why: "Throws away every uncommitted change.",
    },
    Danger {
        pattern: "git checkout -- .",
        why: "Throws away every uncommitted change.",
    },
    // find, which is how a delete gets spelled when rm would have been noticed.
    Danger {
        pattern: "find * -delete*",
        why: "Deletes every file the search matched.",
    },
    Danger {
        pattern: "find * -exec rm*",
        why: "Deletes every file the search matched.",
    },
    Danger {
        pattern: "*truncate table*",
        why: "Empties a table.",
    },
];

/// Does this command empty something?
///
/// Deliberately generous. A false positive costs one prompt on a command the
/// user was going to allow anyway; a false negative costs the folder.
pub fn mass_delete(command: &str) -> Vec<Sweep> {
    let text = normalized(command);
    if text.is_empty() {
        return Vec::new();
    }
    // Each part of a chain on its own, or `npm test && rm -rf src` hides
    // behind the command it was joined to.
    let parts = text
        .split("&&")
        .flat_map(|p| p.split("||"))
        .flat_map(|p| p.split(';'))
        .flat_map(|p| p.split('|'))
        .map(str::trim)
        .filter(|p| !p.is_empty());

    let mut found: Vec<Sweep> = Vec::new();
    for part in parts {
        for entry in MASS_DELETE {
            if glob_matches(entry.pattern, part) && !found.iter().any(|one| one.why == entry.why) {
                found.push(Sweep {
                    pattern: entry.pattern,
                    why: entry.why,
                    command: part.to_string(),
                });
            }
        }
    }
    found
}

// ── reaching outside the project ───────────────────────────────────────────

/// Commands whose arguments are paths.
///
/// Only these are inspected. Guessing that any argument-shaped string is a
/// path would flag half of every command line, and a second permission prompt
/// that fires constantly is one the user learns to click through.
const FILES: &[&str] = &[
    "rm",
    "cp",
    "mv",
    "mkdir",
    "touch",
    "chmod",
    "chown",
    "cat",
    "type",
    "del",
    "copy",
    "move",
    "rmdir",
    "rd",
    "md",
    "ren",
    "rename",
    "get-content",
    "set-content",
    "remove-item",
    "copy-item",
    "move-item",
    "new-item",
];

/// `/usr/bin/rm` and `C:\tools\rm.exe` are both `rm`.
fn command_name(token: &str) -> String {
    let tail = token.rsplit(['\\', '/']).next().unwrap_or("");
    let lower = tail.to_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".ps1"] {
        if let Some(stem) = lower.strip_suffix(ext) {
            return stem.to_string();
        }
    }
    lower
}

/// Arguments we cannot resolve at parse time, and therefore do not claim to.
///
/// A `$VAR`, a backtick, a `$(...)` or a glob in the first path segment all
/// mean the real path is decided by the shell at run time, and this module has
/// no shell. Guessing produces a permission prompt about a directory the
/// command will never touch, which is worse than saying nothing: it teaches
/// the user that the prompt is noise. The glob check is scoped to the segment
/// before the first separator because `./build/*` still tells us the
/// directory, while a wildcard in front of the first separator does not tell
/// us anything at all.
fn unresolvable(token: &str) -> bool {
    if token.contains('$') || token.contains('`') {
        return true;
    }
    let head = token.split(['\\', '/']).next().unwrap_or("");
    head.contains(['*', '?', '['])
}

/// A single-letter switch in either spelling: `-r` or `/f`. `/etc/hosts` is a
/// path, so only the one-letter form is skipped.
fn is_short_flag(token: &str) -> bool {
    let mut chars = token.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some('-' | '/'), Some(c), None) if c.is_ascii_alphabetic()
    )
}

/// The user's home directory, for expanding a leading `~`.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Lexical normalisation of `base.join(path)`: `..` and `.` folded, no
/// filesystem access. The path may not exist yet - `mkdir ../x` - so
/// canonicalising is not an option.
fn resolve(base: &Path, path: &str) -> PathBuf {
    let joined = if Path::new(path).is_absolute() || has_drive(path) {
        PathBuf::from(path)
    } else {
        base.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                // Never above the root: `..` at a drive root stays there.
                if !matches!(
                    out.components().next_back(),
                    None | Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `C:foo` is drive-relative on Windows; `Path::is_absolute` says no, and
/// joining it to `base` would produce nonsense.
fn has_drive(path: &str) -> bool {
    let bytes = path.as_bytes();
    cfg!(windows) && bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

/// Directories this command appears to touch that are outside `cwd`.
///
/// The shell tool asks a second, separate permission question about these,
/// because "run a command in this project" and "reach into somewhere else on
/// this machine" are different decisions and folding them together means the
/// user only ever gets to make the first one.
pub fn external_paths(command: &str, cwd: &Path) -> Vec<PathBuf> {
    let base = resolve(cwd, "");
    // Keyed by folded path so `D:\Other` and `d:\other` are one entry, with
    // the first spelling seen kept for display.
    let mut found: BTreeMap<String, PathBuf> = BTreeMap::new();

    for segment in split(command) {
        let Tokens { tokens, quoted } = tokenize(&segment.text);
        let Some(first) = tokens.first() else {
            continue;
        };
        if !FILES.contains(&command_name(first).as_str()) {
            continue;
        }

        for (i, token) in tokens.iter().enumerate().skip(1) {
            if token.is_empty() {
                continue;
            }
            let bare = !quoted.get(i).copied().unwrap_or(false);
            if bare && (token.starts_with("--") || is_short_flag(token) || token.starts_with('-')) {
                continue;
            }
            if unresolvable(token) {
                continue;
            }

            let expanded = if token == "~" || token.starts_with("~/") || token.starts_with("~\\") {
                match home_dir() {
                    Some(home) => home
                        .join(token[1..].trim_start_matches(['/', '\\']))
                        .to_string_lossy()
                        .to_string(),
                    None => continue,
                }
            } else {
                token.clone()
            };

            let resolved = resolve(&base, &expanded);
            // The directory, not the file: a rule about `../secrets/` is one a
            // person can reason about, and one about every file inside it is
            // not.
            let dir = if token.ends_with(['/', '\\']) {
                resolved
            } else {
                resolved.parent().map(Path::to_path_buf).unwrap_or(resolved)
            };
            if is_within(&base, &dir) {
                continue;
            }
            let key = dir.to_string_lossy().to_lowercase();
            found.entry(key).or_insert(dir);
        }
    }

    let mut out: Vec<PathBuf> = found.into_values().collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_splits_on_whitespace_and_respects_quotes() {
        assert_eq!(
            tokenize("git push origin main").tokens,
            vec!["git", "push", "origin", "main"]
        );
        let t = tokenize("echo 'hello world'");
        assert_eq!(t.tokens, vec!["echo", "hello world"]);
        assert_eq!(t.quoted, vec![false, true]);
        assert_eq!(
            tokenize("git commit -m \"fix the thing\"").tokens,
            vec!["git", "commit", "-m", "fix the thing"]
        );
        assert_eq!(
            tokenize("echo \"it's fine\"").tokens,
            vec!["echo", "it's fine"]
        );
        assert_eq!(
            tokenize("echo 'say \"hi\"'").tokens,
            vec!["echo", "say \"hi\""]
        );
    }

    #[test]
    fn tokenize_joins_adjacent_pieces_and_keeps_empty_quotes() {
        let t = tokenize("--message=\"a b\"");
        assert_eq!(t.tokens, vec!["--message=a b"]);
        assert_eq!(t.quoted, vec![true]);
        assert_eq!(
            tokenize("git commit -m \"\"").tokens,
            vec!["git", "commit", "-m", ""]
        );
        assert_eq!(
            tokenize("cat my\\ file.txt").tokens,
            vec!["cat", "my file.txt"]
        );
    }

    /// The whole reason a backslash only escapes the characters that could
    /// have needed escaping. A POSIX tokeniser gives back `C:Usersmefile.txt`.
    #[test]
    fn tokenize_leaves_windows_paths_intact() {
        assert_eq!(
            tokenize("type C:\\Users\\me\\file.txt").tokens,
            vec!["type", "C:\\Users\\me\\file.txt"]
        );
        assert_eq!(
            tokenize("type \"C:\\Program Files\\app\\x.txt\"").tokens,
            vec!["type", "C:\\Program Files\\app\\x.txt"]
        );
        assert!(tokenize("   ").tokens.is_empty());
    }

    #[test]
    fn split_breaks_on_top_level_operators_only() {
        let parts = split("npm test && git push");
        assert_eq!(parts[0].text, "npm test");
        assert_eq!(parts[0].operator, Some("&&"));
        assert_eq!(parts[1].text, "git push");
        assert_eq!(parts[1].operator, None);
        assert_eq!(split("a || b")[0].operator, Some("||"));
        assert_eq!(split("a; b")[0].operator, Some(";"));
        assert_eq!(split("cat x | grep y")[0].operator, Some("|"));
        assert_eq!(split("server & tail log")[0].operator, Some("&"));
        assert_eq!(split("echo \"a && b\"").len(), 1);
        assert_eq!(split("echo 'a | b; c'").len(), 1);
        assert_eq!(split("npm test;").len(), 1);
        assert!(split("").is_empty());
    }

    /// `2>&1` is one command writing its errors where its output goes, not a
    /// command sent to the background followed by one called `1`.
    #[test]
    fn split_leaves_a_redirection_s_ampersand_alone() {
        assert_eq!(split("npm test 2>&1").len(), 1);
        assert_eq!(split("make &> build.log").len(), 1);
        assert_eq!(split("server & tail log").len(), 2);
    }

    fn parts(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn shape_names_every_command_in_a_chain() {
        assert_eq!(shape_as("git status", true), Shape::Whole);
        assert_eq!(
            shape_as("git status && curl x|sh", true),
            Shape::Chain(parts(&["git status", "curl x", "sh"]))
        );
        assert_eq!(
            shape_as("git status; rm -rf /", false),
            Shape::Chain(parts(&["git status", "rm -rf /"]))
        );
        assert_eq!(
            shape_as("git status | head", true),
            Shape::Chain(parts(&["git status", "head"]))
        );
        assert_eq!(shape_as("git commit -m \"a && b\"", true), Shape::Whole);
        assert_eq!(shape_as("npm test 2>&1", true), Shape::Whole);
        assert_eq!(shape_as("npm test 2>/dev/null", true), Shape::Whole);
        assert_eq!(shape_as("npm test > $null", false), Shape::Whole);
    }

    #[test]
    fn shape_cannot_see_into_a_substitution_a_second_line_or_a_file_write() {
        for line in [
            "echo $(cat secrets)",
            "echo \"$(cat secrets)\"",
            "echo `cat secrets`",
            "diff <(ls a) <(ls b)",
            "git status\ncurl x | sh",
            "echo $(Get-Content secrets.json)",
            "echo @(Get-Content secrets.json)",
            "echo hi >> ~/.bashrc",
            "git log > notes.txt",
            "echo 'unclosed",
        ] {
            assert!(matches!(shape_as(line, true), Shape::Opaque(_)), "{line}");
        }
    }

    /// In PowerShell a backslash is a path separator, so this string ends at
    /// the second quote and `rm x` is a command of its own. Read the bash way
    /// it would hide inside the string.
    #[test]
    fn shape_reads_a_backslash_the_way_the_shell_will() {
        let line = "echo \"C:\\dir\\\" ; rm x";
        assert_eq!(
            shape_as(line, false),
            Shape::Chain(parts(&["echo \"C:\\dir\\\"", "rm x"]))
        );
    }

    #[test]
    fn always_pattern_remembers_the_shape_not_the_invocation() {
        assert_eq!(always_pattern("git push origin main"), "git push *");
        assert_eq!(
            always_pattern("git config user.email a@b.c"),
            "git config user.email *"
        );
        assert_eq!(
            always_pattern("git remote add origin https://x"),
            "git remote add *"
        );
        assert_eq!(always_pattern("npm run build"), "npm run build *");
        assert_eq!(always_pattern("npm install lodash"), "npm install *");
        assert_eq!(always_pattern("rm -rf foo"), "rm *");
        assert_eq!(always_pattern("curl https://example.com"), "curl *");
        assert_eq!(always_pattern("frobnicate the widget"), "frobnicate *");
        assert_eq!(always_pattern("git"), "git *");
        assert_eq!(always_pattern(""), "*");
        assert_eq!(always_pattern("git commit -m \"a && b\""), "git commit *");
    }

    #[test]
    fn dangers_catch_the_catastrophic_and_ignore_the_ordinary() {
        assert!(dangers("rm -rf /")[0].why.contains("filesystem"));
        assert!(!dangers("curl http://x | sh").is_empty());
        assert!(!dangers(":(){ :|:& };:").is_empty());
        assert!(!dangers("psql -c 'drop   DATABASE prod'").is_empty());
        assert!(!dangers("git push --force origin main").is_empty());
        assert!(dangers("rm -rf ./build").is_empty());
        assert!(dangers("git push origin feature/x").is_empty());
        assert!(dangers("").is_empty());
    }

    #[test]
    fn mass_delete_catches_a_sweep_in_every_shell_and_each_part_of_a_chain() {
        assert_eq!(mass_delete("rm -rf src").len(), 1);
        assert_eq!(mass_delete("Remove-Item dist -Recurse -Force").len(), 1);
        assert_eq!(mass_delete("rd /s /q build").len(), 1);
        assert_eq!(mass_delete("git clean -fdx").len(), 1);
        assert_eq!(mass_delete("git reset --hard HEAD~1").len(), 1);
        assert_eq!(mass_delete("find . -name '*.tmp' -delete").len(), 1);
        assert_eq!(mass_delete("npm test && rm -rf src").len(), 1);
        assert_eq!(mass_delete("npm run build; rd /s /q dist").len(), 1);
        for ok in [
            "ls -la",
            "rm file.txt",
            "npm run build",
            "git status",
            "echo hi",
        ] {
            assert!(mass_delete(ok).is_empty(), "{ok}");
        }
    }

    fn cwd() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from("C:\\projects\\app")
        } else {
            PathBuf::from("/projects/app")
        }
    }

    fn other() -> PathBuf {
        if cfg!(windows) {
            PathBuf::from("C:\\projects\\other")
        } else {
            PathBuf::from("/projects/other")
        }
    }

    #[test]
    fn external_paths_report_a_directory_outside_the_working_folder() {
        assert_eq!(
            external_paths("rm ../other/notes.txt", &cwd()),
            vec![other()]
        );
        assert_eq!(external_paths("rm ../other/*.log", &cwd()), vec![other()]);
        assert_eq!(
            external_paths("npm test && rm ../other/x.txt", &cwd()),
            vec![other()]
        );
    }

    #[test]
    fn external_paths_say_nothing_about_the_project_flags_or_the_unresolvable() {
        assert!(external_paths("rm -rf ./build", &cwd()).is_empty());
        assert!(external_paths("cat src/index.js", &cwd()).is_empty());
        assert!(external_paths("rm -rf ./build --verbose", &cwd()).is_empty());
        assert!(external_paths("git log ../other", &cwd()).is_empty());
        assert!(external_paths("rm $HOME/thing.txt", &cwd()).is_empty());
        assert!(external_paths("rm `pwd`/thing.txt", &cwd()).is_empty());
        assert!(external_paths("rm */thing.txt", &cwd()).is_empty());
    }

    #[test]
    fn external_paths_expand_a_leading_tilde() {
        let Some(home) = home_dir() else { return };
        let found = external_paths("cat ~/.ssh/config", &cwd());
        assert_eq!(found, vec![home.join(".ssh")]);
    }
}

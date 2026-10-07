//! The failure log, and the tool that reads it back.
//!
//! Every time something an agent tried does not work - a tool that threw, a
//! command that exited non-zero, a permission the user refused, a provider that
//! answered 500, a routine that never started, an MCP server that would not
//! connect, a subagent that crashed - one record goes into `logs/failures/` in
//! the workspace, as a line of JSON.
//!
//! The reader is an agent - this one, later, or another one asked "why does the
//! build keep failing" - and what it needs is not a stack trace but a record it
//! can query: what was tried, with what arguments, in which folder, and what
//! came back. So the fields are structured, the arguments and output are kept
//! (capped, and with anything that looks like a credential taken out), and
//! every record carries a signature so the same mistake made twice reads as one
//! problem seen twice rather than two problems.
//!
//! Three decisions worth defending:
//!
//! Ninety days, not the week a developer log keeps. A failure is most useful
//! long after it happened - the second time the same command breaks the same
//! way is when anyone wants to know about the first - and a line of JSON is
//! cheap to keep.
//!
//! JSON lines, one file a day. Append-only, so a write mid-crash cannot corrupt
//! what was already there; one file a day, so the first question anyone asks of
//! the folder - "what went wrong on Tuesday" - is answered by the file names.
//!
//! Never fails. Every caller is an error path, and a failure log that can fail
//! the thing it is recording is the one bug this file must not have: `record`
//! returns rather than propagates, and a folder that cannot be written to costs
//! the record, not the turn.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::Result;
use inertia_store::secrets::{self, REDACTED};
use inertia_store::Layout;
use serde_json::{json, Value};

/// Days of files kept. Older ones are deleted the first time this process
/// writes into the folder.
pub const KEEP_DAYS: i64 = 90;

/// Bytes a single day's file may reach. Past this the day stops being recorded
/// rather than the disk filling up - a crash loop writes the same line forever.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// How much of each free-text field survives. Enough to read; not a dump.
const CAP_ARGS: usize = 4000;
const CAP_OUTPUT: usize = 4000;
const CAP_ERROR: usize = 2000;
const CAP_STACK: usize = 3000;
const CAP_PROMPT: usize = 1000;

/// Records the tool will return at once, however high a `limit` is asked for.
const MAX_ROWS: usize = 50;

/// What a record's `kind` may be. Anything else is stored as "other".
pub const KINDS: &[&str] = &[
    "tool",           // a tool call that returned a failure
    "tool-arguments", // the model called a tool with a shape the schema refused
    "tool-unknown",   // the model called a tool that does not exist
    "permission",     // the user, or a rule, refused a call
    "provider",       // the model's endpoint failed the request
    "turn",           // the turn ended in an error that was not the provider's
    "turn-limit",     // the turn ran out of steps
    "loop",           // the same call repeated until the loop guard stopped it
    "routine",        // a scheduled run did not complete
    "subagent",       // a spawned run failed
    "mcp",            // a server would not connect or answer
    "lsp",            // a language server died
    "hook",           // a lifecycle hook failed or timed out
    "crash",          // the app itself
    "other",
];

/// Where the log lives, under the workspace.
///
/// Built a component at a time rather than as `"logs/failures"`: an embedded
/// slash survives into the `PathBuf` on Windows and compares unequal to the
/// same path built properly, which is how a folder gets created twice under two
/// spellings.
pub fn directory(layout: &Layout) -> PathBuf {
    let mut dir = layout.root().to_path_buf();
    dir.push("logs");
    dir.push("failures");
    dir
}

/* -- redaction ----------------------------------------------------------- */

/// Token prefixes that are a credential by shape, and how much has to follow
/// before it is one rather than a word that happens to start that way.
const PREFIXES: &[(&str, usize)] = &[
    ("sk-", 16),
    ("rk-", 16),
    ("pk-", 16),
    ("ghp_", 20),
    ("gho_", 20),
    ("ghu_", 20),
    ("ghs_", 20),
    ("ghr_", 20),
    ("glpat-", 16),
    ("gldt-", 16),
    ("glrt-", 16),
    ("gloas-", 16),
    ("glptt-", 16),
    ("glagent-", 16),
    ("glimt-", 16),
    ("glsoat-", 16),
    ("glcbt-", 16),
    ("glft-", 16),
    ("glffct-", 16),
    ("xoxa-", 10),
    ("xoxb-", 10),
    ("xoxp-", 10),
    ("xoxr-", 10),
    ("xoxs-", 10),
    ("AKIA", 16),
];

/// Key names whose value is a credential whatever the key is called around it.
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "api-key",
    "apikey",
    "access_key",
    "access-key",
    "accesskey",
    "authorization",
    "password",
    "passwd",
    "secret",
    "token",
];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// True when a match starting here starts a word rather than ending one.
fn at_boundary(bytes: &[u8], at: usize) -> bool {
    at == 0 || !is_token_char(bytes[at - 1] as char)
}

/// Takes out anything that reads as a credential.
///
/// Two layers. The prefixes catch the shapes that keys and tokens have, and the
/// `exact` values catch the credentials themselves, which is what the
/// workspace's own secrets file supplies. Neither is perfect; both together
/// mean a stored API key would have to be both unrecognisable by shape and
/// unknown to the app to reach a file an agent will read back into a prompt.
pub fn redact(text: &str, exact: &[String]) -> String {
    let mut out = secrets::scrub(text, exact);
    out = redact_prefixed(&out);
    out = redact_bearer(&out);
    redact_pairs(&out)
}

/// `sk-...`, `ghp_...`, and the rest of the vendor shapes.
fn redact_prefixed(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'outer: while i < bytes.len() {
        if at_boundary(bytes, i) {
            for (prefix, min) in PREFIXES {
                if !text[i..].starts_with(prefix) {
                    continue;
                }
                let start = i + prefix.len();
                let mut end = start;
                while end < bytes.len() && is_token_char(bytes[end] as char) {
                    end += 1;
                }
                if end - start >= *min {
                    out.push_str(REDACTED);
                    i = end;
                    continue 'outer;
                }
            }
        }
        let c = text[i..].chars().next().unwrap_or_default();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// `Bearer <token>`, whatever the header around it is called.
///
/// The word "Bearer" is not the credential and is left in place: replacing the
/// whole thing would leave a reader unable to tell a redacted token from a
/// header that never had one.
fn redact_bearer(text: &str) -> String {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if at_boundary(bytes, i) && lower[i..].starts_with("bearer") {
            let mut at = i + "bearer".len();
            let spaces = at;
            while at < bytes.len() && (bytes[at] == b' ' || bytes[at] == b'\t') {
                at += 1;
            }
            let start = at;
            while at < bytes.len() && is_bearer_char(bytes[at] as char) {
                at += 1;
            }
            if at > spaces && at - start >= 16 {
                out.push_str(&text[i..start]);
                out.push_str(REDACTED);
                i = at;
                continue;
            }
        }
        let c = text[i..].chars().next().unwrap_or_default();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn is_bearer_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "._~+/=-".contains(c)
}

/// `token=...`, `"api_key": "..."`, `password: ...`.
fn redact_pairs(text: &str) -> String {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'outer: while i < bytes.len() {
        if at_boundary(bytes, i) {
            for key in SECRET_KEYS {
                if !lower[i..].starts_with(key) {
                    continue;
                }
                let mut at = i + key.len();
                // An optional closing quote from `"api_key":`, then the
                // separator, then the value's own opening quote.
                if at < bytes.len() && (bytes[at] == b'"' || bytes[at] == b'\'') {
                    at += 1;
                }
                while at < bytes.len() && bytes[at] == b' ' {
                    at += 1;
                }
                if at >= bytes.len() || (bytes[at] != b':' && bytes[at] != b'=') {
                    continue;
                }
                at += 1;
                while at < bytes.len() && bytes[at] == b' ' {
                    at += 1;
                }
                if at < bytes.len() && (bytes[at] == b'"' || bytes[at] == b'\'') {
                    at += 1;
                }
                let start = at;
                while at < bytes.len() && !is_value_end(bytes[at] as char) {
                    at += 1;
                }
                let value = &text[start..at];
                // "Bearer" is the scheme, not the secret, and the token after
                // it has already been taken out. Redacting it here would hide
                // which scheme was used and gain nothing.
                if value.len() >= 6 && !value.eq_ignore_ascii_case("bearer") && value != REDACTED {
                    out.push_str(&text[i..start]);
                    out.push_str(REDACTED);
                    i = at;
                    continue 'outer;
                }
            }
        }
        let c = text[i..].chars().next().unwrap_or_default();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn is_value_end(c: char) -> bool {
    c.is_whitespace() || "\"',;}&".contains(c)
}

/* -- shaping ------------------------------------------------------------- */

/// The head, with a note saying how much was cut.
fn cap(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    let rest = text.chars().count() - limit;
    format!("{head}\n... ({rest} more characters)")
}

/// The tail rather than the head: a failed build's error is at the end.
fn tail(text: &str, limit: usize) -> String {
    let total = text.chars().count();
    if total <= limit {
        return text.to_string();
    }
    let end: String = text.chars().skip(total - limit).collect();
    format!("... ({} earlier characters)\n{end}", total - limit)
}

/// A free-text field as it will be stored: a string stays a string, anything
/// else becomes the JSON a reader can make sense of.
fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

fn string_field(record: &Value, key: &str) -> Option<String> {
    record.get(key).and_then(|v| match v {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    })
}

/// What makes two failures the same failure.
///
/// The kind, the tool and the first line of the error with everything that
/// varies between runs taken out: numbers, paths, quoted strings, hex. "Exit
/// code: 1" on Monday and "Exit code: 2" on Tuesday is one signature, which is
/// what lets the tool say "seen 4 times" instead of listing four lines the
/// reader has to compare by eye.
pub fn signature_of(kind: &str, tool: Option<&str>, error: &str) -> String {
    let first = error.split('\n').next().unwrap_or_default();
    let mut out = String::with_capacity(first.len());
    let bytes = first.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = first[i..].chars().next().unwrap_or_default();

        // A quoted span is a value, whatever is inside it.
        if c == '"' || c == '\'' || c == '`' {
            let mut at = i + c.len_utf8();
            while at < bytes.len() && !first[at..].starts_with(c) {
                at += first[at..].chars().next().map(char::len_utf8).unwrap_or(1);
            }
            if at < bytes.len() {
                out.push_str("<str>");
                i = at + c.len_utf8();
                continue;
            }
        }

        // A posix path, or a Windows one with its drive letter.
        let drive = c.is_ascii_alphabetic()
            && i + 2 < bytes.len()
            && bytes[i + 1] == b':'
            && (bytes[i + 2] == b'\\' || bytes[i + 2] == b'/');
        if c == '/' || drive {
            let mut at = if drive { i + 3 } else { i + 1 };
            while at < bytes.len() {
                let next = first[at..].chars().next().unwrap_or_default();
                if next.is_whitespace() || next == '"' || next == '\'' || next == '`' {
                    break;
                }
                at += next.len_utf8();
            }
            out.push_str("<path>");
            i = at;
            continue;
        }

        if c == '0' && i + 2 < bytes.len() && bytes[i + 1] | 0x20 == b'x' {
            let mut at = i + 2;
            while at < bytes.len() && (bytes[at] as char).is_ascii_hexdigit() {
                at += 1;
            }
            if at > i + 2 {
                out.push_str("<hex>");
                i = at;
                continue;
            }
        }

        if c.is_ascii_digit() {
            let mut at = i;
            while at < bytes.len() && (bytes[at] as char).is_ascii_digit() {
                at += 1;
            }
            if at + 1 < bytes.len() && bytes[at] == b'.' && (bytes[at + 1] as char).is_ascii_digit()
            {
                at += 1;
                while at < bytes.len() && (bytes[at] as char).is_ascii_digit() {
                    at += 1;
                }
            }
            out.push_str("<n>");
            i = at;
            continue;
        }

        out.push(c);
        i += c.len_utf8();
    }

    // Whitespace collapsed last, so the placeholders above cannot leave double
    // spaces behind and split one signature into two.
    let normalised: String = out
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .take(160)
        .collect();
    format!("{kind}|{}|{normalised}", tool.unwrap_or_default())
}

/// The record, cleaned and capped, from whatever a caller handed in.
fn shape(input: &Value, secrets: &[String], counter: u64) -> Value {
    let kind = string_field(input, "kind")
        .filter(|kind| KINDS.contains(&kind.as_str()))
        .unwrap_or_else(|| "other".to_string());
    let tool = string_field(input, "tool");
    let error = redact(
        &cap(&string_field(input, "error").unwrap_or_default(), CAP_ERROR),
        secrets,
    );

    let now = jiff::Timestamp::now();
    let at = string_field(input, "at").unwrap_or_else(|| iso(now));

    let text = |key: &str, limit: usize, from_tail: bool| -> Value {
        match input.get(key).and_then(as_text) {
            None => Value::Null,
            Some(raw) => {
                let trimmed = if from_tail {
                    tail(&raw, limit)
                } else {
                    cap(&raw, limit)
                };
                Value::String(redact(&trimmed, secrets))
            }
        }
    };
    let passthrough = |key: &str| -> Value { input.get(key).cloned().unwrap_or(Value::Null) };

    json!({
        "id": format!("f-{}-{counter}", now.as_millisecond().max(0)),
        "at": at,
        "kind": kind,
        "tool": tool,
        "error": error,
        "signature": signature_of(&kind, tool.as_deref(), &error),
        "agent": passthrough("agent"),
        "agentId": passthrough("agentId"),
        "threadId": passthrough("threadId"),
        "turnId": passthrough("turnId"),
        "runId": passthrough("runId"),
        "sessionId": passthrough("sessionId"),
        "provider": passthrough("provider"),
        "model": passthrough("model"),
        "cwd": passthrough("cwd"),
        "durationMs": input.get("durationMs").and_then(Value::as_f64).map(|ms| ms.round() as i64),
        "args": text("args", CAP_ARGS, false),
        "output": text("output", CAP_OUTPUT, true),
        "stack": text("stack", CAP_STACK, false),
        "prompt": text("prompt", CAP_PROMPT, false),
        "retryable": input.get("retryable").and_then(Value::as_bool),
        "meta": input.get("meta").filter(|meta| meta.is_object()).cloned(),
    })
}

fn iso(at: jiff::Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/* -- files --------------------------------------------------------------- */

/// The day a record belongs to, as the file name says it: local time, because
/// "what went wrong on Tuesday" is the reader's Tuesday.
fn day_of(at: jiff::Timestamp) -> String {
    at.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%Y-%m-%d")
        .to_string()
}

fn day_of_iso(text: &str) -> String {
    text.parse::<jiff::Timestamp>()
        .map(day_of)
        .unwrap_or_else(|_| day_of(jiff::Timestamp::now()))
}

/// The day a file name names, and only for files that are ours: the folder is
/// the user's and anything else in it stays where it is.
fn day_of_file(name: &str) -> Option<String> {
    let day = name.strip_suffix(".jsonl")?;
    let bytes = day.as_bytes();
    let shaped = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    shaped.then(|| day.to_string())
}

/// The file names older than the window, and only ours.
fn stale(names: &[String], today: &str, keep_days: i64) -> Vec<String> {
    let Ok(date) = today.parse::<jiff::civil::Date>() else {
        return Vec::new();
    };
    let cutoff = date
        .checked_sub(jiff::Span::new().days(keep_days - 1))
        .unwrap_or(date)
        .to_string();
    names
        .iter()
        .filter(|name| day_of_file(name).is_some_and(|day| day < cutoff))
        .cloned()
        .collect()
}

fn file_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// Deletes the files past the window. Called the first time this process writes
/// into a folder rather than on a timer: the log is opened on an error path,
/// and a sweep that costs one directory listing a run is cheap enough.
fn prune(dir: &Path) {
    let today = day_of(jiff::Timestamp::now());
    for name in stale(&file_names(dir), &today, KEEP_DAYS) {
        let mut path = dir.to_path_buf();
        path.push(&name);
        let _ = std::fs::remove_file(path);
    }
}

/// Folders this process has already swept, so the sweep is one directory
/// listing per run rather than one per failure.
fn pruned() -> &'static parking_lot::Mutex<std::collections::HashSet<PathBuf>> {
    static PRUNED: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    PRUNED.get_or_init(Default::default)
}

/// A counter so two failures recorded in the same millisecond do not share an
/// id. Process-wide, which is the only scope an id has to be unique in before
/// it reaches the file.
fn next_counter() -> u64 {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Records one failure.
///
/// Never fails and never panics: every caller is already handling something
/// that went wrong, and a log that can take the turn down with it is worse than
/// no log. Returns the record as stored - or `None` when the input was not
/// worth keeping, because no kind and no error is not a failure.
pub fn record(layout: &Layout, entry: Value) -> Option<Value> {
    let has_kind = entry
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(|kind| !kind.is_empty());
    let has_error = entry
        .get("error")
        .map(|error| !matches!(error, Value::Null))
        .unwrap_or(false);
    if !has_kind && !has_error {
        return None;
    }

    // The credential values this workspace knows about, so they come out by
    // value as well as by shape. Read on the error path rather than cached: a
    // key added since the app started is exactly the one most likely to be in
    // the command that just failed.
    let record = shape(&entry, &secrets::values(layout), next_counter());

    let dir = directory(layout);
    if std::fs::create_dir_all(&dir).is_ok() {
        if pruned().lock().insert(dir.clone()) {
            prune(&dir);
        }
        let day = record
            .get("at")
            .and_then(Value::as_str)
            .map(day_of_iso)
            .unwrap_or_else(|| day_of(jiff::Timestamp::now()));
        let mut file = dir.clone();
        file.push(format!("{day}.jsonl"));
        let size = std::fs::metadata(&file).map(|meta| meta.len()).unwrap_or(0);
        // Past the cap the day stops being recorded rather than the disk
        // filling up: a crash loop writes the same line forever.
        if size < MAX_BYTES {
            if let Ok(mut handle) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)
            {
                let _ = writeln!(handle, "{record}");
            }
        }
    }

    Some(record)
}

/* -- reading ------------------------------------------------------------- */

fn read_day(dir: &Path, day: &str) -> Vec<Value> {
    let mut path = dir.to_path_buf();
    path.push(format!("{day}.jsonl"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        // A half-written last line from a crash is skipped; the rest of the
        // file is perfectly good and is what the reader came for.
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

/// Every record within `days`, newest first.
fn load(layout: &Layout, days: i64) -> Vec<Value> {
    let dir = directory(layout);
    let span = days.clamp(1, KEEP_DAYS);
    let today = day_of(jiff::Timestamp::now());
    let cutoff = today
        .parse::<jiff::civil::Date>()
        .ok()
        .and_then(|date| date.checked_sub(jiff::Span::new().days(span - 1)).ok())
        .map(|date| date.to_string())
        .unwrap_or(today);

    let mut days: Vec<String> = file_names(&dir)
        .iter()
        .filter_map(|name| day_of_file(name))
        .collect();
    days.sort();
    days.reverse();

    let mut rows = Vec::new();
    for day in days {
        if day < cutoff {
            break;
        }
        let mut of_day = read_day(&dir, &day);
        of_day.reverse();
        rows.append(&mut of_day);
    }
    rows
}

fn field(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// What a query asked for.
#[derive(Debug, Default, Clone)]
pub struct Query {
    pub days: i64,
    pub kind: Option<String>,
    pub tool: Option<String>,
    pub agent: Option<String>,
    /// Matched against the error, the arguments and the output, because "the
    /// thing with vitest in it" is how a person - or a model - actually asks.
    pub text: Option<String>,
    pub limit: usize,
}

/// The records that match, newest first, each carrying how many share its
/// signature.
#[derive(Debug, Default)]
pub struct Matches {
    pub total: usize,
    pub rows: Vec<Value>,
}

pub fn query(layout: &Layout, request: &Query) -> Matches {
    let needle = request
        .text
        .as_deref()
        .map(|text| text.trim().to_lowercase())
        .filter(|text| !text.is_empty());
    let lower = |value: &Option<String>| value.as_deref().map(str::to_lowercase);
    let (kind, tool, agent) = (
        lower(&request.kind),
        lower(&request.tool),
        lower(&request.agent),
    );

    let matched: Vec<Value> = load(layout, request.days)
        .into_iter()
        .filter(|row| {
            if let Some(wanted) = &kind {
                if field(row, "kind").to_lowercase() != *wanted {
                    return false;
                }
            }
            if let Some(wanted) = &tool {
                if field(row, "tool").to_lowercase() != *wanted {
                    return false;
                }
            }
            if let Some(wanted) = &agent {
                let names = [
                    field(row, "agent").to_lowercase(),
                    field(row, "agentId").to_lowercase(),
                ];
                if !names.contains(wanted) {
                    return false;
                }
            }
            if let Some(needle) = &needle {
                let hay = format!(
                    "{}\n{}\n{}\n{}",
                    field(row, "error"),
                    field(row, "args"),
                    field(row, "output"),
                    field(row, "tool")
                )
                .to_lowercase();
                if !hay.contains(needle) {
                    return false;
                }
            }
            true
        })
        .collect();

    let mut signatures: HashMap<String, usize> = HashMap::new();
    for row in &matched {
        *signatures.entry(field(row, "signature")).or_default() += 1;
    }

    let limit = request.limit.clamp(1, MAX_ROWS);
    let rows = matched
        .iter()
        .take(limit)
        .map(|row| {
            let mut copy = row.clone();
            let seen = signatures
                .get(&field(row, "signature"))
                .copied()
                .unwrap_or(1);
            if let Value::Object(map) = &mut copy {
                map.insert("seen".into(), json!(seen));
            }
            copy
        })
        .collect();

    Matches {
        total: matched.len(),
        rows,
    }
}

/// Counts by kind and by tool, and the most repeated signatures.
#[derive(Debug, Default)]
pub struct Summary {
    pub total: usize,
    pub by_kind: Vec<(String, usize)>,
    pub by_tool: Vec<(String, usize)>,
    /// The signatures seen more than once, most frequent first, each with one
    /// record as an example.
    pub repeated: Vec<(usize, Value)>,
}

pub fn summary(layout: &Layout, days: i64) -> Summary {
    let all = load(layout, days);
    let mut by_kind: Vec<(String, usize)> = Vec::new();
    let mut by_tool: Vec<(String, usize)> = Vec::new();
    let mut by_signature: Vec<(String, usize, Value)> = Vec::new();

    let bump = |list: &mut Vec<(String, usize)>, key: String| match list
        .iter_mut()
        .find(|(name, _)| *name == key)
    {
        Some(entry) => entry.1 += 1,
        None => list.push((key, 1)),
    };

    for row in &all {
        bump(&mut by_kind, field(row, "kind"));
        let tool = field(row, "tool");
        if !tool.is_empty() {
            bump(&mut by_tool, tool);
        }
        let signature = field(row, "signature");
        match by_signature
            .iter_mut()
            .find(|(key, _, _)| *key == signature)
        {
            Some(entry) => entry.1 += 1,
            None => by_signature.push((signature, 1, row.clone())),
        }
    }

    by_kind.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    by_tool.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let mut repeated: Vec<(usize, Value)> = by_signature
        .into_iter()
        .filter(|(_, count, _)| *count > 1)
        // The example is the newest of its kind, because `load` is newest
        // first: the reader wants the most recent shape of a recurring
        // failure, not the first time it ever happened.
        .map(|(_, count, example)| (count, example))
        .collect();
    repeated.sort_by_key(|(count, _)| std::cmp::Reverse(*count));
    repeated.truncate(10);

    Summary {
        total: all.len(),
        by_kind,
        by_tool,
        repeated,
    }
}

/* -- the tool ------------------------------------------------------------ */

/// Reading the failure log.
///
/// A tool rather than a section of the prompt for the same reason skills are:
/// the log can hold months of records and the moment any of it is relevant is
/// rare. When it is relevant - a build that fails the same way it failed last
/// week, a person asking "why does this keep happening", an agent about to try
/// a command that has failed three times before - it is the most useful text in
/// the workspace.
///
/// The output leads with what repeats. One failure is an accident; the same
/// failure four times is a fact about the project, and the reader should meet
/// the fact before the list.
#[derive(Debug)]
pub struct FailuresTool {
    layout: Layout,
}

impl FailuresTool {
    pub fn new(layout: Layout) -> Self {
        Self { layout }
    }
}

fn first_line(text: &str) -> &str {
    text.split('\n')
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
}

fn truncate(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// `2026-09-12 14:07`, in the reader's own time zone.
fn when(iso_at: &str) -> String {
    match iso_at.parse::<jiff::Timestamp>() {
        Ok(at) => at
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y-%m-%d %H:%M")
            .to_string(),
        Err(_) => iso_at.to_string(),
    }
}

/// One record, in a few lines a model can act on.
fn render_row(row: &Value, verbose: bool) -> String {
    let tool = field(row, "tool");
    let agent = field(row, "agent");
    let seen = row.get("seen").and_then(Value::as_u64).unwrap_or(1);
    let mut head = vec![
        format!("[{}]", when(&field(row, "at"))),
        if tool.is_empty() {
            field(row, "kind")
        } else {
            format!("{}:{tool}", field(row, "kind"))
        },
    ];
    if !agent.is_empty() {
        head.push(format!("agent={agent}"));
    }
    if seen > 1 {
        head.push(format!("seen {seen}x"));
    }

    let error = field(row, "error");
    let mut lines = vec![head.join("  ")];
    lines.push(format!(
        "  {}",
        if first_line(&error).is_empty() {
            "(no message)"
        } else {
            first_line(&error)
        }
    ));

    let cwd = field(row, "cwd");
    if !cwd.is_empty() {
        lines.push(format!("  in {cwd}"));
    }

    let args = field(row, "args");
    if !args.is_empty() {
        lines.push(format!(
            "  args: {}",
            if verbose {
                args.clone()
            } else {
                truncate(first_line(&args), 300)
            }
        ));
    }

    let output = field(row, "output");
    if !output.is_empty() {
        if verbose {
            let indented: Vec<String> = output
                .split('\n')
                .map(|line| format!("    {line}"))
                .collect();
            lines.push(format!("  output:\n{}", indented.join("\n")));
        } else {
            let trimmed = output.trim();
            let last: Vec<&str> = trimmed.split('\n').rev().take(2).collect();
            let ends = last.into_iter().rev().collect::<Vec<_>>().join(" | ");
            if !ends.is_empty() {
                lines.push(format!("  output ends: {}", truncate(&ends, 300)));
            }
        }
    }

    lines.join("\n")
}

fn text_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[async_trait]
impl Tool for FailuresTool {
    fn id(&self) -> &str {
        "failures"
    }

    fn description(&self) -> &str {
        "Read the log of things that have gone wrong before: tool calls that failed,\n\
         commands that exited non-zero, refused permissions, provider errors, routine\n\
         and subagent failures, and crashes. Every record has the arguments, the\n\
         error, the tail of the output and the folder it happened in.\n\
         \n\
         Use it when something fails the same way twice, when the user asks why\n\
         something keeps happening or what went wrong earlier, and before retrying a\n\
         command that you suspect has failed before - the log will tell you how it\n\
         failed last time and what was tried. Do not use it for a failure you have\n\
         just seen once and can fix from the message in front of you.\n\
         \n\
         With no filters it summarises the last two weeks: what repeats, then the\n\
         newest records. Narrow with `query` (matched against error, arguments and\n\
         output), `kind`, `tool` or `agent`. Set `verbose` to read full output."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Text to look for in the error, the arguments or the output"
                },
                "kind": {
                    "type": "string",
                    "enum": KINDS,
                    "description": "Only this kind of failure"
                },
                "tool": {
                    "type": "string",
                    "description": "Only failures of this tool, e.g. shell, edit, or an MCP tool id"
                },
                "agent": { "type": "string", "description": "Only failures by this agent, by name" },
                "days": { "type": "integer", "description": "How far back to look. Default 14, maximum 90." },
                "limit": {
                    "type": "integer",
                    "description": "How many records to return. Default 20, maximum 50."
                },
                "verbose": {
                    "type": "boolean",
                    "description": "Include the full captured output of each record",
                    "default": false
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        PermissionRequest::new("failures", "log").with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let bits: Vec<String> = ["query", "kind", "tool", "agent"]
            .iter()
            .filter_map(|key| text_arg(args, key))
            .collect();
        Some(if bits.is_empty() {
            "Recent failures".to_string()
        } else {
            format!("Failures: {}", bits.join(", "))
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let days = args
            .get("days")
            .and_then(Value::as_i64)
            .filter(|days| *days > 0)
            .unwrap_or(14)
            .clamp(1, KEEP_DAYS);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .filter(|limit| *limit > 0)
            .unwrap_or(20)
            .clamp(1, MAX_ROWS as u64) as usize;
        let request = Query {
            days,
            kind: text_arg(&args, "kind"),
            tool: text_arg(&args, "tool"),
            agent: text_arg(&args, "agent"),
            text: text_arg(&args, "query"),
            limit,
        };
        let filtered = request.kind.is_some()
            || request.tool.is_some()
            || request.agent.is_some()
            || request.text.is_some();
        let verbose = args
            .get("verbose")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let result = query(&self.layout, &request);
        let where_it_is = directory(&self.layout).display().to_string();

        if result.total == 0 {
            return Ok(ToolOutcome {
                title: Some("No failures found".to_string()),
                output: if filtered {
                    format!(
                        "Nothing in the last {days} days matches that. The log is at {where_it_is}."
                    )
                } else {
                    format!("Nothing has failed in the last {days} days.")
                },
                metadata: Some(json!({ "total": 0, "days": days, "path": where_it_is })),
                images: Vec::new(),
            });
        }

        let mut lines: Vec<String> = Vec::new();
        if filtered {
            lines.push(format!(
                "{} matching failure{} in the last {days} days, newest first:",
                result.total,
                if result.total == 1 { "" } else { "s" }
            ));
        } else {
            let digest = summary(&self.layout, days);
            let kinds = digest
                .by_kind
                .iter()
                .map(|(kind, count)| format!("{kind} {count}"))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "{} failure{} in the last {days} days: {kinds}.",
                digest.total,
                if digest.total == 1 { "" } else { "s" }
            ));
            // Which tools, when any of them were tools. "shell 12, edit 1" is
            // the line that turns "things have been failing" into somewhere to
            // look, and it costs nothing - the count is already computed.
            if !digest.by_tool.is_empty() {
                let tools = digest
                    .by_tool
                    .iter()
                    .map(|(tool, count)| format!("{tool} {count}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("By tool: {tools}."));
            }
            if !digest.repeated.is_empty() {
                lines.push(String::new());
                lines.push("Repeats, most frequent first:".to_string());
                for (count, example) in &digest.repeated {
                    let tool = field(example, "tool");
                    lines.push(format!(
                        "- {count}x  {}{}  {}",
                        field(example, "kind"),
                        if tool.is_empty() {
                            String::new()
                        } else {
                            format!(":{tool}")
                        },
                        truncate(first_line(&field(example, "error")), 200)
                    ));
                }
            }
            lines.push(String::new());
            lines.push(format!("Newest {}:", limit.min(result.total)));
        }

        lines.push(String::new());
        lines.push(
            result
                .rows
                .iter()
                .map(|row| render_row(row, verbose))
                .collect::<Vec<_>>()
                .join("\n\n"),
        );

        if result.total > result.rows.len() {
            lines.push(String::new());
            lines.push(format!(
                "{} more not shown. Narrow with query, kind, tool or agent, or raise limit.",
                result.total - result.rows.len()
            ));
        }
        lines.push(String::new());
        lines.push(format!(
            "The raw log is {where_it_is}, one JSON line per failure, one file per day."
        ));

        let rows: Vec<Value> = result
            .rows
            .iter()
            .map(|row| {
                json!({
                    "id": field(row, "id"),
                    "at": field(row, "at"),
                    "kind": field(row, "kind"),
                    "tool": row.get("tool").cloned().unwrap_or(Value::Null),
                    "agent": row.get("agent").cloned().unwrap_or(Value::Null),
                    "error": first_line(&field(row, "error")),
                    "seen": row.get("seen").cloned().unwrap_or(json!(1)),
                })
            })
            .collect();

        Ok(ToolOutcome {
            title: Some(if filtered {
                format!("{} matching failures", result.total)
            } else {
                format!("{} failures", result.total)
            }),
            output: lines.join("\n"),
            metadata: Some(json!({
                "total": result.total,
                "shown": result.rows.len(),
                "days": days,
                "path": where_it_is,
                "rows": rows,
            })),
            images: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::permission::Action;
    use inertia_core::tool::{Decision, PermissionGate};
    use std::sync::Arc;

    #[derive(Debug)]
    struct Allow;

    #[async_trait]
    impl PermissionGate for Allow {
        async fn ask(&self, _request: &PermissionRequest) -> Result<Decision> {
            Ok(Decision::Allow)
        }

        async fn verdict(&self, _key: &str, _target: &str) -> Action {
            Action::Allow
        }
    }

    fn ctx() -> ToolContext {
        ToolContext {
            root: std::path::PathBuf::from("."),
            session: SessionId::from_existing("t1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(Allow),
        }
    }

    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        (dir, layout)
    }

    fn all(layout: &Layout) -> Matches {
        query(
            layout,
            &Query {
                days: 14,
                limit: 20,
                ..Default::default()
            },
        )
    }

    #[test]
    fn one_json_line_lands_in_a_file_named_after_the_day() {
        let (_dir, layout) = workspace();
        let entry = record(
            &layout,
            json!({ "kind": "tool", "tool": "shell", "error": "Exit code: 1\n\nnot found" }),
        )
        .expect("a record");

        let mut file = directory(&layout);
        file.push(format!(
            "{}.jsonl",
            day_of_iso(entry["at"].as_str().unwrap_or_default())
        ));
        let text = std::fs::read_to_string(&file).expect("the day's file");
        let lines: Vec<&str> = text.trim().split('\n').collect();
        assert_eq!(lines.len(), 1);
        let stored: Value = serde_json::from_str(lines[0]).expect("a JSON line");
        assert_eq!(stored["id"], entry["id"]);
        assert_eq!(stored["kind"], json!("tool"));
        assert_eq!(stored["tool"], json!("shell"));
    }

    #[test]
    fn a_call_with_nothing_in_it_is_not_a_failure() {
        let (_dir, layout) = workspace();
        assert!(record(&layout, json!({})).is_none());
        assert!(record(&layout, Value::Null).is_none());
        assert_eq!(all(&layout).total, 0);
    }

    #[test]
    fn an_unknown_kind_is_kept_as_other_rather_than_dropped() {
        let (_dir, layout) = workspace();
        let entry = record(&layout, json!({ "kind": "made-up", "error": "boom" })).expect("kept");
        assert_eq!(entry["kind"], json!("other"));
    }

    #[test]
    fn credentials_are_taken_out_by_shape() {
        let (_dir, layout) = workspace();
        let entry = record(
            &layout,
            json!({
                "kind": "tool",
                "tool": "shell",
                "error": "curl failed",
                "args": { "command": "curl -H 'Authorization: Bearer abcdefghijklmnopqrstuvwxyz1234' https://x" },
                "output": "using key sk-ant-api03-abcdefghijklmnopqrstuvwxyz and ghp_abcdefghijklmnopqrstuvwxyz1234",
            }),
        )
        .expect("a record");

        let args = entry["args"].as_str().unwrap_or_default();
        assert!(!args.contains("abcdefghijklmnopqrstuvwxyz1234"), "{args}");
        assert!(args.contains("Bearer [REDACTED]"), "{args}");
        let output = entry["output"].as_str().unwrap_or_default();
        assert!(!output.contains("sk-ant-api03"), "{output}");
        assert!(!output.contains("ghp_"), "{output}");
        assert!(output.contains(REDACTED), "{output}");
    }

    #[test]
    fn a_key_and_its_value_go_whatever_the_key_is_called() {
        let text = redact(
            "api_key=\"abc123def456\" token: zzz999yyy888 password=hunter22",
            &[],
        );
        assert!(!text.contains("abc123def456"), "{text}");
        assert!(!text.contains("zzz999yyy888"), "{text}");
        assert!(!text.contains("hunter22"), "{text}");
    }

    #[test]
    fn the_values_this_workspace_knows_are_secret_go_too() {
        let (_dir, layout) = workspace();
        inertia_store::secrets::set(&layout, "GITHUB_PAT", "s3cr3t-value-here", "")
            .expect("the secret was stored");
        let entry = record(
            &layout,
            json!({ "kind": "tool", "error": "posting s3cr3t-value-here failed" }),
        )
        .expect("a record");
        assert_eq!(entry["error"], json!("posting [REDACTED] failed"));
    }

    #[test]
    fn ordinary_text_is_left_alone() {
        assert_eq!(
            redact("Exit code: 1\nCannot find module 'x'", &[]),
            "Exit code: 1\nCannot find module 'x'"
        );
    }

    #[test]
    fn the_tail_of_the_output_survives_because_that_is_where_the_error_is() {
        let (_dir, layout) = workspace();
        let output = format!("{}THE END", "a".repeat(6000));
        let entry = record(
            &layout,
            json!({ "kind": "tool", "error": "x", "output": output }),
        )
        .expect("kept");
        let stored = entry["output"].as_str().unwrap_or_default();
        assert!(stored.ends_with("THE END"), "{}", &stored[..40]);
        assert!(stored.starts_with("..."));
        assert!(stored.chars().count() < 4200);
    }

    #[test]
    fn the_head_of_the_arguments_survives_and_says_how_much_was_cut() {
        let (_dir, layout) = workspace();
        let entry = record(
            &layout,
            json!({ "kind": "tool", "error": "x", "args": { "command": "b".repeat(9000) } }),
        )
        .expect("kept");
        let stored = entry["args"].as_str().unwrap_or_default();
        assert!(stored.chars().count() < 4200);
        assert!(stored.contains("more characters"), "{stored}");
    }

    #[test]
    fn the_same_failure_with_different_paths_and_numbers_is_one_signature() {
        let a = signature_of(
            "tool",
            Some("shell"),
            "Exit code: 1\n\nCannot find /home/a/x.js",
        );
        let b = signature_of(
            "tool",
            Some("shell"),
            "Exit code: 2\n\nCannot find C:\\Users\\b\\y.js",
        );
        assert_eq!(a, b);
        assert_eq!(a, "tool|shell|exit code: <n>");

        // And a path or a quoted name inside the line the reader sees.
        assert_eq!(
            signature_of("tool", None, "Cannot find module 'left-pad' in /app/src"),
            signature_of("tool", None, "Cannot find module 'right-pad' in /srv/app")
        );
    }

    #[test]
    fn different_tools_are_kept_apart() {
        assert_ne!(
            signature_of("tool", Some("shell"), "timed out"),
            signature_of("tool", Some("read"), "timed out")
        );
    }

    #[test]
    fn repeats_are_counted() {
        let (_dir, layout) = workspace();
        for i in 0..3 {
            record(
                &layout,
                json!({ "kind": "tool", "tool": "shell", "error": format!("Exit code: {i}\n\nvitest not found") }),
            );
        }
        record(
            &layout,
            json!({ "kind": "tool", "tool": "shell", "error": "something else entirely" }),
        );

        let found = query(
            &layout,
            &Query {
                days: 14,
                limit: 20,
                text: Some("vitest".into()),
                ..Default::default()
            },
        );
        // The error's own text is searched, not only the arguments.
        assert_eq!(found.total, 3);
        assert_eq!(found.rows[0]["seen"], json!(3));

        let digest = summary(&layout, 14);
        assert_eq!(digest.total, 4);
        assert_eq!(digest.by_kind[0], ("tool".to_string(), 4));
        assert_eq!(digest.repeated[0].0, 3);
    }

    #[test]
    fn filters_narrow_by_kind_tool_and_agent_newest_first() {
        let (_dir, layout) = workspace();
        record(
            &layout,
            json!({ "kind": "tool", "tool": "shell", "agent": "Atlas", "error": "one" }),
        );
        record(
            &layout,
            json!({ "kind": "permission", "tool": "edit", "agent": "Mercury", "error": "two" }),
        );
        record(
            &layout,
            json!({ "kind": "tool", "tool": "edit", "agent": "Atlas", "error": "three" }),
        );

        let with = |field: &str, value: &str| {
            let mut request = Query {
                days: 14,
                limit: 20,
                ..Default::default()
            };
            match field {
                "kind" => request.kind = Some(value.into()),
                "tool" => request.tool = Some(value.into()),
                _ => request.agent = Some(value.into()),
            }
            query(&layout, &request).total
        };
        assert_eq!(with("kind", "tool"), 2);
        assert_eq!(with("tool", "edit"), 2);
        assert_eq!(with("agent", "atlas"), 2);

        let errors: Vec<String> = all(&layout)
            .rows
            .iter()
            .map(|row| field(row, "error"))
            .collect();
        assert_eq!(errors, vec!["three", "two", "one"]);
    }

    #[test]
    fn the_arguments_and_the_output_are_searched_as_well_as_the_error() {
        let (_dir, layout) = workspace();
        record(
            &layout,
            json!({ "kind": "tool", "tool": "shell", "error": "Exit code: 1", "args": { "command": "npm run lint" } }),
        );
        record(
            &layout,
            json!({ "kind": "tool", "tool": "shell", "error": "Exit code: 1", "output": "ReferenceError: foo" }),
        );
        let find = |text: &str| {
            query(
                &layout,
                &Query {
                    days: 14,
                    limit: 20,
                    text: Some(text.into()),
                    ..Default::default()
                },
            )
            .total
        };
        assert_eq!(find("lint"), 1);
        assert_eq!(find("referenceerror"), 1);
    }

    #[test]
    fn a_half_written_last_line_from_a_crash_costs_that_line_and_no_more() {
        let (_dir, layout) = workspace();
        let entry = record(&layout, json!({ "kind": "tool", "error": "fine" })).expect("kept");
        let mut file = directory(&layout);
        file.push(format!(
            "{}.jsonl",
            day_of_iso(entry["at"].as_str().unwrap_or_default())
        ));
        let mut handle = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .expect("the day's file");
        write!(handle, "{{\"kind\":\"tool\",\"er").expect("a half line");
        drop(handle);

        assert_eq!(all(&layout).total, 1);
    }

    #[test]
    fn only_our_own_files_past_the_window_are_named() {
        let names: Vec<String> = [
            "2026-05-01.jsonl",
            "2026-08-30.jsonl",
            "2026-09-04.jsonl",
            "notes.txt",
        ]
        .iter()
        .map(|name| name.to_string())
        .collect();
        assert_eq!(stale(&names, "2026-09-04", 90), vec!["2026-05-01.jsonl"]);
    }

    #[test]
    fn the_files_past_the_window_are_deleted_and_the_users_are_not() {
        let (_dir, layout) = workspace();
        let dir = directory(&layout);
        std::fs::create_dir_all(&dir).expect("the folder");
        let mut old = dir.clone();
        old.push("2020-01-01.jsonl");
        std::fs::write(&old, "{}\n").expect("an old file");
        let mut theirs = dir.clone();
        theirs.push("keep.txt");
        std::fs::write(&theirs, "mine").expect("their file");

        prune(&dir);
        assert!(!old.exists());
        assert!(theirs.exists());
    }

    #[tokio::test]
    async fn with_nothing_wrong_the_tool_says_so_rather_than_printing_a_header() {
        let (_dir, layout) = workspace();
        let out = FailuresTool::new(layout)
            .execute(json!({}), &ctx())
            .await
            .expect("the tool ran");
        assert_eq!(out.title.as_deref(), Some("No failures found"));
        assert!(out
            .output
            .contains("Nothing has failed in the last 14 days"));
        assert_eq!(
            out.metadata.and_then(|m| m.get("total").cloned()),
            Some(json!(0))
        );
    }

    #[tokio::test]
    async fn the_unfiltered_report_leads_with_what_repeats() {
        let (_dir, layout) = workspace();
        for i in 0..3 {
            record(
                &layout,
                json!({
                    "kind": "tool",
                    "tool": "shell",
                    "agent": "Atlas",
                    "cwd": "/app",
                    "error": format!("Exit code: {i}\n\nvitest not found"),
                    "args": { "command": "npm test" },
                    "output": "ERR! missing script: test",
                }),
            );
        }
        record(
            &layout,
            json!({ "kind": "provider", "error": "500 from upstream" }),
        );

        let out = FailuresTool::new(layout.clone())
            .execute(json!({}), &ctx())
            .await
            .expect("the tool ran");

        let text = out.output;
        assert!(
            text.starts_with("4 failures in the last 14 days: tool 3, provider 1."),
            "{text}"
        );
        assert!(text.contains("Repeats, most frequent first:"), "{text}");
        assert!(text.contains("- 3x  tool:shell  Exit code:"), "{text}");
        // The fact before the list, and then the list.
        assert!(
            text.find("Repeats").unwrap_or(usize::MAX) < text.find("Newest").unwrap_or(0),
            "{text}"
        );
        assert!(text.contains("seen 3x"), "{text}");
        assert!(text.contains("agent=Atlas"), "{text}");
        assert!(text.contains("  in /app"), "{text}");
        assert!(text.contains("args: {\"command\":\"npm test\"}"), "{text}");
        assert!(
            text.contains("output ends: ERR! missing script: test"),
            "{text}"
        );
        assert!(text.contains("one JSON line per failure"), "{text}");

        let metadata = out.metadata.expect("metadata");
        assert_eq!(metadata["total"], json!(4));
        assert_eq!(metadata["shown"], json!(4));
        // Newest first: the provider failure, then the three that repeat.
        assert_eq!(metadata["rows"][0]["kind"], json!("provider"));
        assert_eq!(metadata["rows"][0]["seen"], json!(1));
        assert_eq!(metadata["rows"][1]["seen"], json!(3));
    }

    #[tokio::test]
    async fn a_filtered_report_is_the_matches_and_nothing_else() {
        let (_dir, layout) = workspace();
        record(
            &layout,
            json!({ "kind": "tool", "tool": "shell", "error": "Exit code: 1\n\nvitest not found" }),
        );
        record(
            &layout,
            json!({ "kind": "provider", "error": "500 from upstream" }),
        );

        let out = FailuresTool::new(layout)
            .execute(json!({ "kind": "provider" }), &ctx())
            .await
            .expect("the tool ran");

        assert_eq!(out.title.as_deref(), Some("1 matching failures"));
        assert!(
            out.output
                .starts_with("1 matching failure in the last 14 days, newest first:"),
            "{}",
            out.output
        );
        assert!(!out.output.contains("Repeats"), "{}", out.output);
        assert!(out.output.contains("500 from upstream"), "{}", out.output);
    }

    #[tokio::test]
    async fn verbose_reads_the_whole_captured_output() {
        let (_dir, layout) = workspace();
        record(
            &layout,
            json!({
                "kind": "tool",
                "tool": "shell",
                "error": "Exit code: 1",
                "output": "first line\nsecond line\nthird line",
            }),
        );
        let out = FailuresTool::new(layout)
            .execute(json!({ "tool": "shell", "verbose": true }), &ctx())
            .await
            .expect("the tool ran");
        assert!(out.output.contains("    first line"), "{}", out.output);
        assert!(out.output.contains("    third line"), "{}", out.output);
    }

    #[tokio::test]
    async fn a_limit_says_how_many_were_left_out() {
        let (_dir, layout) = workspace();
        for i in 0..5 {
            record(
                &layout,
                json!({ "kind": "tool", "tool": "shell", "error": format!("failure number {i}") }),
            );
        }
        let out = FailuresTool::new(layout)
            .execute(json!({ "tool": "shell", "limit": 2 }), &ctx())
            .await
            .expect("the tool ran");
        assert!(out.output.contains("3 more not shown"), "{}", out.output);
    }
}

//! Turning a server's diagnostics into something worth spending context on.
//!
//! The value of this whole subsystem is that a mistake comes back on the turn
//! that made it. That value is destroyed twice over if the report is enormous:
//! a file mid-refactor can have four hundred errors, all of them downstream of
//! one missing import, and pasting four hundred lines into the tool result
//! spends the context window on the same fact repeated.
//!
//! So everything here is a cap. Errors and warnings only - a hint is a style
//! suggestion and an information is a note, and neither is something the model
//! broke. Twenty per file, because past twenty the pattern is already obvious.
//! A short line each, because a Rust borrow-checker message runs to a paragraph
//! with an ASCII diagram in it. And a ceiling on the block as a whole, because
//! twenty long messages is still too much.
//!
//! Nothing here spawns, reads or waits. It is a formatter, so it can be tested
//! by handing it a list.

use std::path::Path;

use serde_json::Value;

/// LSP severity 1 and 2. Three is information and four is a hint; both go.
pub const ERROR: u64 = 1;
pub const WARNING: u64 = 2;

/// Diagnostics reported for one file before the rest become a count.
pub const MAX_PER_FILE: usize = 20;

/// One diagnostic's message. rust-analyzer and clang will happily hand back a
/// multi-line explanation with a rendered code frame in it, and the first
/// sentence is the part that says what is wrong.
pub const MAX_MESSAGE_CHARS: usize = 240;

/// The whole `<diagnostics>` block for one file, however few messages it took.
pub const MAX_BLOCK_CHARS: usize = 4000;

fn severity(item: &Value) -> u64 {
    item.get("severity").and_then(Value::as_u64).unwrap_or(ERROR)
}

fn start(item: &Value, field: &str) -> u64 {
    item.pointer(&format!("/range/start/{field}"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// A message on one line.
///
/// Line and character are zero-based on the wire and one-based everywhere a
/// person or a model reads them, which is the off-by-one that makes a model
/// "fix" the line above the error.
pub fn pretty(item: &Value) -> String {
    let label = match item.get("severity").and_then(Value::as_u64) {
        Some(WARNING) => "WARN",
        _ => "ERROR",
    };
    let line = start(item, "line") + 1;
    let column = start(item, "character") + 1;

    // A newline inside the message would break the one-per-line shape the model
    // reads, and the continuation lines are almost always the rendered frame.
    let raw = item
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let first = raw.split('\n').next().unwrap_or("").trim();
    let message = if first.chars().count() > MAX_MESSAGE_CHARS {
        let cut: String = first.chars().take(MAX_MESSAGE_CHARS).collect();
        format!("{cut}...")
    } else {
        first.to_string()
    };

    format!("{label} [{line}:{column}] {message}")
}

/// Errors first, then by position, so the most useful line is never the one
/// cut.
fn order(items: &[Value]) -> Vec<&Value> {
    let mut relevant: Vec<&Value> = items
        .iter()
        .filter(|item| {
            let level = severity(item);
            level == ERROR || level == WARNING
        })
        .collect();
    relevant.sort_by(|a, b| {
        severity(a)
            .cmp(&severity(b))
            .then_with(|| start(a, "line").cmp(&start(b, "line")))
    });
    relevant
}

/// The block for one file, or `""` when there is nothing the model needs to
/// know.
///
/// An empty string rather than a message about there being no errors: a clean
/// file is the normal case, and a line saying so on every single edit is noise
/// the model learns to skip past, taking the real reports with it.
pub fn report(file: &Path, items: &[Value]) -> String {
    let relevant = order(items);
    if relevant.is_empty() {
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut shown = 0usize;

    for item in relevant.iter().take(MAX_PER_FILE) {
        let line = pretty(item);
        // Stopping before the cap is exceeded rather than after keeps the
        // promise about the size of what came back, not the size of what was
        // dropped.
        if used + line.len() + 1 > MAX_BLOCK_CHARS && shown > 0 {
            break;
        }
        used += line.len() + 1;
        shown += 1;
        lines.push(line);
    }

    let hidden = relevant.len() - shown;
    if hidden > 0 {
        lines.push(format!("... and {hidden} more"));
    }

    format!(
        "<diagnostics file=\"{}\">\n{}\n</diagnostics>",
        file.display(),
        lines.join("\n")
    )
}

/// Whether a set of diagnostics has anything worth reporting at all.
pub fn interesting(items: &[Value]) -> bool {
    items.iter().any(|item| {
        let level = severity(item);
        level == ERROR || level == WARNING
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(severity: u64, line: u64, message: &str) -> Value {
        json!({
            "severity": severity,
            "range": { "start": { "line": line, "character": 4 }, "end": { "line": line, "character": 9 } },
            "message": message,
        })
    }

    #[test]
    fn a_clean_file_says_nothing_at_all() {
        assert_eq!(report(Path::new("a.ts"), &[]), "");
        // A hint and an information are not things the model broke.
        assert_eq!(
            report(Path::new("a.ts"), &[item(3, 0, "note"), item(4, 1, "style")]),
            ""
        );
        assert!(!interesting(&[item(4, 0, "style")]));
    }

    /// The off-by-one that makes a model fix the line above the error.
    #[test]
    fn positions_are_one_based() {
        assert_eq!(pretty(&item(1, 0, "boom")), "ERROR [1:5] boom");
        assert_eq!(pretty(&item(2, 41, "careful")), "WARN [42:5] careful");
    }

    #[test]
    fn errors_come_before_warnings() {
        let items = vec![item(2, 9, "later"), item(1, 30, "worse")];
        let block = report(Path::new("a.ts"), &items);
        let worse = block.find("worse").unwrap();
        let later = block.find("later").unwrap();
        assert!(worse < later, "got {block}");
    }

    #[test]
    fn only_the_first_line_of_a_message_survives() {
        let long = item(1, 0, "expected ;\n  --> a.rs:1:1\n   |\n 1 | let x\n");
        assert_eq!(pretty(&long), "ERROR [1:5] expected ;");
    }

    #[test]
    fn a_very_long_message_is_cut_with_an_ellipsis() {
        let long = item(1, 0, &"x".repeat(400));
        let line = pretty(&long);
        assert!(line.ends_with("..."), "got {line}");
        assert!(line.len() < 400);
    }

    #[test]
    fn a_flood_is_capped_and_says_how_many_were_dropped() {
        let items: Vec<Value> = (0..50).map(|i| item(1, i, "broken")).collect();
        let block = report(Path::new("a.ts"), &items);
        assert_eq!(block.matches("ERROR [").count(), MAX_PER_FILE);
        assert!(block.contains("... and 30 more"), "got {block}");
    }

    #[test]
    fn the_block_names_the_file_it_is_about() {
        let block = report(Path::new("src/a.ts"), &[item(1, 0, "boom")]);
        assert!(block.starts_with("<diagnostics file=\""), "got {block}");
        assert!(block.ends_with("</diagnostics>"), "got {block}");
    }

    /// Twenty long messages is still too much to paste into a turn.
    #[test]
    fn the_whole_block_has_a_ceiling_of_its_own() {
        let items: Vec<Value> = (0..MAX_PER_FILE as u64)
            .map(|i| item(1, i, &"y".repeat(MAX_MESSAGE_CHARS)))
            .collect();
        let block = report(Path::new("a.ts"), &items);
        assert!(block.len() < MAX_BLOCK_CHARS + 200, "got {} bytes", block.len());
        assert!(block.contains("more"), "it must say what was dropped");
    }
}

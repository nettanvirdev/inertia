//! The match ladder shared by `edit` and `patch`.
//!
//! A model asked to replace text reproduces it from memory, and gets it very
//! slightly wrong: a tab where the file has spaces, a collapsed blank line, a
//! literal `\n` instead of a newline. Refusing all of those costs a full round
//! trip to re-read a file the model has already read.
//!
//! So this tries progressively more forgiving interpretations, in order,
//! stopping at the first that matches anything. What it never does is *guess
//! between candidates*: if a rung produces more than one match and the caller
//! did not ask for `replace_all`, the edit is refused. Ambiguity resolved by
//! picking the first match is how an agent silently edits the wrong function.

/// Which interpretation succeeded. Reported back to the model so a habitually
/// sloppy needle is visible rather than silently tolerated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Literal substring.
    Exact,
    /// Same lines, ignoring each line's leading and trailing whitespace.
    LineTrimmed,
    /// Every run of whitespace collapsed to one space, on both sides.
    WhitespaceNormalized,
    /// Same lines with a uniform common indent stripped from both sides, so
    /// relative indentation inside the block still has to match.
    IndentationFlexible,
    /// The needle contained literal `\n`-style escapes, which were unescaped
    /// before retrying.
    Escaped,
    /// Anchored on a unique first line and scanned forward to the last.
    BlockAnchor,
}

impl Strategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::LineTrimmed => "line-trimmed",
            Self::WhitespaceNormalized => "whitespace-normalized",
            Self::IndentationFlexible => "indentation-flexible",
            Self::Escaped => "escaped",
            Self::BlockAnchor => "block-anchor",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Replacement {
    pub content: String,
    pub strategy: Strategy,
    pub count: usize,
}

/// Byte range of one candidate match.
type Span = (usize, usize);

/// One rung: a name, and the function that finds candidates for it.
type Rung = (Strategy, fn(&str, &str) -> Vec<Span>);

/// Replaces `old` with `new` in `content`.
///
/// Errors are English sentences addressed to the model, because it is the one
/// that has to act on them.
pub fn replace(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Replacement, String> {
    if old == new {
        return Err(
            "oldString and newString are identical, so there is nothing to change.".into(),
        );
    }
    if old.is_empty() {
        return Err("oldString is empty. Use the write tool to create a file.".into());
    }

    let ladder: &[Rung] = &[
        (Strategy::Exact, find_exact),
        (Strategy::LineTrimmed, find_line_trimmed),
        (Strategy::WhitespaceNormalized, find_whitespace_normalized),
        (Strategy::IndentationFlexible, find_indentation_flexible),
        (Strategy::Escaped, find_escaped),
        (Strategy::BlockAnchor, find_block_anchor),
    ];

    for (strategy, find) in ladder {
        let spans = collapse_overlaps(find(content, old));
        if spans.is_empty() {
            continue;
        }

        // A rung that matched something wildly larger than what was asked for
        // has almost certainly anchored onto the wrong thing.
        if let Some(span) = spans.iter().find(|span| disproportionate(content, **span, old)) {
            let _ = span;
            return Err("The text matched a span much larger than what you asked to \
                 replace. Read the file again and give the exact text, including \
                 whitespace."
                .into());
        }

        if spans.len() > 1 && !replace_all {
            return Err(format!(
                "Found {} matches for that text. Include enough surrounding lines to \
                 identify the one you mean, or set replaceAll to true.",
                spans.len()
            ));
        }

        let spans = if replace_all {
            spans
        } else {
            vec![spans[0]]
        };

        let mut result = String::with_capacity(content.len());
        let mut cursor = 0;
        for (start, end) in &spans {
            result.push_str(&content[cursor..*start]);
            result.push_str(new);
            cursor = *end;
        }
        result.push_str(&content[cursor..]);

        return Ok(Replacement {
            content: result,
            strategy: *strategy,
            count: spans.len(),
        });
    }

    Err("Could not find that text in the file. Read the file again - it may have \
         changed, and the text must match what is actually there."
        .into())
}

/// Rejects a match wildly larger than the needle.
///
/// Exists because the block-anchor rung can otherwise swallow a two-hundred
/// line function when a three-line needle's first and last lines happen to
/// repeat.
fn disproportionate(content: &str, (start, end): Span, old: &str) -> bool {
    let matched = &content[start..end];
    let matched_lines = matched.lines().count();
    let old_lines = old.lines().count();

    if matched_lines >= (old_lines + 3).max(old_lines * 2) {
        return true;
    }

    if old_lines > 1 {
        let asked = old.len();
        if matched.len() > (asked + 500).max(asked * 4) {
            return true;
        }
    }

    false
}

/// Keeps only non-overlapping matches, earliest first.
fn collapse_overlaps(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by_key(|(start, _)| *start);
    let mut kept: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        if kept.last().is_none_or(|(_, end)| span.0 >= *end) {
            kept.push(span);
        }
    }
    kept
}

// ── the rungs ───────────────────────────────────────────────────────────

fn find_exact(content: &str, old: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(at) = content[from..].find(old) {
        let start = from + at;
        spans.push((start, start + old.len()));
        from = start + old.len();
    }
    spans
}

/// Byte ranges of each line, excluding its newline.
fn line_spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start = 0;
    for (at, ch) in text.char_indices() {
        if ch == '\n' {
            let mut end = at;
            // Treat CRLF as one terminator, so a trimmed comparison does not
            // see a stray carriage return.
            if end > start && text.as_bytes()[end - 1] == b'\r' {
                end -= 1;
            }
            spans.push((start, end));
            start = at + 1;
        }
    }
    if start <= text.len() {
        spans.push((start, text.len()));
    }
    spans
}

/// Matches line-by-line with each line's own leading and trailing whitespace
/// ignored.
fn find_line_trimmed(content: &str, old: &str) -> Vec<Span> {
    let needle: Vec<&str> = old.lines().map(str::trim).collect();
    if needle.is_empty() {
        return Vec::new();
    }

    let lines = line_spans(content);
    let trimmed: Vec<&str> = lines
        .iter()
        .map(|(s, e)| content[*s..*e].trim())
        .collect();

    let mut spans = Vec::new();
    if needle.len() > trimmed.len() {
        return spans;
    }

    for start in 0..=(trimmed.len() - needle.len()) {
        if trimmed[start..start + needle.len()] == needle[..] {
            let from = lines[start].0;
            let to = lines[start + needle.len() - 1].1;
            spans.push((from, to));
        }
    }
    spans
}

/// Collapses every run of whitespace to one space on both sides, then matches,
/// mapping the result back onto the original offsets.
fn find_whitespace_normalized(content: &str, old: &str) -> Vec<Span> {
    let (normalized, offsets) = normalize_with_offsets(content);
    let (needle, _) = normalize_with_offsets(old);
    let needle = needle.trim();

    if needle.is_empty() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(at) = normalized[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        // Map normalized positions back to where they came from.
        if let (Some(origin), Some(finish)) = (offsets.get(start), offsets.get(end - 1)) {
            spans.push((*origin, finish + 1));
        }
        from = end;
    }
    spans
}

/// Returns the whitespace-collapsed text, plus the original byte offset each
/// normalized byte came from.
fn normalize_with_offsets(text: &str) -> (String, Vec<usize>) {
    let mut normalized = String::with_capacity(text.len());
    let mut offsets = Vec::with_capacity(text.len());
    let mut in_whitespace = false;

    for (at, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if !in_whitespace && !normalized.is_empty() {
                normalized.push(' ');
                offsets.push(at);
            }
            in_whitespace = true;
        } else {
            in_whitespace = false;
            let before = normalized.len();
            normalized.push(ch);
            // One entry per byte, so indices into the string map directly.
            for _ in before..normalized.len() {
                offsets.push(at);
            }
        }
    }

    (normalized, offsets)
}

/// The whitespace prefix common to every non-blank line.
fn common_indent(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0)
}

/// Matches with a uniform common indent stripped from both sides.
///
/// Distinct from the line-trimmed rung: relative indentation *inside* the
/// block still has to match, so a needle whose body is differently nested is
/// correctly refused.
fn find_indentation_flexible(content: &str, old: &str) -> Vec<Span> {
    let raw: Vec<&str> = old.lines().collect();
    if raw.is_empty() {
        return Vec::new();
    }
    let strip = common_indent(&raw);
    let needle: Vec<String> = raw
        .iter()
        .map(|line| {
            if line.len() >= strip {
                line[strip..].to_string()
            } else {
                line.trim_start().to_string()
            }
        })
        .collect();

    let lines = line_spans(content);
    let text: Vec<&str> = lines.iter().map(|(s, e)| &content[*s..*e]).collect();

    if needle.len() > text.len() {
        return Vec::new();
    }

    let mut spans = Vec::new();
    for start in 0..=(text.len() - needle.len()) {
        let window = &text[start..start + needle.len()];
        let window_strip = common_indent(window);
        let stripped: Vec<String> = window
            .iter()
            .map(|line| {
                if line.len() >= window_strip {
                    line[window_strip..].to_string()
                } else {
                    line.trim_start().to_string()
                }
            })
            .collect();

        if stripped == needle {
            spans.push((lines[start].0, lines[start + needle.len() - 1].1));
        }
    }
    spans
}

/// Unescapes a needle that arrived with literal escape sequences, then retries
/// the two strictest rungs on it.
///
/// Models that have been writing JSON sometimes send `\n` as two characters.
fn find_escaped(content: &str, old: &str) -> Vec<Span> {
    let unescaped = unescape(old);
    if unescaped == old {
        return Vec::new();
    }
    let exact = find_exact(content, &unescaped);
    if !exact.is_empty() {
        return exact;
    }
    find_line_trimmed(content, &unescaped)
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('`') => out.push('`'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Anchors on the needle's first line and scans forward to its last.
///
/// Only for needles of three or more lines, and only when exactly one line in
/// the file matches the anchor - otherwise this rung is guessing, and a wrong
/// guess here replaces an arbitrary span.
fn find_block_anchor(content: &str, old: &str) -> Vec<Span> {
    let needle: Vec<&str> = old.lines().collect();
    if needle.len() < 3 {
        return Vec::new();
    }

    let first = needle[0].trim();
    let last = needle[needle.len() - 1].trim();
    if first.is_empty() || last.is_empty() {
        return Vec::new();
    }

    let lines = line_spans(content);
    let trimmed: Vec<&str> = lines
        .iter()
        .map(|(s, e)| content[*s..*e].trim())
        .collect();

    let starts: Vec<usize> = trimmed
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == first)
        .map(|(i, _)| i)
        .collect();

    // Exactly one, or this rung has no business choosing.
    if starts.len() != 1 {
        return Vec::new();
    }

    let start = starts[0];
    for end in (start + 1)..trimmed.len() {
        if trimmed[end] == last {
            return vec![(lines[start].0, lines[end].1)];
        }
    }

    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(content: &str, old: &str, new: &str) -> Replacement {
        replace(content, old, new, false).expect("expected a match")
    }

    // ── refusals that come before any matching ──────────────────────────

    #[test]
    fn an_identical_pair_is_refused() {
        let err = replace("abc", "b", "b", false).unwrap_err();
        assert!(err.contains("identical"), "got {err}");
    }

    #[test]
    fn an_empty_needle_points_at_the_write_tool() {
        let err = replace("abc", "", "x", false).unwrap_err();
        assert!(err.contains("write tool"), "got {err}");
    }

    // ── rung 1: exact ───────────────────────────────────────────────────

    #[test]
    fn an_exact_match_is_preferred() {
        let result = applied("let x = 1;\nlet y = 2;\n", "let x = 1;", "let x = 9;");
        assert_eq!(result.strategy, Strategy::Exact);
        assert_eq!(result.count, 1);
        assert_eq!(result.content, "let x = 9;\nlet y = 2;\n");
    }

    #[test]
    fn replace_all_replaces_every_occurrence() {
        let result = replace("a\na\na\n", "a", "b", true).unwrap();
        assert_eq!(result.count, 3);
        assert_eq!(result.content, "b\nb\nb\n");
    }

    /// Ambiguity is a hard failure. Picking the first match is how an agent
    /// silently edits the wrong function.
    #[test]
    fn ambiguity_is_refused_rather_than_guessed() {
        let err = replace("a\na\n", "a", "b", false).unwrap_err();
        assert!(err.contains("Found 2 matches"), "got {err}");
        assert!(err.contains("replaceAll"), "got {err}");
    }

    // ── rung 2: line-trimmed ────────────────────────────────────────────

    #[test]
    fn indentation_differences_are_tolerated() {
        let content = "fn main() {\n    let x = 1;\n}\n";
        let result = applied(content, "let x = 1;", "let x = 2;");
        // Matched by trimming, and the replacement takes the whole line.
        assert_eq!(result.strategy, Strategy::Exact);
        assert!(result.content.contains("let x = 2;"));
    }

    /// A needle that is a literal substring is handled by the exact rung even
    /// when surrounded by whitespace, and only that substring is replaced.
    #[test]
    fn surrounding_whitespace_is_left_alone_by_an_exact_match() {
        let content = "a\n      spaced out      \nb\n";
        let result = applied(content, "spaced out", "replaced");
        assert_eq!(result.strategy, Strategy::Exact);
        assert_eq!(result.content, "a\n      replaced      \nb\n");
    }

    /// The line-trimmed rung earns its place on multi-line needles, where the
    /// indentation makes the block impossible to match literally.
    #[test]
    fn a_multi_line_needle_matches_despite_indentation() {
        let content = "start\n    foo();\n    bar();\nend\n";
        let result = applied(content, "foo();\nbar();", "baz();");
        assert_eq!(result.strategy, Strategy::LineTrimmed);
        // The match spans whole lines, so the indentation goes with them.
        assert_eq!(result.content, "start\nbaz();\nend\n");
    }

    // ── rung 3: whitespace-normalized ───────────────────────────────────

    #[test]
    fn collapsed_internal_whitespace_still_matches() {
        let content = "call(a,     b,\n     c);\n";
        let result = applied(content, "call(a, b, c);", "call(x);");
        assert_eq!(result.strategy, Strategy::WhitespaceNormalized);
        assert!(result.content.contains("call(x);"), "got {:?}", result.content);
    }

    // ── rung 4: indentation-flexible ────────────────────────────────────

    /// A block reproduced at a different nesting level still matches.
    ///
    /// Note which rung answers: **line-trimmed**, not indentation-flexible.
    /// Ignoring each line's whitespace entirely is strictly more permissive
    /// than stripping a common prefix, so every window rung 4 could match,
    /// rung 2 has already matched. Rung 4 is therefore unreachable in this
    /// order.
    ///
    /// It is kept, in order, deliberately: this mirrors the ladder being
    /// ported, and moving rung 4 ahead of rung 2 would silently change which
    /// edits succeed on every existing workspace. The dead rung is a fact
    /// about the original worth recording rather than quietly fixing.
    #[test]
    fn a_block_matches_at_a_different_nesting_level() {
        let content = "fn outer() {\n        if x {\n            go();\n        }\n}\n";
        let needle = "if x {\n    go();\n}";
        let result = applied(content, needle, "done();");
        assert_eq!(result.strategy, Strategy::LineTrimmed);
        assert!(result.content.contains("done();"));
    }

    /// Directly exercising rung 4, bypassing the ladder, so the implementation
    /// is covered even though the ordering keeps it from being reached.
    #[test]
    fn the_indentation_rung_preserves_relative_indentation() {
        let content = "fn outer() {\n        if x {\n            go();\n        }\n}\n";

        // Relative indent matches: the body is one level inside the `if`.
        assert!(!find_indentation_flexible(content, "if x {\n    go();\n}").is_empty());

        // Relative indent does not match: the body is flush with the `if`.
        assert!(find_indentation_flexible(content, "if x {\ngo();\n}").is_empty());
    }

    // ── rung 5: escaped ─────────────────────────────────────────────────

    /// A model that has been writing JSON all day sends `\n` as two
    /// characters.
    #[test]
    fn a_literally_escaped_newline_is_recovered() {
        let content = "first\nsecond\n";
        let result = applied(content, "first\\nsecond", "replaced");
        assert_eq!(result.strategy, Strategy::Escaped);
        assert_eq!(result.content, "replaced\n");
    }

    #[test]
    fn unescaping_leaves_unknown_escapes_alone() {
        assert_eq!(unescape(r"a\nb"), "a\nb");
        assert_eq!(unescape(r"a\qb"), r"a\qb");
        assert_eq!(unescape(r"back\\slash"), r"back\slash");
    }

    // ── rung 6: block-anchor ────────────────────────────────────────────

    #[test]
    fn a_block_is_anchored_on_a_unique_first_line() {
        let content = "fn a() {\n  one();\n  two();\n  three();\n}\nfn b() {}\n";
        // The middle is misremembered; the first and last lines are right.
        let needle = "fn a() {\n  WRONG();\n}";
        let result = applied(content, needle, "fn a() { fixed(); }");
        assert_eq!(result.strategy, Strategy::BlockAnchor);
        assert!(result.content.contains("fixed()"));
        assert!(result.content.contains("fn b() {}"));
    }

    /// If the anchor is not unique the rung declines rather than guessing.
    #[test]
    fn a_repeated_anchor_disqualifies_the_block_rung() {
        let content = "if x {\n  a();\n}\nif x {\n  b();\n}\n";
        let needle = "if x {\n  WRONG();\n}";
        // No rung can match this, so it fails outright rather than editing an
        // arbitrary one of the two blocks.
        assert!(replace(content, needle, "z", false).is_err());
    }

    // ── the disproportionate-match guard ────────────────────────────────

    /// The guard the block-anchor rung exists alongside: a short needle whose
    /// first and last lines happen to bracket a huge span.
    #[test]
    fn a_wildly_oversized_match_is_refused() {
        let mut content = String::from("start\n");
        for i in 0..200 {
            content.push_str(&format!("  line {i}\n"));
        }
        content.push_str("end\n");

        let needle = "start\n  middle\nend";
        let err = replace(&content, needle, "x", false).unwrap_err();
        assert!(err.contains("much larger"), "got {err}");
    }

    // ── no match at all ─────────────────────────────────────────────────

    #[test]
    fn a_needle_that_is_not_there_says_so_plainly() {
        let err = replace("a\nb\n", "nowhere to be found", "x", false).unwrap_err();
        assert!(err.contains("Could not find that text"), "got {err}");
    }

    // ── ordering and safety ─────────────────────────────────────────────

    /// The ladder must stop at the first rung that matches, so an exact match
    /// is never reinterpreted by a looser rung.
    #[test]
    fn a_stricter_rung_wins_over_a_looser_one() {
        let content = "  x  \nx\n";
        // Both the exact rung (the bare `x` line) and the line-trimmed rung
        // (both lines) could match. Exact must win.
        let result = replace(content, "x", "y", false);
        // Two exact matches exist, so this is ambiguous rather than falling
        // through to a looser rung - which is the point.
        assert!(result.is_err());
    }

    #[test]
    fn overlapping_matches_are_collapsed() {
        assert_eq!(
            collapse_overlaps(vec![(0, 5), (3, 8), (10, 12)]),
            vec![(0, 5), (10, 12)]
        );
    }

    #[test]
    fn multibyte_content_is_not_split() {
        let content = "let s = \"héllo wörld\";\n";
        let result = applied(content, "héllo", "goodbye");
        assert!(result.content.contains("goodbye wörld"));
    }

    #[test]
    fn crlf_line_endings_do_not_break_line_matching() {
        let content = "a\r\n   target   \r\nb\r\n";
        let result = applied(content, "target", "replaced");
        assert!(result.content.contains("replaced"), "got {:?}", result.content);
    }
}

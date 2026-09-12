//! Capping tool output.
//!
//! A tool that prints a 40MB file does not just cost money - it evicts the
//! actual conversation from the context window. But cutting at a byte limit
//! and stopping loses the end of the output, which for a build log or a test
//! run is the only part anyone wanted.
//!
//! So: keep the head and the tail, drop the middle, and say plainly how much
//! went. The model can then narrow its next call rather than guessing why the
//! output looked odd.

/// What was removed, if anything. Reaches the UI as result metadata; the model
/// sees only the notice embedded in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation {
    /// Size of the original output in bytes.
    pub original_bytes: usize,
    /// How many bytes were dropped from the middle.
    pub dropped_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncated {
    pub output: String,
    pub truncation: Option<Truncation>,
}

/// Default ceiling for one tool result, in bytes.
///
/// Roughly 8k tokens of English. Large enough that ordinary output - a file, a
/// test run, a directory listing - is never touched, small enough that one
/// runaway call cannot displace the conversation around it.
pub const DEFAULT_LIMIT: usize = 32_000;

/// Fraction of the budget given to the head. The tail gets the rest.
///
/// Weighted toward the end because that is where failures report themselves:
/// a compiler's error summary, a test runner's totals, a stack trace. The head
/// is kept at all because it carries the command and its first output, which
/// is what identifies *which* run this was.
const HEAD_SHARE: f64 = 0.4;

pub fn truncate(output: String, limit: usize) -> Truncated {
    if output.len() <= limit {
        return Truncated {
            output,
            truncation: None,
        };
    }

    let original_bytes = output.len();
    let head_budget = (limit as f64 * HEAD_SHARE) as usize;
    let tail_budget = limit.saturating_sub(head_budget);

    // Cut on a character boundary always, and on a line boundary when one is
    // close enough to be worth the characters it costs.
    //
    // "Close enough" is the load-bearing part. Output with no line breaks at
    // all - minified JSON, a base64 blob - has its nearest line boundary at
    // position zero, and snapping to it would drop the entire head. Requiring
    // the boundary to sit within half the budget keeps the alignment when the
    // text has lines and abandons it when it does not.
    let head_cut = floor_char_boundary(&output, head_budget);
    let head_end = match output[..head_cut].rfind('\n') {
        Some(i) if i + 1 >= head_budget / 2 => i + 1,
        _ => head_cut,
    };

    let tail_cut = ceil_char_boundary(&output, original_bytes - tail_budget);
    let tail_start = match output[tail_cut..].find('\n') {
        Some(i) if i <= tail_budget / 2 => tail_cut + i + 1,
        _ => tail_cut,
    };

    let dropped_bytes = tail_start.saturating_sub(head_end);
    let notice = format!(
        "\n\n[... {} of {} bytes omitted from the middle of this output ...]\n\n",
        dropped_bytes, original_bytes
    );

    let mut result = String::with_capacity(head_end + notice.len() + (original_bytes - tail_start));
    result.push_str(&output[..head_end]);
    result.push_str(&notice);
    result.push_str(&output[tail_start..]);

    Truncated {
        output: result,
        truncation: Some(Truncation {
            original_bytes,
            dropped_bytes,
        }),
    }
}

fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    index = index.min(s.len());
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(s: &str, mut index: usize) -> usize {
    index = index.min(s.len());
    while index < s.len() && !s.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_output_is_untouched() {
        let result = truncate("hello".to_string(), 100);
        assert_eq!(result.output, "hello");
        assert!(result.truncation.is_none());
    }

    #[test]
    fn output_at_exactly_the_limit_is_untouched() {
        let text = "x".repeat(100);
        assert!(truncate(text, 100).truncation.is_none());
    }

    #[test]
    fn long_output_keeps_both_ends() {
        let lines: Vec<String> = (0..2000).map(|i| format!("line {i}")).collect();
        let text = lines.join("\n");
        let result = truncate(text, 1000);

        assert!(result.truncation.is_some());
        assert!(result.output.contains("line 0"), "head was lost");
        assert!(result.output.contains("line 1999"), "tail was lost");
        assert!(result.output.contains("omitted"));
    }

    #[test]
    fn the_notice_reports_real_numbers() {
        let text = "a\n".repeat(10_000);
        let original = text.len();
        let result = truncate(text, 1000);

        let truncation = result.truncation.unwrap();
        assert_eq!(truncation.original_bytes, original);
        assert!(truncation.dropped_bytes > 0);
        assert!(result
            .output
            .contains(&format!("of {original} bytes omitted")));
    }

    /// A split multi-byte character is not merely ugly - the result would not
    /// be valid UTF-8, and in Rust that is a panic rather than a mojibake.
    #[test]
    fn multibyte_text_is_never_split_mid_character() {
        // Every character here is 4 bytes, and there are no line breaks to cut
        // on, which forces the character-boundary fallback path.
        let text = "🙂".repeat(5000);
        let result = truncate(text, 1000);
        assert!(result.truncation.is_some());
        assert!(result.output.contains('🙂'));
    }

    #[test]
    fn a_single_enormous_line_still_truncates() {
        let text = "x".repeat(100_000);
        let result = truncate(text, 1000);
        assert!(result.truncation.is_some());
        // Budget plus the notice, not the original.
        assert!(result.output.len() < 2000, "got {}", result.output.len());
    }

    /// Regression: output with no line breaks anywhere - minified JSON, a
    /// base64 blob - has its nearest line boundary at position zero. Snapping
    /// to it dropped the entire head and tail, leaving nothing but the notice.
    #[test]
    fn output_without_line_breaks_keeps_both_ends() {
        let text = format!("HEAD{}TAIL", "x".repeat(50_000));
        let result = truncate(text, 1000);

        assert!(result.truncation.is_some());
        assert!(result.output.starts_with("HEAD"), "the head was dropped");
        assert!(result.output.ends_with("TAIL"), "the tail was dropped");
    }

    /// The tail gets the larger share, because that is where a build log or a
    /// test runner says what actually happened.
    #[test]
    fn the_tail_gets_more_room_than_the_head() {
        let lines: Vec<String> = (0..5000).map(|i| format!("line {i:05}")).collect();
        let result = truncate(lines.join("\n"), 2000);

        let notice = result.output.find("[...").unwrap();
        let head = &result.output[..notice];
        let tail = &result.output[result.output.rfind("...]").unwrap()..];
        assert!(tail.len() > head.len(), "head {} tail {}", head.len(), tail.len());
    }
}

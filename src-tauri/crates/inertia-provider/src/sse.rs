//! Server-sent event framing.
//!
//! A tiny incremental parser rather than a crate dependency, for two reasons.
//!
//! First, chunk boundaries. Bytes arrive in whatever sizes the network decides,
//! and a boundary lands mid-JSON constantly. Parsing per chunk works perfectly
//! against a fast local model and corrupts silently against a slow remote one,
//! which is the worst possible failure shape. Everything not yet terminated by
//! a blank line is carried forward.
//!
//! Second, this deliberately joins multiple `data:` lines with **no separator**
//! rather than the newline the SSE specification calls for. The payloads here
//! are always JSON, and a provider that splits one across two `data:` lines
//! usually splits it mid-token - inserting a newline inside a string literal
//! corrupts it, while joining with nothing reassembles it exactly.

/// Accumulates bytes and yields complete event payloads.
#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds a chunk and returns every event that is now complete.
    ///
    /// A partial event at the end is retained for the next call.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();

        // Both `\n\n` and `\r\n\r\n` occur in the wild, because proxies rewrite
        // line endings on their way through.
        while let Some((raw, rest)) = split_once_blank_line(&self.buffer) {
            let raw = raw.to_string();
            self.buffer = rest.to_string();
            if let Some(payload) = payload_of(&raw) {
                events.push(payload);
            }
        }

        events
    }

    /// Anything left unterminated when the stream ended.
    ///
    /// A well-behaved provider ends on a blank line, but one that closes the
    /// connection right after the last payload would otherwise lose it.
    pub fn finish(&mut self) -> Option<String> {
        let remaining = std::mem::take(&mut self.buffer);
        payload_of(&remaining)
    }
}

fn split_once_blank_line(buffer: &str) -> Option<(&str, &str)> {
    let lf = buffer.find("\n\n").map(|i| (i, i + 2));
    let crlf = buffer.find("\r\n\r\n").map(|i| (i, i + 4));

    // Whichever terminator comes first. Checking only one would leave the other
    // framing buffered forever.
    let (at, after) = match (lf, crlf) {
        (Some(a), Some(b)) => {
            if a.0 <= b.0 {
                a
            } else {
                b
            }
        }
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };

    Some((&buffer[..at], &buffer[after..]))
}

/// Extracts the concatenated `data:` content of one raw event block.
///
/// Comment lines (`: keep-alive`) and other SSE fields (`event:`, `id:`) are
/// dropped - nothing here needs them, and the payload is always JSON or the
/// literal `[DONE]`.
fn payload_of(raw: &str) -> Option<String> {
    let mut payload = String::new();

    for line in raw.lines() {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        payload.push_str(rest.strip_prefix(' ').unwrap_or(rest));
    }

    (!payload.is_empty()).then_some(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_event_parses() {
        let mut parser = SseParser::new();
        let events = parser.push("data: {\"a\":1}\n\n");
        assert_eq!(events, vec![r#"{"a":1}"#]);
    }

    #[test]
    fn several_events_in_one_chunk_all_parse() {
        let mut parser = SseParser::new();
        let events = parser.push("data: one\n\ndata: two\n\ndata: three\n\n");
        assert_eq!(events, vec!["one", "two", "three"]);
    }

    /// The whole reason this is incremental. A boundary mid-JSON must not
    /// produce a truncated payload.
    #[test]
    fn an_event_split_across_chunks_is_reassembled() {
        let mut parser = SseParser::new();
        assert!(parser.push("data: {\"text\":\"hel").is_empty());
        assert!(parser.push("lo wor").is_empty());
        let events = parser.push("ld\"}\n\n");
        assert_eq!(events, vec![r#"{"text":"hello world"}"#]);
    }

    #[test]
    fn crlf_framing_works_too() {
        let mut parser = SseParser::new();
        let events = parser.push("data: {\"a\":1}\r\n\r\n");
        assert_eq!(events, vec![r#"{"a":1}"#]);
    }

    /// A proxy can rewrite some line endings and not others, so both framings
    /// can appear in one stream.
    #[test]
    fn mixed_framings_in_one_stream_both_parse() {
        let mut parser = SseParser::new();
        let events = parser.push("data: one\r\n\r\ndata: two\n\n");
        assert_eq!(events, vec!["one", "two"]);
    }

    /// Joined with nothing, not a newline: these payloads are JSON, and a
    /// split usually lands mid-token.
    #[test]
    fn multiple_data_lines_join_without_a_separator() {
        let mut parser = SseParser::new();
        let events = parser.push("data: {\"text\":\"ab\ndata: cd\"}\n\n");
        assert_eq!(events, vec![r#"{"text":"abcd"}"#]);
    }

    #[test]
    fn comments_and_other_fields_are_ignored() {
        let mut parser = SseParser::new();
        let events = parser.push(": keep-alive\n\nevent: ping\nid: 7\ndata: real\n\n");
        assert_eq!(events, vec!["real"]);
    }

    #[test]
    fn an_event_with_no_data_is_skipped() {
        let mut parser = SseParser::new();
        assert!(parser.push("event: ping\n\n").is_empty());
    }

    #[test]
    fn only_one_leading_space_is_stripped() {
        let mut parser = SseParser::new();
        // The SSE field value starts after one optional space; further
        // whitespace is part of the payload.
        let events = parser.push("data:  indented\n\n");
        assert_eq!(events, vec![" indented"]);
    }

    #[test]
    fn done_passes_through_like_any_payload() {
        let mut parser = SseParser::new();
        assert_eq!(parser.push("data: [DONE]\n\n"), vec!["[DONE]"]);
    }

    /// A provider that closes right after its last payload, without the
    /// trailing blank line, would otherwise lose that payload entirely.
    #[test]
    fn an_unterminated_trailing_event_is_recovered_at_the_end() {
        let mut parser = SseParser::new();
        assert!(parser.push("data: last").is_empty());
        assert_eq!(parser.finish(), Some("last".to_string()));
    }

    #[test]
    fn finishing_a_clean_stream_yields_nothing() {
        let mut parser = SseParser::new();
        parser.push("data: one\n\n");
        assert_eq!(parser.finish(), None);
    }

    /// Bytes arriving one at a time is the pathological case, and it must give
    /// exactly the same result as one whole chunk.
    #[test]
    fn byte_at_a_time_delivery_gives_the_same_result() {
        let stream = "data: {\"a\":1}\n\ndata: {\"b\":2}\n\n";
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        for ch in stream.chars() {
            events.extend(parser.push(&ch.to_string()));
        }
        assert_eq!(events, vec![r#"{"a":1}"#, r#"{"b":2}"#]);
    }
}

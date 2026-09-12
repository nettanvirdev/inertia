//! The wire format a language server speaks.
//!
//! LSP is JSON-RPC in an HTTP-shaped envelope: a `Content-Length` header, a
//! blank line, then exactly that many bytes of JSON. The whole thing arrives
//! over a pipe, and a pipe has no idea what a message is - it hands over
//! whatever happened to be in the kernel buffer when the read returned. So a
//! single chunk can carry three messages, or half a header, or a header whose
//! `Content-Length: 4` is split between the `4` and the `\r`.
//!
//! A parser that assumes one chunk is one message works perfectly against a
//! server that answers instantly with a short reply, which is exactly what the
//! first test looks like, and then corrupts against a real one under load. That
//! is why this is its own file with its own tests: it is the piece where being
//! nearly right is indistinguishable from being right until it is not.
//!
//! The parser is a pure function of the bytes it has been given. It holds no
//! process, no socket and no state beyond the leftover buffer, which is what
//! makes the split-header case testable by handing it two slices.

use serde_json::Value;

/// The header terminator: an empty line after the header block.
const HEADER_END: &[u8] = b"\r\n\r\n";

/// A header block longer than this is not a header block. Without a ceiling, a
/// server that writes garbage to stdout - a stack trace, a progress bar - makes
/// this buffer grow forever while it waits for a `\r\n\r\n` that never comes.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;

/// And a body longer than this is a `Content-Length` we misread or a server
/// that has lost its mind. Allocating what it asks for would be the last thing
/// this process did.
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// One message, framed for the wire.
pub fn encode(message: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(message).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

/// `Content-Length: 42`, case-insensitively, because the spec says so.
fn content_length(header: &str) -> Option<&str> {
    for line in header.split("\r\n") {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            return Some(value.trim());
        }
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// A framer that takes chunks in whatever sizes they arrive and gives back
/// whole messages.
///
/// [`Parser::push`] returns the messages that completed with this chunk, which
/// is usually none or one and is occasionally several. It fails only on a
/// stream that can no longer be trusted - a header with no length, a body that
/// is not JSON, a length past the ceiling - because at that point the byte
/// offsets are wrong and every message after it would be wrong too. The
/// caller's answer to that is to kill the server, not to try the next byte.
#[derive(Debug, Default)]
pub struct Parser {
    buffer: Vec<u8>,
}

impl Parser {
    pub fn new() -> Self {
        Self::default()
    }

    /// How much is held back waiting for the rest of itself. For tests.
    pub fn pending(&self) -> usize {
        self.buffer.len()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, String> {
        self.buffer.extend_from_slice(chunk);

        let mut messages = Vec::new();
        loop {
            let Some(header_end) = find(&self.buffer, HEADER_END) else {
                // The header is still arriving. This is the branch the whole
                // file exists for, and it is reached constantly against a busy
                // server.
                if self.buffer.len() > MAX_HEADER_BYTES {
                    return Err(
                        "No LSP header terminator in the first 8 KB of the stream.".to_string()
                    );
                }
                break;
            };

            let header = String::from_utf8_lossy(&self.buffer[..header_end]).into_owned();
            let Some(raw) = content_length(&header) else {
                return Err("An LSP header block arrived with no Content-Length.".to_string());
            };
            let length = match raw.parse::<usize>() {
                Ok(length) if length <= MAX_BODY_BYTES => length,
                _ => return Err(format!("An LSP message claimed a length of {raw} bytes.")),
            };

            let start = header_end + HEADER_END.len();
            // The body is measured in bytes, not characters, which is why this
            // is all done on the raw buffer: a message containing one emoji
            // would be four bytes short of itself if the length were compared
            // against a decoded string.
            if self.buffer.len() - start < length {
                break;
            }

            let body = self.buffer[start..start + length].to_vec();
            self.buffer.drain(..start + length);

            match serde_json::from_slice::<Value>(&body) {
                Ok(value) => messages.push(value),
                Err(error) => return Err(format!("An LSP message body was not JSON: {error}")),
            }
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_request_and_a_response_round_trip() {
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} });
        let response = json!({ "jsonrpc": "2.0", "id": 1, "result": { "capabilities": {} } });

        let mut wire = encode(&request);
        wire.extend_from_slice(&encode(&response));

        let mut parser = Parser::new();
        let out = parser.push(&wire).unwrap();

        assert_eq!(out.len(), 2, "both messages in one chunk");
        assert_eq!(out[0], request);
        assert_eq!(out[1], response);
        assert_eq!(parser.pending(), 0);
    }

    /// The case a naive parser gets wrong: the pipe split the message, and the
    /// second half arrives on a later read.
    #[test]
    fn a_message_split_across_reads_is_reassembled() {
        let message = json!({ "jsonrpc": "2.0", "id": 7, "result": ["a", "b"] });
        let wire = encode(&message);

        // Split inside the header, so both the header and the body are torn.
        let cut = 10;
        let mut parser = Parser::new();
        assert!(parser.push(&wire[..cut]).unwrap().is_empty());
        assert!(parser.pending() > 0);

        // ...and then split again inside the body.
        let second = wire.len() - 3;
        assert!(parser.push(&wire[cut..second]).unwrap().is_empty());

        let out = parser.push(&wire[second..]).unwrap();
        assert_eq!(out, vec![message]);
        assert_eq!(parser.pending(), 0);
    }

    /// Byte-for-byte, one byte per read. If the framing is right at all, it is
    /// right at this granularity.
    #[test]
    fn one_byte_at_a_time_still_works() {
        let message = json!({ "method": "textDocument/publishDiagnostics", "params": { "uri": "file:///a" } });
        let wire = encode(&message);
        let mut parser = Parser::new();
        let mut seen = Vec::new();
        for byte in &wire {
            seen.extend(parser.push(&[*byte]).unwrap());
        }
        assert_eq!(seen, vec![message]);
    }

    /// The length is bytes, not characters. A multi-byte character measured as
    /// a character leaves the parser three bytes into the next header.
    #[test]
    fn the_length_is_measured_in_bytes() {
        let message = json!({ "message": "caf\u{e9} \u{1f600}" });
        let wire = encode(&message);
        let mut parser = Parser::new();
        assert_eq!(parser.push(&wire).unwrap(), vec![message]);
        assert_eq!(parser.pending(), 0);
    }

    #[test]
    fn a_header_is_matched_case_insensitively() {
        let mut parser = Parser::new();
        let out = parser.push(b"content-length: 2\r\n\r\n{}").unwrap();
        assert_eq!(out, vec![json!({})]);
    }

    #[test]
    fn a_header_with_no_length_is_refused() {
        let mut parser = Parser::new();
        let error = parser.push(b"Content-Type: utf8\r\n\r\n{}").unwrap_err();
        assert!(error.contains("no Content-Length"), "got {error}");
    }

    #[test]
    fn a_body_that_is_not_json_is_refused() {
        let mut parser = Parser::new();
        let error = parser.push(b"Content-Length: 3\r\n\r\nnot").unwrap_err();
        assert!(error.contains("not JSON"), "got {error}");
    }

    #[test]
    fn a_stream_with_no_header_terminator_gives_up() {
        let mut parser = Parser::new();
        let junk = vec![b'x'; MAX_HEADER_BYTES + 1];
        let error = parser.push(&junk).unwrap_err();
        assert!(error.contains("8 KB"), "got {error}");
    }
}

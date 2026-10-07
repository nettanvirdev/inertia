//! JSON-RPC 2.0, as MCP uses it.
//!
//! Hand-rolled rather than taken from an SDK. The wire format is a hundred
//! lines of framing and a map of pending requests, and owning it means the
//! failure modes that actually bite - a server printing a stack trace to
//! stdout, a tool call that legitimately takes ten minutes - are handled the
//! way this app needs rather than the way a general-purpose client assumes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol revision this client speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long a request waits before being abandoned.
///
/// Restarted from zero every time the server reports progress, so a genuinely
/// long operation survives as long as it keeps saying so.
pub const REQUEST_TIMEOUT_MS: u64 = 60_000;

/// Bringing a connection up, including spawning a process.
pub const CONNECT_TIMEOUT_MS: u64 = 30_000;

/// A misbehaving server that always returns a cursor cannot page forever.
pub const MAX_TOOL_PAGES: usize = 20;

/// Method not found. Returned for the server-initiated requests this client
/// does not implement.
pub const METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Request {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC error as it arrives.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

/// What a parsed incoming message turns out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A reply to something we sent.
    Response { id: u64, result: Value },
    /// A failure reply to something we sent.
    Failure { id: u64, error: RpcError },
    /// The server telling us something unprompted.
    Notification { method: String, params: Value },
    /// The server asking *us* for something. Answered with
    /// [`METHOD_NOT_FOUND`] rather than ignored, so it is not left waiting.
    ServerRequest { id: u64, method: String },
}

impl PartialEq for RpcError {
    fn eq(&self, other: &Self) -> bool {
        self.code == other.code && self.message == other.message
    }
}

/// Classifies one incoming message.
///
/// Anything that fits no shape at all returns `None` and is dropped. Servers
/// emit surprising things, and a malformed frame is not worth ending a working
/// connection over.
pub fn classify(message: &Value) -> Option<Incoming> {
    let id = message.get("id").and_then(Value::as_u64);
    let method = message.get("method").and_then(Value::as_str);

    match (id, method) {
        // A request from the server: it has both an id to answer and a method.
        (Some(id), Some(method)) => Some(Incoming::ServerRequest {
            id,
            method: method.to_string(),
        }),
        // A reply: an id and no method.
        (Some(id), None) => {
            if let Some(error) = message.get("error") {
                let error: RpcError = serde_json::from_value(error.clone()).ok()?;
                Some(Incoming::Failure { id, error })
            } else {
                Some(Incoming::Response {
                    id,
                    result: message.get("result").cloned().unwrap_or(Value::Null),
                })
            }
        }
        // A notification: a method and no id.
        (None, Some(method)) => Some(Incoming::Notification {
            method: method.to_string(),
            params: message.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, None) => None,
    }
}

/// Splits newline-delimited JSON into messages.
///
/// Two things it must get right, both learned from real servers:
///
///   - A chunk boundary lands mid-line constantly. The partial trailing line
///     is carried forward rather than parsed and dropped.
///   - Servers print non-protocol noise to stdout - banners, warnings,
///     progress bars. A line that does not parse is skipped silently rather
///     than failing the connection, because the next line usually is protocol.
#[derive(Debug, Default)]
pub struct FrameParser {
    buffer: String,
}

impl FrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &str) -> Vec<Value> {
        self.buffer.push_str(chunk);
        let mut messages = Vec::new();

        while let Some(at) = self.buffer.find('\n') {
            let line = self.buffer[..at].trim().to_string();
            self.buffer.drain(..=at);

            if line.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                messages.push(value);
            }
            // Otherwise: noise. The next line is usually protocol.
        }

        messages
    }
}

/// The `_meta.progressToken` that asks a server to report progress.
///
/// Only attached when the caller wants heartbeats, because a token implies a
/// promise to handle the notifications that come back.
pub fn with_progress_token(params: Option<Value>, token: u64) -> Value {
    let mut params = match params {
        Some(Value::Object(map)) => Value::Object(map),
        _ => Value::Object(serde_json::Map::new()),
    };
    if let Some(map) = params.as_object_mut() {
        map.insert(
            "_meta".into(),
            serde_json::json!({ "progressToken": token }),
        );
    }
    params
}

/// The progress token a notification refers to, if any.
pub fn progress_token_of(params: &Value) -> Option<u64> {
    params.get("progressToken").and_then(Value::as_u64)
}

/// Flattens an MCP tool result's content blocks into text.
///
/// A tool result is a chat message on every provider worth supporting, and
/// none accepts an image inside one, so non-text blocks are described rather
/// than embedded.
pub fn flatten_content(result: &Value) -> String {
    let Some(blocks) = result.get("content").and_then(Value::as_array) else {
        // A server that returns something else entirely: show it rather than
        // pretending there was no output.
        return match result {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
    };

    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            Some("image") => {
                let media = block
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("image");
                parts.push(format!("[{media} returned by the tool]"));
            }
            Some("resource") => {
                if let Some(text) = block.pointer("/resource/text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                } else if let Some(uri) = block.pointer("/resource/uri").and_then(Value::as_str) {
                    parts.push(format!("[resource: {uri}]"));
                }
            }
            _ => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
        }
    }

    parts.join("\n")
}

/// Whether a tool result reports failure.
pub fn is_error_result(result: &Value) -> bool {
    result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Makes a server's tool schema safe to send to a model provider.
///
/// Providers are strict and mutually inconsistent about `$ref`, `anyOf` and
/// `additionalProperties`, and MCP servers emit all three freely. A schema one
/// provider rejects is a tool that silently does not exist, so the unsupported
/// parts are stripped rather than passed through.
pub fn sanitize_schema(schema: &Value) -> Value {
    fn strip(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut cleaned = serde_json::Map::new();
                for (key, child) in map {
                    match key.as_str() {
                        // Unsupported combinators. Dropping them widens the
                        // schema, which is safe: the tool validates its own
                        // arguments anyway.
                        "$ref" | "$defs" | "definitions" | "anyOf" | "oneOf" | "allOf" | "not"
                        | "$schema" => continue,
                        _ => {
                            cleaned.insert(key.clone(), strip(child));
                        }
                    }
                }
                Value::Object(cleaned)
            }
            Value::Array(items) => Value::Array(items.iter().map(strip).collect()),
            other => other.clone(),
        }
    }

    let mut cleaned = strip(schema);

    // Providers insist on the object form even for a tool that takes nothing.
    if !cleaned.is_object() {
        return serde_json::json!({ "type": "object", "properties": {} });
    }
    if let Some(map) = cleaned.as_object_mut() {
        map.entry("type").or_insert(Value::String("object".into()));
        map.entry("properties")
            .or_insert(Value::Object(serde_json::Map::new()));
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── framing ─────────────────────────────────────────────────────────

    #[test]
    fn one_message_per_line() {
        let mut parser = FrameParser::new();
        let messages = parser.push("{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(messages, vec![json!({"a":1}), json!({"b":2})]);
    }

    /// A chunk boundary lands mid-line constantly.
    #[test]
    fn a_partial_line_is_carried_forward() {
        let mut parser = FrameParser::new();
        assert!(parser.push("{\"a\":").is_empty());
        assert_eq!(parser.push("1}\n"), vec![json!({"a":1})]);
    }

    /// Servers print banners and warnings to stdout. The next line is usually
    /// protocol, so noise must not end the connection.
    #[test]
    fn non_protocol_noise_is_skipped() {
        let mut parser = FrameParser::new();
        let messages = parser.push("Starting server v1.2.3...\n{\"a\":1}\nwarning: slow\n");
        assert_eq!(messages, vec![json!({"a":1})]);
    }

    #[test]
    fn blank_lines_are_ignored() {
        let mut parser = FrameParser::new();
        assert_eq!(parser.push("\n\n{\"a\":1}\n\n"), vec![json!({"a":1})]);
    }

    #[test]
    fn byte_at_a_time_gives_the_same_result() {
        let stream = "{\"a\":1}\n{\"b\":2}\n";
        let mut parser = FrameParser::new();
        let mut messages = Vec::new();
        for ch in stream.chars() {
            messages.extend(parser.push(&ch.to_string()));
        }
        assert_eq!(messages.len(), 2);
    }

    // ── classification ──────────────────────────────────────────────────

    #[test]
    fn a_reply_is_recognised() {
        let message = json!({"jsonrpc":"2.0","id":7,"result":{"ok":true}});
        assert_eq!(
            classify(&message),
            Some(Incoming::Response {
                id: 7,
                result: json!({"ok":true})
            })
        );
    }

    #[test]
    fn an_error_reply_is_recognised() {
        let message =
            json!({"jsonrpc":"2.0","id":7,"error":{"code":-32602,"message":"bad params"}});
        let Some(Incoming::Failure { id, error }) = classify(&message) else {
            panic!("expected a failure");
        };
        assert_eq!(id, 7);
        assert_eq!(error.code, -32602);
    }

    #[test]
    fn a_notification_is_recognised() {
        let message =
            json!({"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":3}});
        let Some(Incoming::Notification { method, params }) = classify(&message) else {
            panic!("expected a notification");
        };
        assert_eq!(method, "notifications/progress");
        assert_eq!(progress_token_of(&params), Some(3));
    }

    /// A server request left unanswered hangs the server. It gets a proper
    /// method-not-found instead.
    #[test]
    fn a_server_request_is_distinguished_from_a_notification() {
        let message = json!({"jsonrpc":"2.0","id":9,"method":"sampling/createMessage"});
        assert_eq!(
            classify(&message),
            Some(Incoming::ServerRequest {
                id: 9,
                method: "sampling/createMessage".into()
            })
        );
    }

    #[test]
    fn a_shapeless_message_is_dropped() {
        assert_eq!(classify(&json!({"hello":"world"})), None);
    }

    // ── progress tokens ─────────────────────────────────────────────────

    /// Only attached when asked for: a token implies a promise to handle the
    /// notifications it produces.
    #[test]
    fn a_progress_token_is_attached_under_meta() {
        let params = with_progress_token(Some(json!({"name":"x"})), 5);
        assert_eq!(params["name"], "x");
        assert_eq!(params["_meta"]["progressToken"], 5);
    }

    #[test]
    fn a_progress_token_works_with_no_params() {
        let params = with_progress_token(None, 5);
        assert_eq!(params["_meta"]["progressToken"], 5);
    }

    // ── results ─────────────────────────────────────────────────────────

    #[test]
    fn text_blocks_are_joined() {
        let result = json!({"content":[
            {"type":"text","text":"first"},
            {"type":"text","text":"second"}
        ]});
        assert_eq!(flatten_content(&result), "first\nsecond");
    }

    /// No provider accepts an image inside a tool result, so it is described
    /// rather than embedded.
    #[test]
    fn an_image_block_is_described() {
        let result = json!({"content":[{"type":"image","mimeType":"image/png","data":"AAAA"}]});
        assert_eq!(flatten_content(&result), "[image/png returned by the tool]");
    }

    #[test]
    fn an_embedded_resource_contributes_its_text() {
        let result = json!({"content":[
            {"type":"resource","resource":{"uri":"file:///a.txt","text":"contents"}}
        ]});
        assert_eq!(flatten_content(&result), "contents");
    }

    /// A server that answers with something unexpected: showing it beats
    /// pretending there was no output.
    #[test]
    fn an_unexpected_result_shape_is_shown_rather_than_swallowed() {
        assert_eq!(flatten_content(&json!("bare string")), "bare string");
        assert!(flatten_content(&json!({"unexpected": 1})).contains("unexpected"));
    }

    #[test]
    fn an_error_result_is_recognised() {
        assert!(is_error_result(&json!({"isError":true})));
        assert!(!is_error_result(&json!({"content":[]})));
    }

    // ── schema sanitising ───────────────────────────────────────────────

    /// A schema one provider rejects is a tool that silently does not exist.
    #[test]
    fn unsupported_combinators_are_stripped() {
        let schema = json!({
            "type": "object",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "properties": {
                "mode": { "anyOf": [{ "type": "string" }, { "type": "null" }] },
                "ref": { "$ref": "#/$defs/thing" }
            },
            "$defs": { "thing": { "type": "string" } }
        });

        let cleaned = sanitize_schema(&schema);
        let text = cleaned.to_string();

        assert!(!text.contains("anyOf"), "got {text}");
        assert!(!text.contains("$ref"), "got {text}");
        assert!(!text.contains("$defs"), "got {text}");
        assert!(!text.contains("$schema"), "got {text}");
        // The structure that survives is still usable.
        assert_eq!(cleaned["type"], "object");
        assert!(cleaned["properties"].get("mode").is_some());
    }

    #[test]
    fn a_missing_type_is_filled_in() {
        let cleaned = sanitize_schema(&json!({}));
        assert_eq!(cleaned["type"], "object");
        assert!(cleaned["properties"].is_object());
    }

    #[test]
    fn a_non_object_schema_becomes_an_empty_object_schema() {
        let cleaned = sanitize_schema(&json!("nonsense"));
        assert_eq!(cleaned["type"], "object");
    }

    #[test]
    fn ordinary_schemas_survive_intact() {
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "a path" } },
            "required": ["path"]
        });
        let cleaned = sanitize_schema(&schema);
        assert_eq!(cleaned["properties"]["path"]["description"], "a path");
        assert_eq!(cleaned["required"][0], "path");
    }
}

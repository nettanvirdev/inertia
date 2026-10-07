//! The LLM seam.
//!
//! One trait, implemented once per wire protocol. The agent loop talks to this
//! and never learns whether it reached Anthropic, an OpenAI-compatible
//! endpoint, or a scripted fake in a test.
//!
//! Two decisions worth stating up front, because they shape every
//! implementation:
//!
//! **Failures are events, not errors.** `stream_chat` hands back a stream that
//! always terminates in [`StreamEvent::Done`] or [`StreamEvent::Error`]. A
//! provider that has already streamed nine hundred tokens and then loses the
//! connection cannot un-show them, so the caller needs the failure *in
//! sequence* with the output that preceded it, not thrown past it.
//!
//! **Cancellation is dropping the stream.** No token is threaded through.
//! Dropping the stream drops the in-flight request future, which is what
//! cancellation means in Rust, and it keeps this crate free of a runtime
//! dependency. The agent loop synthesises its own cancelled-turn record.

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::message::{Entry, ThinkingBlock, ToolCall};

/// One conversation turn's worth of request, in the app's own shape.
///
/// Not wire format: each provider translates this itself. Fields the provider
/// has no equivalent for are ignored rather than rejected, so a request built
/// for one provider stays valid against another.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub model: String,
    /// The assembled system prompt. Sent as a top-level field or as message
    /// zero depending on the protocol.
    pub system: String,
    pub history: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Token budget for extended thinking. Providers without a thinking mode
    /// ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<u32>,
    /// "low" | "medium" | "high", for providers that expose a reasoning dial
    /// rather than a budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
}

/// A tool as described to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments. Always an object schema, even when the
    /// tool takes nothing - providers reject a bare or absent schema.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: if parameters.is_object() {
                parameters
            } else {
                serde_json::json!({ "type": "object", "properties": {} })
            },
        }
    }
}

/// Everything that can happen while a turn streams.
///
/// Ordering guarantees an implementation must uphold:
///   - [`Self::Start`] first, if anything is emitted at all.
///   - [`Self::Thinking`] and [`Self::Tool`] are emitted once each, fully
///     assembled, after all deltas and before the terminal event.
///   - exactly one terminal event: [`Self::Done`] or [`Self::Error`].
///
/// Tool calls are buffered rather than streamed because half a JSON argument
/// string is not actionable by any caller. Prose is not buffered, because the
/// entire point is that it appears as it arrives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// `rename_all` renames the variants; `rename_all_fields` renames the fields
// inside them. Both are needed - without the second, `delayMs` goes over the
// wire as `delay_ms` and the frontend reads `undefined` with no error
// anywhere.
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StreamEvent {
    Start {
        model: String,
    },
    /// Prose.
    Delta {
        text: String,
    },
    /// Reasoning the user is allowed to watch.
    Reasoning {
        text: String,
    },
    /// Signed reasoning blocks, to be stored and replayed verbatim next turn.
    Thinking {
        blocks: Vec<ThinkingBlock>,
    },
    /// The model wants these tools run.
    Tool {
        calls: Vec<ToolCall>,
    },
    /// Something was silently worked around - a request downgraded and
    /// retried, say. Worth telling the user; not a failure.
    Notice {
        message: String,
    },
    /// A retry is about to happen. Surfaced so a long stall has a visible
    /// reason rather than looking like a hang.
    Retry {
        attempt: u32,
        of: u32,
        delay_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
        message: String,
    },
    /// Terminal. The turn failed.
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
    },
    /// Terminal. The turn finished.
    Done {
        #[serde(skip_serializing_if = "Option::is_none")]
        finish: Option<FinishReason>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
}

impl StreamEvent {
    /// Whether no further events will follow.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Error { .. })
    }
}

/// Why the model stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// Reached a natural end.
    Stop,
    /// Hit the output token ceiling mid-sentence.
    Length,
    /// Stopped to call tools; the turn continues after they run.
    ToolUse,
    /// Refused on content grounds.
    ContentFilter,
    /// The user stopped it.
    Cancelled,
    /// Something the provider reported that does not map onto the above.
    Other,
}

/// Token accounting for one turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens served from the provider's prompt cache. Billed at a
    /// discount, so this is not merely informational.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cache_read_tokens: u64,
    /// Input tokens written into the cache. Billed at a premium.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cache_write_tokens: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Usage {
    /// Every input token, cached or not.
    pub fn total_input(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

/// One model a provider offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    /// Context window in tokens, where known. Providers frequently do not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u32>,
}

/// A source of model completions.
#[async_trait]
pub trait Provider: Send + Sync + std::fmt::Debug {
    /// Stable identifier for this configured provider, e.g. `"anthropic"`.
    fn id(&self) -> &str;

    /// The models this provider offers.
    ///
    /// Not every endpoint has a model list - plenty of OpenAI-compatible
    /// servers do not implement the route at all. That is a normal outcome,
    /// reported as an empty list rather than an error, because the user can
    /// still type a model id by hand.
    async fn list_models(&self) -> crate::Result<Vec<ModelInfo>>;

    /// Runs one turn.
    ///
    /// The stream always ends in a terminal event. Retries, backoff and
    /// protocol quirks are the implementation's business; the caller sees one
    /// logical turn. Drop the stream to cancel.
    fn stream_chat(&self, request: ChatRequest) -> BoxStream<'static, StreamEvent>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_events_are_recognised() {
        assert!(StreamEvent::Done {
            finish: None,
            usage: None
        }
        .is_terminal());
        assert!(StreamEvent::Error {
            message: "boom".into(),
            status: Some(500)
        }
        .is_terminal());
        assert!(!StreamEvent::Delta { text: "hi".into() }.is_terminal());
    }

    // The frontend matches on `type`, so the tag spelling is a contract.
    #[test]
    fn events_serialise_with_a_type_tag() {
        let json = serde_json::to_value(StreamEvent::Delta { text: "hi".into() }).unwrap();
        assert_eq!(json["type"], "delta");

        let json = serde_json::to_value(StreamEvent::Retry {
            attempt: 1,
            of: 3,
            delay_ms: 500,
            status: Some(429),
            message: "rate limited".into(),
        })
        .unwrap();
        assert_eq!(json["type"], "retry");
        assert_eq!(json["delayMs"], 500);
    }

    #[test]
    fn a_non_object_tool_schema_is_repaired() {
        let spec = ToolSpec::new("x", "does x", serde_json::json!(null));
        assert_eq!(spec.parameters["type"], "object");
    }

    #[test]
    fn cached_tokens_count_toward_input() {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 100,
            cache_write_tokens: 20,
        };
        assert_eq!(usage.total_input(), 130);
    }

    #[test]
    fn zero_cache_counters_are_omitted() {
        let json = serde_json::to_string(&Usage {
            input_tokens: 1,
            output_tokens: 2,
            ..Default::default()
        })
        .unwrap();
        assert!(!json.contains("cacheRead"), "got {json}");
    }
}

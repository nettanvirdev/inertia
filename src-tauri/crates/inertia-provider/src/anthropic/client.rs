//! The Anthropic Messages transport.
//!
//! Spoken natively rather than through an OpenAI-shaped gateway, because a
//! gateway loses prompt caching, extended thinking, and the signature on
//! thinking blocks that has to be replayed across tool calls.
//!
//! It emits exactly the same events as every other provider, so the agent loop
//! cannot tell which one it is talking to.

use std::collections::BTreeMap;

use async_stream::stream;
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use inertia_core::message::{ThinkingBlock, ToolCall};
use inertia_core::provider::{
    ChatRequest, FinishReason, ModelInfo, Provider, StreamEvent, Usage,
};
use inertia_core::ToolCallId;
use secrecy::ExposeSecret;
use serde_json::Value;

use crate::config::ProviderConfig;
use crate::models;
use crate::sse::SseParser;

use super::translate::to_messages;
use super::wire::{Request, Thinking, Tool};

/// Pinned, not floating. A newer default could change response shapes under a
/// build already in the wild.
const API_VERSION: &str = "2023-06-01";

const DEFAULT_MAX_TOKENS: u32 = 8_192;
/// Room left for the answer once thinking has taken its share.
const ANSWER_ALLOWANCE: u32 = 8_192;
/// The API's own floor, and the minimum useful thinking budget.
const MIN_TOKENS: u32 = 1_024;

#[derive(Debug)]
pub struct AnthropicProvider {
    config: ProviderConfig,
    http: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// For tests that need to point at a local server with a short timeout.
    pub fn with_client(config: ProviderConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    fn headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();

        headers.insert("content-type", HeaderValue::from_static("application/json"));

        if !self.config.overrides("anthropic-version") {
            headers.insert("anthropic-version", HeaderValue::from_static(API_VERSION));
        }

        // A config that brings its own auth suppresses ours entirely, rather
        // than sending both and letting the server pick.
        if !self.config.overrides("authorization") && !self.config.overrides("x-api-key") {
            if let Ok(value) = HeaderValue::from_str(self.config.api_key.expose_secret()) {
                headers.insert("x-api-key", value);
            }
        }

        for (name, value) in &self.config.headers {
            if let (Ok(name), Ok(value)) = (
                name.parse::<HeaderName>(),
                HeaderValue::from_str(value),
            ) {
                headers.insert(name, value);
            }
        }

        headers
    }

    fn build(&self, request: &ChatRequest) -> Request {
        let (system, messages) = to_messages(&request.system, &request.history);

        // This API requires max_tokens, unlike OpenAI's optional field, so
        // there is always a number to compute.
        let asked = request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let thinking_budget = request.thinking_budget.unwrap_or(0);
        let wanted = if thinking_budget > 0 {
            asked.max(thinking_budget.saturating_add(ANSWER_ALLOWANCE))
        } else {
            asked
        };
        let ceiling = models::max_output(&request.model).unwrap_or(u32::MAX);
        let max_tokens = wanted.clamp(MIN_TOKENS, ceiling.max(MIN_TOKENS));

        let thinking = (thinking_budget > 0).then(|| Thinking {
            kind: "enabled",
            budget_tokens: thinking_budget
                .clamp(MIN_TOKENS, max_tokens.saturating_sub(MIN_TOKENS).max(MIN_TOKENS)),
        });

        // Both sampling controls are omitted entirely when thinking is on: the
        // API refuses the combination rather than ignoring them.
        let sampling_allowed = thinking.is_none();

        Request {
            model: request.model.clone(),
            max_tokens,
            system,
            messages,
            stream: true,
            tools: request
                .tools
                .iter()
                .map(|t| Tool {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: t.parameters.clone(),
                })
                .collect(),
            thinking,
            temperature: sampling_allowed.then_some(request.temperature).flatten(),
            top_p: sampling_allowed.then_some(request.top_p).flatten(),
        }
    }
}

/// Turns an HTTP status into something a person can act on.
pub fn describe_failure(status: u16, url: &str) -> String {
    match status {
        400 => "The request was rejected.".into(),
        401 => "The API key was rejected.".into(),
        403 => "The key is valid but not allowed to use this.".into(),
        404 => format!("Nothing is at {url}. Check the base URL."),
        413 => "The request was too large for the model's context window.".into(),
        429 => "Rate limited by the provider.".into(),
        500 => "The provider had an internal error.".into(),
        // Anthropic-specific, and genuinely transient.
        529 => "Anthropic is overloaded. This one is worth waiting out.".into(),
        other => format!("The provider returned {other}."),
    }
}

/// One content block being assembled from interleaved deltas.
#[derive(Default, Debug)]
struct PartialBlock {
    kind: String,
    id: String,
    name: String,
    text: String,
    thinking: String,
    signature: String,
    data: String,
    json: String,
}

fn usage_from(value: &Value) -> Usage {
    let read = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        input_tokens: read("input_tokens"),
        output_tokens: read("output_tokens"),
        cache_read_tokens: read("cache_read_input_tokens"),
        cache_write_tokens: read("cache_creation_input_tokens"),
    }
}

fn finish_from(reason: &str) -> FinishReason {
    match reason {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolUse,
        "refusal" => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    async fn list_models(&self) -> inertia_core::Result<Vec<ModelInfo>> {
        let url = self.config.endpoint("models?limit=1000");
        let response = self
            .http
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| inertia_core::Error::Network(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            // A missing model list is not fatal: the user can still type a
            // model id, and plenty of gateways do not implement the route.
            if status.as_u16() == 404 {
                return Ok(Vec::new());
            }
            return Err(inertia_core::Error::upstream(
                Some(status.as_u16()),
                describe_failure(status.as_u16(), &url),
            ));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| inertia_core::Error::Protocol(e.to_string()))?;

        let rows = body
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        Ok(rows
            .iter()
            .filter_map(|row| {
                let id = row.get("id").and_then(Value::as_str)?.to_string();
                let label = row
                    .get("display_name")
                    .or_else(|| row.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string();
                // Anthropic's own /models does not report a context window, so
                // it comes from the local table instead.
                let context = models::context_window(&id);
                Some(ModelInfo { id, label, context })
            })
            .collect())
    }

    fn stream_chat(&self, request: ChatRequest) -> BoxStream<'static, StreamEvent> {
        let body = self.build(&request);
        let headers = self.headers();
        let http = self.http.clone();
        let model = request.model.clone();
        let base = self.config.base_url.clone();
        let mut url = self.config.endpoint("messages");

        Box::pin(stream! {
            yield StreamEvent::Start { model: model.clone() };

            let mut response = match http.post(&url).headers(headers.clone()).json(&body).send().await {
                Ok(response) => response,
                Err(e) => {
                    yield StreamEvent::Error {
                        message: format!("Could not reach the provider: {e}"),
                        status: None,
                    };
                    return;
                }
            };

            // A 404 may mean the base URL was read one way and the gateway
            // mounts it another - `https://api.minimax.io/anthropic` serves its
            // routes under `/v1`, and plenty of gateways do not. The second
            // candidate is tried once, here, rather than reported as a broken
            // provider that is one path segment away from working. The choice
            // is remembered, so this costs one extra request ever, not one per
            // turn.
            if response.status().as_u16() == 404 && crate::base::advance(&base) {
                let retry = crate::base::endpoint(&base, "messages");
                tracing::info!(from = %url, to = %retry, "retrying at the other base URL");
                url = retry;
                match http.post(&url).headers(headers).json(&body).send().await {
                    Ok(next) => response = next,
                    Err(e) => {
                        yield StreamEvent::Error {
                            message: format!("Could not reach the provider: {e}"),
                            status: None,
                        };
                        return;
                    }
                }
            }

            let status = response.status().as_u16();
            if !response.status().is_success() {
                // The body usually carries a more specific reason than the
                // status alone, and the user is the one who has to act on it.
                let detail = response
                    .text()
                    .await
                    .ok()
                    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                    .and_then(|v| {
                        v.get("error")
                            .and_then(|e| e.get("message"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });

                let message = match detail {
                    Some(detail) => format!("{} {detail}", describe_failure(status, &url)),
                    None => describe_failure(status, &url),
                };
                yield StreamEvent::Error { message, status: Some(status) };
                return;
            }

            let mut parser = SseParser::new();
            let mut blocks: BTreeMap<u64, PartialBlock> = BTreeMap::new();
            let mut usage: Option<Usage> = None;
            let mut finish: Option<FinishReason> = None;
            let mut failed: Option<(String, Option<u16>)> = None;

            let mut bytes = response.bytes_stream();
            'reading: while let Some(chunk) = bytes.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        failed = Some((format!("The connection dropped: {e}"), None));
                        break 'reading;
                    }
                };

                // Lossy rather than strict: a multi-byte character split across
                // two chunks would otherwise fail the whole turn, and the
                // parser reassembles the text anyway.
                let text = String::from_utf8_lossy(&chunk);

                for payload in parser.push(&text) {
                    let Ok(event) = serde_json::from_str::<Value>(&payload) else {
                        // Keep-alives and stray lines. Not worth failing over.
                        continue;
                    };

                    match event.get("type").and_then(Value::as_str).unwrap_or("") {
                        "message_start" => {
                            if let Some(u) = event.pointer("/message/usage") {
                                usage = Some(usage_from(u));
                            }
                        }
                        "content_block_start" => {
                            let Some(index) = event.get("index").and_then(Value::as_u64) else {
                                continue;
                            };
                            let block = event.get("content_block").cloned().unwrap_or_default();
                            let kind = block
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string();

                            let read = |key: &str| {
                                block
                                    .get(key)
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string()
                            };

                            blocks.insert(
                                index,
                                PartialBlock {
                                    id: read("id"),
                                    name: read("name"),
                                    text: read("text"),
                                    thinking: read("thinking"),
                                    data: read("data"),
                                    kind,
                                    ..Default::default()
                                },
                            );
                        }
                        "content_block_delta" => {
                            let Some(index) = event.get("index").and_then(Value::as_u64) else {
                                continue;
                            };
                            let Some(block) = blocks.get_mut(&index) else {
                                continue;
                            };
                            let delta = event.get("delta").cloned().unwrap_or_default();
                            let kind = delta.get("type").and_then(Value::as_str).unwrap_or("");
                            let read = |key: &str| {
                                delta.get(key).and_then(Value::as_str).unwrap_or_default()
                            };

                            match kind {
                                "text_delta" => {
                                    let text = read("text");
                                    block.text.push_str(text);
                                    yield StreamEvent::Delta { text: text.to_string() };
                                }
                                "thinking_delta" => {
                                    let text = read("thinking");
                                    block.thinking.push_str(text);
                                    yield StreamEvent::Reasoning { text: text.to_string() };
                                }
                                // Never emitted upward on its own - it is only
                                // meaningful attached to its thinking block.
                                "signature_delta" => block.signature.push_str(read("signature")),
                                // Buffered: half a JSON argument string is not
                                // actionable by anyone.
                                "input_json_delta" => block.json.push_str(read("partial_json")),
                                _ => {}
                            }
                        }
                        "message_delta" => {
                            if let Some(reason) =
                                event.pointer("/delta/stop_reason").and_then(Value::as_str)
                            {
                                finish = Some(finish_from(reason));
                            }
                            // The real output-token count arrives here,
                            // replacing the placeholder from message_start.
                            if let Some(u) = event.get("usage") {
                                let reported = usage_from(u);
                                usage = Some(match usage {
                                    Some(existing) => Usage {
                                        input_tokens: existing.input_tokens.max(reported.input_tokens),
                                        output_tokens: reported.output_tokens.max(existing.output_tokens),
                                        cache_read_tokens: existing
                                            .cache_read_tokens
                                            .max(reported.cache_read_tokens),
                                        cache_write_tokens: existing
                                            .cache_write_tokens
                                            .max(reported.cache_write_tokens),
                                    },
                                    None => reported,
                                });
                            }
                        }
                        "error" => {
                            let message = event
                                .pointer("/error/message")
                                .and_then(Value::as_str)
                                .unwrap_or("The provider reported an error.")
                                .to_string();
                            // Mapped onto a real status so it joins the
                            // ordinary retry-on-transient path.
                            let status = matches!(
                                event.pointer("/error/type").and_then(Value::as_str),
                                Some("overloaded_error")
                            )
                            .then_some(529);
                            failed = Some((message, status));
                            break 'reading;
                        }
                        _ => {}
                    }
                }
            }

            if let Some((message, status)) = failed {
                yield StreamEvent::Error { message, status };
                return;
            }

            // Blocks are read out in index order; BTreeMap keeps them there.
            let ordered: Vec<PartialBlock> = blocks.into_values().collect();

            let thinking: Vec<ThinkingBlock> = ordered
                .iter()
                .filter_map(|block| match block.kind.as_str() {
                    "thinking" => Some(ThinkingBlock::Thinking {
                        thinking: block.thinking.clone(),
                        signature: (!block.signature.is_empty())
                            .then(|| block.signature.clone()),
                    }),
                    "redacted_thinking" => Some(ThinkingBlock::RedactedThinking {
                        data: block.data.clone(),
                    }),
                    _ => None,
                })
                .collect();

            if !thinking.is_empty() {
                yield StreamEvent::Thinking { blocks: thinking };
            }

            let calls: Vec<ToolCall> = ordered
                .iter()
                .enumerate()
                .filter(|(_, b)| b.kind == "tool_use" && !b.name.is_empty())
                .map(|(position, block)| ToolCall {
                    id: if block.id.is_empty() {
                        ToolCallId::from_existing(format!("call_{position}"))
                    } else {
                        ToolCallId::from_existing(block.id.clone())
                    },
                    name: block.name.clone(),
                    // An empty string is not valid JSON, so a call with no
                    // arguments gets an explicit empty object.
                    arguments: if block.json.is_empty() {
                        "{}".to_string()
                    } else {
                        block.json.clone()
                    },
                })
                .collect();

            if !calls.is_empty() {
                yield StreamEvent::Tool { calls };
            }

            yield StreamEvent::Done { finish, usage };
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::message::Entry;

    fn config() -> ProviderConfig {
        ProviderConfig::new("anthropic", "https://api.anthropic.com/v1", "sk-test")
    }

    fn request() -> ChatRequest {
        ChatRequest {
            model: "claude-sonnet-4".into(),
            system: "be helpful".into(),
            history: vec![Entry::user("hi")],
            ..Default::default()
        }
    }

    #[test]
    fn the_api_version_is_pinned() {
        let provider = AnthropicProvider::new(config());
        let headers = provider.headers();
        assert_eq!(headers["anthropic-version"], API_VERSION);
        assert_eq!(headers["x-api-key"], "sk-test");
    }

    /// A gateway bringing its own auth must not end up with both headers.
    #[test]
    fn a_configured_auth_header_suppresses_ours() {
        let provider = AnthropicProvider::new(
            config().with_header("Authorization", "Bearer gateway-token"),
        );
        let headers = provider.headers();
        assert!(!headers.contains_key("x-api-key"));
        assert_eq!(headers["authorization"], "Bearer gateway-token");
    }

    #[test]
    fn max_tokens_is_always_set() {
        let provider = AnthropicProvider::new(config());
        let built = provider.build(&request());
        assert_eq!(built.max_tokens, DEFAULT_MAX_TOKENS);
        assert!(built.stream);
    }

    /// Thinking needs room for itself *and* the answer, so the ceiling rises
    /// to accommodate both rather than starving the reply.
    #[test]
    fn thinking_raises_the_token_ceiling() {
        let provider = AnthropicProvider::new(config());
        let built = provider.build(&ChatRequest {
            thinking_budget: Some(10_000),
            ..request()
        });

        assert!(built.max_tokens >= 10_000 + ANSWER_ALLOWANCE);
        let thinking = built.thinking.unwrap();
        assert_eq!(thinking.kind, "enabled");
        assert_eq!(thinking.budget_tokens, 10_000);
    }

    /// The API refuses sampling controls alongside thinking, so they are
    /// dropped rather than sent and rejected.
    #[test]
    fn sampling_controls_are_dropped_when_thinking_is_on() {
        let provider = AnthropicProvider::new(config());

        let without = provider.build(&ChatRequest {
            temperature: Some(0.7),
            top_p: Some(0.9),
            ..request()
        });
        assert_eq!(without.temperature, Some(0.7));
        assert_eq!(without.top_p, Some(0.9));

        let with = provider.build(&ChatRequest {
            temperature: Some(0.7),
            top_p: Some(0.9),
            thinking_budget: Some(4_000),
            ..request()
        });
        assert_eq!(with.temperature, None);
        assert_eq!(with.top_p, None);
    }

    #[test]
    fn the_thinking_budget_leaves_room_for_an_answer() {
        let provider = AnthropicProvider::new(config());
        let built = provider.build(&ChatRequest {
            max_tokens: Some(4_000),
            thinking_budget: Some(100_000),
            ..request()
        });
        let thinking = built.thinking.unwrap();
        assert!(thinking.budget_tokens <= built.max_tokens - MIN_TOKENS);
    }

    #[test]
    fn tools_are_renamed_to_input_schema() {
        let provider = AnthropicProvider::new(config());
        let built = provider.build(&ChatRequest {
            tools: vec![inertia_core::ToolSpec::new(
                "read",
                "reads a file",
                serde_json::json!({"type": "object"}),
            )],
            ..request()
        });
        assert_eq!(built.tools[0].name, "read");
        assert_eq!(built.tools[0].input_schema["type"], "object");
    }

    #[test]
    fn failures_are_described_in_plain_language() {
        assert!(describe_failure(401, "u").contains("key was rejected"));
        assert!(describe_failure(404, "https://x/v1").contains("https://x/v1"));
        assert!(describe_failure(529, "u").contains("overloaded"));
        assert!(describe_failure(418, "u").contains("418"));
    }

    #[test]
    fn stop_reasons_map_onto_ours() {
        assert_eq!(finish_from("end_turn"), FinishReason::Stop);
        assert_eq!(finish_from("max_tokens"), FinishReason::Length);
        assert_eq!(finish_from("tool_use"), FinishReason::ToolUse);
        assert_eq!(finish_from("something_new"), FinishReason::Other);
    }

    #[test]
    fn usage_reads_the_cache_counters() {
        let usage = usage_from(&serde_json::json!({
            "input_tokens": 10,
            "output_tokens": 20,
            "cache_read_input_tokens": 100,
            "cache_creation_input_tokens": 5,
        }));
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.cache_read_tokens, 100);
        assert_eq!(usage.cache_write_tokens, 5);
        assert_eq!(usage.total_input(), 115);
    }
}

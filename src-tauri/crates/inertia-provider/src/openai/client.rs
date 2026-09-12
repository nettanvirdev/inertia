//! The OpenAI chat-completions transport.
//!
//! The universal one. No vendor branches: anything speaking chat-completions
//! works, including local runtimes like Ollama, llama.cpp and vLLM. That is
//! why the auth header is omitted entirely when there is no key - sending
//! `Bearer undefined` to a keyless local server produces a confusing 401,
//! while sending nothing is the honest request.

use std::collections::BTreeMap;

use async_stream::stream;
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures_util::StreamExt;
use inertia_core::message::ToolCall;
use inertia_core::provider::{
    ChatRequest, FinishReason, ModelInfo, Provider, StreamEvent, Usage,
};
use inertia_core::ToolCallId;
use secrecy::ExposeSecret;
use serde_json::{json, Value};

use crate::config::ProviderConfig;
use crate::sse::SseParser;

use super::translate::to_messages;

/// A model list larger than this is not a model list.
const MAX_MODELS: usize = 2_000;

#[derive(Debug)]
pub struct OpenAiProvider {
    config: ProviderConfig,
    http: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn with_client(config: ProviderConfig, http: reqwest::Client) -> Self {
        Self { config, http }
    }

    fn headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));

        // Omitted entirely when there is no key: a keyless local runtime
        // answers an unauthenticated request happily and a `Bearer undefined`
        // one with a 401 nobody can diagnose.
        let key = self.config.api_key.expose_secret();
        if !key.is_empty() && !self.config.overrides("authorization") {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {key}")) {
                headers.insert("authorization", value);
            }
        }

        for (name, value) in &self.config.headers {
            if let (Ok(name), Ok(value)) =
                (name.parse::<HeaderName>(), HeaderValue::from_str(value))
            {
                headers.insert(name, value);
            }
        }

        headers
    }

    fn build(&self, request: &ChatRequest) -> Value {
        let mut body = json!({
            "model": request.model,
            "messages": to_messages(&request.system, &request.history),
            "stream": true,
            // Always asked for: several gateways report usage only on request,
            // and a turn with no token count breaks the usage meter silently.
            "stream_options": { "include_usage": true },
        });

        // Built as an object immediately above, so this cannot fail - but a
        // panic in a provider is a crashed app, and an empty request that gets
        // a clear 400 back is a far better failure than that.
        let Some(map) = body.as_object_mut() else {
            return body;
        };

        if !request.tools.is_empty() {
            map.insert(
                "tools".into(),
                Value::Array(
                    request
                        .tools
                        .iter()
                        .map(|t| {
                            json!({
                                "type": "function",
                                "function": {
                                    "name": t.name,
                                    "description": t.description,
                                    "parameters": t.parameters,
                                }
                            })
                        })
                        .collect(),
                ),
            );
            map.insert("tool_choice".into(), json!("auto"));
        }

        if let Some(temperature) = request.temperature {
            map.insert("temperature".into(), json!(temperature));
        }
        if let Some(max_tokens) = request.max_tokens {
            map.insert("max_tokens".into(), json!(max_tokens));
        }
        if let Some(top_p) = request.top_p {
            map.insert("top_p".into(), json!(top_p));
        }
        if let Some(effort) = &request.reasoning_effort {
            map.insert("reasoning_effort".into(), json!(effort));
        }

        body
    }
}

pub fn describe_failure(status: u16, url: &str) -> String {
    match status {
        401 => "The API key was rejected.".into(),
        403 => "The key is valid but not allowed to use this.".into(),
        404 => format!("Nothing is at {url}. Check the base URL."),
        429 => "Rate limited by the provider.".into(),
        500 => "The provider had an internal error.".into(),
        502 => "The provider was unreachable.".into(),
        503 => "The provider is overloaded.".into(),
        other => format!("The provider answered {other}."),
    }
}

/// One tool call being assembled from fragments.
#[derive(Default, Debug)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

fn finish_from(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolUse,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

fn usage_from(value: &Value) -> Usage {
    let read = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        input_tokens: read("prompt_tokens"),
        output_tokens: read("completion_tokens"),
        cache_read_tokens: value
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_write_tokens: 0,
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    fn id(&self) -> &str {
        &self.config.id
    }

    async fn list_models(&self) -> inertia_core::Result<Vec<ModelInfo>> {
        let url = self.config.endpoint("models");
        let response = self
            .http
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| inertia_core::Error::Network(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            // Not every endpoint implements this route, and the user can still
            // type a model id by hand.
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

        // Three shapes in the wild: a bare array, `{data:[...]}`, and
        // `{models:[...]}`. All three are common enough to be worth accepting.
        let rows = body
            .as_array()
            .or_else(|| body.get("data").and_then(Value::as_array))
            .or_else(|| body.get("models").and_then(Value::as_array))
            .cloned()
            .unwrap_or_default();

        let mut models: Vec<ModelInfo> = rows
            .iter()
            .take(MAX_MODELS)
            .filter_map(|row| {
                // A row may be a bare string.
                let id = match row {
                    Value::String(id) => id.clone(),
                    _ => row.get("id").and_then(Value::as_str)?.to_string(),
                };
                let label = row
                    .get("name")
                    .or_else(|| row.get("display_name"))
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string();
                let context = row
                    .get("context_length")
                    .or_else(|| row.get("context_window"))
                    .and_then(Value::as_u64)
                    .map(|n| n as u32)
                    .or_else(|| crate::models::context_window(&id));
                Some(ModelInfo { id, label, context })
            })
            .collect();

        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }

    fn stream_chat(&self, request: ChatRequest) -> BoxStream<'static, StreamEvent> {
        let body = self.build(&request);
        let url = self.config.endpoint("chat/completions");
        let headers = self.headers();
        let http = self.http.clone();
        let model = request.model.clone();

        Box::pin(stream! {
            yield StreamEvent::Start { model: model.clone() };

            let response = match http.post(&url).headers(headers).json(&body).send().await {
                Ok(response) => response,
                Err(e) => {
                    yield StreamEvent::Error {
                        message: describe_network_error(&e, &url),
                        status: None,
                    };
                    return;
                }
            };

            let status = response.status().as_u16();
            if !response.status().is_success() {
                let detail = response.text().await.ok().and_then(|raw| {
                    serde_json::from_str::<Value>(&raw)
                        .ok()
                        .and_then(|v| {
                            v.pointer("/error/message")
                                .or_else(|| v.get("message"))
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        // Some servers answer with plain text.
                        .or_else(|| (!raw.trim().is_empty()).then(|| raw.chars().take(400).collect()))
                });

                let message = match detail {
                    Some(detail) => format!("{} {detail}", describe_failure(status, &url)),
                    None => describe_failure(status, &url),
                };
                yield StreamEvent::Error { message, status: Some(status) };
                return;
            }

            let mut parser = SseParser::new();
            // Keyed by the fragment index the provider supplies, so calls stay
            // in the order the model asked for them.
            let mut calls: BTreeMap<u64, PartialCall> = BTreeMap::new();
            let mut usage: Option<Usage> = None;
            let mut finish: Option<FinishReason> = None;
            let mut failed: Option<String> = None;

            let mut bytes = response.bytes_stream();
            'reading: while let Some(chunk) = bytes.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(e) => {
                        failed = Some(format!("The connection dropped: {e}"));
                        break 'reading;
                    }
                };

                let text = String::from_utf8_lossy(&chunk);
                for payload in parser.push(&text) {
                    if payload == "[DONE]" {
                        continue;
                    }
                    let Ok(event) = serde_json::from_str::<Value>(&payload) else {
                        continue;
                    };

                    // A mid-stream error arrives in the body rather than the
                    // status, since the headers were already sent.
                    if let Some(message) = event
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                    {
                        failed = Some(message.to_string());
                        break 'reading;
                    }

                    if let Some(u) = event.get("usage").filter(|u| !u.is_null()) {
                        usage = Some(usage_from(u));
                    }

                    let Some(choice) = event.pointer("/choices/0") else {
                        continue;
                    };

                    if let Some(reason) = choice
                        .get("finish_reason")
                        .and_then(Value::as_str)
                    {
                        finish = Some(finish_from(reason));
                    }

                    let delta = choice
                        .get("delta")
                        .or_else(|| choice.get("message"))
                        .cloned()
                        .unwrap_or_default();

                    // Both spellings occur in the wild.
                    if let Some(text) = delta
                        .get("reasoning_content")
                        .or_else(|| delta.get("reasoning"))
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                    {
                        yield StreamEvent::Reasoning { text: text.to_string() };
                    }

                    if let Some(text) = delta
                        .get("content")
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                    {
                        yield StreamEvent::Delta { text: text.to_string() };
                    }

                    if let Some(fragments) = delta.get("tool_calls").and_then(Value::as_array) {
                        for fragment in fragments {
                            // `index` is the only field reliably present on
                            // every fragment; id and name arrive once, at the
                            // start, and arguments dribble in.
                            let index = fragment
                                .get("index")
                                .and_then(Value::as_u64)
                                .unwrap_or(calls.len() as u64);
                            let call = calls.entry(index).or_default();

                            if let Some(id) = fragment.get("id").and_then(Value::as_str) {
                                if !id.is_empty() {
                                    call.id = id.to_string();
                                }
                            }
                            if let Some(name) = fragment
                                .pointer("/function/name")
                                .and_then(Value::as_str)
                            {
                                if !name.is_empty() {
                                    call.name = name.to_string();
                                }
                            }
                            if let Some(args) = fragment
                                .pointer("/function/arguments")
                                .and_then(Value::as_str)
                            {
                                call.arguments.push_str(args);
                            }
                        }
                    }
                }
            }

            if let Some(message) = failed {
                yield StreamEvent::Error { message, status: None };
                return;
            }

            let assembled: Vec<ToolCall> = calls
                .into_values()
                .enumerate()
                .filter(|(_, call)| !call.name.is_empty())
                .map(|(position, call)| ToolCall {
                    id: if call.id.is_empty() {
                        ToolCallId::from_existing(format!("call_{position}"))
                    } else {
                        ToolCallId::from_existing(call.id)
                    },
                    name: call.name,
                    arguments: if call.arguments.is_empty() {
                        "{}".to_string()
                    } else {
                        call.arguments
                    },
                })
                .collect();

            if !assembled.is_empty() {
                yield StreamEvent::Tool { calls: assembled };
            }

            yield StreamEvent::Done { finish, usage };
        })
    }
}

/// Turns a transport failure into something actionable.
///
/// "Connection refused" is a fact; "is the server running?" is the question
/// the person actually needs to answer.
fn describe_network_error(error: &reqwest::Error, url: &str) -> String {
    if error.is_timeout() {
        return "The provider did not answer in time.".into();
    }
    if error.is_connect() {
        return format!("Nothing is listening at {url}. Is the server running?");
    }
    if error.is_request() {
        return format!("The request could not be sent: {error}");
    }
    format!("The request failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::message::Entry;

    fn config(key: &str) -> ProviderConfig {
        ProviderConfig::new("openai", "https://api.openai.com/v1", key)
    }

    fn request() -> ChatRequest {
        ChatRequest {
            model: "gpt-4o".into(),
            system: "be helpful".into(),
            history: vec![Entry::user("hi")],
            ..Default::default()
        }
    }

    #[test]
    fn a_key_becomes_a_bearer_header() {
        let provider = OpenAiProvider::new(config("sk-test"));
        assert_eq!(provider.headers()["authorization"], "Bearer sk-test");
    }

    /// The point of omitting rather than sending an empty bearer: a keyless
    /// local runtime answers the honest request and 401s the other one.
    #[test]
    fn no_key_means_no_auth_header() {
        let provider = OpenAiProvider::new(config(""));
        assert!(!provider.headers().contains_key("authorization"));
    }

    #[test]
    fn usage_is_always_requested() {
        let provider = OpenAiProvider::new(config("k"));
        let body = provider.build(&request());
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn optional_settings_are_omitted_rather_than_nulled() {
        let provider = OpenAiProvider::new(config("k"));
        let body = provider.build(&request());
        assert!(body.get("temperature").is_none());
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn tools_bring_a_tool_choice_with_them() {
        let provider = OpenAiProvider::new(config("k"));
        let body = provider.build(&ChatRequest {
            tools: vec![inertia_core::ToolSpec::new(
                "read",
                "reads",
                json!({"type": "object"}),
            )],
            ..request()
        });
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "read");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn reasoning_effort_is_passed_through_when_set() {
        let provider = OpenAiProvider::new(config("k"));
        let body = provider.build(&ChatRequest {
            reasoning_effort: Some("high".into()),
            ..request()
        });
        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn finish_reasons_map_onto_ours() {
        assert_eq!(finish_from("stop"), FinishReason::Stop);
        assert_eq!(finish_from("tool_calls"), FinishReason::ToolUse);
        assert_eq!(finish_from("length"), FinishReason::Length);
        assert_eq!(finish_from("who_knows"), FinishReason::Other);
    }

    #[test]
    fn usage_reads_cached_prompt_tokens() {
        let usage = usage_from(&json!({
            "prompt_tokens": 100,
            "completion_tokens": 20,
            "prompt_tokens_details": { "cached_tokens": 80 },
        }));
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_read_tokens, 80);
    }
}

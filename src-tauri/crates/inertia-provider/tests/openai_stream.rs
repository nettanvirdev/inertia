//! The OpenAI transport over a socket.
//!
//! The fragment assembly is the risky part: `id` and `name` arrive once at the
//! start of a call, `arguments` dribbles in a few characters at a time, and
//! `index` is the only field reliably present on every fragment.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use futures::StreamExt;
use inertia_core::message::Entry;
use inertia_core::provider::{ChatRequest, FinishReason, Provider, StreamEvent};
use inertia_provider::{OpenAiProvider, ProviderConfig};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(events: &[&str]) -> String {
    let mut body: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
    body.push_str("data: [DONE]\n\n");
    body
}

async fn stream_against(body: String) -> Vec<StreamEvent> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let provider = OpenAiProvider::new(ProviderConfig::new("openai", server.uri(), "sk-test"));

    provider
        .stream_chat(ChatRequest {
            model: "gpt-4o".into(),
            history: vec![Entry::user("hi")],
            ..Default::default()
        })
        .collect()
        .await
}

fn prose(events: &[StreamEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Delta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn text_streams_and_usage_is_reported() {
    let body = sse(&[
        r#"{"choices":[{"delta":{"content":"Hello"}}]}"#,
        r#"{"choices":[{"delta":{"content":", world"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":3}}"#,
    ]);

    let events = stream_against(body).await;
    assert_eq!(prose(&events), "Hello, world");

    let Some(StreamEvent::Done { finish, usage }) = events.last() else {
        panic!("expected Done, got {:?}", events.last());
    };
    assert_eq!(*finish, Some(FinishReason::Stop));
    assert_eq!(usage.unwrap().input_tokens, 11);
}

/// The shape that actually arrives: identity once, then arguments a few
/// characters at a time.
#[tokio::test]
async fn tool_call_fragments_are_assembled_by_index() {
    let body = sse(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":""}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"pa"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.txt\"}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);

    let events = stream_against(body).await;

    let tool_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::Tool { .. }))
        .collect();
    assert_eq!(tool_events.len(), 1, "tools must be emitted exactly once");

    let StreamEvent::Tool { calls } = tool_events[0] else {
        unreachable!()
    };
    assert_eq!(calls[0].id.as_str(), "call_a");
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].parsed_arguments()["path"], "a.txt");
}

/// Two calls interleave their fragments, and each must accumulate into its own
/// call rather than into whichever arrived last.
#[tokio::test]
async fn parallel_calls_do_not_bleed_into_each_other() {
    let body = sse(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":""}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"grep","arguments":""}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"a\":1}"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"b\":2}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ]);

    let events = stream_against(body).await;
    let StreamEvent::Tool { calls } = events
        .iter()
        .find(|e| matches!(e, StreamEvent::Tool { .. }))
        .unwrap()
    else {
        unreachable!()
    };

    assert_eq!(calls.len(), 2);
    // Index order, not arrival order.
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].parsed_arguments()["a"], 1);
    assert_eq!(calls[1].name, "grep");
    assert_eq!(calls[1].parsed_arguments()["b"], 2);
}

#[tokio::test]
async fn reasoning_is_streamed_under_either_spelling() {
    for field in ["reasoning_content", "reasoning"] {
        let body = sse(&[
            &format!(r#"{{"choices":[{{"delta":{{"{field}":"thinking it over"}}}}]}}"#),
            r#"{"choices":[{"delta":{"content":"done"}}]}"#,
        ]);
        let events = stream_against(body).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::Reasoning { text } if text == "thinking it over")),
            "{field} was not surfaced"
        );
    }
}

/// Once the headers are sent the status cannot change, so a failure arrives in
/// the body instead.
#[tokio::test]
async fn a_mid_stream_error_ends_the_turn() {
    let body = sse(&[
        r#"{"choices":[{"delta":{"content":"partial"}}]}"#,
        r#"{"error":{"message":"the model ran out of capacity"}}"#,
    ]);

    let events = stream_against(body).await;
    let Some(StreamEvent::Error { message, .. }) = events.last() else {
        panic!("expected an error, got {:?}", events.last());
    };
    assert!(message.contains("ran out of capacity"));
    assert_eq!(prose(&events), "partial");
}

#[tokio::test]
async fn a_rejected_key_keeps_the_providers_own_detail() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "message": "Incorrect API key provided" }
        })))
        .mount(&server)
        .await;

    let provider = OpenAiProvider::new(ProviderConfig::new("openai", server.uri(), "bad"));
    let events: Vec<StreamEvent> = provider
        .stream_chat(ChatRequest {
            model: "gpt-4o".into(),
            history: vec![Entry::user("hi")],
            ..Default::default()
        })
        .collect()
        .await;

    let Some(StreamEvent::Error { message, status }) = events.last() else {
        panic!("expected an error");
    };
    assert_eq!(*status, Some(401));
    assert!(message.contains("key was rejected"));
    assert!(message.contains("Incorrect API key"));
}

/// Three shapes occur in the wild and all three are common enough to accept.
#[tokio::test]
async fn model_lists_are_read_in_every_shape_that_occurs() {
    let shapes = [
        serde_json::json!([{ "id": "b" }, { "id": "a" }]),
        serde_json::json!({ "data": [{ "id": "b" }, { "id": "a" }] }),
        serde_json::json!({ "models": [{ "id": "b" }, { "id": "a" }] }),
        // Bare strings, which local runtimes emit.
        serde_json::json!(["b", "a"]),
    ];

    for shape in shapes {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(shape.clone()))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new(ProviderConfig::new("openai", server.uri(), "k"));
        let models = provider.list_models().await.unwrap();

        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"], "sorted by id, for shape {shape}");
    }
}

/// A local runtime with no key must still work end to end.
#[tokio::test]
async fn a_keyless_local_runtime_is_supported() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                sse(&[r#"{"choices":[{"delta":{"content":"local reply"}}]}"#]),
                "text/event-stream",
            ),
        )
        .mount(&server)
        .await;

    let provider = OpenAiProvider::new(ProviderConfig::new("local", server.uri(), ""));
    let events: Vec<StreamEvent> = provider
        .stream_chat(ChatRequest {
            model: "llama-3".into(),
            history: vec![Entry::user("hi")],
            ..Default::default()
        })
        .collect()
        .await;

    assert_eq!(prose(&events), "local reply");
}

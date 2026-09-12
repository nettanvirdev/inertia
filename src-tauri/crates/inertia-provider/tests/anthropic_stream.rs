//! The Anthropic transport against a real HTTP server.
//!
//! The unit tests cover request building and translation; these cover the part
//! that only breaks over a socket - SSE framing, block assembly across
//! interleaved indices, and how failures surface.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use futures::StreamExt;
use inertia_core::message::Entry;
use inertia_core::provider::{ChatRequest, FinishReason, Provider, StreamEvent};
use inertia_provider::{AnthropicProvider, ProviderConfig};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Builds an SSE body from event payloads.
fn sse(events: &[&str]) -> String {
    events
        .iter()
        .map(|e| format!("data: {e}\n\n"))
        .collect::<String>()
}

async fn stream_against(body: String, status: u16) -> Vec<StreamEvent> {
    let server = MockServer::start().await;

    let response = if status == 200 {
        ResponseTemplate::new(200)
            .set_body_raw(body, "text/event-stream")
    } else {
        ResponseTemplate::new(status).set_body_raw(body, "application/json")
    };

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(response)
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new(ProviderConfig::new(
        "anthropic",
        server.uri(),
        "sk-test",
    ));

    let request = ChatRequest {
        model: "claude-sonnet-4".into(),
        system: "be helpful".into(),
        history: vec![Entry::user("hi")],
        ..Default::default()
    };

    provider.stream_chat(request).collect().await
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

/// The reported case: a gateway mounted at a bare path, serving its routes
/// where the base URL says rather than under `/v1`.
///
/// `https://api.minimax.io/anthropic` was joined naively to
/// `/anthropic/messages`, which nobody serves, and the turn failed with
/// "Nothing is at ...". The first candidate is still `/v1` - that is where most
/// endpoints keep their routes - but a 404 on it must fall through to the base
/// as written rather than giving up.
#[tokio::test]
async fn a_404_falls_through_to_the_other_base_url() {
    let server = MockServer::start().await;

    let body = sse(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_stop"}"#,
    ]);

    // Nothing under /v1, which is what a gateway mounted without one answers.
    Mock::given(method("POST"))
        .and(path("/gateway/v1/messages"))
        .respond_with(ResponseTemplate::new(404).set_body_string("no such route"))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/gateway/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;

    let base = format!("{}/gateway", server.uri());
    inertia_provider::base::forget(Some(&base));

    let provider = AnthropicProvider::new(ProviderConfig::new("gw", base.clone(), "sk-test"));
    let request = ChatRequest {
        model: "claude-sonnet-4".into(),
        system: "be helpful".into(),
        history: vec![Entry::user("hi")],
        ..Default::default()
    };

    let events: Vec<StreamEvent> = provider.stream_chat(request).collect().await;
    assert_eq!(prose(&events), "Hi", "the retry should have reached the gateway");
    assert!(
        !events.iter().any(|e| matches!(e, StreamEvent::Error { .. })),
        "a base URL that needed interpreting is not a failure: {events:?}"
    );

    inertia_provider::base::forget(Some(&base));
}

#[tokio::test]
async fn a_text_reply_streams_and_reports_usage() {
    let body = sse(&[
        r#"{"type":"message_start","message":{"usage":{"input_tokens":12,"output_tokens":1}}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":", world"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":7}}"#,
        r#"{"type":"message_stop"}"#,
    ]);

    let events = stream_against(body, 200).await;

    assert!(matches!(events.first(), Some(StreamEvent::Start { .. })));
    assert_eq!(prose(&events), "Hello, world");

    let Some(StreamEvent::Done { finish, usage }) = events.last() else {
        panic!("expected Done, got {:?}", events.last());
    };
    assert_eq!(*finish, Some(FinishReason::Stop));
    let usage = usage.expect("usage should be reported");
    assert_eq!(usage.input_tokens, 12);
    // The real output count arrives in message_delta and replaces the
    // placeholder from message_start.
    assert_eq!(usage.output_tokens, 7);
}

/// Tool arguments dribble in a few characters at a time and must be emitted
/// once, whole - half a JSON string is not actionable.
#[tokio::test]
async fn tool_arguments_are_assembled_before_being_emitted() {
    let body = sse(&[
        r#"{"type":"message_start","message":{"usage":{"input_tokens":5,"output_tokens":0}}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"read"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"pa"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"th\":\"a.txt\"}"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
    ]);

    let events = stream_against(body, 200).await;

    let calls: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::Tool { calls } => Some(calls.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(calls.len(), 1, "tool calls must be emitted exactly once");
    let call = &calls[0][0];
    assert_eq!(call.name, "read");
    assert_eq!(call.id.as_str(), "toolu_1");
    assert_eq!(call.parsed_arguments()["path"], "a.txt");
}

/// Blocks arrive interleaved by index and must be read back in index order,
/// with thinking before tools.
#[tokio::test]
async fn interleaved_blocks_are_ordered_by_index() {
    let body = sse(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"working"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"let me see"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-abc"}}"#,
        r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_9","name":"grep"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
    ]);

    let events = stream_against(body, 200).await;

    let thinking_at = events
        .iter()
        .position(|e| matches!(e, StreamEvent::Thinking { .. }))
        .expect("thinking should be emitted");
    let tool_at = events
        .iter()
        .position(|e| matches!(e, StreamEvent::Tool { .. }))
        .expect("tools should be emitted");
    assert!(thinking_at < tool_at, "thinking must precede tools");

    let StreamEvent::Thinking { blocks } = &events[thinking_at] else {
        unreachable!()
    };
    // The signature arrived as its own delta and must be attached, or the
    // block cannot be replayed next turn.
    assert!(blocks[0].is_replayable(), "the signature was lost");

    // Reasoning is streamed for the user to watch, separately from the signed
    // block kept for replay.
    assert!(events
        .iter()
        .any(|e| matches!(e, StreamEvent::Reasoning { text } if text == "let me see")));
}

/// A call with no arguments must send `{}`, because an empty string is not
/// valid JSON.
#[tokio::test]
async fn a_tool_call_with_no_arguments_gets_an_empty_object() {
    let body = sse(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"now"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
    ]);

    let events = stream_against(body, 200).await;
    let StreamEvent::Tool { calls } = events
        .iter()
        .find(|e| matches!(e, StreamEvent::Tool { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(calls[0].arguments, "{}");
}

#[tokio::test]
async fn a_rejected_key_is_reported_in_plain_language() {
    let body = r#"{"error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
    let events = stream_against(body.to_string(), 401).await;

    let Some(StreamEvent::Error { message, status }) = events.last() else {
        panic!("expected an error, got {:?}", events.last());
    };
    assert_eq!(*status, Some(401));
    assert!(message.contains("key was rejected"), "got {message}");
    // The provider's own detail is kept too - it is often the specific thing
    // the user needs to fix.
    assert!(message.contains("invalid x-api-key"), "got {message}");
}

/// Anthropic reports overload as a mid-stream event rather than a status, and
/// it is genuinely transient, so it is mapped onto 529 to join the ordinary
/// retry path.
#[tokio::test]
async fn a_mid_stream_overload_becomes_a_retryable_status() {
    let body = sse(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"start"}}"#,
        r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
    ]);

    let events = stream_against(body, 200).await;

    let Some(StreamEvent::Error { status, .. }) = events.last() else {
        panic!("expected an error, got {:?}", events.last());
    };
    assert_eq!(*status, Some(529));

    // Prose that already reached the user is still delivered ahead of it.
    assert_eq!(prose(&events), "start");
}

#[tokio::test]
async fn keep_alives_and_unknown_events_are_ignored() {
    let body = format!(
        ": keep-alive\n\n{}",
        sse(&[
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}"#,
            r#"{"type":"something_new","data":"from a future api version"}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        ])
    );

    let events = stream_against(body, 200).await;
    assert_eq!(prose(&events), "ok");
    assert!(matches!(events.last(), Some(StreamEvent::Done { .. })));
}

#[tokio::test]
async fn an_unreachable_provider_fails_without_panicking() {
    let provider = AnthropicProvider::new(ProviderConfig::new(
        "anthropic",
        // Nothing is listening here.
        "http://127.0.0.1:1",
        "sk-test",
    ));

    let events: Vec<StreamEvent> = provider
        .stream_chat(ChatRequest {
            model: "claude-sonnet-4".into(),
            history: vec![Entry::user("hi")],
            ..Default::default()
        })
        .collect()
        .await;

    assert!(matches!(events.last(), Some(StreamEvent::Error { .. })));
}

/// A missing `/models` route is normal - plenty of gateways do not implement
/// it - and must not be treated as a failure.
#[tokio::test]
async fn a_missing_model_list_is_an_empty_list_not_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new(ProviderConfig::new("anthropic", server.uri(), "sk-test"));

    assert_eq!(provider.list_models().await.unwrap(), vec![]);
}

#[tokio::test]
async fn listed_models_get_their_context_window_from_the_table() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                { "id": "claude-sonnet-4-5", "display_name": "Claude Sonnet 4.5" }
            ]
        })))
        .mount(&server)
        .await;

    let provider =
        AnthropicProvider::new(ProviderConfig::new("anthropic", server.uri(), "sk-test"));

    let models = provider.list_models().await.unwrap();
    assert_eq!(models[0].id, "claude-sonnet-4-5");
    assert_eq!(models[0].label, "Claude Sonnet 4.5");
    // The API does not report this; it comes from the local table.
    assert_eq!(models[0].context, Some(200_000));
}

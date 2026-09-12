//! A provider that reads from a script instead of the network.
//!
//! Two jobs. The obvious one is letting the agent loop run without an API key.
//! The less obvious one matters more: it **records every request it is
//! handed**, so a test can assert on the assembled system prompt, the
//! translated history, and the tool list. Those are the things most likely to
//! break silently in a port, and they are invisible from the outside
//! otherwise.

use std::collections::VecDeque;

use async_trait::async_trait;
use futures::stream::{self, BoxStream};
use inertia_core::message::ToolCall;
use inertia_core::provider::{
    ChatRequest, FinishReason, ModelInfo, Provider, StreamEvent, Usage,
};
use inertia_core::ToolCallId;
use parking_lot::Mutex;

/// A provider whose replies are decided in advance.
///
/// Each call to `stream_chat` consumes the next scripted turn. When the script
/// runs out it replies with empty prose rather than panicking - a loop that
/// takes one more step than the test expected should fail on an assertion
/// about the conversation, not on a panic from inside the fake.
#[derive(Debug)]
pub struct MockProvider {
    id: String,
    script: Mutex<VecDeque<Vec<StreamEvent>>>,
    requests: Mutex<Vec<ChatRequest>>,
    models: Vec<ModelInfo>,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockProvider {
    pub fn new() -> Self {
        Self {
            id: "mock".to_string(),
            script: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
            models: vec![ModelInfo {
                id: "mock-model".into(),
                label: "Mock Model".into(),
                context: Some(200_000),
            }],
        }
    }

    /// Appends a turn that streams `text` and stops.
    pub fn replying(self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.turn(vec![
            StreamEvent::Start {
                model: "mock-model".into(),
            },
            StreamEvent::Delta { text },
            StreamEvent::Done {
                finish: Some(FinishReason::Stop),
                usage: Some(Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                }),
            },
        ])
    }

    /// Appends a turn that asks for one tool call.
    ///
    /// The loop is expected to run the tool and come back, so this consumes
    /// one scripted turn and the *next* one answers.
    pub fn calling(self, tool: impl Into<String>, arguments: impl serde::Serialize) -> Self {
        let call = ToolCall {
            id: ToolCallId::new(),
            name: tool.into(),
            // Anything serialisable, so a test can pass `json!({..})` or a real
            // argument struct without converting first.
            arguments: serde_json::to_string(&arguments).unwrap_or_else(|_| "{}".into()),
        };
        self.turn(vec![
            StreamEvent::Start {
                model: "mock-model".into(),
            },
            StreamEvent::Tool { calls: vec![call] },
            StreamEvent::Done {
                finish: Some(FinishReason::ToolUse),
                usage: None,
            },
        ])
    }

    /// Appends a turn that fails.
    pub fn failing(self, message: impl Into<String>, status: Option<u16>) -> Self {
        self.turn(vec![
            StreamEvent::Start {
                model: "mock-model".into(),
            },
            StreamEvent::Error {
                message: message.into(),
                status,
            },
        ])
    }

    /// Appends a turn spelled out event by event, for cases the helpers above
    /// do not cover.
    pub fn turn(self, events: Vec<StreamEvent>) -> Self {
        self.script.lock().push_back(events);
        self
    }

    /// Every request this provider was handed, in order.
    ///
    /// This is the assertion surface for prompt assembly: what the system
    /// prompt actually said, which tools were offered, what history survived
    /// windowing.
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().clone()
    }

    /// The most recent request, for the common single-turn case.
    pub fn last_request(&self) -> Option<ChatRequest> {
        self.requests.lock().last().cloned()
    }

    /// How many turns the script still holds. A test that ends with this
    /// non-zero asked for fewer turns than it set up.
    pub fn remaining(&self) -> usize {
        self.script.lock().len()
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn id(&self) -> &str {
        &self.id
    }

    async fn list_models(&self) -> inertia_core::Result<Vec<ModelInfo>> {
        Ok(self.models.clone())
    }

    fn stream_chat(&self, request: ChatRequest) -> BoxStream<'static, StreamEvent> {
        self.requests.lock().push(request);

        let events = self.script.lock().pop_front().unwrap_or_else(|| {
            vec![
                StreamEvent::Start {
                    model: "mock-model".into(),
                },
                StreamEvent::Done {
                    finish: Some(FinishReason::Stop),
                    usage: None,
                },
            ]
        });

        Box::pin(stream::iter(events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    async fn drain(provider: &MockProvider, request: ChatRequest) -> Vec<StreamEvent> {
        provider.stream_chat(request).collect().await
    }

    #[tokio::test]
    async fn it_replies_from_the_script_in_order() {
        let provider = MockProvider::new().replying("first").replying("second");

        let one = drain(&provider, ChatRequest::default()).await;
        assert!(matches!(&one[1], StreamEvent::Delta { text } if text == "first"));

        let two = drain(&provider, ChatRequest::default()).await;
        assert!(matches!(&two[1], StreamEvent::Delta { text } if text == "second"));

        assert_eq!(provider.remaining(), 0);
    }

    // Overrunning the script must not panic - the test should fail on its own
    // assertion, which is far easier to diagnose.
    #[tokio::test]
    async fn it_keeps_answering_after_the_script_runs_out() {
        let provider = MockProvider::new().replying("only one");
        drain(&provider, ChatRequest::default()).await;

        let extra = drain(&provider, ChatRequest::default()).await;
        assert!(extra.last().is_some_and(StreamEvent::is_terminal));
    }

    #[tokio::test]
    async fn it_records_what_it_was_asked() {
        let provider = MockProvider::new().replying("ok");
        let request = ChatRequest {
            model: "mock-model".into(),
            system: "you are a test".into(),
            ..Default::default()
        };
        drain(&provider, request).await;

        let recorded = provider.last_request().unwrap();
        assert_eq!(recorded.system, "you are a test");
    }

    #[tokio::test]
    async fn a_tool_turn_carries_the_arguments_verbatim() {
        let provider = MockProvider::new().calling("read", serde_json::json!({"path": "a.txt"}));
        let events = drain(&provider, ChatRequest::default()).await;

        let StreamEvent::Tool { calls } = &events[1] else {
            panic!("expected a tool event, got {:?}", events[1]);
        };
        assert_eq!(calls[0].name, "read");
        assert_eq!(calls[0].parsed_arguments()["path"], "a.txt");
    }

    #[tokio::test]
    async fn every_scripted_turn_ends_terminally() {
        let provider = MockProvider::new()
            .replying("a")
            .calling("read", serde_json::json!({}))
            .failing("boom", Some(500));

        for _ in 0..3 {
            let events = drain(&provider, ChatRequest::default()).await;
            assert!(
                events.last().is_some_and(StreamEvent::is_terminal),
                "a turn ended without a terminal event: {events:?}"
            );
        }
    }
}

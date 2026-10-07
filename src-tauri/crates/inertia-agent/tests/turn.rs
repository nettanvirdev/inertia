//! End-to-end tests for the turn loop.
//!
//! These run the *real* loop against the *real* registry, with only the
//! provider and the permission gate faked. So what they exercise is the
//! shipping code path: schema validation, permission checks, truncation and
//! transcript assembly all really happen.

// An `unwrap` in a test is a readable assertion, not a latent panic.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value
)]

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use inertia_agent::{Agent, AgentConfig, AgentEvent, StopReason, Turn};
use inertia_core::message::Entry;
use inertia_core::permission::{Action, Rule};
use inertia_core::tool::ToolRegistry;
use inertia_core::SessionId;
use inertia_mock::{Behaviour, MockGate, MockProvider, MockTool, Policy};
use inertia_tools::Registry;
use serde_json::json;

struct Harness {
    events: Vec<AgentEvent>,
    provider: Arc<MockProvider>,
}

impl Harness {
    fn stopped(&self) -> StopReason {
        self.events
            .iter()
            .find_map(|e| match e {
                AgentEvent::Done { stopped, .. } => Some(stopped.clone()),
                _ => None,
            })
            .expect("the turn ended without a Done event")
    }

    fn history(&self) -> Vec<Entry> {
        self.events
            .iter()
            .find_map(|e| match e {
                AgentEvent::Done { history, .. } => Some(history.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn prose(&self) -> String {
        self.events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::Delta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn tool_outputs(&self) -> Vec<String> {
        self.events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ToolFinished { result } => Some(result.output.clone()),
                _ => None,
            })
            .collect()
    }

    fn steps(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, AgentEvent::Step { .. }))
            .count()
    }
}

async fn run(
    provider: MockProvider,
    registry: Registry,
    gate: Arc<MockGate>,
    config: AgentConfig,
) -> Harness {
    let provider = Arc::new(provider);
    let agent = Agent::new(
        provider.clone(),
        Arc::new(registry) as Arc<dyn ToolRegistry>,
        gate,
        config,
    );

    let turn = Turn {
        session: SessionId::new(),
        system: "you are a test".into(),
        history: vec![Entry::user("do the thing")],
        root: PathBuf::from("."),
    };

    let events = agent.run(turn).collect::<Vec<_>>().await;
    Harness { events, provider }
}

fn config() -> AgentConfig {
    AgentConfig {
        model: "mock-model".into(),
        ..Default::default()
    }
}

// ── the ordinary path ───────────────────────────────────────────────────

#[tokio::test]
async fn a_plain_reply_completes_in_one_step() {
    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new().replying("here you go"),
        Registry::new(gate.clone()),
        gate,
        config(),
    )
    .await;

    assert_eq!(harness.stopped(), StopReason::Complete);
    assert_eq!(harness.prose(), "here you go");
    assert_eq!(harness.steps(), 1);

    // The user's message, then the reply.
    let history = harness.history();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].text(), "here you go");
}

#[tokio::test]
async fn a_tool_round_trip_lands_in_the_transcript_in_order() {
    let tool = Arc::new(MockTool::new("echo"));
    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new()
            .calling("echo", json!({"input": "hello"}))
            .replying("done"),
        Registry::new(gate.clone()).with_tool(tool.clone()),
        gate,
        config(),
    )
    .await;

    assert_eq!(harness.stopped(), StopReason::Complete);
    assert_eq!(tool.call_count(), 1);
    assert_eq!(harness.steps(), 2);

    // user → assistant(tool_calls) → tool result → assistant(prose)
    let history = harness.history();
    assert_eq!(history.len(), 4);
    assert!(matches!(&history[0], Entry::User(_)));
    let Entry::Assistant(asked) = &history[1] else {
        panic!("expected the assistant's tool call, got {:?}", history[1]);
    };
    assert_eq!(asked.tool_calls.len(), 1);
    let Entry::Tool(result) = &history[2] else {
        panic!("expected a tool result, got {:?}", history[2]);
    };
    // The result must answer the call that was made, or the next request is
    // rejected outright by strict providers.
    assert_eq!(result.tool_call_id, asked.tool_calls[0].id);
    assert_eq!(result.ok, Some(true));
    assert_eq!(history[3].text(), "done");
}

#[tokio::test]
async fn the_system_prompt_and_tool_list_reach_the_provider() {
    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new().replying("ok"),
        Registry::new(gate.clone()).with_tool(Arc::new(MockTool::new("echo"))),
        gate,
        config(),
    )
    .await;

    let request = harness.provider.last_request().unwrap();
    assert_eq!(request.system, "you are a test");
    assert_eq!(request.tools.len(), 1);
    assert_eq!(request.tools[0].name, "echo");
}

// ── failure, refusal, and the difference between them ───────────────────

/// A tool that fails has produced something the model can read and react to,
/// so the turn carries on. This is the difference between a bad path and a
/// refusal.
#[tokio::test]
async fn a_failing_tool_does_not_end_the_turn() {
    let tool =
        Arc::new(MockTool::new("broken").with_behaviour(Behaviour::Failure("no such file".into())));
    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new()
            .calling("broken", json!({"input": "x"}))
            .replying("I will try something else"),
        Registry::new(gate.clone()).with_tool(tool),
        gate,
        config(),
    )
    .await;

    assert_eq!(harness.stopped(), StopReason::Complete);
    assert_eq!(harness.prose(), "I will try something else");

    let history = harness.history();
    let Entry::Tool(result) = &history[2] else {
        panic!("expected a tool result");
    };
    assert_eq!(result.ok, Some(false));
    assert!(result.content.contains("no such file"));
}

/// A refusal will be a refusal again next time, so looping on it would just
/// ask the user the same question repeatedly.
#[tokio::test]
async fn a_refused_tool_ends_the_turn() {
    let tool = Arc::new(MockTool::new("echo"));
    let gate = Arc::new(MockGate::deny_all());
    let harness = run(
        MockProvider::new().calling("echo", json!({"input": "x"})),
        Registry::new(gate.clone()).with_tool(tool.clone()),
        gate,
        config(),
    )
    .await;

    assert_eq!(harness.stopped(), StopReason::Refused);
    assert_eq!(tool.call_count(), 0);

    // The refusal is still recorded, so the transcript stays well-formed and
    // the model can see what happened if the conversation continues.
    let history = harness.history();
    let Entry::Tool(result) = &history[2] else {
        panic!("expected the refusal to be recorded, got {:?}", history[2]);
    };
    assert_eq!(result.ok, Some(false));
}

#[tokio::test]
async fn a_provider_failure_ends_the_turn_and_says_why() {
    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new().failing("upstream is down", Some(503)),
        Registry::new(gate.clone()),
        gate,
        config(),
    )
    .await;

    match harness.stopped() {
        StopReason::Error { message } => assert!(message.contains("upstream is down")),
        other => panic!("expected an error, got {other:?}"),
    }
}

/// Prose the user already watched appear must survive a mid-stream failure,
/// or the saved transcript disagrees with what was on screen.
#[tokio::test]
async fn prose_streamed_before_a_failure_is_kept() {
    use inertia_core::provider::StreamEvent;

    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new().turn(vec![
            StreamEvent::Start {
                model: "mock-model".into(),
            },
            StreamEvent::Delta {
                text: "I was saying something".into(),
            },
            StreamEvent::Error {
                message: "connection reset".into(),
                status: None,
            },
        ]),
        Registry::new(gate.clone()),
        gate,
        config(),
    )
    .await;

    assert!(harness.stopped().is_failure());
    let history = harness.history();
    assert_eq!(history.last().unwrap().text(), "I was saying something");
}

// ── the guards ──────────────────────────────────────────────────────────

/// Past the step budget the model gets one final turn with no tools, so the
/// user gets a summary instead of the turn simply vanishing.
#[tokio::test]
async fn the_step_budget_forces_a_prose_wrap_up() {
    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new()
            .calling("echo", json!({"input": "1"}))
            .calling("echo", json!({"input": "2"}))
            .replying("here is where I got to"),
        Registry::new(gate.clone()).with_tool(Arc::new(MockTool::new("echo"))),
        gate,
        AgentConfig {
            max_steps: 2,
            ..config()
        },
    )
    .await;

    assert_eq!(harness.stopped(), StopReason::MaxSteps);
    assert_eq!(harness.prose(), "here is where I got to");

    // The wind-up request offers no tools at all - that is what actually
    // forces prose; the note only explains why.
    let last = harness.provider.last_request().unwrap();
    assert!(
        last.tools.is_empty(),
        "tools were still offered at the limit"
    );
    assert!(last.history.last().unwrap().text().contains("step limit"));
}

/// Four byte-identical calls in a row is a stuck model, not progress. With
/// nobody available to approve it, the safe answer is no.
#[tokio::test]
async fn a_repeating_call_is_challenged_and_stopped() {
    let tool = Arc::new(MockTool::new("echo"));
    let gate = Arc::new(MockGate::new(Policy::Rules(vec![
        Rule::for_any("mock", Action::Allow),
        Rule::for_any("doom_loop", Action::Deny),
    ])));

    let mut provider = MockProvider::new();
    for _ in 0..5 {
        provider = provider.calling("echo", json!({"input": "same"}));
    }
    let provider = provider.replying("I will stop repeating myself");

    let harness = run(
        provider,
        Registry::new(gate.clone()).with_tool(tool.clone()),
        gate,
        config(),
    )
    .await;

    // The first three identical calls run; the fourth is blocked.
    assert_eq!(
        tool.call_count(),
        4,
        "expected three before the guard fired and one after it reset"
    );
    assert!(
        harness
            .tool_outputs()
            .iter()
            .any(|o| o.contains("same") && o.contains("stopped")),
        "the repeating call was never challenged: {:?}",
        harness.tool_outputs()
    );
}

/// A differing argument is a different call, however similar it looks.
#[tokio::test]
async fn varying_calls_are_not_treated_as_a_loop() {
    let tool = Arc::new(MockTool::new("echo"));
    let gate = Arc::new(MockGate::new(Policy::Rules(vec![
        Rule::for_any("mock", Action::Allow),
        Rule::for_any("doom_loop", Action::Deny),
    ])));

    let mut provider = MockProvider::new();
    for i in 0..5 {
        provider = provider.calling("echo", json!({ "input": format!("call {i}") }));
    }
    let provider = provider.replying("done");

    let harness = run(
        provider,
        Registry::new(gate.clone()).with_tool(tool.clone()),
        gate,
        config(),
    )
    .await;

    assert_eq!(harness.stopped(), StopReason::Complete);
    assert_eq!(tool.call_count(), 5, "a varying call was wrongly blocked");
}

// ── the event stream ────────────────────────────────────────────────────

#[tokio::test]
async fn the_stream_always_ends_in_done() {
    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new().replying("ok"),
        Registry::new(gate.clone()),
        gate,
        config(),
    )
    .await;

    assert!(matches!(
        harness.events.first(),
        Some(AgentEvent::Start { .. })
    ));
    assert!(matches!(
        harness.events.last(),
        Some(AgentEvent::Done { .. })
    ));
}

/// A UI pairs starts with finishes by call id. A finish with no matching
/// start leaves an orphan card on screen, so *every* call the model made must
/// be announced - including one the repetition guard is about to block.
#[tokio::test]
async fn every_finished_call_was_announced_first() {
    let gate = Arc::new(MockGate::new(Policy::Rules(vec![
        Rule::for_any("mock", Action::Allow),
        Rule::for_any("doom_loop", Action::Deny),
    ])));

    let mut provider = MockProvider::new();
    for _ in 0..5 {
        provider = provider.calling("echo", json!({"input": "same"}));
    }

    let harness = run(
        provider.replying("done"),
        Registry::new(gate.clone()).with_tool(Arc::new(MockTool::new("echo"))),
        gate,
        config(),
    )
    .await;

    let mut announced = std::collections::HashSet::new();
    for event in &harness.events {
        match event {
            AgentEvent::ToolStarted { call, .. } => {
                announced.insert(call.id.clone());
            }
            AgentEvent::ToolFinished { result } => {
                assert!(
                    announced.contains(&result.call_id),
                    "{} finished without ever being announced",
                    result.tool
                );
            }
            _ => {}
        }
    }
    assert_eq!(announced.len(), 5, "not every call was announced");
}

#[tokio::test]
async fn a_tool_call_is_announced_before_it_runs() {
    let gate = Arc::new(MockGate::allow_all());
    let harness = run(
        MockProvider::new()
            .calling("echo", json!({"input": "x"}))
            .replying("done"),
        Registry::new(gate.clone()).with_tool(Arc::new(MockTool::new("echo"))),
        gate,
        config(),
    )
    .await;

    let started = harness
        .events
        .iter()
        .position(|e| matches!(e, AgentEvent::ToolStarted { .. }));
    let finished = harness
        .events
        .iter()
        .position(|e| matches!(e, AgentEvent::ToolFinished { .. }));
    assert!(started < finished, "the tool finished before it started");
}

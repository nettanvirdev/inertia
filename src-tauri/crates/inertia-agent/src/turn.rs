//! The turn loop.
//!
//! One user message goes in; some number of provider calls and tool
//! round-trips happen; a finished transcript comes out. The loop is written
//! entirely against traits, so the same code runs against Anthropic and
//! against a scripted fake, and neither can tell.
//!
//! Shape of one iteration:
//!
//! ```text
//!   ┌─► build request ─► stream from provider ─► accumulate
//!   │                                              │
//!   │                          ┌───────────────────┴─ no tool calls ─► done
//!   │                          │
//!   │                     tool calls
//!   │                          │
//!   └── append results ◄── run each (registry asks permission)
//! ```
//!
//! Every exit names itself in [`StopReason`], because "the turn just stopped"
//! with no explanation is the single most confusing thing an agent can do.

use std::sync::Arc;

use async_stream::stream;
use futures::stream::BoxStream;
use futures::StreamExt;
use inertia_core::message::{AssistantEntry, Entry, ThinkingBlock, ToolCall, ToolEntry};
use inertia_core::provider::{ChatRequest, Provider, StreamEvent, Usage};
use inertia_core::tool::{PermissionGate, PermissionRequest, ToolContext, ToolRegistry};
use inertia_core::SessionId;

use crate::event::{AgentEvent, StopReason};
use crate::steer::Steer;

/// How many provider calls one turn may make before being wound up.
///
/// High enough that no honest task hits it, low enough that a model stuck in a
/// loop stops costing money within a few minutes.
pub const MAX_STEPS: u32 = 100;

/// How many byte-identical tool calls in a row before the loop stops assuming
/// it is progress.
///
/// Three, because two identical calls is a plausible retry - reading a file
/// before and after writing it, say - and four is never anything but a stuck
/// model.
pub const LOOP_THRESHOLD: usize = 3;

/// The permission key used when challenging a repeating call. Distinct from
/// the tool's own key so a person can answer "stop doing that" without
/// changing what the tool is allowed to do in general.
pub const DOOM_LOOP_KEY: &str = "doom_loop";

/// The framing a steered message arrives in.
///
/// The text itself is the person talking, so it goes in as a user message and
/// not as one of the system notes elsewhere in this file. But it needs the
/// framing, and the failure it prevents is specific: a model handed "check the
/// tests first" after four hundred lines of its own reasoning treats it as a
/// new request queued behind the old one, finishes what it was doing, and only
/// then turns to it - which is the opposite of what someone interrupting means.
pub const STEER_NOTE: &str = "The person you are talking to typed this while you were working, so it \
arrived in the middle of your turn rather than as a new request after it. \
Their words follow, and they are about the job you are doing right now. \
Read them against the plan you were following, change or abandon whatever \
they contradict, and carry on from there. Do not finish what you had \
started and answer them afterwards.";

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub max_steps: u32,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// How many tokens the model may spend thinking before it answers, for the
    /// providers that take a budget. Zero and `None` both mean "do not think".
    ///
    /// Carried here rather than read from the agent record inside the loop,
    /// because the loop has no idea what an agent record is - but it has to
    /// reach the request, and until it did, an agent configured for twenty-four
    /// thousand tokens of reasoning produced none and showed none.
    pub thinking_budget: Option<u32>,
    /// `"low" | "medium" | "high"`, for the providers that expose a dial rather
    /// than a budget.
    pub reasoning_effort: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_steps: MAX_STEPS,
            model: String::new(),
            temperature: None,
            max_tokens: None,
            thinking_budget: None,
            reasoning_effort: None,
        }
    }
}

/// What one turn needs to run.
#[derive(Debug, Clone)]
pub struct Turn {
    pub session: SessionId,
    /// The assembled system prompt.
    pub system: String,
    /// The conversation so far, including the message being answered.
    pub history: Vec<Entry>,
    /// Where tools resolve relative paths.
    pub root: std::path::PathBuf,
}

pub struct Agent {
    provider: Arc<dyn Provider>,
    tools: Arc<dyn ToolRegistry>,
    permissions: Arc<dyn PermissionGate>,
    config: AgentConfig,
    /// Whoever wants to be told at the named moments of the turn - lifecycle
    /// hooks, in the app. Optional because the loop is complete without one,
    /// and because `inertia-devkit` runs it with nothing attached.
    lifecycle: Option<Arc<dyn inertia_core::Lifecycle>>,
    /// What the person says while the turn is running. Always present rather
    /// than optional: an empty queue costs nothing to look at once a step, and
    /// a caller that forgot to attach one would silently have an agent nobody
    /// can interrupt.
    steer: Steer,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("provider", &self.provider.id())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Agent {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Arc<dyn ToolRegistry>,
        permissions: Arc<dyn PermissionGate>,
        config: AgentConfig,
    ) -> Self {
        Self {
            provider,
            tools,
            permissions,
            config,
            lifecycle: None,
            steer: Steer::new(),
        }
    }

    /// Attach a listener for the named moments of a turn.
    pub fn with_lifecycle(mut self, lifecycle: Arc<dyn inertia_core::Lifecycle>) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// Steer this turn through a queue the caller already holds.
    ///
    /// The app registers a turn - its cancellation handle, its approval gate -
    /// before it has built the agent, and the queue has to be registered with
    /// them or a message sent in the first second of a turn has nowhere to go.
    /// So the caller may mint the queue first and hand it over here.
    pub fn with_steering(mut self, steer: Steer) -> Self {
        self.steer = steer;
        self
    }

    /// The handle the window steers this turn through.
    ///
    /// Taken before `run`, and kept beside the turn's cancellation handle:
    /// steering is addressed by turn id the same way cancelling is, because
    /// that is the only handle a window has on work it does not own.
    pub fn steering(&self) -> Steer {
        self.steer.clone()
    }

    /// Runs a turn, reporting as it goes.
    ///
    /// The stream always ends in [`AgentEvent::Done`]. Dropping it cancels the
    /// turn: the in-flight provider request and any running tool are dropped
    /// with it, which is what cancellation means here.
    pub fn run(&self, turn: Turn) -> BoxStream<'static, AgentEvent> {
        let provider = self.provider.clone();
        let tools = self.tools.clone();
        let permissions = self.permissions.clone();
        let config = self.config.clone();
        let lifecycle = self.lifecycle.clone();
        let steer = self.steer.clone();

        Box::pin(stream! {
            // Held for the life of the stream. A cancelled turn is a dropped
            // stream with no code running after it, and without this it would
            // go on accepting messages nothing will ever read.
            let _closing = steer.closing();
            let mut history = turn.history;
            let mut step: u32 = 0;
            let mut usage: Option<Usage> = None;
            // Fingerprints of recent tool calls, for the repetition guard.
            let mut recent: Vec<String> = Vec::new();

            yield AgentEvent::Start { model: config.model.clone() };

            let stopped = 'turn: loop {
                step += 1;
                // One final iteration past the budget, with no tools offered,
                // so the model wraps up in prose instead of the turn simply
                // vanishing mid-task.
                let winding_up = step > config.max_steps;

                yield AgentEvent::Step { step };

                // Anything the person said while the last step was running
                // goes in here.
                //
                // This is the one safe place for it. Every tool call the model
                // asked for has been answered by now and the next request has
                // not been built yet, so a user message lands between two
                // complete rounds rather than inside one - which is the
                // difference between a valid transcript and one a strict
                // provider refuses.
                for said in steer.take() {
                    history.push(steer_entry(&said));
                    yield AgentEvent::Steered { text: said };
                }

                let mut request = ChatRequest {
                    model: config.model.clone(),
                    system: turn.system.clone(),
                    history: history.clone(),
                    temperature: config.temperature,
                    max_tokens: config.max_tokens,
                    thinking_budget: config.thinking_budget,
                    reasoning_effort: config.reasoning_effort.clone(),
                    ..Default::default()
                };

                if winding_up {
                    // Offering no tools is what actually forces prose; the
                    // note explains why, so the reply does not read as the
                    // model abandoning the task for no reason.
                    request.tools = Vec::new();
                    request.history.push(Entry::user(
                        "You have reached the step limit for this turn. \
                         Do not call any more tools. Summarise what you did, \
                         what you found, and what remains.",
                    ));
                } else {
                    request.tools = tools.specs().await;
                }

                // ── stream one provider call ────────────────────────────
                let mut text = String::new();
                let mut thinking: Vec<ThinkingBlock> = Vec::new();
                let mut calls: Vec<ToolCall> = Vec::new();
                let mut failure: Option<String> = None;

                let mut events = provider.stream_chat(request);
                while let Some(event) = events.next().await {
                    match event {
                        StreamEvent::Start { .. } => {}
                        StreamEvent::Delta { text: chunk } => {
                            text.push_str(&chunk);
                            yield AgentEvent::Delta { text: chunk };
                        }
                        StreamEvent::Reasoning { text: chunk } => {
                            yield AgentEvent::Reasoning { text: chunk };
                        }
                        StreamEvent::Thinking { blocks } => {
                            // Kept so it can be replayed verbatim next step;
                            // the provider refuses to continue its own
                            // reasoning across a tool call otherwise.
                            thinking = blocks;
                        }
                        StreamEvent::Tool { calls: asked } => {
                            calls = asked;
                        }
                        StreamEvent::Notice { message } => {
                            yield AgentEvent::Notice { message };
                        }
                        StreamEvent::Retry { message, attempt, of, .. } => {
                            // Surfaced so a long stall has a visible reason
                            // rather than looking like a hang.
                            yield AgentEvent::Notice {
                                message: format!("Retrying ({attempt}/{of}): {message}"),
                            };
                        }
                        StreamEvent::Error { message, .. } => {
                            failure = Some(message);
                        }
                        StreamEvent::Done { usage: reported, .. } => {
                            if let Some(u) = reported {
                                usage = Some(u);
                            }
                        }
                    }
                }
                drop(events);

                if let Some(message) = failure {
                    // Whatever prose arrived before the failure is kept: the
                    // user already watched it appear, and dropping it would
                    // make the transcript disagree with what they saw.
                    if !text.is_empty() {
                        history.push(Entry::Assistant(AssistantEntry {
                            content: Some(text),
                            ..Default::default()
                        }));
                    }
                    break 'turn StopReason::Error { message };
                }

                history.push(Entry::Assistant(AssistantEntry {
                    content: (!text.is_empty()).then_some(text),
                    tool_calls: calls.clone(),
                    thinking,
                    // Who wrote it is the window's business: the loop runs one
                    // seat at a time and does not know which, and the window
                    // already draws this reply against the agent it started.
                    ..Default::default()
                }));

                // ── nothing further asked for: the turn is done ─────────
                if calls.is_empty() {
                    // A message that landed while the model was writing this
                    // answer keeps the turn alive rather than being lost
                    // between the last token and the return. Someone who typed
                    // "wait, not that one" as the reply began said it to this
                    // turn, and the alternative is accepting their words and
                    // then finishing without ever showing them to the model.
                    if steer.pending() {
                        continue;
                    }

                    // Unless something objects. "Do not stop until the tests
                    // pass" is made of exactly this: the reason goes back as a
                    // user turn, because the model has to read it to act on it.
                    //
                    // Not while winding up, though. The step budget is already
                    // spent, and a hook that sent the model round again there
                    // would be an unbounded turn with the budget switched off.
                    if !winding_up {
                        if let Some(lifecycle) = &lifecycle {
                            let reaction = lifecycle.stopping().await;
                            if let Some(message) = &reaction.message {
                                yield AgentEvent::Notice { message: message.clone() };
                            }
                            if reaction.block {
                                let reason = reaction.reason.unwrap_or_else(||
                                    "A hook asked you to keep going.".into());
                                yield AgentEvent::Notice { message: reason.clone() };
                                history.push(user_entry(&reason));
                                continue;
                            }
                            for context in reaction.context {
                                history.push(user_entry(&context));
                            }
                        }
                    }

                    break 'turn if winding_up {
                        StopReason::MaxSteps
                    } else {
                        StopReason::Complete
                    };
                }

                // ── run what was asked for ──────────────────────────────
                // A step where everything was *refused* ends the turn: the
                // answers will not change, and going round again only asks the
                // user the same questions. A step where everything merely
                // *failed* does not - a bad path or a non-zero exit is
                // something the model can act on, and usually does.
                let mut any_allowed = false;
                let mut any_denied = false;

                for mut call in calls {
                    let ctx = ToolContext {
                        root: turn.root.clone(),
                        session: turn.session.clone(),
                        call_id: call.id.clone(),
                        permissions: permissions.clone(),
                    };

                    // Announced before the guard runs, not after. Every call
                    // the model made gets a start event, including one that is
                    // about to be blocked - a UI pairing starts with finishes
                    // by call id would otherwise be handed an orphan.
                    yield AgentEvent::ToolStarted { call: call.clone(), title: None };

                    // Before the guard and before the tool: a refusal here is
                    // the user's own rule - "never touch the migrations
                    // folder" - and it has to reach the model as a refused
                    // call rather than as a tool that quietly did nothing.
                    if let Some(lifecycle) = &lifecycle {
                        let reaction = lifecycle.pre_tool(&call).await;
                        if let Some(message) = &reaction.message {
                            yield AgentEvent::Notice { message: message.clone() };
                        }
                        if reaction.block {
                            let result = inertia_core::tool::ToolResult::failed(
                                call.id.clone(),
                                &call.name,
                                reaction.reason.unwrap_or_else(|| "Blocked by a hook.".into()),
                            );
                            any_denied = true;
                            history.push(tool_entry(&result));
                            yield AgentEvent::ToolFinished { result };
                            continue;
                        }
                        if let Some(updated) = reaction.updated_input {
                            call.arguments = updated.to_string();
                        }
                        for context in reaction.context {
                            history.push(user_entry(&context));
                        }
                    }

                    // Repetition guard, before anything runs. A model looping
                    // on an identical call is not making progress, and under
                    // an unattended approval policy the safe answer to "shall
                    // I do this a fourth time" is no.
                    let fingerprint = format!("{}\u{0}{}", call.name, call.arguments);
                    let repeating = recent.len() >= LOOP_THRESHOLD
                        && recent.iter().rev().take(LOOP_THRESHOLD).all(|f| *f == fingerprint);

                    if repeating {
                        let request = PermissionRequest::new(DOOM_LOOP_KEY, call.name.clone());
                        let allowed = permissions
                            .ask(&request)
                            .await
                            .map(|d| d.is_allowed())
                            .unwrap_or(false);

                        if !allowed {
                            let result = inertia_core::tool::ToolResult::failed(
                                call.id.clone(),
                                &call.name,
                                format!(
                                    "This is the same `{}` call with the same arguments \
                                     {} times running. It was stopped. Try a different \
                                     approach, or explain what is blocking you.",
                                    call.name,
                                    LOOP_THRESHOLD + 1
                                ),
                            );
                            history.push(tool_entry(&result));
                            yield AgentEvent::ToolFinished { result };
                            recent.clear();
                            continue;
                        }
                        recent.clear();
                    }
                    recent.push(fingerprint);

                    match tools.run(&call, &ctx).await {
                        Ok(result) => {
                            if result.was_denied() {
                                any_denied = true;
                            } else {
                                any_allowed = true;
                            }
                            history.push(tool_entry(&result));
                            // Images a tool produced are shown to the model as
                            // a following user message - the one shape every
                            // vision-capable provider accepts inside a
                            // transcript.
                            if !result.images.is_empty() {
                                history.push(images_entry(&result.images));
                            }

                            // After the result is in the transcript, so a hook
                            // that objects to it - "that edit left a syntax
                            // error" - has its objection read *after* the thing
                            // it is objecting to.
                            if let Some(lifecycle) = &lifecycle {
                                let reaction = lifecycle.post_tool(&result).await;
                                if let Some(message) = &reaction.message {
                                    yield AgentEvent::Notice { message: message.clone() };
                                }
                                if reaction.block {
                                    history.push(user_entry(&reaction.reason.clone().unwrap_or_else(
                                        || "A hook rejected that result.".into(),
                                    )));
                                }
                                for context in reaction.context {
                                    history.push(user_entry(&context));
                                }
                            }

                            yield AgentEvent::ToolFinished { result };
                        }
                        Err(e) if e.is_cancellation() => break 'turn StopReason::Cancelled,
                        Err(e) => break 'turn StopReason::Error { message: e.to_string() },
                    }
                }

                if any_denied && !any_allowed {
                    break 'turn StopReason::Refused;
                }
            };

            yield AgentEvent::Done { stopped, history, usage };
        })
    }
}

/// Every tool result becomes a transcript entry, successful or not.
///
/// An unanswered tool call makes the *next* request invalid on strict
/// providers, so a result is appended even for a refusal - the model needs to
/// read that it was refused, and the transcript needs to stay well-formed.
fn tool_entry(result: &inertia_core::tool::ToolResult) -> Entry {
    Entry::Tool(ToolEntry {
        tool_call_id: result.call_id.clone(),
        content: result.output.clone(),
        ok: Some(result.ok),
        pruned: false,
    })
}

fn images_entry(images: &[String]) -> Entry {
    use inertia_core::message::{ImageUrl, Part, UserEntry};
    Entry::User(UserEntry {
        parts: Some(
            images
                .iter()
                .map(|url| Part::ImageUrl {
                    image_url: ImageUrl::Bare(url.clone()),
                })
                .collect(),
        ),
        ..Default::default()
    })
}

/// A note put in front of the model as a user turn.
///
/// A user turn rather than a system one because it is the only role every
/// provider accepts in the middle of a transcript, and because what it carries -
/// a hook's refusal, a hook's extra context - genuinely is the user speaking:
/// they wrote the program that said it.
fn user_entry(text: &str) -> Entry {
    use inertia_core::message::UserEntry;
    Entry::User(UserEntry {
        content: Some(text.to_string()),
        ..Default::default()
    })
}

/// A steered message, framed so the model reads it as an interruption.
///
/// A user turn for the same reason [`user_entry`] is one, and with the note in
/// front of the text rather than around it: the person's own words have to be
/// the last thing the model reads, or the framing is what it answers.
fn steer_entry(text: &str) -> Entry {
    user_entry(&format!("{STEER_NOTE}\n\n{text}"))
}

#[cfg(test)]
mod steering_tests {
    //! Steering, driven through the real loop.
    //!
    //! The stream is pulled by hand rather than collected, because that is the
    //! only way to be in the middle of a turn: a `stream!` runs no further than
    //! the last event taken from it, so sending between two `next()` calls is
    //! exactly "the person typed this while the tool was running".

    use super::*;
    use futures::StreamExt;
    use inertia_core::message::Entry;
    use inertia_core::tool::ToolRegistry;
    use inertia_core::SessionId;
    use inertia_mock::{MockGate, MockProvider, MockTool};
    use serde_json::json;
    use std::sync::Arc;

    fn config() -> AgentConfig {
        AgentConfig {
            model: "mock-model".into(),
            ..Default::default()
        }
    }

    fn turn() -> Turn {
        Turn {
            session: SessionId::new(),
            system: "you are a test".into(),
            history: vec![Entry::user("do the thing")],
            root: std::path::PathBuf::from("."),
        }
    }

    /// The text of every user entry in a request's history.
    fn user_texts(request: &ChatRequest) -> Vec<String> {
        request
            .history
            .iter()
            .filter_map(|entry| match entry {
                Entry::User(_) => Some(entry.text().to_string()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn a_message_sent_mid_turn_reaches_the_next_request_and_is_announced() {
        let gate = Arc::new(MockGate::allow_all());
        let tool = Arc::new(MockTool::new("echo"));
        let provider = Arc::new(
            MockProvider::new()
                .calling("echo", json!({ "input": "hello" }))
                .replying("changed course"),
        );
        let registry = Arc::new(inertia_tools::Registry::new(gate.clone()).with_tool(tool))
            as Arc<dyn ToolRegistry>;

        let agent = Agent::new(provider.clone(), registry, gate, config());
        let steer = agent.steering();

        let mut events = agent.run(turn());
        let mut seen: Vec<AgentEvent> = Vec::new();
        let mut sent = false;

        while let Some(event) = events.next().await {
            let finished = matches!(event, AgentEvent::ToolFinished { .. });
            seen.push(event);
            // Mid-turn: the first step's tool has answered and the second
            // step's request has not been built yet.
            if finished && !sent {
                assert!(steer.send("stop and read the tests first"));
                sent = true;
            }
        }

        assert!(sent, "the turn never ran a tool, so nothing was steered");

        // Announced, so the transcript can draw it where it was said.
        let announced: Vec<&str> = seen
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Steered { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(announced, vec!["stop and read the tests first"]);

        // And in front of the model, in the second call, framed as an
        // interruption rather than as a fresh request.
        let requests = provider.requests();
        assert_eq!(requests.len(), 2, "expected two provider calls");
        assert_eq!(user_texts(&requests[0]), vec!["do the thing".to_string()]);

        let second = user_texts(&requests[1]);
        assert_eq!(second.len(), 2, "the steered message is a user entry");
        assert!(second[1].starts_with(STEER_NOTE));
        assert!(second[1].ends_with("stop and read the tests first"));

        // The transcript handed back carries it too, so a reload shows it.
        let history = seen
            .iter()
            .find_map(|event| match event {
                AgentEvent::Done { history, .. } => Some(history.clone()),
                _ => None,
            })
            .unwrap_or_default();
        assert!(history
            .iter()
            .any(|entry| entry.text().contains("stop and read the tests first")));
    }

    #[tokio::test]
    async fn a_message_sent_while_the_answer_was_being_written_keeps_the_turn_alive() {
        let gate = Arc::new(MockGate::allow_all());
        let provider = Arc::new(
            MockProvider::new()
                .replying("all done")
                .replying("actually, here is the other thing"),
        );
        let registry =
            Arc::new(inertia_tools::Registry::new(gate.clone())) as Arc<dyn ToolRegistry>;

        let agent = Agent::new(provider.clone(), registry, gate, config());
        let steer = agent.steering();

        let mut events = agent.run(turn());
        let mut steps = 0;
        let mut sent = false;
        while let Some(event) = events.next().await {
            match event {
                AgentEvent::Step { .. } => steps += 1,
                // Said as the first reply was being written: the turn must not
                // end on it.
                AgentEvent::Delta { .. } if !sent => {
                    assert!(steer.send("wait, not that one"));
                    sent = true;
                }
                _ => {}
            }
        }

        assert_eq!(steps, 2, "the turn ended without answering the message");
        let requests = provider.requests();
        assert!(user_texts(&requests[1])
            .iter()
            .any(|text| text.ends_with("wait, not that one")));
    }

    #[tokio::test]
    async fn a_finished_turn_refuses_a_message_rather_than_swallowing_it() {
        let gate = Arc::new(MockGate::allow_all());
        let provider = Arc::new(MockProvider::new().replying("done"));
        let registry =
            Arc::new(inertia_tools::Registry::new(gate.clone())) as Arc<dyn ToolRegistry>;

        let agent = Agent::new(provider, registry, gate, config());
        let steer = agent.steering();
        let _ = agent.run(turn()).collect::<Vec<_>>().await;

        // False is the answer the window needs: it sends the message as a new
        // turn instead of dropping it.
        assert!(!steer.send("too late"));
    }

    #[tokio::test]
    async fn a_queue_handed_in_before_the_turn_is_the_one_the_loop_reads() {
        let gate = Arc::new(MockGate::allow_all());
        let provider = Arc::new(MockProvider::new().replying("done").replying("and again"));
        let registry =
            Arc::new(inertia_tools::Registry::new(gate.clone())) as Arc<dyn ToolRegistry>;

        // Minted first, the way the app registers a turn before it builds one.
        let steer = Steer::new();
        let agent = Agent::new(provider.clone(), registry, gate, config())
            .with_steering(steer.clone());
        assert!(steer.send("said before the turn started"));

        let _ = agent.run(turn()).collect::<Vec<_>>().await;

        let requests = provider.requests();
        assert!(user_texts(&requests[0])
            .iter()
            .any(|text| text.ends_with("said before the turn started")));
        assert!(steer.is_closed(), "the turn must close the queue it was given");
    }

    #[tokio::test]
    async fn a_cancelled_turn_closes_the_queue_too() {
        let gate = Arc::new(MockGate::allow_all());
        let provider = Arc::new(MockProvider::new().replying("working on it"));
        let registry =
            Arc::new(inertia_tools::Registry::new(gate.clone())) as Arc<dyn ToolRegistry>;

        let agent = Agent::new(provider, registry, gate, config());
        let steer = agent.steering();

        let mut events = agent.run(turn());
        events.next().await;
        // Dropping the stream is what cancellation is here.
        drop(events);

        assert!(steer.is_closed());
        assert!(!steer.send("anyone there"));
    }
}

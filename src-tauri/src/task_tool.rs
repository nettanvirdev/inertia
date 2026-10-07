//! `task` - delegating to a subagent.
//!
//! The reason this exists is context, not parallelism. A question like "which of
//! these forty files reference the old auth flow" is answered by reading forty
//! files, and forty files in the main conversation means the actual work starts
//! with a context window that is already mostly search results. A subagent reads
//! them in a conversation of its own and returns two sentences.
//!
//! So the shape is deliberate: the subagent gets a prompt and returns text. It
//! does not stream into the parent's transcript, it cannot ask the parent
//! questions, and its tool calls are its own. What comes back is a report.
//!
//! A subagent cannot spawn a subagent. Enforced by not offering the tool at
//! depth rather than by refusing the call, because nested delegation multiplies
//! cost and latency in a way nobody has ever actually wanted.
//!
//! # Why the machinery arrives through a trait
//!
//! Running a subagent needs a provider, a registry, a permission gate and an
//! assembled system prompt - all of which live in the app crate and two of which
//! need a window. Behind [`Delegate`] they can be supplied by mocks instead,
//! which is what lets the tests below drive this exact `execute` - resolution,
//! model choice, the loop, the formatting, the empty-answer case - rather than a
//! parallel implementation written to be testable.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use inertia_agent::{Agent as Loop, AgentConfig, AgentEvent, StopReason, Turn as AgentTurn};
use inertia_core::id::SessionId;
use inertia_core::message::Entry;
use inertia_core::tool::{
    PermissionGate, PermissionRequest, Tool, ToolContext, ToolOutcome, ToolRegistry, ToolSource,
};
use inertia_core::{Error, Provider, Result};
use inertia_store::Layout;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::agents::Agent;

/// What the app supplies so a subagent can actually run.
#[async_trait]
pub trait Delegate: Send + Sync + std::fmt::Debug {
    /// The workspace, for resolving who was named.
    fn layout(&self) -> &Layout;

    /// A provider and the model id to ask it for.
    fn provider(&self, reference: &str)
        -> std::result::Result<(Arc<dyn Provider>, String), String>;

    /// The subagent's own tools, and the gate they are checked against.
    ///
    /// Both together because the loop needs both and they must be the same
    /// gate: a registry checking one set of rules while the loop consults
    /// another is a permission system with a hole in the middle of it.
    fn registry(
        &self,
        session: &str,
        agent: &Agent,
        withheld: &[&str],
    ) -> (Arc<dyn ToolRegistry>, Arc<dyn PermissionGate>);

    /// The system prompt for a subagent turn.
    /// The subagent's system prompt. `tools` is what its registry answered
    /// with, so the prompt cannot promise a capability the helper was not given.
    fn system(&self, agent: &Agent, model: &str, tools: &[String]) -> String;

    /// Progress, for the card in the parent's transcript. A subagent can run for
    /// a minute, and a card that says nothing for a minute reads as a hang.
    fn progress(&self, call_id: &str, metadata: Value);
}

/// A subagent cannot delegate again, and has no room to invite anyone into.
const WITHHELD_AT_DEPTH: &[&str] = &["task", "invite", "handover", "part", "present_plan"];

/// Finished subagent sessions, so a follow-up can continue one instead of
/// paying for a fresh read of everything it already read.
#[derive(Debug, Default)]
pub struct Sessions {
    runs: Mutex<std::collections::HashMap<String, Vec<Entry>>>,
    counter: Mutex<u64>,
}

impl Sessions {
    fn next_id(&self) -> String {
        let mut counter = self.counter.lock();
        *counter += 1;
        format!("task-{counter}")
    }

    fn history(&self, id: &str) -> Option<Vec<Entry>> {
        self.runs.lock().get(id).cloned()
    }

    fn remember(&self, id: &str, history: Vec<Entry>) {
        self.runs.lock().insert(id.to_string(), history);
    }

    /// Drops every subagent run belonging to a conversation.
    pub fn forget(&self, session: &str) {
        let prefix = format!("{session}/");
        self.runs
            .lock()
            .retain(|id, _| !id.starts_with(&prefix) && id != session);
    }
}

/// Which model a subagent speaks with.
///
/// Its own if it pinned one, otherwise the parent's. A cheap agent asked to do
/// careful work quietly does it badly, and an agent with no model of its own is
/// not asking for the cheapest one - it never chose.
pub fn model_for(agent: &Agent, parent: &str) -> String {
    agent
        .text("model")
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| parent.to_string())
}

#[derive(Debug)]
pub struct TaskTool {
    delegate: Arc<dyn Delegate>,
    sessions: Arc<Sessions>,
    /// The conversation this turn belongs to. A subagent's session hangs off it,
    /// so deleting the conversation forgets the subagents with it.
    parent: String,
    /// The model this turn is using, as the subagent's fallback.
    parent_model: String,
}

impl TaskTool {
    pub fn new(
        delegate: Arc<dyn Delegate>,
        sessions: Arc<Sessions>,
        parent: String,
        parent_model: String,
    ) -> Self {
        Self {
            delegate,
            sessions,
            parent,
            parent_model,
        }
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn id(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Hand a self-contained piece of work to another agent and get its answer back.\n\
         \n\
         Use it when the work would fill your own conversation with material you do not \
         need to keep: searching a large codebase, reading many files to answer one \
         question, checking a claim across several places. The subagent does the reading \
         and gives you the conclusion.\n\
         \n\
         Do not use it when you already know which file you want - read it. Do not use it \
         for a single search - use grep. Do not use it to do work you could do in two tool \
         calls; starting a subagent costs a whole conversation.\n\
         \n\
         The subagent cannot see this conversation and cannot ask you anything, so the \
         prompt has to stand alone. Say what you want, what it needs to know, and what \
         shape the answer should take. Vague prompts come back with vague answers.\n\
         \n\
         Pass a previous task_id to continue that subagent's session rather than starting \
         a fresh one, which is much cheaper when you are following up on its answer."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "A short description of the task, three to five words." },
                "prompt": { "type": "string", "description": "The full instructions for the subagent. It sees nothing else." },
                "subagent_type": { "type": "string", "description": "Which agent to use, by name." },
                "task_id": { "type": "string", "description": "Continue a previous subagent's session instead of starting a new one." }
            },
            "required": ["description", "prompt", "subagent_type"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        PermissionRequest::new(
            "task",
            args.get("subagent_type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let who = args.get("subagent_type").and_then(Value::as_str)?;
        let what = args
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        Some(format!("{who}: {what}"))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };

        let description = text("description").unwrap_or_else(|| "task".into());
        let prompt = text("prompt").ok_or_else(|| {
            Error::Other("`prompt` is required: the subagent sees nothing else.".into())
        })?;
        let wanted = text("subagent_type")
            .ok_or_else(|| Error::Other("`subagent_type` is required.".into()))?;

        let agent =
            crate::agents::resolve(self.delegate.layout(), &wanted).map_err(Error::Other)?;
        if agent.is_paused() {
            return Err(Error::Other(format!(
                "{} is paused, so it will not take work. Ask the person to turn it back \
                 on, use a different agent, or do it yourself.",
                agent.name
            )));
        }

        // A follow-up continues that run; anything else starts a fresh one. The
        // id is namespaced by the conversation so deleting the conversation
        // forgets its subagents with it.
        let previous =
            text("task_id").and_then(|id| self.sessions.history(&id).map(|history| (id, history)));
        let (id, mut history) = match previous {
            Some((id, history)) => (id, history),
            None => (
                format!("{}/{}", self.parent, self.sessions.next_id()),
                Vec::new(),
            ),
        };
        history.push(Entry::user(prompt));

        let reference = model_for(&agent, &self.parent_model);
        let (provider, model_id) = self.delegate.provider(&reference).map_err(Error::Other)?;

        let call_id = ctx.call_id.as_str().to_string();
        self.delegate.progress(
            &call_id,
            json!({ "taskId": id, "agent": agent.name, "state": "running" }),
        );

        let (registry, gate) = self.delegate.registry(&id, &agent, WITHHELD_AT_DEPTH);
        // The helper's own tool list, for its own prompt.
        let held: Vec<String> = registry
            .specs()
            .await
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        let subagent = Loop::new(
            provider,
            registry,
            gate,
            AgentConfig {
                model: model_id,
                // The helper's own dials, not the parent's: an agent given a
                // thinking budget was given it for the work it does.
                thinking_budget: crate::agents::thinking_of(&agent.record).0,
                reasoning_effort: crate::agents::thinking_of(&agent.record).1,
                ..Default::default()
            },
        );

        let mut events = subagent.run(AgentTurn {
            session: SessionId::from_existing(id.clone()),
            system: self.delegate.system(&agent, &reference, &held),
            history: history.clone(),
            root: ctx.root.clone(),
        });

        let mut said = String::new();
        let mut steps: Vec<String> = Vec::new();
        let mut stopped = StopReason::Complete;
        let mut transcript = Vec::new();

        while let Some(event) = events.next().await {
            match event {
                AgentEvent::Delta { text } => said.push_str(&text),
                AgentEvent::ToolStarted { call, title } => {
                    steps.push(title.unwrap_or(call.name));
                    self.delegate.progress(
                        &call_id,
                        json!({
                            "taskId": id,
                            "agent": agent.name,
                            "state": "running",
                            // The last few only: a subagent that read forty
                            // files should not put forty lines in the parent's
                            // transcript, which is the thing it exists to stop.
                            "steps": steps.iter().rev().take(8).rev().collect::<Vec<_>>(),
                        }),
                    );
                }
                AgentEvent::Done {
                    stopped: reason,
                    history: full,
                    ..
                } => {
                    stopped = reason;
                    transcript = full;
                    break;
                }
                _ => {}
            }
        }

        if !transcript.is_empty() {
            self.sessions.remember(&id, transcript);
        }

        if said.trim().is_empty() {
            return Err(Error::Other(format!(
                "The {} subagent finished without producing an answer ({}). Try again with \
                 a more specific prompt, or do the work yourself.",
                agent.name,
                stop_word(&stopped)
            )));
        }

        Ok(ToolOutcome {
            title: Some(format!("{}: {description}", agent.name)),
            // Fenced with the agent and the id, so a follow-up turn can name the
            // run it is following up on and the model can tell a report from its
            // own words.
            output: format!(
                "<task id=\"{id}\" agent=\"{}\" state=\"{}\">\n{}\n</task>",
                agent.name,
                stop_word(&stopped),
                said.trim()
            ),
            metadata: Some(json!({
                "taskId": id,
                "agent": agent.name,
                "steps": steps,
                "state": stop_word(&stopped),
            })),
            images: Vec::new(),
        })
    }
}

/// The real machinery, for a turn running in the app.
///
/// Everything here needs either the window or the open workspace, which is
/// exactly why it is on this side of the trait and the tool is on the other.
pub struct AppDelegate {
    pub app: tauri::AppHandle,
    pub workspace: Arc<crate::state::Workspace>,
    pub project: Option<std::path::PathBuf>,
    /// Where the subagent works. The parent's folder, not the agent record's:
    /// a helper asked about this project should be looking at this project.
    pub cwd: std::path::PathBuf,
    /// The parent's turn and thread, so a progress update lands on the right
    /// card in the right transcript.
    pub turn_id: String,
    pub thread_id: String,
    /// The approval regime the subagent inherits. A helper spawned by a turn
    /// nobody is watching is equally unwatched, and one under "ask" should stop
    /// and ask rather than quietly deciding it has permission.
    pub approval: Option<String>,
}

/// Hand-written because `Workspace` holds live provider connections and has no
/// `Debug` of its own - and a turn id is the only field worth printing anyway.
impl std::fmt::Debug for AppDelegate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppDelegate")
            .field("turn_id", &self.turn_id)
            .field("cwd", &self.cwd)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Delegate for AppDelegate {
    fn layout(&self) -> &Layout {
        &self.workspace.layout
    }

    fn provider(
        &self,
        reference: &str,
    ) -> std::result::Result<(Arc<dyn Provider>, String), String> {
        crate::state::provider_for(&self.workspace.settings, reference)
    }

    fn registry(
        &self,
        session: &str,
        agent: &Agent,
        withheld: &[&str],
    ) -> (Arc<dyn ToolRegistry>, Arc<dyn PermissionGate>) {
        // The subagent's own id on the gate, not the parent's: per-agent
        // permission rules belong to whoever is actually running the tool, and
        // a helper inheriting its caller's rules is a way around them.
        let gate = crate::state::gate_for(
            &self.app,
            &self.workspace,
            session.to_string(),
            Some(agent.id.clone()),
        );
        let registry = crate::state::registry_with(
            &self.workspace,
            gate.clone(),
            self.project.clone(),
            Vec::new(),
            withheld,
        );
        (registry, gate)
    }

    fn system(&self, agent: &Agent, model: &str, tools: &[String]) -> String {
        // No memories: a subagent is working to a brief it was handed, not
        // continuing a relationship, and the parent's recalled memories are
        // about a conversation it cannot see.
        crate::prompt::build(
            &self.workspace,
            &self.cwd,
            "",
            &crate::prompt::Turn {
                agent_id: Some(agent.id.clone()),
                // A subagent is working to a brief, not to a conversation mode,
                // so `build` omits the mode paragraph entirely and adds the
                // subagent framing instead.
                mode: None,
                approval: self.approval.clone(),
                model: model.to_string(),
                is_subagent: true,
                worktree: None,
                // A subagent starts where the parent is; it has no history of
                // its own to have moved from.
                previous_cwd: None,
                tools: tools.to_vec(),
                room: None,
            },
        )
    }

    fn progress(&self, call_id: &str, metadata: Value) {
        use tauri::Emitter;
        let _ = self.app.emit(
            crate::turn::AGENT_EVENT,
            json!({
                "type": "tool-update",
                "id": self.turn_id,
                "threadId": self.thread_id,
                "callId": call_id,
                "metadata": metadata,
            }),
        );
    }
}

/// The same delegate, for the team tools.
///
/// Only one method differs: a spawned run is handed tools of its own - its
/// nested team set, when its policy allows one - and those have to reach the
/// registry that run is given rather than the parent's.
impl crate::crew::CrewDelegate for AppDelegate {
    fn registry_with_extras(
        &self,
        session: &str,
        agent: &Agent,
        withheld: &[&str],
        extra: Vec<Arc<dyn inertia_core::tool::Tool>>,
    ) -> (Arc<dyn ToolRegistry>, Arc<dyn PermissionGate>) {
        let gate = crate::state::gate_for(
            &self.app,
            &self.workspace,
            session.to_string(),
            Some(agent.id.clone()),
        );
        let registry = crate::state::registry_with(
            &self.workspace,
            gate.clone(),
            self.project.clone(),
            extra,
            withheld,
        );
        (registry, gate)
    }
}

fn stop_word(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::Complete => "complete",
        StopReason::MaxSteps => "maxSteps",
        StopReason::Error { .. } => "error",
        StopReason::Cancelled => "cancelled",
        StopReason::Refused => "refused",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::ToolCallId;
    use inertia_mock::{MockGate, MockProvider};
    use inertia_tools::Registry;

    /// A `Delegate` made of mocks, so the tests below drive the shipping
    /// `execute` rather than a rehearsal of it.
    #[derive(Debug)]
    struct Fake {
        layout: Layout,
        provider: Arc<MockProvider>,
        /// Every `(session, withheld)` a registry was built for.
        built: Mutex<Vec<(String, Vec<String>)>>,
        /// Every progress update pushed at the parent's card.
        progress: Mutex<Vec<Value>>,
        /// The system prompt handed to the last subagent.
        system: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Delegate for Fake {
        fn layout(&self) -> &Layout {
            &self.layout
        }

        fn provider(
            &self,
            reference: &str,
        ) -> std::result::Result<(Arc<dyn Provider>, String), String> {
            Ok((self.provider.clone(), format!("resolved:{reference}")))
        }

        fn registry(
            &self,
            session: &str,
            _agent: &Agent,
            withheld: &[&str],
        ) -> (Arc<dyn ToolRegistry>, Arc<dyn PermissionGate>) {
            self.built.lock().push((
                session.to_string(),
                withheld.iter().map(|w| (*w).to_string()).collect(),
            ));
            let gate: Arc<dyn PermissionGate> = Arc::new(MockGate::allow_all());
            (Arc::new(Registry::new(gate.clone())), gate)
        }

        fn system(&self, agent: &Agent, model: &str, _tools: &[String]) -> String {
            let prompt = format!("You are {}, on {model}.", agent.name);
            self.system.lock().push(prompt.clone());
            prompt
        }

        fn progress(&self, _call_id: &str, metadata: Value) {
            self.progress.lock().push(metadata);
        }
    }

    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        for record in [
            json!({ "id": "agent-scraper", "name": "Scraper", "role": "Collection" }),
            json!({ "id": "agent-pinned", "name": "Pinned", "model": "openai/gpt-5" }),
            json!({ "id": "agent-asleep", "name": "Asleep", "status": "paused" }),
        ] {
            inertia_store::collections::put(&layout, inertia_store::Collection::Agents, record)
                .expect("written");
        }
        (dir, layout)
    }

    fn tool(provider: MockProvider) -> (tempfile::TempDir, TaskTool, Arc<Fake>) {
        let (dir, layout) = workspace();
        let fake = Arc::new(Fake {
            layout,
            provider: Arc::new(provider),
            built: Mutex::new(Vec::new()),
            progress: Mutex::new(Vec::new()),
            system: Mutex::new(Vec::new()),
        });
        let tool = TaskTool::new(
            fake.clone(),
            Arc::new(Sessions::default()),
            "thread-1".into(),
            "anthropic/claude-sonnet-5".into(),
        );
        (dir, tool, fake)
    }

    fn ctx(root: &std::path::Path) -> ToolContext {
        ToolContext {
            root: root.to_path_buf(),
            session: SessionId::from_existing("thread-1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(MockGate::allow_all()),
        }
    }

    fn brief(agent: &str) -> Value {
        json!({
            "description": "check the docs",
            "prompt": "Read the README and say what this project is.",
            "subagent_type": agent
        })
    }

    #[tokio::test]
    async fn a_subagent_runs_and_its_answer_comes_back_as_the_result() {
        let (dir, tool, fake) = tool(MockProvider::new().replying("It is a desktop agent app."));

        let out = tool
            .execute(brief("Scraper"), &ctx(dir.path()))
            .await
            .expect("the subagent ran");

        assert_eq!(out.title.as_deref(), Some("Scraper: check the docs"));
        assert!(
            out.output.contains("It is a desktop agent app."),
            "{}",
            out.output
        );
        assert!(out
            .output
            .starts_with("<task id=\"thread-1/task-1\" agent=\"Scraper\" state=\"complete\">"));

        // It really ran: the provider was asked, with the subagent's own prompt.
        assert_eq!(fake.provider.requests().len(), 1);
        assert_eq!(
            fake.system.lock().first().map(String::as_str),
            Some("You are Scraper, on anthropic/claude-sonnet-5.")
        );
    }

    #[tokio::test]
    async fn the_subagent_cannot_delegate_again() {
        let (dir, tool, fake) = tool(MockProvider::new().replying("done"));
        tool.execute(brief("Scraper"), &ctx(dir.path()))
            .await
            .expect("the subagent ran");

        let (session, withheld) = fake
            .built
            .lock()
            .first()
            .cloned()
            .expect("a registry was built");
        assert_eq!(session, "thread-1/task-1");
        // Nested delegation multiplies cost and latency in a way nobody wanted.
        assert!(withheld.contains(&"task".to_string()));
        // And a subagent has no room to invite anybody into.
        for group in ["invite", "handover", "part"] {
            assert!(
                withheld.contains(&group.to_string()),
                "{group} was not withheld"
            );
        }
    }

    #[tokio::test]
    async fn an_agent_with_its_own_model_keeps_it_and_one_without_inherits() {
        let (dir, tool, fake) = tool(MockProvider::new().replying("a").replying("b"));

        tool.execute(brief("Pinned"), &ctx(dir.path()))
            .await
            .expect("ran");
        assert!(fake.system.lock()[0].contains("openai/gpt-5"));

        tool.execute(brief("Scraper"), &ctx(dir.path()))
            .await
            .expect("ran");
        assert!(fake.system.lock()[1].contains("anthropic/claude-sonnet-5"));
    }

    #[tokio::test]
    async fn a_follow_up_continues_the_same_run_rather_than_starting_a_fresh_one() {
        let (dir, tool, fake) = tool(
            MockProvider::new()
                .replying("The README says it is an agent app.")
                .replying("Yes, and it ships as a desktop binary."),
        );

        let first = tool
            .execute(brief("Scraper"), &ctx(dir.path()))
            .await
            .expect("ran");
        let id = first.metadata.as_ref().expect("metadata")["taskId"]
            .as_str()
            .expect("a task id")
            .to_string();
        assert_eq!(id, "thread-1/task-1");

        let second = tool
            .execute(
                json!({
                    "description": "follow up",
                    "prompt": "Does it ship as a binary?",
                    "subagent_type": "Scraper",
                    "task_id": id
                }),
                &ctx(dir.path()),
            )
            .await
            .expect("ran");

        // Same run, not a second one.
        assert_eq!(
            second.metadata.expect("metadata")["taskId"],
            json!("thread-1/task-1")
        );
        // And it was asked knowing what it already said: the second request
        // carries the first exchange, which is the whole point of a follow-up.
        let requests = fake.provider.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].history.len() > requests[0].history.len());
    }

    #[tokio::test]
    async fn a_paused_agent_is_reported_rather_than_quietly_run() {
        let (dir, tool, fake) = tool(MockProvider::new().replying("should never run"));
        let error = tool
            .execute(brief("Asleep"), &ctx(dir.path()))
            .await
            .expect_err("a paused agent does not take work");

        assert!(error.to_string().contains("is paused"), "{error}");
        assert_eq!(
            fake.provider.requests().len(),
            0,
            "the paused agent was run anyway"
        );
    }

    #[tokio::test]
    async fn a_name_nobody_has_comes_back_with_the_list() {
        let (dir, tool, _) = tool(MockProvider::new().replying("x"));
        let error = tool
            .execute(brief("the designer"), &ctx(dir.path()))
            .await
            .expect_err("nobody by that name");
        assert!(error.to_string().contains("Scraper"), "{error}");
    }

    #[tokio::test]
    async fn a_subagent_that_said_nothing_is_a_failure_the_parent_can_act_on() {
        let (dir, tool, _) = tool(MockProvider::new().replying(""));
        let error = tool
            .execute(brief("Scraper"), &ctx(dir.path()))
            .await
            .expect_err("an empty report is not an answer");
        assert!(
            error.to_string().contains("without producing an answer"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn the_parent_card_is_told_the_subagent_started() {
        // A subagent can run for a minute, and a card that says nothing for a
        // minute reads as a hang.
        let (dir, tool, fake) = tool(MockProvider::new().replying("done"));
        tool.execute(brief("Scraper"), &ctx(dir.path()))
            .await
            .expect("ran");

        let progress = fake.progress.lock();
        assert!(!progress.is_empty());
        assert_eq!(progress[0]["state"], json!("running"));
        assert_eq!(progress[0]["agent"], json!("Scraper"));
    }

    #[tokio::test]
    async fn task_reaches_an_autonomous_turn_and_never_a_chat_one() {
        // The end of the chain: the tool exists, it is added as a per-turn
        // extra, and the mode filter is what decides whether the model sees it.
        // Asserted against the real registry rather than the withheld list, so
        // this catches the wiring as well as the rule.
        use inertia_agent::prompt::Mode;

        let dir = tempfile::tempdir().expect("a temp dir");
        let workspace =
            Arc::new(crate::state::Workspace::open(dir.path().to_path_buf()).expect("workspace"));
        let fake = Arc::new(Fake {
            layout: workspace.layout.clone(),
            provider: Arc::new(MockProvider::new()),
            built: Mutex::new(Vec::new()),
            progress: Mutex::new(Vec::new()),
            system: Mutex::new(Vec::new()),
        });

        let named = |mode: Mode| {
            let tool: Arc<dyn Tool> = Arc::new(TaskTool::new(
                fake.clone(),
                Arc::new(Sessions::default()),
                "thread-1".into(),
                "anthropic/claude-sonnet-5".into(),
            ));
            let gate: Arc<dyn inertia_core::tool::PermissionGate> = Arc::new(MockGate::allow_all());
            let registry =
                crate::state::registry_with(&workspace, gate, None, vec![tool], &mode.withheld());
            async move {
                registry
                    .specs()
                    .await
                    .into_iter()
                    .map(|spec| spec.name)
                    .collect::<Vec<_>>()
            }
        };

        assert!(named(Mode::Autonomous).await.contains(&"task".to_string()));
        assert!(named(Mode::Group).await.contains(&"task".to_string()));
        // A turn that may not write a file must not be able to ask a helper to.
        assert!(!named(Mode::Chat).await.contains(&"task".to_string()));
        assert!(!named(Mode::Plan).await.contains(&"task".to_string()));
    }

    #[test]
    fn forgetting_a_conversation_forgets_its_subagents() {
        let sessions = Sessions::default();
        sessions.remember("thread-1/task-1", vec![Entry::user("x")]);
        sessions.remember("thread-2/task-1", vec![Entry::user("y")]);

        sessions.forget("thread-1");
        assert!(sessions.history("thread-1/task-1").is_none());
        assert!(sessions.history("thread-2/task-1").is_some());
    }
}

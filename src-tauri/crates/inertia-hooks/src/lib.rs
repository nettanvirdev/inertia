//! Lifecycle hooks.
//!
//! A hook is a program the user wrote that runs at a named moment of an agent's
//! work: before a tool is used, after it, when the model wants to stop, when a
//! turn begins. It is how a person automates the things they would otherwise
//! have to say every time - "run the linter after every edit", "never let it
//! touch the migrations folder", "do not stop until the tests pass" - and it is
//! the difference between a coding agent that can be customised and one that
//! can only be prompted.
//!
//! The shape is Claude Code's, deliberately. Its hooks format is the one people
//! have already written scripts against, the events are the ones that turned
//! out to matter in practice, and Codex adopted the same engine for the same
//! reason. A `hooks.json` written for either runs here unchanged, which is
//! worth more than a nicer shape nobody has scripts for.
//!
//! Handlers for one event run in parallel and cannot see each other. The first
//! block wins; additional context accumulates. **A handler that crashes or
//! times out is a failure in the log and a warning in the transcript, never a
//! reason to end the turn** - a broken hook must cost the user the hook, not
//! the agent.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod config;
pub mod outcome;
pub mod run;

pub use config::{
    is_blockable, load, matched_by, matches, parse_config, select, Handler, Kind, Loaded, EVENTS,
    PROJECT_FILE,
};
pub use outcome::{extract_json, parse_output, Decision, Outcome};
pub use run::{run_command, run_prompt, Ask, RunResult, Status};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Runs kept for the settings pane, newest first.
const MAX_RECENT: usize = 100;

/// One handler's run, as the settings pane and the transcript show it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub at: i64,
    pub event: String,
    pub subject: Option<String>,
    pub handler: String,
    pub source: String,
    /// Not `type`: this summary becomes a transcript event, whose `type` says
    /// what kind of event it is, and a hook run that called itself "command"
    /// was filed under a tool that does not exist.
    pub handler_type: String,
    pub status: String,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub reason: Option<String>,
    pub stderr: Option<String>,
    pub agent: Option<String>,
    pub thread_id: Option<String>,
}

/// Who the turn was, so a run can say whose hook it was.
#[derive(Debug, Clone, Default)]
pub struct Context {
    pub session_id: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub agent: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub model: Option<String>,
    pub conversation_mode: Option<String>,
}

type Listener = Arc<dyn Fn(&RunSummary) + Send + Sync>;

/// The last hundred runs and whoever is watching them.
///
/// Process-wide rather than per-workspace, matching what it is for: the
/// settings pane shows what has run since the app started, and a hook that
/// fired in a thread the user has since closed is exactly the one they are
/// looking for when something went wrong.
#[derive(Default)]
pub struct Recorder {
    recent: Mutex<Vec<RunSummary>>,
    listeners: Mutex<Vec<(u64, Listener)>>,
    next: Mutex<u64>,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("recent", &self.recent.lock().len())
            .field("listeners", &self.listeners.lock().len())
            .finish()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The last hundred runs, newest first.
    pub fn recent(&self) -> Vec<RunSummary> {
        self.recent.lock().clone()
    }

    /// Watch every run. The returned id unsubscribes.
    pub fn on_run(&self, listener: Listener) -> u64 {
        let mut next = self.next.lock();
        *next += 1;
        let id = *next;
        self.listeners.lock().push((id, listener));
        id
    }

    pub fn off(&self, id: u64) {
        self.listeners.lock().retain(|(known, _)| *known != id);
    }

    fn remember(&self, summary: &RunSummary) {
        {
            let mut recent = self.recent.lock();
            recent.insert(0, summary.clone());
            recent.truncate(MAX_RECENT);
        }
        // Cloned out of the lock before calling: a listener that publishes an
        // event and comes back round would otherwise deadlock here.
        let listeners: Vec<Listener> = self
            .listeners
            .lock()
            .iter()
            .map(|(_, listener)| listener.clone())
            .collect();
        for listener in listeners {
            listener(summary);
        }
    }

    #[cfg(test)]
    fn clear(&self) {
        self.recent.lock().clear();
        self.listeners.lock().clear();
    }
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// Fire one event: run every matching handler, merge what they said.
///
/// Returns the merged outcome and every run that produced it. Nothing in here
/// returns an error: a hook is the user's own program and its failure is
/// theirs to see, not the turn's to die of.
pub async fn fire(
    event: &str,
    subject: Option<&str>,
    input: Value,
    handlers: &[Handler],
    context: &Context,
    ask: Option<&dyn Ask>,
    recorder: &Recorder,
) -> (Outcome, Vec<RunSummary>) {
    let chosen = select(handlers, event, subject);
    if chosen.is_empty() {
        return (Outcome::default(), Vec::new());
    }

    let payload = payload_for(event, &input, context);
    let env = env_for(event, context);
    let cwd = context.cwd.clone();

    // In parallel and unable to see each other, so two hooks on one event
    // cannot be made to depend on an order that is not promised.
    let results = futures::future::join_all(chosen.iter().map(|handler| {
        run_one(
            handler,
            event,
            subject,
            &payload,
            cwd.as_deref(),
            &env,
            ask,
            recorder,
        )
    }))
    .await;

    let mut merged = Outcome::default();
    let mut runs = Vec::new();
    for (outcome, summary) in results {
        runs.push(summary);
        if outcome.block && !merged.block {
            merged.block = true;
            merged.reason = outcome.reason.clone();
        }
        // The most cautious answer wins, which is what the ordering on
        // `Decision` is for.
        if outcome.decision > merged.decision {
            merged.decision = outcome.decision;
        }
        if merged.updated_input.is_none() {
            merged.updated_input = outcome.updated_input.clone();
        }
        merged.context.extend(outcome.context.iter().cloned());
        if let Some(message) = &outcome.message {
            merged.message = Some(match &merged.message {
                Some(existing) => format!("{existing}\n{message}"),
                None => message.clone(),
            });
        }
        if outcome.stop {
            merged.stop = true;
            merged.stop_reason = merged.stop_reason.take().or(outcome.stop_reason);
        }
    }

    (merged, runs)
}

#[allow(clippy::too_many_arguments)]
async fn run_one(
    handler: &Handler,
    event: &str,
    subject: Option<&str>,
    payload: &Value,
    cwd: Option<&std::path::Path>,
    env: &HashMap<String, String>,
    ask: Option<&dyn Ask>,
    recorder: &Recorder,
) -> (Outcome, RunSummary) {
    let result = match handler.kind {
        Kind::Prompt => run_prompt(handler, payload, ask).await,
        Kind::Command => run_command(handler, payload, cwd, env).await,
    };

    let mut outcome = if result.status == Status::Ok {
        let mut read = parse_output(
            extract_json(&result.stdout).as_ref(),
            event,
            result.exit_code.or(Some(0)),
            &result.stdout,
            &result.stderr,
        );
        // A prompt hook answers approve/block; on the permission events that
        // block is the same thing as a denial.
        if handler.kind == Kind::Prompt
            && read.block
            && (event == "PreToolUse" || event == "PermissionRequest")
        {
            read.decision = Some(Decision::Deny);
        }
        read
    } else {
        Outcome::default()
    };

    if !is_blockable(event) && outcome.block {
        // A block on an event that cannot be blocked is a message, not a veto.
        outcome.message = outcome.message.take().or_else(|| outcome.reason.clone());
        outcome.block = false;
        outcome.reason = None;
    }

    let summary = RunSummary {
        at: now_ms(),
        event: event.to_string(),
        subject: subject.map(str::to_string),
        handler: handler
            .name
            .clone()
            .or_else(|| handler.command.clone())
            .unwrap_or_else(|| "prompt".into()),
        source: handler.source.clone(),
        handler_type: match handler.kind {
            Kind::Prompt => "prompt".into(),
            Kind::Command => "command".into(),
        },
        status: if result.status == Status::Ok {
            if outcome.block {
                "blocked".into()
            } else if outcome.stop {
                "stopped".into()
            } else {
                "ok".into()
            }
        } else {
            result.status.as_str().to_string()
        },
        duration_ms: result.duration_ms,
        error: result.error.clone(),
        reason: outcome.reason.clone().or_else(|| outcome.message.clone()),
        stderr: Some(result.stderr.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(2000).collect()),
        agent: context_agent(handler),
        thread_id: None,
    };

    recorder.remember(&summary);
    (outcome, summary)
}

/// The agent a handler belongs to, when it came from one.
fn context_agent(handler: &Handler) -> Option<String> {
    handler.source.strip_prefix("agent:").map(str::to_string)
}

fn payload_for(event: &str, input: &Value, context: &Context) -> Value {
    let path = |p: &Option<PathBuf>| {
        p.as_ref()
            .map(|p| Value::String(p.display().to_string()))
            .unwrap_or(Value::Null)
    };
    let text = |t: &Option<String>| t.clone().map(Value::String).unwrap_or(Value::Null);

    let mut payload = json!({
        "session_id": text(&context.session_id),
        "thread_id": text(&context.thread_id),
        "turn_id": text(&context.turn_id),
        "agent_name": text(&context.agent),
        "agent_id": text(&context.agent_id),
        "cwd": path(&context.cwd),
        "workspace": path(&context.root),
        "model": text(&context.model),
        "permission_mode": text(&context.conversation_mode),
        "hook_event_name": event,
    });

    if let (Some(target), Some(extra)) = (payload.as_object_mut(), input.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    payload
}

fn env_for(event: &str, context: &Context) -> HashMap<String, String> {
    let path = |p: &Option<PathBuf>| {
        p.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    };

    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.insert("INERTIA_PROJECT_DIR".into(), path(&context.cwd));
    env.insert("INERTIA_WORKSPACE".into(), path(&context.root));
    env.insert(
        "INERTIA_SESSION_ID".into(),
        context.session_id.clone().unwrap_or_default(),
    );
    env.insert(
        "INERTIA_AGENT".into(),
        context.agent.clone().unwrap_or_default(),
    );
    env.insert("INERTIA_HOOK_EVENT".into(), event.to_string());
    // Claude Code's name as well, so a script written for it needs no edit.
    env.insert("CLAUDE_PROJECT_DIR".into(), path(&context.cwd));
    env
}

/// An example file, written when the user asks for one.
///
/// Two of the three are switched off. Somebody asking for an example wants to
/// read what a hook looks like, not to discover that their next turn is being
/// judged by a hook they have not read yet.
pub fn example() -> Value {
    json!({
        "enabled": true,
        "PreToolUse": [{
            "matcher": "shell",
            "hooks": [{
                "name": "no force push",
                "type": "command",
                "command": "node -e \"const i=JSON.parse(require('fs').readFileSync(0,'utf8'));if(/git push[^|]*--force/.test(i.tool_input.command||'')){console.error('Force pushes are not allowed here.');process.exit(2)}\"",
                "timeout": 10,
            }],
        }],
        "PostToolUse": [{
            "matcher": "edit|write",
            "hooks": [{
                "name": "lint on save",
                "type": "prompt",
                "prompt": "The agent just changed a file. If the edit obviously introduced a syntax error or left a TODO the user did not ask for, say so briefly. Otherwise approve.",
                "enabled": false,
            }],
        }],
        "Stop": [{
            "hooks": [{
                "name": "finish the list",
                "type": "prompt",
                "prompt": "The agent wants to stop. If the task list it was working from still has items in_progress or pending that it never mentioned, block and tell it which. Otherwise approve.",
                "enabled": false,
            }],
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handlers(doc: &Value) -> Vec<Handler> {
        parse_config(Some(doc), "test").handlers
    }

    async fn fire_stop(doc: Value) -> (Outcome, Vec<RunSummary>) {
        let recorder = Recorder::new();
        let loaded = handlers(&doc);
        fire(
            "Stop",
            None,
            json!({}),
            &loaded,
            &Context::default(),
            None,
            &recorder,
        )
        .await
    }

    #[tokio::test]
    async fn an_event_with_no_handlers_runs_nothing() {
        let (outcome, runs) = fire_stop(json!({})).await;
        assert!(!outcome.block);
        assert!(runs.is_empty());
    }

    #[tokio::test]
    async fn a_blocking_hook_blocks_a_blockable_event() {
        let doc = json!({
            "Stop": [{ "hooks": [{
                "command": "echo the tests are red 1>&2 && exit 2",
            }] }],
        });
        let (outcome, runs) = fire_stop(doc).await;
        assert!(outcome.block);
        assert!(outcome.reason.unwrap().contains("tests are red"));
        assert_eq!(runs[0].status, "blocked");
    }

    /// The rule that keeps a broken hook from ending a turn.
    #[tokio::test]
    async fn a_hook_that_fails_does_not_block_anything() {
        let doc = json!({ "Stop": [{ "hooks": [{ "command": "exit 9" }] }] });
        let (outcome, runs) = fire_stop(doc).await;
        assert!(!outcome.block);
        assert!(!outcome.stop);
        assert_eq!(runs[0].status, "failed");
        assert!(runs[0].error.is_some());
    }

    #[tokio::test]
    async fn a_block_on_an_unblockable_event_becomes_a_message() {
        let recorder = Recorder::new();
        let loaded = handlers(&json!({
            "Notification": [{ "hooks": [{ "command": "echo not allowed 1>&2 && exit 2" }] }],
        }));
        let (outcome, _) = fire(
            "Notification",
            None,
            json!({}),
            &loaded,
            &Context::default(),
            None,
            &recorder,
        )
        .await;
        assert!(!outcome.block);
        assert!(outcome.message.unwrap().contains("not allowed"));
    }

    #[tokio::test]
    async fn several_handlers_all_run_and_their_context_accumulates() {
        let doc = json!({
            "SessionStart": [{ "hooks": [
                { "command": "echo one" },
                { "command": "echo two" },
            ] }],
        });
        let recorder = Recorder::new();
        let loaded = handlers(&doc);
        let (outcome, runs) = fire(
            "SessionStart",
            None,
            json!({}),
            &loaded,
            &Context::default(),
            None,
            &recorder,
        )
        .await;
        assert_eq!(runs.len(), 2);
        let joined = outcome.context.join(" ");
        assert!(joined.contains("one") && joined.contains("two"), "{joined}");
    }

    #[tokio::test]
    async fn the_most_cautious_decision_wins() {
        let doc = json!({
            "PreToolUse": [{ "hooks": [
                { "command": "echo {\\\"decision\\\":\\\"approve\\\"}" },
                { "command": "echo blocked 1>&2 && exit 2" },
            ] }],
        });
        let recorder = Recorder::new();
        let loaded = handlers(&doc);
        let (outcome, _) = fire(
            "PreToolUse",
            Some("shell"),
            json!({}),
            &loaded,
            &Context::default(),
            None,
            &recorder,
        )
        .await;
        assert_eq!(outcome.decision, Some(Decision::Deny));
    }

    #[tokio::test]
    async fn a_matcher_keeps_a_hook_off_other_tools() {
        let doc = json!({
            "PreToolUse": [{ "matcher": "shell", "hooks": [{ "command": "exit 2" }] }],
        });
        let recorder = Recorder::new();
        let loaded = handlers(&doc);
        let (outcome, runs) = fire(
            "PreToolUse",
            Some("read"),
            json!({}),
            &loaded,
            &Context::default(),
            None,
            &recorder,
        )
        .await;
        assert!(!outcome.block);
        assert!(runs.is_empty());
    }

    #[tokio::test]
    async fn the_event_payload_carries_the_turn_and_the_caller_s_own_fields() {
        let context = Context {
            thread_id: Some("t-1".into()),
            ..Default::default()
        };
        let payload = payload_for("PreToolUse", &json!({ "tool_name": "shell" }), &context);
        assert_eq!(payload["hook_event_name"], "PreToolUse");
        assert_eq!(payload["thread_id"], "t-1");
        assert_eq!(payload["tool_name"], "shell");
        assert_eq!(payload["session_id"], Value::Null);
    }

    #[tokio::test]
    async fn every_run_is_remembered_and_announced() {
        let recorder = Recorder::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        recorder.on_run(Arc::new(move |summary: &RunSummary| {
            sink.lock().push(summary.event.clone());
        }));

        let loaded = handlers(&json!({ "Stop": [{ "hooks": [{ "command": "echo hi" }] }] }));
        fire(
            "Stop",
            None,
            json!({}),
            &loaded,
            &Context::default(),
            None,
            &recorder,
        )
        .await;

        assert_eq!(recorder.recent().len(), 1);
        assert_eq!(seen.lock().as_slice(), ["Stop"]);
        recorder.clear();
        assert!(recorder.recent().is_empty());
    }

    /// Against the recorder rather than through `fire`: this is a property of
    /// the ring, and proving it by spawning a hundred processes would put half
    /// a minute in the test suite to learn nothing more.
    #[test]
    fn the_recent_list_is_newest_first_and_capped() {
        let recorder = Recorder::new();
        for n in 0..(MAX_RECENT + 5) {
            let mut summary = summary_named(&n.to_string());
            summary.at = n as i64;
            recorder.remember(&summary);
        }
        let recent = recorder.recent();
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(recent[0].handler, (MAX_RECENT + 4).to_string());
        assert!(recent[0].at > recent[recent.len() - 1].at);
    }

    fn summary_named(handler: &str) -> RunSummary {
        RunSummary {
            at: 0,
            event: "Stop".into(),
            subject: None,
            handler: handler.to_string(),
            source: "test".into(),
            handler_type: "command".into(),
            status: "ok".into(),
            duration_ms: 0,
            error: None,
            reason: None,
            stderr: None,
            agent: None,
            thread_id: None,
        }
    }

    #[test]
    fn the_example_is_valid_and_mostly_switched_off() {
        let doc = example();
        let parsed = parse_config(Some(&doc), "example");
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(parsed.handlers.len(), 3);
        assert_eq!(parsed.handlers.iter().filter(|h| h.enabled).count(), 1);
    }

    #[test]
    fn the_environment_carries_claude_codes_names_too() {
        let env = env_for(
            "PreToolUse",
            &Context {
                cwd: Some(PathBuf::from("/repo")),
                ..Default::default()
            },
        );
        assert_eq!(env["INERTIA_HOOK_EVENT"], "PreToolUse");
        assert_eq!(env["CLAUDE_PROJECT_DIR"], env["INERTIA_PROJECT_DIR"]);
    }
}

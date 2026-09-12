//! Working alongside other agents instead of waiting on them.
//!
//! `task` hands a job to a subagent and waits for the answer. That is the right
//! shape for one question, and it is the wrong shape for a piece of work: three
//! `task` calls are three sessions run one after another, and the parent - which
//! has a model, tools and a plan - sits and does nothing through all three.
//!
//! These tools are the other shape.
//!
//!   `spawn`       start a run, get an id back now, keep working
//!   `collect`     wait for ids, when their answers are actually needed
//!   `wait`        block until anything happens - a run settles or a message
//!                 arrives - whichever is first
//!   `team`        who is doing what, and what they have said to you
//!   `agent_send`  say something to one of them without waiting
//!   `interrupt`   stop a run mid-turn but keep it, and what it has read
//!   `followup`    give a settled run a new brief, with its context intact
//!
//! The split between `spawn` and `collect` is the entire feature. Everything
//! else here - the table, the mailbox, the snapshot the panel draws - is
//! bookkeeping around the fact that starting work and needing its result are
//! two different moments, and the time between them is what a parent agent can
//! spend doing its own share.
//!
//! # Where the machinery comes from
//!
//! A run is a subagent turn, and it is started exactly the way `task` starts
//! one: through [`Delegate`], which supplies the provider, the registry, the
//! gate and the system prompt. The one thing `task` does not need and this does
//! is a way to hand a child its own team tools, so it can nest when its policy
//! allows - that is [`CrewDelegate::registry_with_extras`], which defaults to
//! ignoring the extras so an app that has not wired nesting simply has none.
//!
//! # What stops this from becoming a fork bomb
//!
//! Per-agent policy, checked before anything starts: [`spawn_refusal`]. An agent
//! may or may not delegate at all; its children may or may not delegate
//! further; it may or may not start a full configured agent as opposed to a
//! temporary helper; and it may have a ceiling on how many of its own runs are
//! live at once. Above every policy sits [`TEAM_LIMITS`]: twenty live and a
//! hundred ever, per conversation. Not a policy anyone tunes; the wall a runaway
//! hits.
//!
//! # What the table is and is not
//!
//! [`Runs`] is in memory, on `AppState`, and not on disk - deliberately. A run
//! is a live tokio task with a stop handle attached, and neither survives a
//! restart, so a persisted row would come back claiming to be running with
//! nothing behind it. Transcripts are kept for as long as the conversation is
//! open, because the whole complaint that started this was that a subagent's
//! work was invisible, and a finished helper whose output has been thrown away
//! is still invisible.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::StreamExt;
use inertia_agent::{Agent as Loop, AgentConfig, AgentEvent, StopReason, Turn as AgentTurn};
use inertia_core::id::SessionId;
use inertia_core::message::Entry;
use inertia_core::provider::Usage;
use inertia_core::tool::{
    PermissionGate, PermissionRequest, Tool, ToolContext, ToolOutcome, ToolRegistry, ToolSource,
};
use inertia_core::Result;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::sync::{oneshot, watch};

use crate::agents::Agent;
use crate::task_tool::Delegate;

/// How long `collect` waits by default before reporting back unfinished.
const DEFAULT_COLLECT_SECONDS: u64 = 600;

/// The most a single `collect` or `wait` may block, however hopeful the model is.
const MAX_COLLECT_SECONDS: u64 = 1800;

/// `wait` is for "whatever happens first", so its default is shorter.
const DEFAULT_WAIT_SECONDS: u64 = 300;

/// How much of the parent's conversation `share_context` hands over.
const SHARED_CONTEXT_CHARS: usize = 12_000;

/// How many events one run keeps before the oldest are dropped.
const MAX_EVENTS: usize = 300;

/// How much of a run's own reply is kept for the live view.
const MAX_TEXT: usize = 40_000;

/// A spawned run cannot `task` (its team tools are the nested form) and has no
/// room to invite anyone into.
const WITHHELD_IN_A_RUN: &[&str] = &["task", "invite", "handover", "part", "present_plan"];

// ── policy ──────────────────────────────────────────────────────────────

/// The ceiling on a whole conversation, whatever each agent's policy says.
///
/// Per-agent `maxConcurrent` defaults to unlimited on purpose. This is the
/// other kind of limit: not "how many may this agent run" but "how many may
/// exist at all under one chat", for the case no policy anticipates - a
/// coordinator that spawns a helper per file in a repository of three hundred
/// files, or a helper that spawns a helper that spawns a helper.
pub const TEAM_LIMITS: TeamLimits = TeamLimits {
    live: 20,
    total: 100,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamLimits {
    /// Runs in flight at once, across the whole conversation.
    pub live: usize,
    /// Runs ever started in one conversation, including finished ones.
    pub total: usize,
}

/// What an agent may do about building a team.
///
/// Defaults chosen so an existing workspace behaves the way it did yesterday
/// and no further: every agent could already delegate one level deep, so
/// `subagents` is on; none of them could nest, spawn a peer, or run unbounded,
/// so those are off until someone turns them on for that agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnPolicy {
    /// May hand work to a temporary subagent at all.
    pub subagents: bool,
    /// May start another configured agent as a peer, not only as a helper.
    pub agents: bool,
    /// May its children spawn children of their own.
    pub recursive: bool,
    /// How many of its own children may be live at once. 0 is unlimited.
    pub max_concurrent: u32,
}

impl Default for SpawnPolicy {
    fn default() -> Self {
        Self {
            subagents: true,
            agents: false,
            recursive: false,
            max_concurrent: 0,
        }
    }
}

/// JavaScript truthiness, because the record was written by a renderer that
/// reads it with `Boolean(...)` and the two sides must agree on `1` and `"yes"`.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// One agent's policy, whatever shape the record on disk is in.
pub fn spawn_policy(record: &Value) -> SpawnPolicy {
    let Some(stored) = record.get("spawn").filter(|stored| stored.is_object()) else {
        return SpawnPolicy::default();
    };
    let max = stored
        .get("maxConcurrent")
        .and_then(|max| match max {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        })
        .filter(|max| max.is_finite() && *max > 0.0)
        .map(|max| max.floor() as u32)
        .unwrap_or(0);
    SpawnPolicy {
        subagents: stored.get("subagents").and_then(Value::as_bool) != Some(false),
        agents: truthy(stored.get("agents")),
        recursive: truthy(stored.get("recursive")),
        max_concurrent: max,
    }
}

/// May this turn hold the team tools at all?
///
/// Two different questions wearing one word. At the top of a conversation the
/// question is whether this agent delegates; below it, the question is whether
/// it may nest - and those are separate switches because the failure modes are
/// different. A parent that delegates too eagerly costs a few sessions. A tree
/// that nests without a limit costs a tree.
pub fn may_delegate(record: &Value, depth: u32) -> bool {
    let policy = spawn_policy(record);
    if depth == 0 {
        policy.subagents
    } else {
        policy.subagents && policy.recursive
    }
}

/// What is already going on, for [`spawn_refusal`] to weigh.
#[derive(Debug, Clone, Copy, Default)]
pub struct Load {
    pub depth: u32,
    /// This agent's own live children.
    pub running: usize,
    /// Whether a configured agent, rather than a temporary helper, was named.
    pub peer: bool,
    /// Runs live across the whole conversation.
    pub live: usize,
    /// Runs ever started in the conversation.
    pub total: usize,
}

/// Why a particular spawn cannot happen, or `None` if it can.
///
/// The sentence is written for the model, because the model is who reads it and
/// the model is who has to do something else instead. "Denied" tells it nothing;
/// "you may not nest, so do this one yourself" tells it what to do next.
pub fn spawn_refusal(record: &Value, name: &str, load: Load) -> Option<String> {
    let policy = spawn_policy(record);
    if load.total >= TEAM_LIMITS.total {
        return Some(format!(
            "This conversation has already started {} runs, which is the most one conversation may. \
             Finish with what exists, or ask the user to start a fresh conversation for the rest.",
            load.total
        ));
    }
    if load.live >= TEAM_LIMITS.live {
        return Some(format!(
            "There are already {} runs in flight in this conversation, which is the most that may run \
             at once. Call `wait` or `collect` until some finish before spawning more.",
            load.live
        ));
    }
    if !policy.subagents {
        return Some(format!(
            "{name} is not allowed to delegate work. Do it yourself, or ask the user to turn on \
             subagents for this agent under Agents."
        ));
    }
    if load.depth > 0 && !policy.recursive {
        return Some(format!(
            "You are already a subagent, and {name} is not allowed to spawn subagents of its own. \
             Do this part yourself and report back."
        ));
    }
    if load.peer && !policy.agents {
        return Some(format!(
            "{name} may spawn helpers but not full agents. Spawn this as a subagent instead, or ask \
             the user to allow spawning agents."
        ));
    }
    if policy.max_concurrent > 0 && load.running >= policy.max_concurrent as usize {
        return Some(format!(
            "{name} already has {} runs in flight, which is its limit. Call `collect` on one of them \
             before spawning another.",
            load.running
        ));
    }
    None
}

// ── the table ───────────────────────────────────────────────────────────

/// The statuses a run can be in.
///
/// Deliberately few. A status is what someone reading the panel needs in order
/// to know whether to wait; finer grain belongs in `activity`, which is free
/// text and changes every few seconds. Spelled the way the renderer's
/// `shared/crew.js` spells them, because the panel switches on the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
    Interrupted,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    /// Not finished. Its answer is still coming.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    /// Settled, but with its conversation kept, so a new brief can pick it up.
    /// `cancelled` is stopped for good and is not.
    pub fn can_follow_up(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Interrupted)
    }
}

/// A short present-tense phrase for what a run is doing right now, derived from
/// the tool it is calling rather than asked for. Coarse on purpose: the panel is
/// glanced at, not read.
pub fn activity_for(tool: &str) -> &'static str {
    match tool {
        "" => "Thinking",
        "read" | "ls" | "glob" | "grep" => "Reading",
        "write" | "edit" | "patch" => "Writing",
        "spawn" | "task" | "collect" | "team" | "interrupt" | "followup" => "Coordinating",
        "wait" => "Waiting on the team",
        "agent_send" => "Messaging",
        "question" | "present_plan" => "Waiting on the user",
        "todowrite" | "skill" => "Planning",
        _ if tool == "shell" || tool.starts_with("shell_") => "Running commands",
        _ if tool.starts_with("computer") => "Driving a computer",
        _ => "Working",
    }
}

#[derive(Debug, Clone)]
pub struct Step {
    pub at: u64,
    pub tool: String,
    pub title: String,
}

/// One agent saying something to another, sitting in an inbox until read.
#[derive(Debug, Clone)]
pub struct Message {
    pub at: u64,
    pub from: Option<String>,
    pub from_name: String,
    pub text: String,
    pub read: bool,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub at: u64,
    pub kind: &'static str,
    pub text: String,
}

/// One agent doing one piece of work. Everything the panel draws, and nothing
/// that cannot cross a bridge.
#[derive(Debug, Clone)]
pub struct Run {
    pub id: String,
    pub conversation: String,
    /// The run that spawned this one; `None` for a child of the top-level turn.
    pub parent_id: Option<String>,
    pub depth: u32,
    pub agent_id: Option<String>,
    pub agent_name: String,
    pub description: String,
    /// The brief, kept so the panel can answer "what was it actually asked?"
    pub prompt: String,
    pub model: String,
    pub status: Status,
    pub activity: String,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub steps: Vec<Step>,
    pub text: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub usage: Option<Usage>,
    pub collected: bool,
    pub follow_ups: u32,
    pub inbox: Vec<Message>,
    pub events: Vec<Event>,
    /// Which attempt at this row is the live one.
    ///
    /// A row is reused when a run is followed up, so the same id can have had
    /// several sessions behind it. An interrupted one keeps unwinding for a
    /// moment, and its ending would otherwise land on the attempt that
    /// replaced it. Every attempt carries the generation it belongs to, and
    /// anything arriving from an older one is dropped.
    pub generation: u64,
    /// Creation order, for a stable listing when two runs start in one tick.
    seq: u64,
}

/// How to put a settled run back to work: reopen the row and start a session
/// from its transcript plus the new brief. A closure rather than stored
/// arguments, because starting a session needs the provider, the agent record
/// and the folder - everything the tool call had and the panel never sees.
type Relauncher = Arc<dyn Fn(String) -> std::result::Result<(), String> + Send + Sync>;

#[derive(Default)]
struct Table {
    runs: HashMap<String, Run>,
    /// Dropping one stops the run's loop. Emptied the moment a run settles.
    stops: HashMap<String, oneshot::Sender<()>>,
    /// What each run said and heard, kept so a follow-up can continue it. The
    /// opposite lifetime from `stops`: useful only from settlement on.
    transcripts: HashMap<String, Vec<Entry>>,
    relaunchers: HashMap<String, Relauncher>,
    /// Messages addressed to a top-level turn, which has no run row of its own.
    mailboxes: HashMap<String, Vec<Message>>,
    seq: u64,
}

/// Told when anything changed, and in which conversation, so the window can be
/// pushed a new snapshot and a waiter can look again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    pub seq: u64,
    pub conversation: String,
}

/// Every run this process knows about. One per app, on `AppState`.
pub struct Runs {
    table: Mutex<Table>,
    changed: watch::Sender<Change>,
}

impl std::fmt::Debug for Runs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runs")
            .field("runs", &self.table.lock().runs.len())
            .finish_non_exhaustive()
    }
}

impl Default for Runs {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn log(run: &mut Run, kind: &'static str, text: impl Into<String>) {
    run.events.push(Event {
        at: now_ms(),
        kind,
        text: text.into(),
    });
    if run.events.len() > MAX_EVENTS {
        let excess = run.events.len() - MAX_EVENTS;
        run.events.drain(..excess);
    }
}

impl Runs {
    pub fn new() -> Self {
        let (changed, _) = watch::channel(Change::default());
        Self {
            table: Mutex::new(Table::default()),
            changed,
        }
    }

    /// The conversation a run belongs to.
    ///
    /// Spawned sessions are addressed as `thread-7/run-3a9f`, so the
    /// conversation is whatever comes before the first slash however deep the
    /// nesting goes. The routing below splits inline; this is where the rule is
    /// written down and checked, so it is built only for the tests that do so.
    #[cfg(test)]
    pub fn conversation_of(session: &str) -> String {
        let head = session.split('/').next().unwrap_or_default();
        if head.is_empty() {
            "unknown".to_string()
        } else {
            head.to_string()
        }
    }

    /// A feed of changes. The coordinator emits a snapshot on each; the
    /// waiting tools look again on each.
    pub fn subscribe(&self) -> watch::Receiver<Change> {
        self.changed.subscribe()
    }

    fn notify(&self, conversation: &str) {
        self.changed.send_modify(|change| {
            change.seq += 1;
            change.conversation = conversation.to_string();
        });
    }

    /// Start tracking a run. It is not going yet - `began` does that.
    ///
    /// The row exists before the session does so the panel can show a spawn the
    /// instant the model asks for one, rather than after the provider has
    /// answered the child's first request.
    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        conversation: &str,
        parent_id: Option<&str>,
        depth: u32,
        agent: &Agent,
        description: &str,
        prompt: &str,
        model: &str,
    ) -> Run {
        let id = format!(
            "run-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let mut table = self.table.lock();
        table.seq += 1;
        let mut run = Run {
            id: id.clone(),
            conversation: conversation.to_string(),
            parent_id: parent_id.map(str::to_string),
            depth,
            agent_id: (!agent.id.is_empty()).then(|| agent.id.clone()),
            agent_name: agent.name.clone(),
            description: {
                let trimmed = description.trim();
                if trimmed.is_empty() { "work" } else { trimmed }.to_string()
            },
            prompt: prompt.to_string(),
            model: model.to_string(),
            status: Status::Queued,
            activity: "Starting".to_string(),
            started_at: now_ms(),
            ended_at: None,
            steps: Vec::new(),
            text: String::new(),
            result: None,
            error: None,
            usage: None,
            collected: false,
            follow_ups: 0,
            inbox: Vec::new(),
            events: Vec::new(),
            generation: 1,
            seq: table.seq,
        };
        // Built before the borrow: `log` takes the run mutably, and reading
        // two of its fields inside the call is two borrows at once.
        let announced = format!("{} was asked to {}", run.agent_name, run.description);
        log(&mut run, "spawned", announced);
        table.runs.insert(id, run.clone());
        drop(table);
        self.notify(conversation);
        run
    }

    fn attach(&self, id: &str, stop: oneshot::Sender<()>, relaunch: Relauncher) {
        let mut table = self.table.lock();
        table.stops.insert(id.to_string(), stop);
        table.relaunchers.insert(id.to_string(), relaunch);
    }

    /// Whether a report is from the attempt that is currently live.
    fn current(run: &Run, generation: u64) -> bool {
        run.generation == generation
    }

    fn began(&self, id: &str, generation: u64) {
        let conversation = {
            let mut table = self.table.lock();
            let Some(run) = table.runs.get_mut(id).filter(|run| Self::current(run, generation)) else {
                return;
            };
            run.status = Status::Running;
            run.activity = "Thinking".to_string();
            log(run, "started", "Started");
            run.conversation.clone()
        };
        self.notify(&conversation);
    }

    fn observe_tool(&self, id: &str, generation: u64, tool: &str, title: Option<&str>) {
        let conversation = {
            let mut table = self.table.lock();
            let Some(run) = table.runs.get_mut(id).filter(|run| Self::current(run, generation)) else {
                return;
            };
            let title = title.unwrap_or(tool).to_string();
            run.activity = activity_for(tool).to_string();
            run.steps.push(Step {
                at: now_ms(),
                tool: tool.to_string(),
                title: title.clone(),
            });
            log(run, "tool", title);
            run.conversation.clone()
        };
        self.notify(&conversation);
    }

    fn observe_text(&self, id: &str, generation: u64, text: &str) {
        let conversation = {
            let mut table = self.table.lock();
            let Some(run) = table.runs.get_mut(id).filter(|run| Self::current(run, generation)) else {
                return;
            };
            run.text.push_str(text);
            if run.text.len() > MAX_TEXT {
                // On a character boundary: slicing a UTF-8 string mid-glyph panics.
                let mut cut = run.text.len() - MAX_TEXT;
                while !run.text.is_char_boundary(cut) {
                    cut += 1;
                }
                run.text.drain(..cut);
            }
            run.activity = "Writing".to_string();
            run.conversation.clone()
        };
        self.notify(&conversation);
    }

    /// Remember what a run said and heard, for a follow-up. Dropped when it
    /// comes from an attempt that has since been superseded.
    fn keep(&self, id: &str, generation: u64, transcript: Vec<Entry>) {
        if transcript.is_empty() {
            return;
        }
        let mut table = self.table.lock();
        if table.runs.get(id).is_some_and(|run| Self::current(run, generation)) {
            table.transcripts.insert(id.to_string(), transcript);
        }
    }

    fn finish(
        &self,
        id: &str,
        generation: u64,
        result: &str,
        usage: Option<Usage>,
        transcript: Vec<Entry>,
    ) {
        self.keep(id, generation, transcript);
        let conversation = {
            let mut table = self.table.lock();
            let Some(run) = table.runs.get_mut(id).filter(|run| Self::current(run, generation)) else {
                return;
            };
            // Already settled by an interrupt or a cancel that got here first.
            // The loop returning afterwards is the same run, not a second
            // ending, and the status it was given by whoever stopped it stands.
            if run.status.is_active() {
                run.status = Status::Done;
                run.activity = "Completed".to_string();
                run.ended_at = Some(now_ms());
                run.result = Some(result.to_string());
                if usage.is_some() {
                    run.usage = usage;
                }
                log(run, "result", "Finished and returned an answer");
            }
            let conversation = run.conversation.clone();
            table.stops.remove(id);
            conversation
        };
        self.notify(&conversation);
    }

    fn fail(&self, id: &str, generation: u64, message: &str, cancelled: bool, transcript: Vec<Entry>) {
        self.keep(id, generation, transcript);
        let conversation = {
            let mut table = self.table.lock();
            let Some(run) = table.runs.get_mut(id).filter(|run| Self::current(run, generation)) else {
                return;
            };
            if run.status.is_active() {
                run.status = if cancelled { Status::Cancelled } else { Status::Failed };
                run.activity = if cancelled { "Cancelled" } else { "Failed" }.to_string();
                run.ended_at = Some(now_ms());
                run.error = Some(message.to_string());
                log(run, if cancelled { "cancelled" } else { "failed" }, message);
            }
            let conversation = run.conversation.clone();
            table.stops.remove(id);
            conversation
        };
        self.notify(&conversation);
    }

    /// Stop a run's current turn and keep the run.
    ///
    /// The difference from `cancel` is what is left afterwards. A cancelled run
    /// is over. An interrupted run has had its turn cut short and keeps its
    /// transcript up to that point, so a follow-up can say "stop, the brief has
    /// changed, do this instead" and be understood in the light of everything
    /// the run had already read. Its children are left alone: they were started
    /// for work the run may still want.
    pub fn interrupt(&self, id: &str, reason: &str) -> std::result::Result<(), String> {
        let conversation = {
            let mut table = self.table.lock();
            let Some(run) = table.runs.get_mut(id) else {
                return Err(format!("There is no run called {id}."));
            };
            if !run.status.is_active() {
                return Err(format!("{id} is not running."));
            }
            run.status = Status::Interrupted;
            run.activity = "Interrupted".to_string();
            run.ended_at = Some(now_ms());
            run.error = Some(reason.to_string());
            log(run, "interrupted", reason);
            let conversation = run.conversation.clone();
            // Dropping the sender is what stops the loop.
            table.stops.remove(id);
            conversation
        };
        self.notify(&conversation);
        Ok(())
    }

    /// Stop a run, and by default everything underneath it.
    ///
    /// The subtree goes with it because the alternative is orphans: a child
    /// whose parent has been cancelled has nobody left to report to, and it
    /// would keep spending money to produce an answer no one will read.
    pub fn cancel(&self, id: &str, descendants: bool, reason: &str) -> bool {
        let Some(run) = self.get(id) else { return false };
        if descendants {
            for child in self.children_of(id) {
                self.cancel(&child.id, true, reason);
            }
        }
        self.table.lock().stops.remove(id);
        if run.status.is_active() {
            self.fail(id, run.generation, reason, true, Vec::new());
        }
        true
    }

    /// Stop everything still running in one conversation. For the coordinator
    /// to call when the turn that started them is stopped.
    pub fn cancel_conversation(&self, conversation: &str, reason: &str) -> Vec<String> {
        let mut stopped = Vec::new();
        for run in self.for_conversation(conversation) {
            if run.status.is_active() {
                self.cancel(&run.id, false, reason);
                stopped.push(run.id);
            }
        }
        stopped
    }

    /// Put a settled run back to work on the same row.
    ///
    /// The row is reused rather than replaced because the whole point is
    /// continuity: the panel shows one helper that was asked three things in
    /// turn, with one history of steps and one bill.
    fn reopen(&self, id: &str, prompt: &str) -> Option<Run> {
        let run = {
            let mut table = self.table.lock();
            let run = table.runs.get_mut(id)?;
            run.status = Status::Queued;
            run.activity = "Starting".to_string();
            run.ended_at = None;
            run.result = None;
            run.error = None;
            run.collected = false;
            run.follow_ups += 1;
            // A new attempt on the same row. Anything still unwinding from the
            // last one now belongs to a generation nobody is listening to.
            run.generation += 1;
            let shown: String = prompt.chars().take(120).collect();
            log(run, "followup", format!("Asked to {shown}"));
            run.clone()
        };
        self.notify(&run.conversation);
        Some(run)
    }

    /// Send a settled run a new brief.
    ///
    /// Refuses, with a reason the model can act on, when the run is still
    /// going (`agent_send` is the tool for that) or when it was cancelled,
    /// which is final. Also the panel's follow-up button, which is why it is
    /// public.
    pub fn follow_up(&self, id: &str, prompt: &str) -> std::result::Result<(), String> {
        let Some(run) = self.get(id) else {
            return Err(format!("There is no run called {id}."));
        };
        if run.status.is_active() {
            return Err(format!(
                "{id} is still {}. Send it a message with `agent_send`, or `interrupt` it first.",
                run.status.as_str()
            ));
        }
        if !run.status.can_follow_up() {
            return Err(format!(
                "{id} was cancelled, and a cancelled run is over. Spawn a new one."
            ));
        }
        let relaunch = {
            let table = self.table.lock();
            table
                .relaunchers
                .get(id)
                .cloned()
                .filter(|_| table.transcripts.contains_key(id))
        };
        let Some(relaunch) = relaunch else {
            return Err(format!(
                "{id} finished before the app was last restarted, so its conversation is gone. \
                 Spawn a new run."
            ));
        };
        relaunch(prompt.to_string())
    }

    pub fn get(&self, id: &str) -> Option<Run> {
        self.table.lock().runs.get(id).cloned()
    }

    fn transcript_of(&self, id: &str) -> Option<Vec<Entry>> {
        self.table.lock().transcripts.get(id).cloned()
    }

    /// Every run in one conversation, oldest first.
    pub fn for_conversation(&self, conversation: &str) -> Vec<Run> {
        let mut out: Vec<Run> = self
            .table
            .lock()
            .runs
            .values()
            .filter(|run| run.conversation == conversation)
            .cloned()
            .collect();
        out.sort_by_key(|run| run.seq);
        out
    }

    /// This run's direct children.
    pub fn children_of(&self, id: &str) -> Vec<Run> {
        let mut out: Vec<Run> = self
            .table
            .lock()
            .runs
            .values()
            .filter(|run| run.parent_id.as_deref() == Some(id))
            .cloned()
            .collect();
        out.sort_by_key(|run| run.seq);
        out
    }

    /// The runs a given parent still has in flight. `None` is the
    /// conversation's own turn, which has no row - so this is also how a turn
    /// finds out whether it is about to end while its team is still working.
    pub fn active_children(&self, conversation: &str, parent: Option<&str>) -> Vec<Run> {
        self.for_conversation(conversation)
            .into_iter()
            .filter(|run| run.parent_id.as_deref() == parent && run.status.is_active())
            .collect()
    }

    fn mark_collected(&self, ids: &[String]) {
        let mut table = self.table.lock();
        for id in ids {
            if let Some(run) = table.runs.get_mut(id) {
                if !run.status.is_active() {
                    run.collected = true;
                }
            }
        }
    }

    /// One agent saying something to another.
    ///
    /// Asynchronous by construction: this drops a note in a mailbox and
    /// returns. The recipient reads it the next time it calls `team`. Two
    /// agents that block on each other's replies are a deadlock with a
    /// friendly name. `to: None` is the conversation's own turn.
    pub fn post(
        &self,
        conversation: &str,
        from: Option<&str>,
        from_name: &str,
        to: Option<&str>,
        text: &str,
    ) -> bool {
        let message = Message {
            at: now_ms(),
            from: from.map(str::to_string),
            from_name: from_name.to_string(),
            text: text.to_string(),
            read: false,
        };
        let target = {
            let mut table = self.table.lock();
            match to {
                None => {
                    table
                        .mailboxes
                        .entry(conversation.to_string())
                        .or_default()
                        .push(message);
                    conversation.to_string()
                }
                Some(id) => {
                    let Some(run) = table.runs.get_mut(id) else { return false };
                    let shown: String = message.text.chars().take(120).collect();
                    log(run, "message", format!("{}: {shown}", message.from_name));
                    run.inbox.push(message);
                    run.conversation.clone()
                }
            }
        };
        self.notify(&target);
        true
    }

    /// Take everything unread, marking it read on the way out.
    pub fn drain(&self, conversation: &str, run: Option<&str>) -> Vec<Message> {
        let mut table = self.table.lock();
        match run {
            None => table.mailboxes.remove(conversation).unwrap_or_default(),
            Some(id) => {
                let Some(run) = table.runs.get_mut(id) else { return Vec::new() };
                let mut out = Vec::new();
                for message in run.inbox.iter_mut().filter(|message| !message.read) {
                    message.read = true;
                    out.push(message.clone());
                }
                out
            }
        }
    }

    fn unread(&self, conversation: &str, run: Option<&str>) -> usize {
        let table = self.table.lock();
        match run {
            None => table.mailboxes.get(conversation).map_or(0, Vec::len),
            Some(id) => table
                .runs
                .get(id)
                .map_or(0, |run| run.inbox.iter().filter(|m| !m.read).count()),
        }
    }

    /// What the panel draws: the same keys the Electron snapshot carried, so
    /// `CrewPanel.jsx` reads it unchanged. `message` is always null because the
    /// folded transcript is not kept here; the panel reads null as "not open"
    /// and draws the summary instead.
    pub fn snapshot(&self, conversation: &str) -> Vec<Value> {
        let table = self.table.lock();
        let mut runs: Vec<&Run> = table
            .runs
            .values()
            .filter(|run| run.conversation == conversation)
            .collect();
        runs.sort_by_key(|run| run.seq);
        runs.into_iter()
            .map(|run| {
                let tail = |len: usize, keep: usize| len.saturating_sub(keep);
                json!({
                    "id": run.id,
                    "parentId": run.parent_id,
                    "depth": run.depth,
                    "agentId": run.agent_id,
                    "agentName": run.agent_name,
                    "description": run.description,
                    "prompt": run.prompt,
                    "model": run.model,
                    "status": run.status.as_str(),
                    "activity": run.activity,
                    "startedAt": run.started_at,
                    "endedAt": run.ended_at,
                    "steps": run.steps[tail(run.steps.len(), 40)..].iter().map(|step| json!({
                        "at": step.at, "tool": step.tool, "title": step.title,
                    })).collect::<Vec<_>>(),
                    "text": run.text,
                    "result": run.result,
                    "error": run.error,
                    "usage": run.usage.as_ref().and_then(|usage| serde_json::to_value(usage).ok()),
                    "collected": run.collected,
                    "restartedAs": Value::Null,
                    "restartOf": Value::Null,
                    "followUps": run.follow_ups,
                    "canFollowUp": run.status.can_follow_up()
                        && table.transcripts.contains_key(&run.id)
                        && table.relaunchers.contains_key(&run.id),
                    "inbox": run.inbox[tail(run.inbox.len(), 20)..].iter().map(message_json).collect::<Vec<_>>(),
                    "events": run.events[tail(run.events.len(), 80)..].iter().map(|event| json!({
                        "at": event.at, "kind": event.kind, "text": event.text,
                    })).collect::<Vec<_>>(),
                    "message": Value::Null,
                })
            })
            .collect()
    }

    /// Everything that happened in one conversation, in the order it happened,
    /// each entry carrying the run it belongs to.
    pub fn timeline(&self, conversation: &str) -> Vec<Value> {
        let mut out: Vec<(u64, Value)> = Vec::new();
        for run in self.for_conversation(conversation) {
            for event in &run.events {
                out.push((
                    event.at,
                    json!({
                        "at": event.at,
                        "kind": event.kind,
                        "text": event.text,
                        "runId": run.id,
                        "agentName": run.agent_name,
                    }),
                ));
            }
        }
        out.sort_by_key(|(at, _)| *at);
        out.into_iter().map(|(_, value)| value).collect()
    }

    /// Forget a conversation's runs. Called when a thread is deleted, not when a
    /// turn ends: the panel's whole purpose is being able to look at what a
    /// helper did after it finished.
    pub fn forget(&self, conversation: &str) {
        for run in self.for_conversation(conversation) {
            self.cancel(&run.id, false, "The conversation was closed");
            let mut table = self.table.lock();
            table.runs.remove(&run.id);
            table.stops.remove(&run.id);
            table.relaunchers.remove(&run.id);
            table.transcripts.remove(&run.id);
        }
        self.table.lock().mailboxes.remove(conversation);
        self.notify(conversation);
    }
}

fn message_json(message: &Message) -> Value {
    json!({
        "at": message.at,
        "from": message.from,
        "fromName": message.from_name,
        "text": message.text,
        "read": message.read,
    })
}

// ── starting a run ──────────────────────────────────────────────────────

/// [`Delegate`], plus the one thing a nested team needs.
///
/// A child that may spawn needs team tools of its own in its registry, and
/// `Delegate::registry` builds a registry from the app's state alone. The
/// default here ignores the extras, so an app that has not wired nesting
/// compiles and simply has children that cannot delegate further.
pub trait CrewDelegate: Delegate {
    fn registry_with_extras(
        &self,
        session: &str,
        agent: &Agent,
        withheld: &[&str],
        extra: Vec<Arc<dyn Tool>>,
    ) -> (Arc<dyn ToolRegistry>, Arc<dyn PermissionGate>) {
        let _ = extra;
        self.registry(session, agent, withheld)
    }
}

/// Who is calling the team tools.
///
/// `ToolContext` carries the conversation and nothing about the turn, and the
/// policy is entirely about the turn: which agent is speaking, how deep in the
/// tree it is, and which run it is - so a child's `agent_send` to "parent" can
/// find its parent.
#[derive(Clone)]
pub struct Caller {
    /// The conversation every run hangs off, as `thread/run`.
    pub conversation: String,
    /// The run this turn is, or `None` for the conversation's own turn.
    pub run_id: Option<String>,
    /// How many spawns deep. Zero for the conversation's own turn.
    pub depth: u32,
    /// Whose spawn policy applies. `None` reads as the defaults.
    pub agent: Option<Agent>,
    /// The model this turn is using, as the child's fallback.
    pub model: String,
    /// A digest of the parent's recent steps, for `share_context`. Without one
    /// the flag is accepted and ignored, and the output says so.
    pub recent_context: Option<Arc<dyn Fn(usize) -> String + Send + Sync>>,
}

impl std::fmt::Debug for Caller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Caller")
            .field("conversation", &self.conversation)
            .field("run_id", &self.run_id)
            .field("depth", &self.depth)
            .finish_non_exhaustive()
    }
}

/// Everything the seven tools share.
struct Core {
    delegate: Arc<dyn CrewDelegate>,
    runs: Arc<Runs>,
    caller: Caller,
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core").field("caller", &self.caller).finish_non_exhaustive()
    }
}

/// What a run is asked to do, kept so a follow-up can start it again the same way.
#[derive(Debug, Clone)]
struct Brief {
    description: String,
    /// The text the run reads first - the prompt, or the prompt behind a
    /// digest of the parent's context.
    text: String,
    requested_model: Option<String>,
}

/// A helper that exists only for this piece of work.
///
/// Built from a role and nothing else. It has no record on disk and no
/// permissions of its own, so it runs under the parent's rules and disappears
/// when the conversation does. Its spawn policy is inherited from the parent
/// rather than defaulted, because a temporary agent that could spawn when its
/// creator could not would make the whole policy meaningless.
fn temporary_agent(role: &str, parent: Option<&Agent>) -> Agent {
    let name = {
        let trimmed = role.trim();
        if trimmed.is_empty() { "Helper" } else { trimmed }.to_string()
    };
    Agent {
        id: String::new(),
        name: name.clone(),
        record: json!({
            "id": Value::Null,
            "name": name,
            "role": name,
            "temporary": true,
            "systemPrompt": "",
            "spawn": parent.and_then(|agent| agent.record.get("spawn").cloned()).unwrap_or(Value::Null),
        }),
    }
}

impl Core {
    fn record(&self) -> Value {
        self.caller
            .agent
            .as_ref()
            .map(|agent| agent.record.clone())
            .unwrap_or_else(|| json!({}))
    }

    fn name(&self) -> String {
        self.caller
            .agent
            .as_ref()
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| "This agent".to_string())
    }

    fn load(&self, peer: bool, charge_total: bool) -> Load {
        let all = self.runs.for_conversation(&self.caller.conversation);
        Load {
            depth: self.caller.depth,
            running: all
                .iter()
                .filter(|run| run.parent_id == self.caller.run_id && run.status.is_active())
                .count(),
            peer,
            live: all.iter().filter(|run| run.status.is_active()).count(),
            total: if charge_total { all.len() } else { 0 },
        }
    }

    /// A run in this conversation, or the sentence for the model when there is
    /// none by that id.
    fn own(&self, id: &str) -> std::result::Result<Run, String> {
        self.runs
            .get(id)
            .filter(|run| run.conversation == self.caller.conversation)
            .ok_or_else(|| {
                format!("No run in this conversation called {id}. Call `team` to see which exist.")
            })
    }

    /// Start a run and return without waiting for it.
    ///
    /// The loop is handed to a tokio task rather than awaited here, which is
    /// the one thing that makes any of this concurrent. `spawn` returns as soon
    /// as there is an id to return.
    fn launch(
        self: &Arc<Self>,
        agent: Agent,
        brief: Brief,
        reopen: Option<Run>,
        history: Vec<Entry>,
        root: &Path,
        call_id: Option<String>,
    ) -> std::result::Result<Run, String> {
        // Which model a child runs on: its own pin, then whatever was asked,
        // then the parent's.
        let reference = agent
            .text("model")
            .or_else(|| brief.requested_model.clone())
            .unwrap_or_else(|| self.caller.model.clone());
        // Resolved before the row exists: a provider nobody configured is a
        // sentence for the model now, not a red row in the panel.
        let (provider, model_id) = self.delegate.provider(&reference)?;

        let run = match reopen {
            Some(run) => run,
            None => self.runs.create(
                &self.caller.conversation,
                self.caller.run_id.as_deref(),
                self.caller.depth + 1,
                &agent,
                &brief.description,
                &brief.text,
                &reference,
            ),
        };
        let generation = run.generation;
        let id = run.id.clone();
        let session = format!("{}/{}", self.caller.conversation, id);

        // The child's own team tools, when its policy lets it nest. Decided
        // here by `may_delegate` rather than by refusing the call, the way
        // `task` withholds itself at depth.
        let child_depth = self.caller.depth + 1;
        let extras = if may_delegate(&agent.record, child_depth) {
            crew_tools(
                self.delegate.clone(),
                self.runs.clone(),
                Caller {
                    conversation: self.caller.conversation.clone(),
                    run_id: Some(id.clone()),
                    depth: child_depth,
                    agent: Some(agent.clone()),
                    model: reference.clone(),
                    recent_context: None,
                },
            )
        } else {
            Vec::new()
        };
        let (registry, gate) =
            self.delegate
                .registry_with_extras(&session, &agent, WITHHELD_IN_A_RUN, extras);
        // Kept alongside the loop so the prompt can name the tools this run
        // actually holds. Asking costs an await, which is why it happens on the
        // task rather than here: `launch` returns an id without blocking, and
        // that is the whole point of spawning.
        let announced = registry.clone();

        let (thinking_budget, reasoning_effort) = crate::agents::thinking_of(&agent.record);
        let subagent = Loop::new(
            provider,
            registry,
            gate,
            AgentConfig {
                model: model_id,
                thinking_budget,
                reasoning_effort,
                ..Default::default()
            },
        );
        let starting = history;

        // How to continue this run, kept after it has settled. The saved
        // transcript plus the new brief is the whole of what "with its context
        // intact" means; the row is the same row.
        let relaunch: Relauncher = {
            let core = self.clone();
            let agent = agent.clone();
            let brief = brief.clone();
            let id = id.clone();
            let root = root.to_path_buf();
            Arc::new(move |prompt: String| {
                let mut history = core.runs.transcript_of(&id).unwrap_or_default();
                let reopened = core
                    .runs
                    .reopen(&id, &prompt)
                    .ok_or_else(|| format!("There is no run called {id}."))?;
                history.push(Entry::user(prompt));
                core.launch(agent.clone(), brief.clone(), Some(reopened), history, &root, None)
                    .map(|_| ())
            })
        };

        let (stop, mut stopped) = oneshot::channel::<()>();
        self.runs.attach(&id, stop, relaunch);
        // The brief alone is a transcript worth keeping from the first moment:
        // a run interrupted before its loop has been scheduled must still be
        // something a follow-up can continue.
        self.runs.keep(&id, generation, starting.clone());
        self.runs.began(&id, generation);

        if let Some(call) = &call_id {
            self.delegate.progress(
                call,
                json!({ "runId": id, "agent": agent.name, "state": "running", "crew": true }),
            );
        }

        let runs = self.runs.clone();
        let delegate = self.delegate.clone();
        let agent_name = agent.name.clone();
        let task_id = id.clone();
        let prompt_agent = agent.clone();
        let prompt_model = reference.clone();
        let prompt_root = root.to_path_buf();
        let prompt_history = starting.clone();
        tokio::spawn(async move {
            // The run's own tool list, for its own prompt: a helper that lost
            // `spawn` to the depth limit must not be told it can delegate.
            let held: Vec<String> = announced
                .specs()
                .await
                .into_iter()
                .map(|spec| spec.name)
                .collect();
            let mut events = subagent.run(AgentTurn {
                session: SessionId::from_existing(session),
                system: delegate.system(&prompt_agent, &prompt_model, &held),
                history: prompt_history,
                root: prompt_root,
            });

            let mut said = String::new();
            loop {
                tokio::select! {
                    biased;

                    // Whoever stopped it has already written the status. What
                    // is left to do is keep what it had read, so a follow-up
                    // can pick the thread up. Dropping the stream is what
                    // cancels the provider request.
                    _ = &mut stopped => {
                        drop(events);
                        let mut transcript = starting;
                        if !said.trim().is_empty() {
                            transcript.push(Entry::assistant(said));
                        }
                        runs.keep(&task_id, generation, transcript);
                        break;
                    }

                    next = events.next() => {
                        let Some(event) = next else {
                            runs.fail(&task_id, generation, "The run ended without reporting how.", false, Vec::new());
                            break;
                        };
                        match event {
                            AgentEvent::Delta { text } => {
                                said.push_str(&text);
                                runs.observe_text(&task_id, generation, &text);
                            }
                            AgentEvent::ToolStarted { call, title } => {
                                runs.observe_tool(&task_id, generation, &call.name, title.as_deref());
                            }
                            AgentEvent::Done { stopped: reason, history, usage } => {
                                match reason {
                                    StopReason::Error { message } => {
                                        runs.fail(&task_id, generation, &message, false, history);
                                    }
                                    StopReason::Cancelled => {
                                        runs.fail(&task_id, generation, "The run was cancelled.", true, history);
                                    }
                                    StopReason::Complete | StopReason::MaxSteps | StopReason::Refused => {
                                        runs.finish(&task_id, generation, &said, usage, history);
                                    }
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
            // The card in the parent's transcript learns how it ended, so a
            // spawn card does not say "running" for ever.
            if let (Some(call), Some(run)) = (&call_id, runs.get(&task_id)) {
                if Runs::current(&run, generation) {
                    delegate.progress(
                        call,
                        json!({ "runId": task_id, "agent": agent_name, "state": run.status.as_str(), "crew": true }),
                    );
                }
            }
        });

        Ok(run)
    }
}

// ── the tools ───────────────────────────────────────────────────────────

/// The seven team tools, for one turn.
///
/// `delegate` starts the runs, `runs` is the app-wide table, and `caller` is
/// who this turn is. Spawned children are handed their own set with `caller`
/// pointing at them, when their policy allows nesting.
pub fn crew_tools(
    delegate: Arc<dyn CrewDelegate>,
    runs: Arc<Runs>,
    caller: Caller,
) -> Vec<Arc<dyn Tool>> {
    let core = Arc::new(Core {
        delegate,
        runs,
        caller,
    });
    vec![
        Arc::new(Spawn(core.clone())),
        Arc::new(Collect(core.clone())),
        Arc::new(Wait(core.clone())),
        Arc::new(Team(core.clone())),
        Arc::new(AgentSend(core.clone())),
        Arc::new(Interrupt(core.clone())),
        Arc::new(Followup(core)),
    ]
}

fn text_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn ids_arg(args: &Value) -> Vec<String> {
    args.get("run_ids")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::trim))
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn seconds_arg(args: &Value, default: u64) -> u64 {
    args.get("timeout_seconds")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| s as u64)
        .unwrap_or(default)
        .clamp(1, MAX_COLLECT_SECONDS)
}

/// A failure the model can act on: a result, not an error that ends its turn.
fn refuse(message: impl Into<String>) -> Result<ToolOutcome> {
    Ok(ToolOutcome::text(message))
}

fn task_permission(target: &str) -> PermissionRequest {
    PermissionRequest::new("task", target).with_always("*")
}

/// One run, rendered for the model rather than for the panel.
fn report(run: &Run) -> String {
    let head = format!(
        "<run id=\"{}\" agent=\"{}\" status=\"{}\" work=\"{}\">",
        run.id,
        run.agent_name,
        run.status.as_str(),
        run.description
    );
    let body = match run.status {
        Status::Done => {
            let said = run.result.as_deref().unwrap_or("").trim();
            if said.is_empty() {
                "(it finished without saying anything)".to_string()
            } else {
                said.to_string()
            }
        }
        Status::Failed => format!("It failed: {}", run.error.as_deref().unwrap_or("")),
        Status::Cancelled => format!("It was cancelled: {}", run.error.as_deref().unwrap_or("")),
        Status::Interrupted => format!("It was interrupted: {}", run.error.as_deref().unwrap_or("")),
        Status::Queued | Status::Running => format!(
            "Still {} - {} steps so far. Not finished.",
            run.activity.to_lowercase(),
            run.steps.len()
        ),
    };
    format!("{head}\n{body}\n</run>")
}

/// Block until one of `ids` settles or the deadline passes. Returns the ids
/// still going. Built on the change feed rather than polling: the panel already
/// needs every change announced, so a waiter is one more listener.
async fn until_settled(runs: &Runs, ids: &[String], all: bool, deadline: Instant) -> Vec<String> {
    let mut changes = runs.subscribe();
    loop {
        // Mark the current version seen before looking, so a change between the
        // look and the wait is not missed.
        changes.borrow_and_update();
        let active: Vec<String> = ids
            .iter()
            .filter(|id| runs.get(id).is_some_and(|run| run.status.is_active()))
            .cloned()
            .collect();
        let satisfied = if all { active.is_empty() } else { active.len() < ids.len() };
        if satisfied || Instant::now() >= deadline {
            return active;
        }
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return active;
                }
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {}
        }
    }
}

#[derive(Debug)]
struct Spawn(Arc<Core>);

#[async_trait]
impl Tool for Spawn {
    fn id(&self) -> &str {
        "spawn"
    }

    fn description(&self) -> &str {
        "Start another agent on a piece of work and carry on with your own.\n\
         \n\
         Unlike `task`, this does not wait. You get a run id back immediately, the work\n\
         happens alongside yours, and you call `collect` when you actually need the\n\
         answer. Spawn everything that can start now, then do the part only you can do.\n\
         \n\
         Use it when a job splits into pieces that do not depend on each other: research\n\
         while you read the code, tests written while the feature is being built, three\n\
         areas of a codebase surveyed at once. Use `task` instead when you need the\n\
         answer before you can do anything else - waiting on purpose is simpler than\n\
         spawning and immediately collecting.\n\
         \n\
         Name an existing agent with `agent`, or describe a temporary helper with `role`\n\
         and one will be made for this job and forgotten afterwards. A temporary helper\n\
         runs under your permissions and has no computer of its own.\n\
         \n\
         The run cannot see this conversation. The prompt has to stand alone: say what\n\
         you want, everything it needs to know, and what shape the answer should take.\n\
         Give it the context it needs and no more - a helper handed your whole\n\
         conversation costs more and reads worse. If it genuinely needs what you have\n\
         just read, pass `share_context: true` and a digest of your recent steps goes\n\
         with the brief - still write the brief as if it did not."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "A short description of the work, three to five words" },
                "prompt": { "type": "string", "description": "The full, self-contained brief. The run sees nothing else." },
                "agent": { "type": "string", "description": "An existing agent to run this. Omit to create a temporary helper." },
                "role": { "type": "string", "description": "What the temporary helper is for, e.g. 'Test writer'. Used when `agent` is omitted." },
                "model": { "type": "string", "description": "Override the model for this run - a cheaper one for simple work." },
                "isolation": { "type": "string", "description": "'worktree' to run the helper on a git worktree of its own - a copy of the repository on branch inertia/<name> - so it can edit and run commands without touching the checkout you and the person are in. The worktree is named from the description; a run that already has one of that name reuses it. Its work comes back as a branch." },
                "share_context": { "type": "boolean", "description": "Also hand the run a digest of your recent steps - what you asked, read and decided. Costs more; use it when the run would otherwise have to redo your reading." }
            },
            "required": ["description", "prompt"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = text_arg(args, "agent")
            .or_else(|| text_arg(args, "role"))
            .unwrap_or_else(|| "helper".to_string());
        task_permission(&target)
    }

    fn render(&self, args: &Value) -> Option<String> {
        let who = text_arg(args, "agent")
            .or_else(|| text_arg(args, "role"))
            .unwrap_or_else(|| "helper".to_string());
        Some(format!(
            "{who}: {}",
            text_arg(args, "description").unwrap_or_default()
        ))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let core = &self.0;
        let description = text_arg(&args, "description").unwrap_or_else(|| "work".to_string());
        let Some(prompt) = text_arg(&args, "prompt") else {
            return refuse("`prompt` is required: the run sees nothing else, so it needs a full brief.");
        };
        let wanted = text_arg(&args, "agent");

        if let Some(why) = spawn_refusal(&core.record(), &core.name(), core.load(wanted.is_some(), true)) {
            return refuse(why);
        }

        let agent = match &wanted {
            Some(wanted) => match crate::agents::resolve(core.delegate.layout(), wanted) {
                Ok(agent) if agent.is_paused() => {
                    return refuse(format!(
                        "{} is paused, so it will not take work. Ask the person to turn it back on, \
                         use a different agent, or omit `agent` and pass `role` to make a temporary helper.",
                        agent.name
                    ));
                }
                Ok(agent) => agent,
                Err(why) => {
                    return refuse(format!(
                        "{why} Or omit `agent` and pass `role` to make a temporary helper instead."
                    ));
                }
            },
            None => temporary_agent(
                &text_arg(&args, "role").unwrap_or_else(|| description.clone()),
                core.caller.agent.as_ref(),
            ),
        };

        // Said now, in words, rather than quietly running the helper in the
        // checkout it was told to stay out of.
        if text_arg(&args, "isolation").as_deref() == Some("worktree") {
            return refuse(
                "Worktree isolation is not available in this build, so the run was not started. \
                 Spawn it without `isolation` and tell it in the brief which files it may touch, \
                 or do the isolated part yourself.",
            );
        }

        // What the run starts from: its brief alone by default, preceded by a
        // digest of the parent's recent work when asked and when there is one.
        let share = args.get("share_context").and_then(Value::as_bool).unwrap_or(false);
        let shared = if share {
            core.caller.recent_context.as_ref().map(|recent| recent(SHARED_CONTEXT_CHARS))
        } else {
            None
        };
        let text = match &shared {
            Some(context) => [
                "What the agent that started you has been doing, most recent last. Read it for",
                "context; the brief that follows is your job.",
                "",
                "<parent_context>",
                context.as_str(),
                "</parent_context>",
                "",
                prompt.as_str(),
            ]
            .join("\n"),
            None => prompt.clone(),
        };

        let brief = Brief {
            description: description.clone(),
            text: text.clone(),
            requested_model: text_arg(&args, "model"),
        };
        let run = match core.launch(
            agent.clone(),
            brief,
            None,
            vec![Entry::user(text)],
            &ctx.root,
            Some(ctx.call_id.as_str().to_string()),
        ) {
            Ok(run) => run,
            Err(why) => return refuse(why),
        };

        let mut output = format!(
            "Started {} on \"{description}\" as {}.\n\
             \n\
             It is running now and you are not waiting for it. Do your own share of the\n\
             work, spawn anything else that can start in parallel, and call `collect`\n\
             with {} when you actually need the answer. `team` will tell you how\n\
             it is getting on without blocking.",
            agent.name, run.id, run.id
        );
        if share && shared.is_none() {
            output.push_str(
                "\n\nNo digest of your recent steps was available, so it has only the brief.",
            );
        }

        Ok(ToolOutcome {
            title: Some(format!("{}: {description}", agent.name)),
            output,
            metadata: Some(json!({
                "runId": run.id,
                "agent": agent.name,
                "peer": wanted.is_some(),
                "state": "running",
                "crew": true,
            })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct Collect(Arc<Core>);

#[async_trait]
impl Tool for Collect {
    fn id(&self) -> &str {
        "collect"
    }

    fn description(&self) -> &str {
        "Wait for runs you started with `spawn`, and read what they produced.\n\
         \n\
         Pass every id you are waiting on at once rather than one call each: they finish\n\
         in parallel and collecting them one at a time turns that back into a queue.\n\
         \n\
         A run that failed comes back as a failure, not as an error that ends your turn.\n\
         That is deliberate - one helper crashing is information, and you decide what to\n\
         do about it: spawn a replacement, do that part yourself, or carry on without it.\n\
         \n\
         If the wait runs out, you are told which are still going and you can collect\n\
         them again later. Nothing is lost by a timeout."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_ids": { "type": "array", "items": { "type": "string", "description": "A run id from `spawn`" }, "description": "The runs to wait for" },
                "timeout_seconds": { "type": "integer", "description": "How long to wait before reporting back unfinished" }
            },
            "required": ["run_ids"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        task_permission("")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("Collect {} run(s)", ids_arg(args).len()))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let core = &self.0;
        let ids = ids_arg(&args);
        if ids.is_empty() {
            return refuse("Pass at least one run id from `spawn`.");
        }

        let unknown: Vec<&String> = ids.iter().filter(|id| core.own(id).is_err()).collect();
        if !unknown.is_empty() {
            let known = core
                .runs
                .for_conversation(&core.caller.conversation)
                .iter()
                .map(|run| format!("{} ({})", run.id, run.description))
                .collect::<Vec<_>>()
                .join(", ");
            let names = unknown.iter().map(|id| id.as_str()).collect::<Vec<_>>().join(", ");
            return refuse(format!(
                "No run in this conversation called {names}. {}",
                if known.is_empty() {
                    "Nothing has been spawned yet.".to_string()
                } else {
                    format!("The runs here are: {known}.")
                }
            ));
        }

        let seconds = seconds_arg(&args, DEFAULT_COLLECT_SECONDS);
        // All of them at once. Waiting on the slowest is the point; waiting on
        // each in turn would be the sequential delegation this tool replaces.
        until_settled(&core.runs, &ids, true, Instant::now() + Duration::from_secs(seconds)).await;
        core.runs.mark_collected(&ids);

        let runs: Vec<Run> = ids.iter().filter_map(|id| core.runs.get(id)).collect();
        let waiting: Vec<&Run> = runs.iter().filter(|run| run.status.is_active()).collect();
        let failed = runs
            .iter()
            .filter(|run| matches!(run.status, Status::Failed | Status::Cancelled | Status::Interrupted))
            .count();

        let mut note = Vec::new();
        if !waiting.is_empty() {
            note.push(format!(
                "{} still running after {seconds}s ({}). They have not been stopped - collect \
                 them again, or carry on without them.",
                waiting.len(),
                waiting.iter().map(|run| run.id.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
        if failed > 0 {
            note.push(format!(
                "{failed} did not finish successfully. Decide what to do: spawn a replacement, \
                 do that part yourself, or continue without it. Do not simply report the failure \
                 as the result."
            ));
        }

        let mut parts: Vec<String> = runs.iter().map(report).collect();
        if !note.is_empty() {
            parts.push(String::new());
            parts.push(note.join("\n"));
        }

        Ok(ToolOutcome {
            title: Some(format!("Collected {}/{}", runs.len() - waiting.len(), runs.len())),
            output: parts.join("\n\n"),
            metadata: Some(json!({
                "crew": true,
                "collected": runs.iter().map(|run| json!({ "id": run.id, "status": run.status.as_str() })).collect::<Vec<_>>(),
            })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct Wait(Arc<Core>);

#[async_trait]
impl Tool for Wait {
    fn id(&self) -> &str {
        "wait"
    }

    fn description(&self) -> &str {
        "Wait until something happens: a run you started finishes, a message arrives\n\
         for you, or the person you are working for says something. Whichever is first.\n\
         \n\
         Use it when you have nothing useful to do until your team reports - and only\n\
         then. `collect` is for waiting on particular runs whose answers you need;\n\
         `wait` is for \"tell me when anything changes\". Runs that finished are\n\
         returned with their answers, marked as collected.\n\
         \n\
         Returns immediately, saying so, if nothing is running and nothing is waiting."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_ids": { "type": "array", "items": { "type": "string", "description": "A run id from `spawn`" }, "description": "Only wake for these runs. Omit to wake for any run you started." },
                "timeout_seconds": { "type": "integer", "description": format!("How long to wait before reporting back. Default {DEFAULT_WAIT_SECONDS}, most {MAX_COLLECT_SECONDS}.") }
            },
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        task_permission("")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let ids = ids_arg(args);
        Some(if ids.is_empty() {
            "Wait for the team".to_string()
        } else {
            format!("Wait for {} run(s)", ids.len())
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let core = &self.0;
        let seconds = seconds_arg(&args, DEFAULT_WAIT_SECONDS);
        let named = ids_arg(&args);
        let parent = core.caller.run_id.as_deref();

        core.delegate.progress(
            ctx.call_id.as_str(),
            json!({ "crew": true, "waiting": true }),
        );

        // What is watched: the named runs, or every run this turn started.
        let watched: Vec<String> = if named.is_empty() {
            core.runs
                .active_children(&core.caller.conversation, parent)
                .into_iter()
                .map(|run| run.id)
                .collect()
        } else {
            named
                .iter()
                .filter(|id| core.runs.get(id).is_some_and(|run| run.status.is_active()))
                .cloned()
                .collect()
        };

        let unread = || core.runs.unread(&core.caller.conversation, parent);

        let reason = if unread() > 0 {
            "message"
        } else if watched.is_empty() {
            "nothing"
        } else {
            let deadline = Instant::now() + Duration::from_secs(seconds);
            let mut changes = core.runs.subscribe();
            loop {
                changes.borrow_and_update();
                let any_settled = watched
                    .iter()
                    .any(|id| core.runs.get(id).is_some_and(|run| !run.status.is_active()));
                if any_settled {
                    break "settled";
                }
                if unread() > 0 {
                    break "message";
                }
                if Instant::now() >= deadline {
                    break "timeout";
                }
                tokio::select! {
                    changed = changes.changed() => {
                        if changed.is_err() {
                            break "timeout";
                        }
                    }
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {}
                }
            }
        };

        let settled: Vec<Run> = watched
            .iter()
            .filter_map(|id| core.runs.get(id))
            .filter(|run| !run.status.is_active())
            .collect();
        let still: Vec<String> = watched
            .iter()
            .filter(|id| core.runs.get(id).is_some_and(|run| run.status.is_active()))
            .cloned()
            .collect();
        core.runs.mark_collected(&watched);

        let described = match reason {
            "settled" => format!("{} run(s) finished.", settled.len()),
            "message" => format!(
                "You have {} new message(s). Call `team` to read them.",
                unread()
            ),
            "timeout" => format!(
                "Nothing happened in {seconds}s. {} run(s) still going: {}. They have not been stopped.",
                still.len(),
                still.join(", ")
            ),
            _ => "There was nothing to wait for: no run of yours is going and no message is \
                  waiting. Spawn something, or carry on."
                .to_string(),
        };

        let mut body = vec![described];
        body.extend(settled.iter().map(report));
        if settled
            .iter()
            .any(|run| matches!(run.status, Status::Failed | Status::Cancelled | Status::Interrupted))
        {
            body.push(
                "One of them did not finish successfully. Decide what to do about it rather than \
                 reporting the failure as your result."
                    .to_string(),
            );
        }

        Ok(ToolOutcome {
            title: Some(match reason {
                "settled" => format!("{} finished", settled.len()),
                "message" => "A message arrived".to_string(),
                "timeout" => format!("Nothing in {seconds}s"),
                _ => "Nothing to wait for".to_string(),
            }),
            output: body.join("\n\n"),
            metadata: Some(json!({
                "crew": true,
                "reason": reason,
                "settled": settled.iter().map(|run| run.id.clone()).collect::<Vec<_>>(),
            })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct Interrupt(Arc<Core>);

#[async_trait]
impl Tool for Interrupt {
    fn id(&self) -> &str {
        "interrupt"
    }

    fn description(&self) -> &str {
        "Stop a run in the middle of what it is doing, and keep it.\n\
         \n\
         Different from letting it finish and different from abandoning it: its current\n\
         turn ends now, its transcript up to that point is kept, and you can give it a\n\
         new brief with `followup` that it will read knowing everything it had already\n\
         found. Use it when the brief has changed under a run - the file it is editing\n\
         was just rewritten, the approach it is taking is wrong - and a message via\n\
         `agent_send` would arrive too late. Its own children keep running."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "The run to stop" },
                "reason": { "type": "string", "description": "Why, in a few words. Shown in the panel and to the run when you follow up." }
            },
            "required": ["run_id"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        task_permission("")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("Interrupt {}", text_arg(args, "run_id").unwrap_or_default()))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let core = &self.0;
        let Some(id) = text_arg(&args, "run_id") else {
            return refuse("`run_id` is required. Call `team` to see which runs exist.");
        };
        let run = match core.own(&id) {
            Ok(run) => run,
            Err(why) => return refuse(why),
        };
        let reason = match text_arg(&args, "reason") {
            Some(reason) => format!("Interrupted: {reason}"),
            None => "Interrupted".to_string(),
        };
        if let Err(why) = core.runs.interrupt(&id, &reason) {
            return refuse(why);
        }
        Ok(ToolOutcome {
            title: Some(format!("Interrupted {id}")),
            output: format!(
                "{} ({id}) was stopped after {} step(s). Its transcript is kept.\n\
                 Send it a new brief with `followup` when you know what it should do instead, or leave it.",
                run.agent_name,
                run.steps.len()
            ),
            metadata: Some(json!({ "crew": true, "runId": id, "state": "interrupted" })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct Followup(Arc<Core>);

#[async_trait]
impl Tool for Followup {
    fn id(&self) -> &str {
        "followup"
    }

    fn description(&self) -> &str {
        "Give a run that has finished, failed or been interrupted a new brief.\n\
         \n\
         It continues with everything it already read and did, which is the point:\n\
         a helper that surveyed a module can be asked a second question about it\n\
         without reading it again, and one that got the first attempt wrong can be\n\
         told exactly what to change. The run keeps its id; `collect` or `wait` for\n\
         it as before.\n\
         \n\
         Not for a run that is still going - `agent_send` reaches those - and not a\n\
         way to fix a run that went wrong because of its context; `spawn` a fresh one\n\
         for that."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "A run that has settled" },
                "prompt": { "type": "string", "description": "What to do next. It remembers its earlier brief and work; say what is new." }
            },
            "required": ["run_id", "prompt"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        task_permission(&text_arg(args, "run_id").unwrap_or_default())
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("Follow up with {}", text_arg(args, "run_id").unwrap_or_default()))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let core = &self.0;
        let Some(id) = text_arg(&args, "run_id") else {
            return refuse("`run_id` is required. Call `team` to see which runs exist.");
        };
        let Some(prompt) = text_arg(&args, "prompt") else {
            return refuse("`prompt` is required: say what the run should do next.");
        };
        let run = match core.own(&id) {
            Ok(run) => run,
            Err(why) => return refuse(why),
        };
        // A follow-up is not a new run, so the lifetime total is not charged.
        if let Some(why) = spawn_refusal(&core.record(), &core.name(), core.load(false, false)) {
            return refuse(why);
        }
        if let Err(why) = core.runs.follow_up(&id, &prompt) {
            return refuse(why);
        }
        core.delegate.progress(
            ctx.call_id.as_str(),
            json!({ "runId": id, "agent": run.agent_name, "state": "running", "crew": true }),
        );
        Ok(ToolOutcome {
            title: Some(format!("{}: follow-up", run.agent_name)),
            output: format!(
                "{} ({id}) is going again with your new brief and everything it had before.\n\
                 You are not waiting for it. `collect` or `wait` when you need the answer.",
                run.agent_name
            ),
            metadata: Some(json!({ "runId": id, "agent": run.agent_name, "state": "running", "crew": true })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct Team(Arc<Core>);

#[async_trait]
impl Tool for Team {
    fn id(&self) -> &str {
        "team"
    }

    fn description(&self) -> &str {
        "What everyone you started is doing right now, and anything they have said to you.\n\
         \n\
         Returns immediately and never waits. Use it to decide whether to keep working,\n\
         spawn more, or collect - and read your messages, because a run that hit a\n\
         question or found something you need to know sends it here rather than\n\
         interrupting you."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        task_permission("")
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some("Check on the team".to_string())
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let core = &self.0;
        let all = core.runs.for_conversation(&core.caller.conversation);
        let messages = core
            .runs
            .drain(&core.caller.conversation, core.caller.run_id.as_deref());

        if all.is_empty() && messages.is_empty() {
            return Ok(ToolOutcome::text("You have not started any runs.").with_title("Nobody is running"));
        }

        let lines: Vec<String> = all
            .iter()
            .map(|run| {
                let owner = run
                    .parent_id
                    .as_ref()
                    .map(|parent| format!(" (under {parent})"))
                    .unwrap_or_default();
                let state = if run.status.is_active() {
                    format!("{} - {}, {} steps", run.status.as_str(), run.activity, run.steps.len())
                } else {
                    run.status.as_str().to_string()
                };
                format!("{}{owner}  {}  {state}  \"{}\"", run.id, run.agent_name, run.description)
            })
            .collect();

        let mut output = if lines.is_empty() {
            "No runs.".to_string()
        } else {
            format!("Runs:\n{}", lines.join("\n"))
        };
        if !messages.is_empty() {
            output.push_str("\n\nMessages for you:\n");
            output.push_str(
                &messages
                    .iter()
                    .map(|message| format!("{}: {}", message.from_name, message.text))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }

        Ok(ToolOutcome {
            title: Some(format!(
                "{} running",
                all.iter().filter(|run| run.status.is_active()).count()
            )),
            output,
            metadata: Some(json!({ "crew": true, "runs": all.len() })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct AgentSend(Arc<Core>);

#[async_trait]
impl Tool for AgentSend {
    fn id(&self) -> &str {
        "agent_send"
    }

    fn description(&self) -> &str {
        "Say something to another run without waiting for a reply.\n\
         \n\
         For passing on what you have learned, correcting a brief that has gone stale,\n\
         or telling a run its work is no longer needed. It is delivered to that run's\n\
         inbox and read the next time it checks `team`, so it never interrupts a thought\n\
         in progress and never blocks you.\n\
         \n\
         Pass `to: \"parent\"` to send it upwards. Send is not ask - if you need an\n\
         answer before you can continue, do the work yourself or collect a run that\n\
         produces it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "to": { "type": "string", "description": "A run id, or \"parent\" for whoever started you" },
                "message": { "type": "string", "description": "What to tell them. Self-contained: they have not read your conversation." }
            },
            "required": ["to", "message"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        task_permission("")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("Message {}", text_arg(args, "to").unwrap_or_default()))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let core = &self.0;
        let Some(to) = text_arg(&args, "to") else {
            return refuse("`to` is required: a run id, or \"parent\".");
        };
        let Some(message) = text_arg(&args, "message") else {
            return refuse("`message` is required.");
        };

        // "parent" is whoever started this run; for the conversation's own
        // turn that is the person's mailbox, which `team` reads.
        let target: Option<String> = if to == "parent" {
            core.caller
                .run_id
                .as_deref()
                .and_then(|mine| core.runs.get(mine))
                .and_then(|run| run.parent_id)
        } else {
            Some(to.clone())
        };

        if let Some(id) = &target {
            if core.runs.get(id).is_none() {
                return refuse(format!(
                    "No run called {id}. Call `team` to see which runs exist, or use \"parent\"."
                ));
            }
        }

        let from_name = core
            .caller
            .agent
            .as_ref()
            .map(|agent| agent.name.clone())
            .unwrap_or_else(|| "an agent".to_string());
        let delivered = core.runs.post(
            &core.caller.conversation,
            core.caller.run_id.as_deref(),
            &from_name,
            target.as_deref(),
            &message,
        );
        if !delivered {
            return refuse(format!(
                "{} is no longer running, so it was not delivered.",
                target.as_deref().unwrap_or("parent")
            ));
        }

        Ok(ToolOutcome {
            title: Some(format!(
                "Sent to {}",
                target.as_deref().unwrap_or("the coordinator")
            )),
            output: "Delivered. They will read it the next time they check on the team - you are \
                     not waiting for a reply."
                .to_string(),
            metadata: Some(json!({ "crew": true, "to": target })),
            images: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::BoxStream;
    use inertia_core::id::ToolCallId;
    use inertia_core::provider::{ChatRequest, ModelInfo, Provider, StreamEvent};
    use std::path::PathBuf;
    use inertia_mock::{MockGate, MockProvider};
    use inertia_store::Layout;
    use inertia_tools::Registry;

    /// A provider whose first call never answers, and whose later calls reply
    /// from a script. The shape an interrupt is for.
    #[derive(Debug)]
    struct Stalling {
        calls: Mutex<u32>,
        then: MockProvider,
    }

    #[async_trait]
    impl Provider for Stalling {
        fn id(&self) -> &str {
            "stalling"
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }

        fn stream_chat(&self, request: ChatRequest) -> BoxStream<'static, StreamEvent> {
            let call = {
                let mut calls = self.calls.lock();
                *calls += 1;
                *calls
            };
            if call == 1 {
                Box::pin(futures::stream::pending())
            } else {
                self.then.stream_chat(request)
            }
        }
    }

    /// A delegate made of mocks, so the tests drive the shipping tools.
    /// One registry build, as the tests read it back.
    type Built = (String, Vec<String>, Vec<String>);

    #[derive(Debug)]
    struct Fake {
        layout: Layout,
        provider: Arc<dyn Provider>,
        /// Every `(session, withheld, extra tool ids)` a registry was built for.
        built: Mutex<Vec<Built>>,
        progress: Mutex<Vec<Value>>,
    }

    #[async_trait]
    impl Delegate for Fake {
        fn layout(&self) -> &Layout {
            &self.layout
        }

        fn provider(&self, reference: &str) -> std::result::Result<(Arc<dyn Provider>, String), String> {
            Ok((self.provider.clone(), format!("resolved:{reference}")))
        }

        fn registry(
            &self,
            session: &str,
            agent: &Agent,
            withheld: &[&str],
        ) -> (Arc<dyn ToolRegistry>, Arc<dyn PermissionGate>) {
            self.registry_with_extras(session, agent, withheld, Vec::new())
        }

        fn system(&self, agent: &Agent, model: &str, _tools: &[String]) -> String {
            format!("You are {}, on {model}.", agent.name)
        }

        fn progress(&self, _call_id: &str, metadata: Value) {
            self.progress.lock().push(metadata);
        }
    }

    impl CrewDelegate for Fake {
        fn registry_with_extras(
            &self,
            session: &str,
            _agent: &Agent,
            withheld: &[&str],
            extra: Vec<Arc<dyn Tool>>,
        ) -> (Arc<dyn ToolRegistry>, Arc<dyn PermissionGate>) {
            self.built.lock().push((
                session.to_string(),
                withheld.iter().map(|w| (*w).to_string()).collect(),
                extra.iter().map(|tool| tool.id().to_string()).collect(),
            ));
            let gate: Arc<dyn PermissionGate> = Arc::new(MockGate::allow_all());
            (Arc::new(Registry::new(gate.clone()).with_tools(extra)), gate)
        }
    }

    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        for record in [
            json!({ "id": "agent-scraper", "name": "Scraper", "role": "Collection" }),
            json!({ "id": "agent-asleep", "name": "Asleep", "status": "paused" }),
        ] {
            inertia_store::collections::put(&layout, inertia_store::Collection::Agents, record)
                .expect("written");
        }
        (dir, layout)
    }

    struct Bench {
        _dir: tempfile::TempDir,
        fake: Arc<Fake>,
        runs: Arc<Runs>,
        tools: HashMap<String, Arc<dyn Tool>>,
        root: PathBuf,
        /// The scripted provider, when the bench built one. Held as itself
        /// rather than recovered from the trait object: `Arc<dyn Provider>` has
        /// no downcast, and a test that wants to read back the requests a run
        /// actually sent needs the concrete thing.
        mock: Option<Arc<MockProvider>>,
    }

    impl Bench {
        fn tool(&self, id: &str) -> &Arc<dyn Tool> {
            self.tools.get(id).expect("a crew tool")
        }

        fn ctx(&self) -> ToolContext {
            ToolContext {
                root: self.root.clone(),
                session: SessionId::from_existing("thread-1"),
                call_id: ToolCallId::from_existing("tc_1"),
                permissions: Arc::new(MockGate::allow_all()),
            }
        }

        async fn call(&self, id: &str, args: Value) -> ToolOutcome {
            self.tool(id)
                .execute(args, &self.ctx())
                .await
                .expect("crew tools only fail on cancellation")
        }

        fn run_id(out: &ToolOutcome) -> String {
            out.metadata.as_ref().expect("metadata")["runId"]
                .as_str()
                .expect("a run id")
                .to_string()
        }
    }

    fn coordinator() -> Agent {
        Agent {
            id: "agent-lead".into(),
            name: "Nova".into(),
            record: json!({ "id": "agent-lead", "name": "Nova", "spawn": { "subagents": true, "agents": true } }),
        }
    }

    fn bench_with(provider: Arc<dyn Provider>, agent: Option<Agent>, depth: u32, run_id: Option<&str>) -> Bench {
        let (dir, layout) = workspace();
        let fake = Arc::new(Fake {
            layout,
            provider,
            built: Mutex::new(Vec::new()),
            progress: Mutex::new(Vec::new()),
        });
        let runs = Arc::new(Runs::new());
        let tools = crew_tools(
            fake.clone(),
            runs.clone(),
            Caller {
                conversation: "thread-1".into(),
                run_id: run_id.map(str::to_string),
                depth,
                agent,
                model: "anthropic/claude-sonnet-5".into(),
                recent_context: None,
            },
        )
        .into_iter()
        .map(|tool| (tool.id().to_string(), tool))
        .collect();
        Bench {
            root: dir.path().to_path_buf(),
            _dir: dir,
            fake,
            runs,
            tools,
            mock: None,
        }
    }

    fn bench(provider: MockProvider) -> Bench {
        let provider = Arc::new(provider);
        let mut bench = bench_with(provider.clone(), Some(coordinator()), 0, None);
        bench.mock = Some(provider);
        bench
    }

    fn brief(description: &str, role: &str) -> Value {
        json!({ "description": description, "prompt": format!("Please {description}."), "role": role })
    }

    #[tokio::test]
    async fn spawn_returns_before_the_work_is_done_and_collect_brings_both_answers_back() {
        let bench = bench(MockProvider::new().replying("routes: a, b").replying("tests: 12 pass"));

        let first = bench.call("spawn", brief("survey the routes", "Surveyor")).await;
        let second = bench.call("spawn", brief("run the tests", "Tester")).await;
        // This is the whole feature: the tool call is over, the session is not.
        assert!(first.output.contains("not waiting for it"), "{}", first.output);
        assert_eq!(first.title.as_deref(), Some("Surveyor: survey the routes"));
        let a = Bench::run_id(&first);
        let b = Bench::run_id(&second);
        assert!(a.starts_with("run-") && a != b);

        let team = bench.call("team", json!({})).await;
        assert_eq!(team.title.as_deref(), Some("2 running"));
        assert!(team.output.contains(&format!("{a}  Surveyor  running")), "{}", team.output);

        let collected = bench.call("collect", json!({ "run_ids": [a, b] })).await;
        assert_eq!(collected.title.as_deref(), Some("Collected 2/2"));
        assert!(collected.output.contains("routes: a, b"), "{}", collected.output);
        assert!(collected.output.contains("tests: 12 pass"), "{}", collected.output);
        assert!(collected.output.contains("status=\"done\""));

        let team = bench.call("team", json!({})).await;
        assert_eq!(team.title.as_deref(), Some("0 running"));
        assert!(team.output.contains("Tester  done"), "{}", team.output);

        // Both really ran, each with its own brief and the coordinator's model.
        let progress = bench.fake.progress.lock();
        assert!(progress.iter().any(|p| p["state"] == "running" && p["crew"] == true));
        assert!(progress.iter().any(|p| p["state"] == "done"));
        let built = bench.fake.built.lock();
        assert_eq!(built.len(), 2);
        assert!(built[0].0.starts_with("thread-1/run-"));
        assert!(built[0].1.contains(&"task".to_string()));
    }

    #[tokio::test]
    async fn wait_returns_when_something_finishes_and_says_so() {
        let bench = bench(MockProvider::new().replying("done"));
        let out = bench.call("spawn", brief("look around", "Scout")).await;
        let id = Bench::run_id(&out);

        let waited = bench.call("wait", json!({})).await;
        assert_eq!(waited.title.as_deref(), Some("1 finished"));
        assert_eq!(waited.metadata.as_ref().expect("metadata")["reason"], "settled");
        assert!(waited.output.contains("done"), "{}", waited.output);
        assert!(bench.runs.get(&id).expect("run").collected);

        // Nothing left to wait on: says so at once rather than sleeping.
        let idle = bench.call("wait", json!({})).await;
        assert_eq!(idle.title.as_deref(), Some("Nothing to wait for"));
    }

    #[tokio::test]
    async fn collect_reports_a_timeout_without_stopping_anything() {
        let provider = Arc::new(Stalling {
            calls: Mutex::new(0),
            then: MockProvider::new(),
        });
        let bench = bench_with(provider, Some(coordinator()), 0, None);
        let id = Bench::run_id(&bench.call("spawn", brief("think for ever", "Dreamer")).await);

        let out = bench.call("collect", json!({ "run_ids": [id], "timeout_seconds": 1 })).await;
        assert_eq!(out.title.as_deref(), Some("Collected 0/1"));
        assert!(out.output.contains("still running after 1s"), "{}", out.output);
        assert!(bench.runs.get(&id).expect("run").status.is_active());
    }

    #[tokio::test]
    async fn interrupt_stops_a_slow_run_and_a_followup_continues_it_with_its_context() {
        let provider = Arc::new(Stalling {
            calls: Mutex::new(0),
            then: MockProvider::new().replying("Picked up where I left off."),
        });
        let bench = bench_with(provider.clone(), Some(coordinator()), 0, None);
        let id = Bench::run_id(&bench.call("spawn", brief("read the module", "Reader")).await);

        // Let the loop reach the provider, so what is interrupted is a request in flight.
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert_eq!(*provider.calls.lock(), 1);

        let stopped = bench
            .call("interrupt", json!({ "run_id": id, "reason": "the file changed" }))
            .await;
        assert!(stopped.output.contains("Its transcript is kept"), "{}", stopped.output);
        let run = bench.runs.get(&id).expect("run");
        assert_eq!(run.status, Status::Interrupted);
        assert_eq!(run.error.as_deref(), Some("Interrupted: the file changed"));

        // Interrupting twice is a sentence, not a crash.
        let again = bench.call("interrupt", json!({ "run_id": id })).await;
        assert!(again.output.contains("is not running"), "{}", again.output);

        let followed = bench
            .call("followup", json!({ "run_id": id, "prompt": "Read the new version instead." }))
            .await;
        assert_eq!(followed.title.as_deref(), Some("Reader: follow-up"));
        assert_eq!(Bench::run_id(&followed), id, "a follow-up keeps the id");

        let collected = bench.call("collect", json!({ "run_ids": [id] })).await;
        assert!(collected.output.contains("Picked up where I left off."), "{}", collected.output);
        let run = bench.runs.get(&id).expect("run");
        assert_eq!(run.status, Status::Done);
        assert_eq!(run.follow_ups, 1);
        assert_eq!(run.generation, 2);

        // The second request carried the first brief: that is what "with its
        // context intact" means.
        let requests = provider.then.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].history.len() >= 2, "{:?}", requests[0].history);
        assert!(requests[0].history[0].text().contains("read the module"));
        assert!(requests[0].history.last().expect("the new brief").text().contains("new version"));
    }

    #[tokio::test]
    async fn a_followup_to_a_finished_run_reaches_the_same_session() {
        let bench = bench(
            MockProvider::new()
                .replying("The README says it is an agent app.")
                .replying("Yes, and it ships as a desktop binary."),
        );
        let id = Bench::run_id(&bench.call("spawn", brief("check the docs", "Docs")).await);
        bench.call("collect", json!({ "run_ids": [id] })).await;

        // Still running is the wrong moment; that is what `agent_send` is for.
        let running = bench_with(Arc::new(Stalling { calls: Mutex::new(0), then: MockProvider::new() }), Some(coordinator()), 0, None);
        let busy = Bench::run_id(&running.call("spawn", brief("keep going", "Busy")).await);
        let refused = running.call("followup", json!({ "run_id": busy, "prompt": "x" })).await;
        assert!(refused.output.contains("is still running"), "{}", refused.output);

        let followed = bench
            .call("followup", json!({ "run_id": id, "prompt": "Does it ship as a binary?" }))
            .await;
        assert_eq!(Bench::run_id(&followed), id);
        let collected = bench.call("collect", json!({ "run_ids": [id] })).await;
        assert!(collected.output.contains("desktop binary"), "{}", collected.output);

        let mock = bench.mock.clone().expect("the bench uses a MockProvider");
        let requests = mock.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].history.len() > requests[0].history.len());
    }

    #[tokio::test]
    async fn messages_land_in_an_inbox_and_team_reads_them() {
        let bench = bench_with(
            Arc::new(Stalling { calls: Mutex::new(0), then: MockProvider::new() }),
            Some(coordinator()),
            0,
            None,
        );
        let id = Bench::run_id(&bench.call("spawn", brief("watch the build", "Watcher")).await);

        let sent = bench
            .call("agent_send", json!({ "to": id, "message": "The build is green now." }))
            .await;
        assert_eq!(sent.title.as_deref(), Some(&format!("Sent to {id}")[..]));
        let inbox = bench.runs.drain("thread-1", Some(&id));
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].from_name, "Nova");
        assert!(bench.runs.drain("thread-1", Some(&id)).is_empty(), "read once");

        // From the top-level turn, "parent" is the person's own mailbox.
        bench.call("agent_send", json!({ "to": "parent", "message": "Note to self." })).await;
        let team = bench.call("team", json!({})).await;
        assert!(team.output.contains("Messages for you:\nNova: Note to self."), "{}", team.output);

        let nobody = bench.call("agent_send", json!({ "to": "run-nope", "message": "x" })).await;
        assert!(nobody.output.contains("No run called run-nope"), "{}", nobody.output);
    }

    #[tokio::test]
    async fn policy_refusals_are_sentences_the_model_can_act_on() {
        let forbidden = Agent {
            id: "a".into(),
            name: "Quiet".into(),
            record: json!({ "spawn": { "subagents": false } }),
        };
        let bench = bench_with(Arc::new(MockProvider::new()), Some(forbidden), 0, None);
        let out = bench.call("spawn", brief("anything", "Helper")).await;
        assert!(out.output.contains("not allowed to delegate"), "{}", out.output);
        assert!(bench.runs.for_conversation("thread-1").is_empty(), "nothing was started");

        // Nesting is off by default.
        let nested = bench_with(
            Arc::new(MockProvider::new()),
            Some(Agent { id: "a".into(), name: "Nova".into(), record: json!({}) }),
            1,
            Some("run-parent"),
        );
        let out = nested.call("spawn", brief("anything", "Helper")).await;
        assert!(out.output.contains("already a subagent"), "{}", out.output);

        // Helpers yes, peers no.
        let helpers_only = bench_with(
            Arc::new(MockProvider::new()),
            Some(Agent { id: "a".into(), name: "Nova".into(), record: json!({}) }),
            0,
            None,
        );
        let out = helpers_only
            .call("spawn", json!({ "description": "scrape", "prompt": "go", "agent": "Scraper" }))
            .await;
        assert!(out.output.contains("may spawn helpers but not full agents"), "{}", out.output);

        // A ceiling counts only this agent's own live children.
        let capped = bench_with(
            Arc::new(Stalling { calls: Mutex::new(0), then: MockProvider::new() }),
            Some(Agent {
                id: "a".into(),
                name: "Nova".into(),
                record: json!({ "spawn": { "subagents": true, "maxConcurrent": 1 } }),
            }),
            0,
            None,
        );
        capped.call("spawn", brief("first", "One")).await;
        let out = capped.call("spawn", brief("second", "Two")).await;
        assert!(out.output.contains("which is its limit"), "{}", out.output);
    }

    #[tokio::test]
    async fn a_named_agent_resolves_and_a_paused_or_unknown_one_is_reported() {
        let bench = bench(MockProvider::new().replying("scraped"));
        let out = bench
            .call("spawn", json!({ "description": "scrape", "prompt": "go", "agent": "Scraper" }))
            .await;
        assert_eq!(out.metadata.as_ref().expect("metadata")["peer"], true);
        assert_eq!(bench.runs.get(&Bench::run_id(&out)).expect("run").agent_id.as_deref(), Some("agent-scraper"));

        let asleep = bench
            .call("spawn", json!({ "description": "x", "prompt": "go", "agent": "Asleep" }))
            .await;
        assert!(asleep.output.contains("is paused"), "{}", asleep.output);

        let nobody = bench
            .call("spawn", json!({ "description": "x", "prompt": "go", "agent": "Designer" }))
            .await;
        assert!(nobody.output.contains("Scraper"), "{}", nobody.output);
        assert!(nobody.output.contains("pass `role`"), "{}", nobody.output);
    }

    #[tokio::test]
    async fn a_child_gets_team_tools_only_when_its_policy_lets_it_nest() {
        let bench = bench(MockProvider::new().replying("a").replying("b"));
        // A temporary helper inherits the coordinator's policy, which does not nest.
        bench.call("spawn", brief("plain", "Plain")).await;
        assert!(bench.fake.built.lock()[0].2.is_empty(), "a helper that may not nest holds no team tools");

        let nesting = bench_with(
            Arc::new(MockProvider::new().replying("a")),
            Some(Agent {
                id: "a".into(),
                name: "Lead".into(),
                record: json!({ "spawn": { "subagents": true, "recursive": true } }),
            }),
            0,
            None,
        );
        nesting.call("spawn", brief("nested", "Nester")).await;
        let extras = nesting.fake.built.lock()[0].2.clone();
        for tool in ["spawn", "collect", "wait", "team", "agent_send", "interrupt", "followup"] {
            assert!(extras.contains(&tool.to_string()), "{tool} missing from a nesting child");
        }
    }

    #[tokio::test]
    async fn a_worktree_request_is_refused_in_words_rather_than_silently_ignored() {
        let bench = bench(MockProvider::new());
        let out = bench
            .call("spawn", json!({ "description": "edit", "prompt": "go", "role": "Editor", "isolation": "worktree" }))
            .await;
        assert!(out.output.contains("not available"), "{}", out.output);
        assert!(bench.runs.for_conversation("thread-1").is_empty());
    }

    #[tokio::test]
    async fn the_snapshot_carries_the_keys_the_panel_reads() {
        let bench = bench(MockProvider::new().replying("hello"));
        let id = Bench::run_id(&bench.call("spawn", brief("say hi", "Greeter")).await);
        bench.call("collect", json!({ "run_ids": [id] })).await;

        let snapshot = bench.runs.snapshot("thread-1");
        assert_eq!(snapshot.len(), 1);
        let run = &snapshot[0];
        for key in [
            "id", "parentId", "depth", "agentId", "agentName", "description", "prompt", "model",
            "status", "activity", "startedAt", "endedAt", "steps", "text", "result", "error",
            "usage", "collected", "restartedAs", "restartOf", "followUps", "canFollowUp", "inbox",
            "events", "message",
        ] {
            assert!(run.get(key).is_some(), "snapshot is missing {key}");
        }
        assert_eq!(run["status"], "done");
        assert_eq!(run["result"], "hello");
        assert_eq!(run["collected"], true);
        assert_eq!(run["canFollowUp"], true);
        assert_eq!(run["usage"]["inputTokens"], 10);
        assert!(!bench.runs.timeline("thread-1").is_empty());

        bench.runs.forget("thread-1");
        assert!(bench.runs.snapshot("thread-1").is_empty());
    }

    #[test]
    fn may_delegate_asks_two_different_questions_at_two_depths() {
        let dev = json!({ "spawn": { "subagents": true, "agents": true, "recursive": true, "maxConcurrent": 10 } });
        assert!(may_delegate(&dev, 0));
        assert!(may_delegate(&dev, 2));
        assert_eq!(spawn_policy(&dev).max_concurrent, 10);

        // Defaults: one level deep, and no further.
        assert!(may_delegate(&json!({}), 0));
        assert!(!may_delegate(&json!({}), 1));
        assert!(!may_delegate(&json!({ "spawn": { "subagents": false, "recursive": true } }), 0));
        // Odd shapes read as the defaults rather than as a panic.
        assert_eq!(spawn_policy(&json!({ "spawn": "yes" })), SpawnPolicy::default());
        assert_eq!(spawn_policy(&json!({ "spawn": { "maxConcurrent": -3 } })).max_concurrent, 0);
    }

    #[test]
    fn the_conversation_is_whatever_comes_before_the_first_slash() {
        assert_eq!(Runs::conversation_of("thread-7/run-3a9f"), "thread-7");
        assert_eq!(Runs::conversation_of("thread-7/run-1/run-2"), "thread-7");
        assert_eq!(Runs::conversation_of(""), "unknown");
    }

    #[test]
    fn activity_is_coarse_on_purpose() {
        assert_eq!(activity_for("grep"), "Reading");
        assert_eq!(activity_for("shell_kill"), "Running commands");
        assert_eq!(activity_for("computer_act"), "Driving a computer");
        assert_eq!(activity_for("spawn"), "Coordinating");
        assert_eq!(activity_for("something_new"), "Working");
    }
}

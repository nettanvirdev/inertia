//! The panel's view of the team, over the command surface.
//!
//! [`crate::crew`] owns every run: it started the sessions, it holds the stop
//! handles, and it is the only thing that survives a reload of the window. This
//! module is the window's way of reading that table and pressing its buttons,
//! and it holds no state of its own - a renderer that decided locally that a
//! run had finished would be a second source of truth, and the first one it
//! disagreed with would be right.
//!
//! # Push rather than poll
//!
//! The interesting thing about a live subagent is that it changes, and a status
//! line that updates when somebody remembers to open a menu is not a live
//! status line. But the changes come thick: a run streaming a reply produces
//! one per chunk, and four runs streaming at once would put messages on the
//! bridge faster than the renderer can paint.
//!
//! So [`Runs`] already announces every change on a `tokio::sync::watch`
//! channel, and [`push_snapshots`] is the one listener that turns that into
//! window events. Anything that happens marks its conversation dirty; a short
//! timer flushes the dirty set into one snapshot per conversation, a few times
//! a second at most. The renderer redraws from the snapshot and never has to
//! reconcile a stream of deltas against the tree - which matters because the
//! tree changes shape mid-flight when a run spawns two more.
//!
//! # Whole snapshots, not deltas
//!
//! `snapshot` carries every key `CrewPanel.jsx` reads, and the panel replaces
//! its list wholesale on each one. That is deliberate: the shape of the tree is
//! knowledge the main process already has, and asking the window to rebuild it
//! from increments is asking it to be wrong occasionally.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::crew::Runs;
use crate::state::AppState;

/// The one channel the panel listens on. Spelled the way the Electron preload
/// spelled it, because `bridge/crew.js` subscribes to this exact name.
pub const CREW_EVENT: &str = "crew:event";

/// Fast enough to feel live, slow enough that four streaming runs cannot flood
/// the bridge.
const PUSH_INTERVAL_MS: u64 = 200;

/// Said from the panel rather than by an agent, so the event log can tell the
/// two apart when somebody asks later why a run stopped.
const STOPPED_FROM_PANEL: &str = "Stopped from the panel";
const INTERRUPTED_FROM_PANEL: &str = "Interrupted from the panel";

/// Where a crew snapshot goes.
///
/// A trait rather than an `AppHandle` so the push loop can be driven in a test
/// against something that simply collects what it was handed. Constructing the
/// real app state in a test links the window chrome into the test binary and
/// the whole thing fails to start, so every seam that would otherwise need it
/// takes one of these instead.
pub trait Emitter: Send + Sync + std::fmt::Debug {
    fn emit(&self, payload: Value);
}

/// The window, when there is one.
#[derive(Debug)]
struct Window(AppHandle);

impl Emitter for Window {
    fn emit(&self, payload: Value) {
        use tauri::Emitter as _;
        // A change that lands while the window is closing has nobody to tell.
        let _ = self.0.emit(CREW_EVENT, payload);
    }
}

/// Start pushing snapshots to the window. Called once, at setup.
pub fn start(app: &AppHandle) {
    let Some(state) = tauri::Manager::try_state::<AppState>(app) else {
        return;
    };
    let runs = state.runs.clone();
    let events: Arc<dyn Emitter> = Arc::new(Window(app.clone()));
    tauri::async_runtime::spawn(push_snapshots(
        runs,
        events,
        Duration::from_millis(PUSH_INTERVAL_MS),
    ));
}

/// Turn the table's change feed into window events, coalesced.
///
/// Runs until the table is dropped, which in the app is never. Split out from
/// [`start`] with its interval as an argument so a test can drive the real loop
/// with a table it populated itself.
pub async fn push_snapshots(runs: Arc<Runs>, events: Arc<dyn Emitter>, interval: Duration) {
    let mut changes = runs.subscribe();
    loop {
        // The first change of a burst. Anything that arrives while the timer
        // below is running joins the same flush.
        if changes.changed().await.is_err() {
            return;
        }
        let mut dirty = BTreeSet::new();
        // Scoped: the guard holds the watch lock, and holding it across an
        // await would stop every run in the process from reporting anything.
        {
            dirty.insert(changes.borrow_and_update().conversation.clone());
        }

        let flush_at = tokio::time::Instant::now() + interval;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(flush_at) => break,
                changed = changes.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    dirty.insert(changes.borrow_and_update().conversation.clone());
                }
            }
        }

        for conversation in dirty {
            // The channel's initial value carries no conversation. Pushing a
            // snapshot of "" would have the panel replace its runs with an
            // empty list the first time anything at all happened.
            if conversation.is_empty() {
                continue;
            }
            events.emit(json!({
                "conversationId": conversation,
                "runs": runs.snapshot(&conversation),
            }));
        }
    }
}

// ── what each command does, without Tauri in the way ────────────────────
//
// Every command below is a two-line wrapper over one of these, so the tests
// drive the same code the window does without building an `AppState`.

/// Every run under one conversation, in the shape `CrewPanel.jsx` reads.
pub fn snapshot(runs: &Runs, conversation: &str) -> Vec<Value> {
    runs.snapshot(conversation)
}

/// Stop one run from the panel.
///
/// The subtree goes with it by default, and that is the honest default: a
/// helper whose coordinator has been stopped is working for nobody. The
/// checkbox that says otherwise lives in the panel, not assumed here.
pub fn cancel(runs: &Runs, id: &str, descendants: bool) -> Value {
    json!({ "cancelled": runs.cancel(id, descendants, STOPPED_FROM_PANEL) })
}

/// Stop a run's turn and keep the run, so it can be given a new brief.
///
/// Its children are left running, unlike a cancel: somebody stopping a
/// coordinator in order to redirect it usually wants what it already handed
/// out.
pub fn interrupt(runs: &Runs, id: &str) -> Value {
    match runs.interrupt(id, INTERRUPTED_FROM_PANEL) {
        Ok(()) => json!({ "interrupted": true }),
        Err(reason) => json!({ "interrupted": false, "reason": reason }),
    }
}

/// A new brief for a settled run, from the panel rather than from an agent.
pub fn follow_up(runs: &Runs, id: &str, prompt: &str) -> Value {
    let text = prompt.trim();
    if text.is_empty() {
        return json!({ "started": false, "reason": "Say what it should do next." });
    }
    match runs.follow_up(id, text) {
        Ok(()) => json!({ "started": true }),
        Err(reason) => json!({ "started": false, "reason": reason }),
    }
}

/// Everything that happened in one conversation, merged and in order.
///
/// On demand rather than pushed with the snapshot: the live view wants the last
/// few events per run, this wants all of them across every run, and putting the
/// second on the wire several times a second to serve the first would be paying
/// for a history nobody has opened.
pub fn timeline(runs: &Runs, conversation: &str) -> Vec<Value> {
    runs.timeline(conversation)
}

/// A thread was deleted. Its runs have nowhere left to be shown.
pub fn forget(runs: &Runs, conversation: &str) -> Value {
    runs.forget(conversation);
    json!({ "forgotten": true })
}

/// Not yet: what the panel is told when a button has no machinery behind it.
///
/// A sentence rather than a bare `false`, because the three unbuilt controls
/// fail in three different ways and "nothing happened" is indistinguishable
/// from "the run id was wrong". The panel ignores the reply and leaves the row
/// as it was, which is the honest drawing of a request that did not happen.
fn not_yet(verb: &str, reason: &str) -> Value {
    json!({ verb: false, "reason": reason })
}

// ── the commands ────────────────────────────────────────────────────────

#[tauri::command]
pub fn crew_snapshot(state: State<'_, AppState>, conversation_id: Option<String>) -> Vec<Value> {
    snapshot(&state.runs, conversation_id.as_deref().unwrap_or_default())
}

#[tauri::command]
pub fn crew_cancel(
    state: State<'_, AppState>,
    run_id: Option<String>,
    options: Option<Value>,
) -> Value {
    // Absent means yes: the panel sends `{ descendants: true }` and only ever
    // sends `false` when somebody has deliberately unticked the box.
    let descendants = options
        .as_ref()
        .and_then(|options| options.get("descendants"))
        .and_then(Value::as_bool)
        != Some(false);
    cancel(
        &state.runs,
        run_id.as_deref().unwrap_or_default(),
        descendants,
    )
}

/// Hold a run at its next boundary.
///
/// There is nothing behind this yet. Pausing means a gate the agent loop waits
/// at between steps, and [`crate::crew::Runs`] has no such gate and no `paused`
/// status - so the truthful answer is that it did not happen, rather than a
/// row that claims to be paused while its session keeps spending money.
#[tauri::command]
pub fn crew_pause(_state: State<'_, AppState>, _run_id: Option<String>, _options: Option<Value>) -> Value {
    not_yet(
        "paused",
        "Pausing a run is not built yet. Stop it, or interrupt it and follow up when you are ready.",
    )
}

#[tauri::command]
pub fn crew_resume(_state: State<'_, AppState>, _run_id: Option<String>, _options: Option<Value>) -> Value {
    not_yet("resumed", "Pausing a run is not built yet, so there is nothing to resume.")
}

/// Run the same brief again as a new run, linked to the one it replaces.
///
/// Not built: the table has no way to start a second run carrying `restartOf`,
/// and its snapshot writes `restartedAs` and `restartOf` as null for exactly
/// that reason. Deliberately not faked with a follow-up, which is a different
/// thing - a follow-up reuses the row and keeps the transcript, so the helper
/// would be reading its own earlier attempt rather than starting again.
#[tauri::command]
pub fn crew_restart(_state: State<'_, AppState>, _run_id: Option<String>) -> Value {
    not_yet(
        "restarted",
        "Restarting a run is not built yet. Spawn it again, or follow up on the run you have.",
    )
}

#[tauri::command]
pub fn crew_interrupt(state: State<'_, AppState>, run_id: Option<String>) -> Value {
    interrupt(&state.runs, run_id.as_deref().unwrap_or_default())
}

#[tauri::command]
pub fn crew_followup(
    state: State<'_, AppState>,
    run_id: Option<String>,
    prompt: Option<String>,
) -> Value {
    follow_up(
        &state.runs,
        run_id.as_deref().unwrap_or_default(),
        prompt.as_deref().unwrap_or_default(),
    )
}

#[tauri::command]
pub fn crew_timeline(state: State<'_, AppState>, conversation_id: Option<String>) -> Vec<Value> {
    timeline(&state.runs, conversation_id.as_deref().unwrap_or_default())
}

/// Say which run the window is reading.
///
/// Answered and otherwise ignored. In Electron this is what made the pushed
/// snapshot carry that one run's folded transcript; here the snapshot writes
/// `message` as null for every run, so there is nothing for a watch to select
/// and remembering it would be state nothing reads. The panel is written for
/// that: with no `message` it draws the run's own text and result instead, and
/// it calls this on every open and close regardless of what comes back.
#[tauri::command]
pub fn crew_watch(
    _state: State<'_, AppState>,
    _conversation_id: Option<String>,
    run_id: Option<String>,
) -> Value {
    json!({ "watching": run_id })
}

#[tauri::command]
pub fn crew_forget(state: State<'_, AppState>, conversation_id: Option<String>) -> Value {
    forget(&state.runs, conversation_id.as_deref().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::path::PathBuf;

    use async_trait::async_trait;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::provider::Provider;
    use inertia_core::tool::{
        PermissionGate, Tool, ToolContext, ToolOutcome, ToolRegistry,
    };
    use inertia_mock::{MockGate, MockProvider};
    use inertia_store::Layout;
    use inertia_tools::Registry;
    use parking_lot::Mutex;

    use crate::agents::Agent;
    use crate::crew::{crew_tools, Caller, CrewDelegate};
    use crate::task_tool::Delegate;

    /// A delegate made of mocks, so the table is populated by the shipping
    /// `spawn` tool rather than by reaching into the table's private innards.
    /// A run put there any other way would not have a session, a stop handle or
    /// a relauncher, and half of what these commands do is press those.
    #[derive(Debug)]
    struct Fake {
        layout: Layout,
        provider: Arc<dyn Provider>,
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
            _session: &str,
            _agent: &Agent,
            _withheld: &[&str],
        ) -> (Arc<dyn ToolRegistry>, Arc<dyn PermissionGate>) {
            let gate: Arc<dyn PermissionGate> = Arc::new(MockGate::allow_all());
            (Arc::new(Registry::new(gate.clone())), gate)
        }

        fn system(&self, agent: &Agent, model: &str, _tools: &[String]) -> String {
            format!("You are {}, on {model}.", agent.name)
        }

        fn progress(&self, _call_id: &str, _metadata: Value) {}
    }

    impl CrewDelegate for Fake {}

    /// Collects what it was pushed, so the loop can be checked without a window.
    #[derive(Debug, Default)]
    struct Collected(Mutex<Vec<Value>>);

    impl Emitter for Collected {
        fn emit(&self, payload: Value) {
            self.0.lock().push(payload);
        }
    }

    struct Bench {
        _dir: tempfile::TempDir,
        runs: Arc<Runs>,
        tools: HashMap<String, Arc<dyn Tool>>,
        root: PathBuf,
    }

    impl Bench {
        async fn spawn(&self, description: &str, role: &str) -> String {
            let ctx = ToolContext {
                root: self.root.clone(),
                session: SessionId::from_existing("thread-1"),
                call_id: ToolCallId::from_existing("tc_1"),
                permissions: Arc::new(MockGate::allow_all()),
            };
            let out: ToolOutcome = self
                .tools
                .get("spawn")
                .expect("the spawn tool")
                .execute(
                    json!({
                        "description": description,
                        "prompt": format!("Please {description}."),
                        "role": role,
                    }),
                    &ctx,
                )
                .await
                .expect("spawning only fails on cancellation");
            out.metadata.as_ref().expect("metadata")["runId"]
                .as_str()
                .expect("a run id")
                .to_string()
        }

        /// Wait for a run to settle. Built on the same change feed the panel
        /// is pushed from, so a slow machine waits rather than flaking.
        async fn settled(&self, id: &str) {
            let mut changes = self.runs.subscribe();
            loop {
                changes.borrow_and_update();
                if self
                    .runs
                    .get(id)
                    .is_some_and(|run| !run.status.is_active())
                {
                    return;
                }
                changes.changed().await.expect("the table outlives the test");
            }
        }
    }

    fn bench(provider: MockProvider) -> Bench {
        let dir = tempfile::tempdir().expect("a temp dir");
        let fake = Arc::new(Fake {
            layout: Layout::new(dir.path()),
            provider: Arc::new(provider),
        });
        let runs = Arc::new(Runs::new());
        let tools = crew_tools(
            fake,
            runs.clone(),
            Caller {
                conversation: "thread-1".into(),
                run_id: None,
                depth: 0,
                agent: Some(Agent {
                    id: "agent-lead".into(),
                    name: "Nova".into(),
                    record: json!({
                        "id": "agent-lead",
                        "name": "Nova",
                        "spawn": { "subagents": true, "agents": true },
                    }),
                }),
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
            runs,
            tools,
        }
    }

    /// A provider that never answers, so a run stays running for the commands
    /// that only make sense against one that is.
    #[derive(Debug)]
    struct Stalling;

    #[async_trait]
    impl Provider for Stalling {
        fn id(&self) -> &str {
            "stalling"
        }

        async fn list_models(&self) -> inertia_core::Result<Vec<inertia_core::provider::ModelInfo>> {
            Ok(Vec::new())
        }

        fn stream_chat(
            &self,
            _request: inertia_core::provider::ChatRequest,
        ) -> futures::stream::BoxStream<'static, inertia_core::provider::StreamEvent> {
            Box::pin(futures::stream::pending())
        }
    }

    fn stalling_bench() -> Bench {
        let dir = tempfile::tempdir().expect("a temp dir");
        let fake = Arc::new(Fake {
            layout: Layout::new(dir.path()),
            provider: Arc::new(Stalling),
        });
        let runs = Arc::new(Runs::new());
        let tools = crew_tools(
            fake,
            runs.clone(),
            Caller {
                conversation: "thread-1".into(),
                run_id: None,
                depth: 0,
                agent: Some(Agent {
                    id: "agent-lead".into(),
                    name: "Nova".into(),
                    record: json!({ "id": "agent-lead", "name": "Nova" }),
                }),
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
            runs,
            tools,
        }
    }

    /// Every key `CrewPanel.jsx` and `CrewTimeline.jsx` read, on a real run.
    #[tokio::test]
    async fn the_snapshot_carries_what_the_panel_draws() {
        let bench = bench(MockProvider::new().replying("routes: a, b"));
        let id = bench.spawn("survey the routes", "Surveyor").await;
        bench.settled(&id).await;

        let rows = snapshot(&bench.runs, "thread-1");
        assert_eq!(rows.len(), 1);
        let run = &rows[0];
        for key in [
            "id", "parentId", "depth", "agentId", "agentName", "description", "prompt", "model",
            "status", "activity", "startedAt", "endedAt", "steps", "text", "result", "error",
            "usage", "collected", "restartedAs", "restartOf", "followUps", "canFollowUp", "inbox",
            "events", "message",
        ] {
            assert!(run.get(key).is_some(), "the snapshot is missing {key}");
        }
        assert_eq!(run["id"], id.as_str());
        assert_eq!(run["status"], "done");
        assert_eq!(run["agentName"], "Surveyor");
        assert_eq!(run["result"], "routes: a, b");

        // The timeline draws a badge per event kind and a bar per run, so both
        // the kind and the run it belongs to have to travel with each row.
        let events = timeline(&bench.runs, "thread-1");
        assert!(!events.is_empty());
        assert!(events
            .iter()
            .all(|event| event["runId"] == id.as_str() && event["at"].is_number()));
        assert!(events.iter().any(|event| event["kind"] == "spawned"));

        // A conversation nobody has run anything in is an empty panel, not a
        // failure.
        assert!(snapshot(&bench.runs, "thread-nothing").is_empty());
        assert!(timeline(&bench.runs, "thread-nothing").is_empty());
    }

    /// Stopping reaches the run the panel named, and only that one.
    #[tokio::test]
    async fn cancel_stops_the_named_run_and_leaves_the_others() {
        let bench = stalling_bench();
        let doomed = bench.spawn("read everything", "Reader").await;
        let spared = bench.spawn("write it up", "Writer").await;

        assert_eq!(cancel(&bench.runs, &doomed, true), json!({ "cancelled": true }));
        assert_eq!(
            bench.runs.get(&doomed).expect("the run").status.as_str(),
            "cancelled"
        );
        assert!(bench.runs.get(&spared).expect("the run").status.is_active());

        // A run id nobody recognises says so rather than pretending.
        assert_eq!(
            cancel(&bench.runs, "run-nope", true),
            json!({ "cancelled": false })
        );
    }

    /// Interrupt keeps the run; follow-up puts the same row back to work.
    #[tokio::test]
    async fn interrupt_keeps_the_run_and_a_follow_up_reopens_it() {
        let bench = stalling_bench();
        let id = bench.spawn("draft the plan", "Planner").await;

        assert_eq!(interrupt(&bench.runs, &id), json!({ "interrupted": true }));
        let run = bench.runs.get(&id).expect("the run");
        assert_eq!(run.status.as_str(), "interrupted");
        assert!(run.ended_at.is_some());

        // Interrupting the same run twice is a sentence, not a second ending.
        let again = interrupt(&bench.runs, &id);
        assert_eq!(again["interrupted"], false);
        assert!(
            again["reason"].as_str().expect("a reason").contains("not running"),
            "{again}"
        );
        assert!(interrupt(&bench.runs, "run-nope")["reason"]
            .as_str()
            .expect("a reason")
            .contains("no run"));

        // An empty brief is refused before anything is started.
        let empty = follow_up(&bench.runs, &id, "   ");
        assert_eq!(empty, json!({ "started": false, "reason": "Say what it should do next." }));

        assert_eq!(follow_up(&bench.runs, &id, "do the other half"), json!({ "started": true }));
        let reopened = bench.runs.get(&id).expect("the run");
        assert!(reopened.status.is_active(), "{}", reopened.status.as_str());
        assert_eq!(reopened.follow_ups, 1);

        // A cancelled run is over, and says so in words the panel can show.
        let gone = bench.spawn("something else", "Other").await;
        cancel(&bench.runs, &gone, true);
        let refused = follow_up(&bench.runs, &gone, "try again");
        assert_eq!(refused["started"], false);
        assert!(
            refused["reason"].as_str().expect("a reason").contains("cancelled"),
            "{refused}"
        );
    }

    /// The three controls with no machinery behind them say so.
    #[tokio::test]
    async fn pause_resume_and_restart_are_honest_about_not_existing() {
        for (key, value) in [
            ("paused", not_yet("paused", "Pausing a run is not built yet.")),
            ("resumed", not_yet("resumed", "Pausing a run is not built yet.")),
            ("restarted", not_yet("restarted", "Restarting a run is not built yet.")),
        ] {
            assert_eq!(value[key], false);
            assert!(value["reason"].as_str().is_some_and(|r| !r.is_empty()));
        }
    }

    /// Forgetting a conversation empties it, and the panel is told.
    #[tokio::test]
    async fn forget_clears_the_conversation() {
        let bench = bench(MockProvider::new().replying("done"));
        let id = bench.spawn("look around", "Scout").await;
        bench.settled(&id).await;

        assert_eq!(forget(&bench.runs, "thread-1"), json!({ "forgotten": true }));
        assert!(snapshot(&bench.runs, "thread-1").is_empty());
        assert!(timeline(&bench.runs, "thread-1").is_empty());
    }

    /// The change channel is what drives the event stream.
    ///
    /// The whole point of the watch channel is that nothing polls: a run that
    /// changes has to produce a `crew:event` carrying that conversation's
    /// snapshot, without anybody asking.
    #[tokio::test]
    async fn a_change_to_a_run_produces_an_event_carrying_the_snapshot() {
        let bench = stalling_bench();
        let events = Arc::new(Collected::default());
        let pusher = tokio::spawn(push_snapshots(
            bench.runs.clone(),
            events.clone(),
            Duration::from_millis(10),
        ));

        // Let the loop reach its first await, which is where it subscribes.
        // Spawning the run first would have every change it makes land before
        // there was a listener, and the test would be waiting for a push that
        // nothing was ever going to send.
        tokio::task::yield_now().await;

        let id = bench.spawn("watch the build", "Watcher").await;

        // Wait for the push rather than sleeping a fixed amount: the loop
        // coalesces, so the spawn and its "started" event may arrive as one.
        let pushed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(payload) = events.0.lock().last().cloned() {
                    if !payload["runs"].as_array().expect("a list").is_empty() {
                        return payload;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the change channel produced an event");

        assert_eq!(pushed["conversationId"], "thread-1");
        let rows = pushed["runs"].as_array().expect("a list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], id.as_str());
        assert_eq!(rows[0]["agentName"], "Watcher");

        // Coalesced, not one message per change: a run streaming a reply would
        // otherwise put a message on the bridge per chunk.
        let before = events.0.lock().len();
        assert!(before < 10, "{before} events for one spawn");

        // Stopping it is a change too, and it reaches the window.
        cancel(&bench.runs, &id, true);
        let stopped = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let last = events.0.lock().last().cloned();
                if let Some(payload) = last {
                    if payload["runs"][0]["status"] == "cancelled" {
                        return payload;
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the cancel was pushed");
        assert_eq!(stopped["runs"][0]["status"], "cancelled");

        pusher.abort();
    }
}

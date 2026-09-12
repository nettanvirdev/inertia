//! Noticing, after the fact, what a conversation was worth remembering.
//!
//! The hard part of this was never the extraction, it was finding a moment to
//! do it. A conversation does not end - it goes quiet, and might resume in ten
//! seconds or never. There is no event for it: a turn finishing happens every
//! turn, the only true end-of-thread signal fires when a thread is *deleted*,
//! which is the worst possible moment to mine one for durable facts, and
//! quitting aborts everything before anything could run.
//!
//! So: a timer per conversation, armed when a turn finishes and disarmed when
//! the next one starts. Whatever is still armed at shutdown is run then. That
//! covers a conversation left alone and a conversation left open at quitting
//! time, which between them is how conversations actually end. A hard kill loses
//! the pass, and nothing here pretends otherwise.
//!
//! Everything is best-effort by construction. It runs after the person has
//! stopped watching, so a failure has nowhere to be reported: it must never take
//! anything else down with it, never block a quit past its budget, and never
//! write a half-understood record.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::extract;
use crate::record;
use crate::store::Store;
use crate::Settings;

/// How long a conversation must sit still before it counts as over.
///
/// Long enough that a person thinking about their next message does not trigger
/// it, short enough that a session abandoned after lunch is captured while the
/// app is still running rather than being left to the quit path. Neither number
/// is precious; this one is a judgement, not a measurement.
pub const IDLE: Duration = Duration::from_secs(4 * 60);

/// The wait when the user asked to capture after every message.
///
/// Not zero. A person who sends two messages in a row has not ended a
/// conversation between them, and a pass that fired on the first would read half
/// a thought and write it down.
pub const AFTER_TURN: Duration = Duration::from_secs(20);

/// How long a pass may take before the app stops waiting for it at shutdown.
pub const DRAIN: Duration = Duration::from_secs(12);

/// The one thing this crate cannot do alone: ask a model what a conversation
/// was worth remembering.
///
/// A trait rather than a provider handle so the crate depends on no particular
/// model, and so a test can answer with a fixed reply and never touch a network.
#[async_trait::async_trait]
pub trait Complete: Send + Sync + std::fmt::Debug {
    async fn complete(&self, system: &str, text: &str) -> Result<String, String>;
}

/// What one conversation needs for its pass, held from the moment it goes quiet.
///
/// Everything the pass will need, because by the time it runs the turn that
/// produced it is long gone. No credential is among them: `Complete` resolves
/// whatever it needs when it is called, so a key is never sitting in a map for
/// the minutes a conversation stays quiet.
#[derive(Clone)]
pub struct Armed {
    pub store: Store,
    pub settings: Settings,
    pub transcript: String,
    pub agent_id: Option<String>,
    /// How many messages of this conversation this transcript covers.
    pub count: usize,
    pub model: Arc<dyn Complete>,
}

impl std::fmt::Debug for Armed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Armed")
            .field("count", &self.count)
            .field("agent_id", &self.agent_id)
            .finish_non_exhaustive()
    }
}

/// What a pass wrote.
#[derive(Debug, Clone, Default)]
pub struct Captured {
    pub stored: Vec<String>,
    pub note: bool,
}

#[derive(Default)]
struct State {
    armed: HashMap<String, (Armed, JoinHandle<()>)>,
    running: BTreeSet<String>,
    /// How much of each conversation has already been looked at.
    ///
    /// A pass is armed at the end of every turn, so a conversation that goes
    /// quiet, gets one more message, and goes quiet again would otherwise be
    /// read from the top a second time - a whole model call to rediscover what
    /// was already written down. The count is the number of messages covered.
    covered: HashMap<String, usize>,
}

/// The armed conversations, for the life of the app.
#[derive(Debug, Default)]
pub struct Capture {
    state: Mutex<State>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("armed", &self.armed.len())
            .field("running", &self.running.len())
            .finish()
    }
}

impl Capture {
    pub fn new() -> Self {
        Self::default()
    }

    /// A conversation has gone quiet, or is about to.
    pub fn arm(self: &Arc<Self>, thread_id: &str, entry: Armed) {
        if entry.settings.capture == "off" || !entry.settings.enabled {
            return;
        }
        let wait = if entry.settings.capture == "turn" { AFTER_TURN } else { IDLE };

        self.disarm(thread_id);

        let held = Arc::clone(self);
        let id = thread_id.to_string();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = held.run(&id).await;
        });
        self.state.lock().armed.insert(thread_id.into(), (entry, timer));
    }

    /// The conversation is alive again, so there is nothing to capture yet.
    pub fn disarm(&self, thread_id: &str) {
        if let Some((_, timer)) = self.state.lock().armed.remove(thread_id) {
            timer.abort();
        }
    }

    /// The conversation is gone, so what was learned from it is no longer
    /// pending.
    ///
    /// Separate from `disarm`, which runs at the start of every turn: standing a
    /// timer down is not the same as forgetting how much of a thread has been
    /// read, and conflating them would make every turn look like a fresh
    /// conversation.
    pub fn forget(&self, thread_id: &str) {
        self.disarm(thread_id);
        self.state.lock().covered.remove(thread_id);
    }

    pub fn covered_for(&self, thread_id: &str) -> usize {
        self.state.lock().covered.get(thread_id).copied().unwrap_or(0)
    }

    /// Run the pass for one conversation.
    pub async fn run(&self, thread_id: &str) -> Option<Captured> {
        let entry = {
            let mut state = self.state.lock();
            if state.running.contains(thread_id) {
                return None;
            }
            let (entry, timer) = state.armed.remove(thread_id)?;
            timer.abort();

            // Nothing has been said since the last pass, so there is nothing to
            // find. This is the common case at shutdown: the conversation was
            // already captured when it went quiet.
            if entry.count <= state.covered.get(thread_id).copied().unwrap_or(0) {
                return None;
            }
            state.running.insert(thread_id.into());
            entry
        };

        let outcome = self.pass(&entry).await;

        let mut state = self.state.lock();
        state.running.remove(thread_id);
        // Marked only once the pass has actually run. A failure leaves the
        // conversation uncovered so the next quiet moment tries again.
        if outcome.is_some() {
            state.covered.insert(thread_id.into(), entry.count);
        }
        outcome
    }

    async fn pass(&self, entry: &Armed) -> Option<Captured> {
        let Armed { store, settings, transcript, agent_id, model, .. } = entry;

        let existing = store.list();
        let folder = store.project().map(|p| p.to_string_lossy().to_string());
        let scoped: Vec<Value> = record::applicable(&existing, folder.as_deref())
            .into_iter()
            .cloned()
            .collect();
        let previous = store.note(&existing);
        let previous_note = previous
            .as_ref()
            .map(|row| record::text(row, "body"))
            .unwrap_or_default();

        let prompt = extract::build_prompt(
            transcript,
            &settings.instructions,
            &extract::known_of(&scoped),
            &previous_note,
            folder.as_deref(),
        );

        let reply = model.complete(&prompt.system, &prompt.text).await.ok()?;
        let parsed = extract::parse_reply(&reply)?;

        let ids: BTreeSet<String> = existing.iter().map(|row| record::text(row, "id")).collect();
        let wanted = extract::harvest(&parsed, folder.as_deref(), agent_id.as_deref(), &ids);

        let mut stored = Vec::new();
        for item in wanted {
            // A memory the user has to approve is written in the same place
            // with a flag, not held in a queue somewhere else: it is a real
            // record, and the Memory screen is where a person expects to find
            // it.
            let mut row = match item.replaces.as_deref().and_then(|id| find(&existing, id)) {
                // Superseding keeps what the old memory had accumulated - its
                // tags, its pin, the day it was written - and takes only the
                // new wording.
                Some(old) => record::merged(old, &item.record),
                None => item.record.clone(),
            };
            if settings.review {
                row["pending"] = Value::Bool(true);
            }

            let title = record::text(&row, "title");
            let written = match item.replaces.as_deref() {
                Some(id) => store.update(id, row),
                // One record failing is not a reason to lose the others.
                None => store.remember(&row, true),
            };
            if written.is_ok() {
                stored.push(title);
            }
        }

        let note = folder
            .as_deref()
            .and_then(|folder| extract::harvest_note(&parsed).map(|note| (folder, note)));
        let wrote_note = match note {
            Some((folder, note)) => {
                self.write_note(store, settings, previous.as_ref(), folder, &note).is_ok()
            }
            None => false,
        };

        Some(Captured { stored, note: wrote_note })
    }

    /// Replace the folder's handover note, never append to it.
    ///
    /// One record per project, rewritten each time. That is what keeps the cost
    /// flat: the eleventh session costs exactly what the second did, where a
    /// note that grew would make every conversation more expensive than the
    /// last.
    fn write_note(
        &self,
        store: &Store,
        settings: &Settings,
        previous: Option<&Value>,
        folder: &str,
        note: &str,
    ) -> Result<Value, String> {
        let mut record = serde_json::json!({
            "title": format!("Where we left off in {}", extract::basename(folder)),
            "body": note,
            "kind": "handover",
            "scope": "project",
            "folder": folder,
            "tags": ["handover"],
            "source": "learned",
            // Always carried into the prompt: it is the one memory whose whole
            // purpose is to be read at the start of the next conversation.
            "pinned": true,
        });
        if settings.review {
            record["pending"] = Value::Bool(true);
        }

        match previous {
            Some(old) => store.update(&record::text(old, "id"), record),
            None => store.remember(&record, false),
        }
    }

    /// Run everything still armed, now, and stop waiting after a budget.
    ///
    /// Called from the app's teardown. It works there precisely because it holds
    /// no turn's cancellation: by that point every turn has been cancelled, and
    /// a capture hung off one of those would be cancelled with it.
    pub async fn drain(self: &Arc<Self>) -> usize {
        let threads: Vec<String> = self.state.lock().armed.keys().cloned().collect();
        if threads.is_empty() {
            return 0;
        }

        let held = Arc::clone(self);
        let passes = async move {
            let mut captured = 0;
            for thread in threads {
                if held.run(&thread).await.is_some() {
                    captured += 1;
                }
            }
            captured
        };

        tokio::time::timeout(DRAIN, passes).await.unwrap_or(0)
    }

    /// For a workspace being closed, and for tests.
    pub fn reset(&self) {
        let mut state = self.state.lock();
        for (_, (_, timer)) in state.armed.drain() {
            timer.abort();
        }
        state.running.clear();
        state.covered.clear();
    }
}

fn find<'a>(records: &'a [Value], id: &str) -> Option<&'a Value> {
    records.iter().find(|row| record::text(row, "id") == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_store::Layout;

    #[derive(Debug)]
    struct Fixed(String);

    #[async_trait::async_trait]
    impl Complete for Fixed {
        async fn complete(&self, _system: &str, _text: &str) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    #[derive(Debug)]
    struct Refuses;

    #[async_trait::async_trait]
    impl Complete for Refuses {
        async fn complete(&self, _system: &str, _text: &str) -> Result<String, String> {
            Err("the model is unreachable".into())
        }
    }

    fn armed(dir: &tempfile::TempDir, reply: Arc<dyn Complete>, count: usize) -> Armed {
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).expect("a project dir");
        Armed {
            store: Store::new(Layout::new(dir.path().join("workspace")), Some(project)),
            settings: Settings::default(),
            transcript: "we decided to deploy to fly.io".into(),
            agent_id: None,
            count,
            model: reply,
        }
    }

    #[tokio::test]
    async fn a_pass_writes_what_the_model_found() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let reply = Arc::new(Fixed(
            r#"{"memories":[{"title":"Deploys go to fly.io","body":"never render","scope":"project"}],
                "note":"The project deploys to fly.io and the migration is unfinished."}"#
                .into(),
        ));
        let entry = armed(&dir, reply, 4);
        let store = entry.store.clone();

        let capture = Arc::new(Capture::new());
        capture.arm("t1", entry);
        let out = capture.run("t1").await.expect("a pass ran");

        assert_eq!(out.stored, ["Deploys go to fly.io"]);
        assert!(out.note);

        // The memory and the handover note, both filed against the project.
        let rows = store.list();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| record::text(row, "kind") == "handover"));
    }

    #[tokio::test]
    async fn nothing_new_since_the_last_pass_costs_no_model_call() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let reply = Arc::new(Fixed(r#"{"memories":[{"title":"One","body":"fact"}]}"#.into()));
        let capture = Arc::new(Capture::new());

        capture.arm("t1", armed(&dir, reply.clone(), 4));
        assert!(capture.run("t1").await.is_some());
        assert_eq!(capture.covered_for("t1"), 4);

        // Armed again with nothing said in between.
        capture.arm("t1", armed(&dir, reply, 4));
        assert!(capture.run("t1").await.is_none());
    }

    #[tokio::test]
    async fn a_failed_pass_leaves_the_conversation_uncovered() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let capture = Arc::new(Capture::new());
        capture.arm("t1", armed(&dir, Arc::new(Refuses), 4));

        assert!(capture.run("t1").await.is_none());
        // So the next quiet moment tries again rather than assuming it is done.
        assert_eq!(capture.covered_for("t1"), 0);
    }

    #[tokio::test]
    async fn a_reply_that_is_not_json_writes_nothing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let entry = armed(&dir, Arc::new(Fixed("I could not decide.".into())), 4);
        let store = entry.store.clone();

        let capture = Arc::new(Capture::new());
        capture.arm("t1", entry);
        assert!(capture.run("t1").await.is_none());
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn review_writes_the_memory_but_does_not_believe_it() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let reply = Arc::new(Fixed(r#"{"memories":[{"title":"One","body":"fact"}]}"#.into()));
        let mut entry = armed(&dir, reply, 4);
        entry.settings.review = true;
        let store = entry.store.clone();

        let capture = Arc::new(Capture::new());
        capture.arm("t1", entry);
        capture.run("t1").await.expect("a pass ran");

        let rows = store.list();
        assert_eq!(rows.len(), 1);
        assert!(record::is_pending(&rows[0]));
        // Written down, visible, and in force for nothing.
        assert!(store.in_scope(&rows).is_empty());
    }

    #[tokio::test]
    async fn capture_switched_off_arms_nothing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let reply = Arc::new(Fixed(r#"{"memories":[{"title":"One","body":"fact"}]}"#.into()));
        let mut entry = armed(&dir, reply, 4);
        entry.settings.capture = "off".into();

        let capture = Arc::new(Capture::new());
        capture.arm("t1", entry);
        assert!(capture.run("t1").await.is_none());
        assert_eq!(capture.drain().await, 0);
    }

    #[tokio::test]
    async fn a_conversation_that_came_back_to_life_is_not_captured() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let reply = Arc::new(Fixed(r#"{"memories":[{"title":"One","body":"fact"}]}"#.into()));
        let entry = armed(&dir, reply, 4);
        let store = entry.store.clone();

        let capture = Arc::new(Capture::new());
        capture.arm("t1", entry);
        capture.disarm("t1");

        assert!(capture.run("t1").await.is_none());
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn the_note_is_replaced_rather_than_added_to() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let capture = Arc::new(Capture::new());

        let note = |text: &str| {
            Arc::new(Fixed(format!(r#"{{"memories":[],"note":"{text}"}}"#))) as Arc<dyn Complete>
        };

        // Both passes write against the same folder, so the second finds what
        // the first left.
        let entry = armed(&dir, note("First: the project is a scraper and nothing is done."), 2);
        let store = entry.store.clone();
        capture.arm("t1", entry);
        capture.run("t1").await.expect("a pass ran");

        capture.arm(
            "t1",
            armed(&dir, note("Second: the scraper works and the export is left."), 5),
        );
        capture.run("t1").await.expect("a second pass ran");

        let rows = store.list();
        let notes: Vec<&Value> = rows
            .iter()
            .filter(|row| record::text(row, "kind") == "handover")
            .collect();
        assert_eq!(notes.len(), 1, "one note per project, rewritten");
        assert!(record::text(notes[0], "body").starts_with("Second:"));
    }
}

//! A turn, written down as it happens.
//!
//! The loop lives in this process, and that was supposed to be the point: a
//! reloaded window loses the view of a turn but not the turn. Without this it
//! was only half true. The events went to the window and nowhere else, so a
//! reload during a turn left the work running - tools executing, files being
//! written - with every byte of its output going to a listener that no longer
//! existed. The turn finished into the void.
//!
//! This is the other half. Every event the turn emits is appended to a record
//! held here and written to the workspace under `history/sessions`, which buys
//! three things:
//!
//! - **A reload can rejoin.** The record holds the whole event log, so a
//!   window that comes back asks what is still running and replays it into the
//!   message it left behind. Nothing is reconstructed or guessed; the same
//!   events are played through the same fold the live stream uses.
//! - **A crash keeps the tail.** The window saved on a debounce and could lose
//!   the last half-second of a conversation. This writes on a debounce too, but
//!   it also writes the moment a turn ends, and it is not competing with a
//!   window that may be gone.
//! - **There is a history.** What ran, with what tools, for how many tokens, is
//!   a record the folder should have had from the start.
//!
//! Deltas are merged into the event before them rather than appended one at a
//! time. A long reply is thousands of them, and a log that stores each
//! separately is a megabyte of JSON describing one page of text. Merging is
//! lossless for replay: the fold that consumes them concatenates anyway.
//!
//! The record is a `serde_json::Value`, like every record in this app, and the
//! shape is the Electron shell's byte for byte: the renderer's `rejoinEvents`
//! and history screen were written against that shape and read it back
//! unchanged. A port of `src/main/session/store.cjs`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use inertia_store::{Collection, Layout};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use tauri::State;

use crate::state::AppState;

/// Long enough to batch a burst of tool events, short enough that a crash
/// costs a moment rather than a turn.
pub const WRITE_DELAY: Duration = Duration::from_millis(500);

/// How many events one turn may hold before the log stops growing.
///
/// A runaway turn is bounded by the loop's step limit, but a single tool can
/// emit updates without limit - a build running for ten minutes streams its
/// whole output - and a record big enough to stall a JSON write is worse than a
/// truncated one. The tail is what matters for rejoining, so the head is what
/// gets dropped.
pub const MAX_EVENTS: usize = 4000;

/// How many finished turns keep their events in memory.
///
/// The events are what is big, and they are the part that is already on disk:
/// a turn is written in full the moment it ends. So a finished turn past the
/// cap keeps its metadata - the history list reads that, and it is a few
/// hundred bytes - and drops its events, with a flag saying where the full copy
/// is. `find` reads the file when it meets one of these. Live turns are never
/// touched, whatever the count: a running turn's events are how a reloaded
/// window rejoins it, and there is no copy of those guaranteed current.
pub const KEEP_FINISHED: usize = 20;

/// How many recorded screens keep their picture.
///
/// A screenshot is around six hundred kilobytes of base64 in the event that
/// carries it, and this record is rewritten in full every half-second while a
/// turn runs. The record exists so a reloaded window can rejoin a turn in
/// progress, and what that needs is the screen as it is now - not every screen
/// the turn has ever seen. The conversation keeps the full history; this keeps
/// the tail.
pub const KEEP_RECORDED_FRAMES: usize = 3;

/// Events that are worth replaying into a message. `tool-update` is not: it is
/// a progress line that the following `tool-end` supersedes entirely. `usage`
/// and `step` are folded into the record's own fields instead.
const REPLAYABLE: &[&str] = &[
    "delta",
    // What the turn changed on disk. A window that reloads after the turn
    // still gets the strip and the way back.
    "changes",
    "reasoning",
    "tool-start",
    "tool-end",
    // What the person said mid-turn, for the same reason their first message
    // is: a window that reloads between the steer and the answer would
    // otherwise show a reply that changes direction for no visible reason.
    "steer",
    // A rate limit waited out or a thread compacted took real time and the
    // person should still see why, on a window opened after it happened.
    "warning",
    // A hook that refused something or sent the agent back in is a turn that
    // changed direction, and a window that reloads afterwards should see why.
    "hook",
    // Whose turn it is in a room. A window that reloads mid-chain has to know
    // who is speaking before the next delta lands.
    "group",
    "error",
    "done",
];

/// What is known about a turn before its first event.
///
/// `layout` may be `None`: the app runs perfectly well before a workspace is
/// chosen, and a turn in that state is still worth holding in memory so a
/// reload can rejoin it. It simply is not written down.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub thread_id: Option<String>,
    pub message_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub model_ref: Option<String>,
    pub cwd: Option<String>,
    pub layout: Option<Layout>,
}

/// One turn as held in memory: the record the folder gets, plus the two
/// things the folder must not - where to write it, and whether the events have
/// already been shed to disk.
#[derive(Debug)]
struct Record {
    value: Map<String, Value>,
    layout: Option<Layout>,
    shed: bool,
    /// A write is already scheduled. One timer per turn, not one per event.
    write_pending: bool,
}

#[derive(Debug, Default)]
struct Inner {
    records: Mutex<HashMap<String, Record>>,
}

/// Turns this process has seen, live and just-finished.
///
/// Cheap to clone: every clone is the same recorder. The debounced write runs
/// on a spawned task that needs a handle to come back to, and a global is what
/// the one process-wide instance is anyway.
#[derive(Debug, Clone, Default)]
pub struct Recorder {
    inner: Arc<Inner>,
}

static GLOBAL: LazyLock<Recorder> = LazyLock::new(Recorder::new);

fn now_iso() -> String {
    // The renderer writes `new Date().toISOString()`, which is UTC with
    // milliseconds and a `Z`. Matching it keeps a folder's timestamps
    // comparable as plain strings no matter which side wrote them.
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn opt(value: &Option<String>) -> Value {
    value.clone().map(Value::String).unwrap_or(Value::Null)
}

fn text<'a>(record: &'a Map<String, Value>, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Whether an event carries an inline picture.
fn has_frame(event: &Value) -> bool {
    let metadata = event.get("metadata");
    metadata
        .and_then(|m| m.get("screenshot"))
        .is_some_and(|v| !v.is_null())
        || metadata
            .and_then(|m| m.get("closeup"))
            .is_some_and(|v| !v.is_null())
}

/// Drop the pictures from all but the last few events that carry one.
fn forget_old_frames(events: &mut [Value], keep: usize) {
    let mut remaining = keep;
    for event in events.iter_mut().rev() {
        if !has_frame(event) {
            continue;
        }
        if remaining > 0 {
            remaining -= 1;
            continue;
        }
        if let Some(Value::Object(metadata)) = event.get_mut("metadata") {
            metadata.remove("screenshot");
            metadata.remove("closeup");
        }
    }
}

/// The event without its routing fields, which the record already carries.
fn strip(event: &Value) -> Value {
    match event {
        Value::Object(map) => {
            let mut rest = map.clone();
            rest.remove("id");
            rest.remove("threadId");
            Value::Object(rest)
        }
        other => other.clone(),
    }
}

/// A turn that was running when this process last stopped.
///
/// Nothing is going to finish it - the loop it belonged to died with the
/// process - so a record that still says "running" on the way in is a lie the
/// same way a message stuck on "streaming" is. It is settled to `interrupted`,
/// which is a status the history screen can show honestly.
pub fn settle_stored(mut record: Value) -> Value {
    if record.get("status").and_then(Value::as_str) != Some("running") {
        return record;
    }
    if let Value::Object(map) = &mut record {
        map.insert("status".into(), json!("interrupted"));
        let ended = map
            .get("endedAt")
            .filter(|v| !v.is_null())
            .cloned()
            .or_else(|| map.get("startedAt").cloned())
            .unwrap_or(Value::Null);
        map.insert("endedAt".into(), ended);
    }
    record
}

/// Settle the records a workspace inherited.
///
/// Run once when a workspace opens. A turn that was running when the app was
/// killed has nobody left to finish it, and a folder full of records that still
/// claim to be running is the same lie as a message stuck on "streaming" - it
/// just takes longer to notice, because nothing renders it until someone opens
/// the history. Cheap: it only writes the ones that are wrong.
pub fn settle_workspace(layout: &Layout) -> usize {
    let mut settled = 0;
    for turn in inertia_store::collections::list(layout, Collection::Turns) {
        if turn.get("status").and_then(Value::as_str) != Some("running") {
            continue;
        }
        if inertia_store::collections::put(layout, Collection::Turns, settle_stored(turn)).is_ok() {
            settled += 1;
        }
    }
    settled
}

/// Every turn a conversation has run, oldest first, read off the folder.
///
/// `thread_id` of `None` is every turn in the workspace.
pub fn history_of(layout: &Layout, thread_id: Option<&str>) -> Vec<Value> {
    let mut turns: Vec<Value> = inertia_store::collections::list(layout, Collection::Turns)
        .into_iter()
        .filter(|turn| match thread_id {
            Some(wanted) => turn.get("threadId").and_then(Value::as_str) == Some(wanted),
            None => true,
        })
        .map(settle_stored)
        .collect();
    turns.sort_by(|a, b| {
        let started = |turn: &Value| {
            turn.get("startedAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        started(a).cmp(&started(b))
    });
    turns
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The one recorder the app's turns report to.
    pub fn global() -> Recorder {
        GLOBAL.clone()
    }

    /// Open a record for a turn that is starting.
    pub fn start(&self, turn_id: &str, meta: Meta) {
        let mut value = Map::new();
        value.insert("id".into(), json!(turn_id));
        value.insert("threadId".into(), opt(&meta.thread_id));
        value.insert("messageId".into(), opt(&meta.message_id));
        value.insert("agentId".into(), opt(&meta.agent_id));
        value.insert("agentName".into(), opt(&meta.agent_name));
        value.insert("provider".into(), opt(&meta.provider));
        value.insert("model".into(), opt(&meta.model));
        value.insert("modelRef".into(), opt(&meta.model_ref));
        value.insert("cwd".into(), opt(&meta.cwd));
        value.insert("startedAt".into(), json!(now_iso()));
        value.insert("endedAt".into(), Value::Null);
        value.insert("status".into(), json!("running"));
        value.insert("steps".into(), json!(0));
        value.insert("usage".into(), Value::Null);
        value.insert("events".into(), json!([]));
        value.insert("truncated".into(), json!(false));

        self.inner.records.lock().insert(
            turn_id.to_string(),
            Record {
                value,
                layout: meta.layout,
                shed: false,
                write_pending: false,
            },
        );
    }

    /// Append one event, exactly as it went to the window.
    ///
    /// Silently ignores a turn nobody opened: a subagent's progress lands here
    /// under the parent's id and is recorded, but an id this recorder has never
    /// seen is not a reason to invent a record.
    pub fn event(&self, turn_id: &str, event: &Value) {
        let Some(kind) = event.get("type").and_then(Value::as_str) else {
            return;
        };

        let (running, scheduled) = {
            let mut records = self.inner.records.lock();
            let Some(record) = records.get_mut(turn_id) else {
                return;
            };
            let value = &mut record.value;

            if kind == "step" {
                let next = event
                    .get("index")
                    .or_else(|| event.get("step"))
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| value.get("steps").and_then(Value::as_u64).unwrap_or(0) + 1);
                value.insert("steps".into(), json!(next));
            }
            if kind == "usage" {
                if let Some(usage) = event
                    .get("totals")
                    .or_else(|| event.get("usage"))
                    .filter(|v| !v.is_null())
                {
                    value.insert("usage".into(), usage.clone());
                }
            }

            if kind == "done" || kind == "error" {
                let status = if kind == "error" {
                    "error"
                } else if event.get("stopped").and_then(Value::as_str) == Some("cancelled") {
                    "cancelled"
                } else {
                    "complete"
                };
                value.insert("status".into(), json!(status));
                value.insert("endedAt".into(), json!(now_iso()));
                if let Some(usage) = event.get("usage").filter(|v| !v.is_null()) {
                    value.insert("usage".into(), usage.clone());
                }
            }

            if REPLAYABLE.contains(&kind) {
                let events = match value.get_mut("events") {
                    Some(Value::Array(events)) => events,
                    _ => {
                        value.insert("events".into(), json!([]));
                        match value.get_mut("events") {
                            Some(Value::Array(events)) => events,
                            _ => unreachable!("just inserted an array"),
                        }
                    }
                };

                // One reply is thousands of deltas. Merged, it is one.
                let merged = matches!(kind, "delta" | "reasoning")
                    && events
                        .last()
                        .and_then(|last| last.get("type"))
                        .and_then(Value::as_str)
                        == Some(kind);
                if merged {
                    if let Some(Value::Object(last)) = events.last_mut() {
                        let mut joined = last
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        joined.push_str(event.get("text").and_then(Value::as_str).unwrap_or_default());
                        last.insert("text".into(), Value::String(joined));
                    }
                } else {
                    events.push(strip(event));
                    if has_frame(event) {
                        forget_old_frames(events, KEEP_RECORDED_FRAMES);
                    }
                }

                if events.len() > MAX_EVENTS {
                    let excess = events.len() - MAX_EVENTS;
                    events.drain(0..excess);
                    value.insert("truncated".into(), json!(true));
                }
            }

            let running = text(value, "status") == "running";
            let scheduled = running && record.layout.is_some() && !record.write_pending;
            if scheduled {
                record.write_pending = true;
            }
            (running, scheduled)
        };

        if running {
            if scheduled {
                self.schedule(turn_id);
            }
        } else {
            self.flush(turn_id);
            self.shed_old_records();
        }
    }

    /// Settle a turn that ended without a `done` of its own, and write it.
    ///
    /// The loop's own `done` settles the record through `event`, so this is for
    /// the other endings: a turn stopped from outside, a task that panicked, a
    /// process shutting down. A record already settled is only flushed, so
    /// calling this after every turn is safe and is the intended use.
    pub fn finish(&self, turn_id: &str, status: &str) {
        {
            let mut records = self.inner.records.lock();
            let Some(record) = records.get_mut(turn_id) else {
                return;
            };
            if text(&record.value, "status") == "running" {
                record.value.insert("status".into(), json!(status));
                record.value.insert("endedAt".into(), json!(now_iso()));
            }
        }
        self.flush(turn_id);
        self.shed_old_records();
    }

    /// Mark every live turn as stopped, for teardown.
    pub fn end_all(&self, status: &str) {
        let live: Vec<String> = self
            .inner
            .records
            .lock()
            .iter()
            .filter(|(_, record)| text(&record.value, "status") == "running")
            .map(|(id, _)| id.clone())
            .collect();
        for id in live {
            self.finish(&id, status);
        }
    }

    /// Everything still running, for a window that has just come back.
    pub fn active(&self) -> Vec<Value> {
        self.inner
            .records
            .lock()
            .values()
            .filter(|record| text(&record.value, "status") == "running")
            .map(|record| {
                let value = &record.value;
                json!({
                    "id": value.get("id"),
                    "threadId": value.get("threadId"),
                    "messageId": value.get("messageId"),
                    "agentId": value.get("agentId"),
                    "model": value.get("model"),
                    "startedAt": value.get("startedAt"),
                    "events": value.get("events"),
                    // A log that has shed its head cannot rebuild the reply
                    // from nothing; the window keeps what it saved and takes
                    // only the live stream.
                    "truncated": value.get("truncated"),
                })
            })
            .collect()
    }

    /// The memory-only answer.
    ///
    /// `find` below is what the app asks, because it also reads the folder for
    /// a turn whose events have been shed. This is the half that answers
    /// without touching disk, which is what the tests here check against.
    #[cfg(test)]
    pub fn get(&self, turn_id: &str) -> Option<Value> {
        self.inner
            .records
            .lock()
            .get(turn_id)
            .map(|record| Value::Object(record.value.clone()))
    }

    /// One turn in full, from memory or from the folder.
    ///
    /// For the screen that asks about a turn from an hour ago, which may have
    /// had its events shed. The file is the durable copy, so reading it back is
    /// not a fallback so much as the other half of the design. `layout` is
    /// where to look when the record itself does not remember - a turn this
    /// process never saw, from before a restart.
    pub fn find(&self, turn_id: &str, layout: Option<&Layout>) -> Option<Value> {
        let (held, remembered) = {
            let records = self.inner.records.lock();
            match records.get(turn_id) {
                Some(record) if !record.shed => return Some(Value::Object(record.value.clone())),
                Some(record) => (Some(Value::Object(record.value.clone())), record.layout.clone()),
                None => (None, None),
            }
        };
        let layout = remembered.as_ref().or(layout)?;
        match inertia_store::collections::get(layout, Collection::Turns, turn_id) {
            Ok(Some(stored)) => Some(settle_stored(stored)),
            _ => held,
        }
    }

    /// Forget the finished turns of one conversation.
    ///
    /// Held in memory only until the conversation goes; the workspace copy is
    /// the durable one. A live turn is never dropped - forgetting the record of
    /// something still running would strand it.
    pub fn forget(&self, thread_id: &str) {
        self.inner.records.lock().retain(|_, record| {
            text(&record.value, "threadId") != thread_id || text(&record.value, "status") == "running"
        });
    }

    /// Write now, whatever the timer says.
    pub fn flush(&self, turn_id: &str) {
        let snapshot = {
            let mut records = self.inner.records.lock();
            let Some(record) = records.get_mut(turn_id) else {
                return;
            };
            record.write_pending = false;
            record
                .layout
                .clone()
                .map(|layout| (layout, Value::Object(record.value.clone())))
        };
        if let Some((layout, value)) = snapshot {
            // A workspace that cannot be written to is not a reason to kill a
            // turn that is otherwise going fine. The window still has the live
            // stream.
            // A tool's output is the agent's view of the machine, and a key it
            // was handed comes back in it - `printenv`, a failing curl with the
            // header echoed. Taken out here, the one place a trace reaches the
            // disk; the live stream to the window and the model keeps it.
            let value = inertia_store::secrets::scrubbed(&layout, value);
            if let Err(error) = inertia_store::collections::put(&layout, Collection::Turns, value) {
                tracing::warn!(turn = turn_id, %error, "a turn record could not be written");
            }
        }
    }

    /// Write in a moment, once, however many events arrive before then.
    ///
    /// Off the runtime when there is one; straight away when there is not. A
    /// caller with no runtime cannot be handed a timer, and a write per event
    /// is the right price for a caller that unusual.
    fn schedule(&self, turn_id: &str) {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let recorder = self.clone();
                let id = turn_id.to_string();
                handle.spawn(async move {
                    tokio::time::sleep(WRITE_DELAY).await;
                    recorder.flush(&id);
                });
            }
            Err(_) => self.flush(turn_id),
        }
    }

    fn shed_old_records(&self) {
        let mut records = self.inner.records.lock();
        let mut finished: Vec<(String, String)> = records
            .iter()
            .filter(|(_, record)| {
                text(&record.value, "status") != "running"
                    && !record.shed
                    && record
                        .value
                        .get("events")
                        .and_then(Value::as_array)
                        .is_some_and(|events| !events.is_empty())
            })
            .map(|(id, record)| (text(&record.value, "endedAt").to_string(), id.clone()))
            .collect();
        if finished.len() <= KEEP_FINISHED {
            return;
        }
        // Oldest first, by when they ended.
        finished.sort();
        for (_, id) in finished.iter().take(finished.len() - KEEP_FINISHED) {
            if let Some(record) = records.get_mut(id) {
                record.value.insert("events".into(), json!([]));
                record.shed = true;
            }
        }
    }
}

/* -- the commands -------------------------------------------------------- */

/// One turn's record, live or finished, for a window that did not start it.
///
/// `null` for a turn nobody has heard of, which is what the renderer's
/// `turnRecord` expects to spread into nothing.
#[tauri::command]
pub fn agent_record(state: State<'_, AppState>, id: String) -> Value {
    let workspace = state.workspace().ok();
    Recorder::global()
        .find(&id, workspace.as_ref().map(|w| &w.layout))
        .unwrap_or(Value::Null)
}

/// Every turn a conversation has run, oldest first, read off the folder.
///
/// An empty list rather than an error before a workspace is chosen: the
/// history screen is asking what has happened, and nothing has.
#[tauri::command]
pub fn agent_history(state: State<'_, AppState>, thread_id: Option<String>) -> Vec<Value> {
    let Ok(workspace) = state.workspace() else {
        return Vec::new();
    };
    history_of(
        &workspace.layout,
        thread_id.as_deref().map(str::trim).filter(|id| !id.is_empty()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        layout.scaffold().expect("scaffolded");
        (dir, layout)
    }

    fn meta(layout: Option<&Layout>) -> Meta {
        Meta {
            thread_id: Some("thr-1".into()),
            message_id: Some("msg-1".into()),
            model: Some("m".into()),
            layout: layout.cloned(),
            ..Default::default()
        }
    }

    fn stored(layout: &Layout, id: &str) -> Option<Value> {
        inertia_store::collections::get(layout, Collection::Turns, id)
            .expect("readable")
    }

    /// The shape a reopened window rejoins on.
    ///
    /// It matches a running turn to the message it is writing into and replays
    /// the events it missed, so an answer of bare ids is the same as no answer:
    /// the window skips every row and the reply spins forever over work that is
    /// still happening. That was the bug - `agent_active` answered with ids
    /// while this, written for the window, went uncalled.
    #[test]
    fn a_running_turn_is_reported_with_enough_to_rejoin_it() {
        let recorder = Recorder::new();
        recorder.start("turn-a", meta(None));
        recorder.event("turn-a", &json!({ "type": "delta", "text": "half a" }));
        recorder.start("turn-b", meta(None));
        recorder.finish("turn-b", "done");

        let live = recorder.active();
        assert_eq!(live.len(), 1, "a finished turn is not still running");
        let turn = &live[0];
        assert_eq!(turn["id"], json!("turn-a"));
        assert_eq!(turn["threadId"], json!("thr-1"));
        assert_eq!(turn["messageId"], json!("msg-1"));
        assert_eq!(turn["events"][0]["type"], json!("delta"));
    }

    /// A deleted conversation takes its turns with it, but never a live one:
    /// dropping the record of something still running strands it.
    #[test]
    fn forgetting_a_conversation_keeps_whatever_is_still_running() {
        let recorder = Recorder::new();
        recorder.start("turn-done", meta(None));
        recorder.finish("turn-done", "done");
        recorder.start("turn-live", meta(None));

        recorder.forget("thr-1");

        assert!(recorder.get("turn-done").is_none(), "a finished turn is dropped");
        assert!(recorder.get("turn-live").is_some(), "a running turn is kept");
    }

    #[test]
    fn a_stream_of_deltas_is_one_event() {
        let recorder = Recorder::new();
        recorder.start("turn-a", meta(None));
        for ch in "hello there".chars() {
            recorder.event("turn-a", &json!({ "type": "delta", "text": ch.to_string() }));
        }
        let active = recorder.active();
        assert_eq!(active.len(), 1);
        let events = active[0]["events"].as_array().expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], json!({ "type": "delta", "text": "hello there" }));
    }

    #[test]
    fn a_tool_call_and_its_result_stay_separate_and_progress_is_not_kept() {
        let recorder = Recorder::new();
        recorder.start("turn-b", meta(None));
        recorder.event("turn-b", &json!({ "type": "delta", "text": "Reading" }));
        recorder.event("turn-b", &json!({ "type": "tool-start", "callId": "c1", "name": "read", "id": "turn-b", "threadId": "thr-1" }));
        recorder.event("turn-b", &json!({ "type": "tool-update", "callId": "c1", "metadata": { "lines": 2 } }));
        recorder.event("turn-b", &json!({ "type": "tool-end", "callId": "c1", "ok": true, "output": "one" }));

        let turn = recorder.get("turn-b").expect("held");
        let kinds: Vec<&str> = turn["events"]
            .as_array()
            .expect("events")
            .iter()
            .map(|e| e["type"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(kinds, ["delta", "tool-start", "tool-end"]);
        // The routing fields are the record's, not the event's.
        assert!(turn["events"][1].get("id").is_none());
        assert!(turn["events"][1].get("threadId").is_none());
    }

    #[test]
    fn a_finished_turn_is_no_longer_active_and_remembers_what_it_cost() {
        let recorder = Recorder::new();
        recorder.start("turn-c", meta(None));
        recorder.event("turn-c", &json!({ "type": "step", "step": 1 }));
        recorder.event("turn-c", &json!({ "type": "usage", "usage": { "input": 10 } }));
        recorder.event("turn-c", &json!({ "type": "step", "step": 2 }));
        recorder.event("turn-c", &json!({ "type": "done", "stopped": "complete", "usage": { "input": 15, "output": 4 } }));

        assert!(recorder.active().is_empty());
        let turn = recorder.get("turn-c").expect("held");
        assert_eq!(turn["status"], "complete");
        assert_eq!(turn["steps"], 2);
        assert_eq!(turn["usage"], json!({ "input": 15, "output": 4 }));
        assert!(turn["endedAt"].is_string());
    }

    #[test]
    fn a_cancelled_turn_is_told_from_a_failed_one() {
        let recorder = Recorder::new();
        recorder.start("turn-d", Meta::default());
        recorder.event("turn-d", &json!({ "type": "done", "stopped": "cancelled" }));
        assert_eq!(recorder.get("turn-d").expect("held")["status"], "cancelled");

        recorder.start("turn-e", Meta::default());
        recorder.event("turn-e", &json!({ "type": "error", "message": "no" }));
        assert_eq!(recorder.get("turn-e").expect("held")["status"], "error");
    }

    /// Debounced while it runs, so a chatty tool does not write a file per
    /// line; written the moment it ends, so a crash costs a moment, not a turn.
    #[tokio::test]
    async fn it_writes_itself_into_the_workspace_and_again_when_it_ends() {
        let (_dir, layout) = workspace();
        let recorder = Recorder::new();
        recorder.start("turn-f", meta(Some(&layout)));
        recorder.event("turn-f", &json!({ "type": "delta", "text": "hi" }));

        assert!(stored(&layout, "turn-f").is_none(), "written before the debounce");
        tokio::time::sleep(WRITE_DELAY + Duration::from_millis(150)).await;
        let running = stored(&layout, "turn-f").expect("written after the debounce");
        assert_eq!(running["status"], "running");

        recorder.event("turn-f", &json!({ "type": "done", "stopped": "complete" }));
        let done = stored(&layout, "turn-f").expect("written on done");
        assert_eq!(done["status"], "complete");
        assert_eq!(done["events"][0]["text"], "hi");
        assert_eq!(done["threadId"], "thr-1");
        // Where to write it is not a fact about the turn.
        assert!(done.get("root").is_none());
        assert!(done.get("layout").is_none());
    }

    #[test]
    fn a_turn_with_no_workspace_is_still_held_in_memory() {
        let recorder = Recorder::new();
        recorder.start("turn-g", meta(None));
        recorder.event("turn-g", &json!({ "type": "delta", "text": "still here" }));
        assert_eq!(recorder.active()[0]["events"][0]["text"], "still here");
    }

    #[test]
    fn the_head_is_dropped_when_a_turn_will_not_stop_talking() {
        let recorder = Recorder::new();
        recorder.start("turn-h", Meta::default());
        for i in 0..(MAX_EVENTS + 50) {
            recorder.event("turn-h", &json!({ "type": "tool-start", "callId": format!("c{i}"), "name": "shell" }));
        }
        let turn = recorder.get("turn-h").expect("held");
        let events = turn["events"].as_array().expect("events");
        assert_eq!(events.len(), MAX_EVENTS);
        assert_eq!(turn["truncated"], true);
        // The tail is what a rejoining window needs.
        assert_eq!(events[events.len() - 1]["callId"], format!("c{}", MAX_EVENTS + 49));
    }

    #[test]
    fn a_stored_turn_that_was_running_when_the_process_died_reads_as_interrupted() {
        let settled = settle_stored(json!({ "id": "turn-i", "status": "running", "startedAt": "2026-09-01T00:00:00Z" }));
        assert_eq!(settled["status"], "interrupted");
        assert_eq!(settled["endedAt"], "2026-09-01T00:00:00Z");

        let untouched = settle_stored(json!({ "id": "turn-j", "status": "complete" }));
        assert_eq!(untouched["status"], "complete");
    }

    #[test]
    fn settling_a_workspace_rewrites_only_the_records_that_lie() {
        let (_dir, layout) = workspace();
        for record in [
            json!({ "id": "t-running", "threadId": "thr-1", "status": "running", "startedAt": "2026-01-01T00:00:00.000Z" }),
            json!({ "id": "t-done", "threadId": "thr-1", "status": "complete", "startedAt": "2026-01-02T00:00:00.000Z" }),
        ] {
            inertia_store::collections::put(&layout, Collection::Turns, record).expect("written");
        }
        assert_eq!(settle_workspace(&layout), 1);
        assert_eq!(stored(&layout, "t-running").expect("kept")["status"], "interrupted");
        assert_eq!(stored(&layout, "t-done").expect("kept")["status"], "complete");
    }

    #[test]
    fn forgetting_a_conversation_keeps_its_live_turn() {
        let recorder = Recorder::new();
        recorder.start("turn-k", meta(None));
        recorder.event("turn-k", &json!({ "type": "done" }));
        recorder.start("turn-l", meta(None));

        recorder.forget("thr-1");
        assert!(recorder.get("turn-k").is_none());
        // Forgetting something still running would strand work that is still
        // writing files.
        assert_eq!(recorder.get("turn-l").expect("held")["status"], "running");
    }

    #[test]
    fn every_live_turn_is_closed_on_teardown() {
        let (_dir, layout) = workspace();
        let recorder = Recorder::new();
        recorder.start("turn-m", meta(Some(&layout)));
        recorder.end_all("cancelled");
        assert_eq!(recorder.get("turn-m").expect("held")["status"], "cancelled");
        assert_eq!(stored(&layout, "turn-m").expect("written")["status"], "cancelled");
    }

    /// A turn stopped from outside owes the folder its ending too, and a turn
    /// that already ended must not be re-ended with a different word.
    #[test]
    fn finishing_settles_an_open_turn_and_leaves_a_settled_one_alone() {
        let (_dir, layout) = workspace();
        let recorder = Recorder::new();
        recorder.start("turn-n", meta(Some(&layout)));
        recorder.finish("turn-n", "cancelled");
        assert_eq!(stored(&layout, "turn-n").expect("written")["status"], "cancelled");

        recorder.start("turn-o", meta(Some(&layout)));
        recorder.event("turn-o", &json!({ "type": "error", "message": "boom" }));
        recorder.finish("turn-o", "cancelled");
        assert_eq!(stored(&layout, "turn-o").expect("written")["status"], "error");
    }

    #[test]
    fn the_newest_screens_keep_their_picture_and_the_rest_lose_it() {
        let frame = |n: usize| {
            json!({
                "type": "tool-end",
                "callId": format!("c{n}"),
                "name": "computer_act",
                "output": format!("did {n} action"),
                "metadata": { "frameId": format!("f{n}"), "screenshot": format!("PIC{n}") },
            })
        };
        let recorder = Recorder::new();
        recorder.start("t1", Meta::default());
        for n in 1..=6 {
            recorder.event("t1", &frame(n));
        }
        let turn = recorder.get("t1").expect("held");
        let events = turn["events"].as_array().expect("events");
        let kept: Vec<&str> = events
            .iter()
            .filter(|e| e["metadata"].get("screenshot").is_some())
            .map(|e| e["callId"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(kept, ["c4", "c5", "c6"]);
        // What the call did is still there; only the picture went.
        assert_eq!(events[0]["output"], "did 1 action");
        assert_eq!(events[0]["metadata"]["frameId"], "f1");
        assert!(!events[0].to_string().contains("PIC1"));
    }

    #[test]
    fn a_thread_history_is_read_off_the_folder_oldest_first_and_settled() {
        let (_dir, layout) = workspace();
        for record in [
            json!({ "id": "t2", "threadId": "thr-1", "status": "complete", "startedAt": "2026-01-02T00:00:00.000Z" }),
            json!({ "id": "t1", "threadId": "thr-1", "status": "running", "startedAt": "2026-01-01T00:00:00.000Z" }),
            json!({ "id": "t3", "threadId": "thr-2", "status": "complete", "startedAt": "2026-01-03T00:00:00.000Z" }),
        ] {
            inertia_store::collections::put(&layout, Collection::Turns, record).expect("written");
        }

        let history = history_of(&layout, Some("thr-1"));
        let ids: Vec<&str> = history.iter().map(|t| t["id"].as_str().unwrap_or_default()).collect();
        assert_eq!(ids, ["t1", "t2"]);
        assert_eq!(history[0]["status"], "interrupted");

        assert_eq!(history_of(&layout, None).len(), 3);
        assert!(history_of(&layout, Some("thr-9")).is_empty());
    }

    /// The path a routine's turn takes: the window never started it, asks for
    /// the record by id, and gets the same events the live stream carried.
    #[test]
    fn a_record_is_found_in_memory_while_live_and_on_disk_once_shed() {
        let (_dir, layout) = workspace();
        let recorder = Recorder::new();
        recorder.start("live", meta(Some(&layout)));
        recorder.event("live", &json!({ "type": "delta", "text": "working" }));
        let found = recorder.find("live", None).expect("found in memory");
        assert_eq!(found["events"][0]["text"], "working");
        assert_eq!(found["status"], "running");

        // A turn from before this process, only on disk.
        inertia_store::collections::put(
            &layout,
            Collection::Turns,
            json!({ "id": "old", "threadId": "thr-0", "status": "complete", "events": [{ "type": "delta", "text": "long ago" }] }),
        )
        .expect("written");
        assert!(recorder.find("old", None).is_none());
        let old = recorder.find("old", Some(&layout)).expect("read off the folder");
        assert_eq!(old["events"][0]["text"], "long ago");
        assert!(recorder.find("nobody", Some(&layout)).is_none());
    }

    #[test]
    fn finished_turns_past_the_cap_shed_their_events_and_are_read_back_from_disk() {
        let (_dir, layout) = workspace();
        let recorder = Recorder::new();
        for i in 0..(KEEP_FINISHED + 3) {
            let id = format!("turn-{i:03}");
            recorder.start(&id, meta(Some(&layout)));
            recorder.event(&id, &json!({ "type": "delta", "text": format!("reply {i}") }));
            recorder.event(&id, &json!({ "type": "done", "stopped": "complete" }));
        }

        // The oldest were shed from memory...
        let first = recorder.get("turn-000").expect("metadata is kept");
        assert_eq!(first["status"], "complete");
        assert!(first["events"].as_array().expect("events").is_empty());
        // ...and the newest were not.
        let last = recorder.get(&format!("turn-{:03}", KEEP_FINISHED + 2)).expect("held");
        assert_eq!(last["events"][0]["text"], format!("reply {}", KEEP_FINISHED + 2));

        // `find` goes to the folder for a shed one, where the events still are.
        let found = recorder.find("turn-000", None).expect("read off the folder");
        assert_eq!(found["events"][0]["text"], "reply 0");
    }
}

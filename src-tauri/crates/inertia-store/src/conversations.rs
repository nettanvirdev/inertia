//! Threads and messages.
//!
//! A thread is one conversation; its messages live in a single file holding
//! the whole array, rewritten in full on each save. Not an append log - the
//! array is small, is edited in place when a message streams, and is far
//! easier to read in a diff as one document.
//!
//! The subtle part is [`settle`]. A message written to disk mid-stream is
//! ambiguous: the turn may still be running, or the process may have been
//! killed. Getting that wrong leaves a conversation permanently showing "Stop"
//! with no turn behind it, which the user cannot clear by any means short of
//! editing JSON by hand.

use serde::{Deserialize, Serialize};

use crate::fsx;
use crate::layout::{Collection, Layout};

/// What a message that was interrupted says about its unfinished tool calls.
const INTERRUPTED: &str = "This call was interrupted and never finished.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Sent,
    Streaming,
    Error,
}

/// How a conversation is being run.
///
/// `Other` is not a placeholder - it is the reason this type is safe to have at
/// all. The window owns the list of modes and has shipped more of them than
/// this enum names (`group`, for one). A closed enum turned a perfectly good
/// thread carrying `"mode": "group"` into a parse failure, which the reader
/// then filed as a damaged record: the conversation vanished from the list and
/// a `.damaged` copy appeared beside it. Unknown modes are carried through
/// verbatim instead, so a thread written by a newer build survives an older one
/// reading it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Chat,
    Plan,
    /// The default: a conversation is for getting something done unless the
    /// user says otherwise.
    #[default]
    Autonomous,
    /// A mode this build does not know about, kept exactly as it was written.
    #[serde(untagged)]
    Other(String),
}

/// One conversation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Thread {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub title: String,
    pub mode: Mode,
    /// "ask" | "edits" | "auto". A per-conversation dial, independent of the
    /// permission rules.
    pub approval: String,
    pub pinned: bool,
    pub unread: u32,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// First ~120 characters of the last message, for the list view.
    pub preview: String,
    pub message_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub computer_attached: Option<String>,
    /// Everything the window keeps on a thread that Rust has no opinion about:
    /// the room roster of a group conversation, the context gauge, whether the
    /// thread is a draft.
    ///
    /// Carried rather than modelled. Without this, reading a thread here and
    /// writing it back deleted the roster of every group conversation - a save
    /// that silently destroyed work the user could see on screen a moment
    /// earlier. Anything this build does not understand is still the user's.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A piece of an assistant reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Part {
    Text {
        text: String,
    },
    Tool {
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        name: String,
        state: ToolState,
        /// The raw JSON argument string, kept so the next turn's request can
        /// be rebuilt exactly. Without it a resumed conversation cannot
        /// reproduce the `tool_use` block, and strict providers reject the
        /// orphaned result that follows.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        arguments: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        /// Whether the call succeeded. Absent means unknown, as elsewhere.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<bool>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolState {
    Running,
    Done,
    Failed,
}

/// One message, as stored.
///
/// The role is a string rather than an enum because `agent` and `assistant`
/// both appear in existing files and neither means anything different.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Message {
    pub id: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub content: String,
    pub created_at: String,
    pub status: Option<Status>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Signed reasoning blocks, replayed verbatim on the next request. Stored
    /// opaquely because only the provider that issued them can interpret them,
    /// and altering one invalidates its signature.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking: Vec<serde_json::Value>,
}

/// The file that holds one thread's messages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessageFile {
    pub id: String,
    pub thread_id: String,
    pub messages: Vec<Message>,
}

/// Resolves a message that was still streaming when it reached disk.
///
/// Called **only** for a message this process has never seen. A message the
/// running app still holds in memory is genuinely mid-flight and disk is
/// simply behind it, written on a debounce - settling that one would truncate
/// a reply that is still arriving.
///
/// Partial text is kept rather than discarded: the user watched it appear, and
/// deleting it would make the record disagree with what was on screen.
pub fn settle(message: &mut Message) {
    if message.status != Some(Status::Streaming) {
        return;
    }

    message.status = Some(Status::Sent);
    message.stopped = true;

    for part in &mut message.parts {
        if let Part::Tool { state, output, .. } = part {
            if *state == ToolState::Running {
                *state = ToolState::Failed;
                if output.is_none() {
                    *output = Some(INTERRUPTED.to_string());
                }
            }
        }
    }
}

/// Reads and writes conversations.
#[derive(Debug, Clone)]
pub struct Conversations {
    layout: Layout,
}

impl Conversations {
    pub fn new(layout: Layout) -> Self {
        Self { layout }
    }

    /// Every thread, newest first.
    ///
    /// A thread whose file is damaged is skipped rather than failing the whole
    /// listing - one unreadable conversation must not hide the other two
    /// hundred.
    pub fn list_threads(&self) -> Vec<Thread> {
        let dir = self.layout.collection_dir(Collection::Threads);
        let mut threads: Vec<Thread> = fsx::list_dir(
            &dir,
            fsx::ListOptions {
                ext: Some("json"),
                ..Default::default()
            },
        )
        .into_iter()
        .filter_map(|name| {
            let (thread, outcome) = fsx::read_json::<Thread>(&dir.join(&name));
            match outcome {
                fsx::ReadOutcome::Loaded => Some(thread),
                fsx::ReadOutcome::Damaged { kept_at } => {
                    tracing::warn!(
                        file = %name,
                        kept_at = %kept_at.display(),
                        "a conversation file could not be read; the original was preserved"
                    );
                    None
                }
                fsx::ReadOutcome::Absent => None,
            }
        })
        .collect();

        threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        threads
    }

    pub fn read_thread(&self, id: &str) -> Option<Thread> {
        let path = self.layout.record(Collection::Threads, id);
        match fsx::read_json::<Thread>(&path) {
            (thread, fsx::ReadOutcome::Loaded) => Some(thread),
            _ => None,
        }
    }

    pub fn write_thread(&self, thread: &Thread) -> fsx::Result<()> {
        let path = self.layout.record(Collection::Threads, &thread.id);
        fsx::write_json(&path, thread)
    }

    /// Reads a thread's messages, settling anything left mid-stream.
    ///
    /// `live` names messages the running app still holds; those are left alone.
    /// Pass an empty slice when loading cold, which is the case that matters -
    /// a crash or a force-quit must never leave a conversation stuck.
    pub fn read_messages(&self, thread_id: &str, live: &[String]) -> Vec<Message> {
        let path = self.layout.record(Collection::Messages, thread_id);
        let (file, outcome) = fsx::read_json::<MessageFile>(&path);

        if let fsx::ReadOutcome::Damaged { kept_at } = &outcome {
            tracing::warn!(
                thread = thread_id,
                kept_at = %kept_at.display(),
                "messages could not be read; the original was preserved"
            );
        }

        let mut messages = file.messages;
        for message in &mut messages {
            if !live.contains(&message.id) {
                settle(message);
            }
        }
        messages
    }

    pub fn write_messages(&self, thread_id: &str, messages: &[Message]) -> fsx::Result<()> {
        let path = self.layout.record(Collection::Messages, thread_id);
        fsx::write_json(
            &path,
            &MessageFile {
                id: thread_id.to_string(),
                thread_id: thread_id.to_string(),
                messages: messages.to_vec(),
            },
        )
    }

    /// Removes a thread and its messages.
    pub fn delete_thread(&self, id: &str) -> fsx::Result<()> {
        fsx::remove(&self.layout.record(Collection::Threads, id))?;
        fsx::remove(&self.layout.record(Collection::Messages, id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Conversations) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        layout.scaffold().unwrap();
        (dir, Conversations::new(layout))
    }

    fn streaming_message(id: &str) -> Message {
        Message {
            id: id.into(),
            role: "agent".into(),
            content: "half a th".into(),
            status: Some(Status::Streaming),
            parts: vec![
                Part::Text {
                    text: "half a th".into(),
                },
                Part::Tool {
                    call_id: Some("tc_1".into()),
                    name: "read".into(),
                    arguments: "{}".into(),
                    state: ToolState::Running,
                    output: None,
                    ok: None,
                },
            ],
            ..Default::default()
        }
    }

    // ── settling ────────────────────────────────────────────────────────

    /// The rule that stops a crash leaving a conversation showing "Stop"
    /// forever.
    #[test]
    fn an_abandoned_streaming_message_is_settled() {
        let mut message = streaming_message("msg_1");
        settle(&mut message);

        assert_eq!(message.status, Some(Status::Sent));
        assert!(message.stopped);

        let Part::Tool { state, output, .. } = &message.parts[1] else {
            panic!("expected a tool part");
        };
        assert_eq!(*state, ToolState::Failed);
        assert_eq!(output.as_deref(), Some(INTERRUPTED));
    }

    /// The user watched this text appear. Discarding it would make the record
    /// disagree with what was on screen.
    #[test]
    fn settling_keeps_the_partial_text() {
        let mut message = streaming_message("msg_1");
        settle(&mut message);
        assert_eq!(message.content, "half a th");
        assert!(matches!(&message.parts[0], Part::Text { text } if text == "half a th"));
    }

    #[test]
    fn settling_leaves_finished_messages_alone() {
        let mut message = Message {
            id: "msg_1".into(),
            status: Some(Status::Sent),
            ..Default::default()
        };
        let before = message.clone();
        settle(&mut message);
        assert_eq!(message.status, before.status);
        assert!(!message.stopped);
    }

    // ── reading and writing ─────────────────────────────────────────────

    #[test]
    fn messages_round_trip() {
        let (_dir, store) = store();
        let messages = vec![Message {
            id: "msg_1".into(),
            role: "user".into(),
            content: "hello".into(),
            status: Some(Status::Sent),
            ..Default::default()
        }];

        store.write_messages("thr_1", &messages).unwrap();
        let read = store.read_messages("thr_1", &[]);

        assert_eq!(read.len(), 1);
        assert_eq!(read[0].content, "hello");
    }

    /// Loading cold, after a crash: nothing is live, so everything mid-stream
    /// settles.
    #[test]
    fn loading_cold_settles_everything_mid_stream() {
        let (_dir, store) = store();
        store
            .write_messages("thr_1", &[streaming_message("msg_1")])
            .unwrap();

        let read = store.read_messages("thr_1", &[]);
        assert!(read[0].stopped, "an abandoned message was left streaming");
    }

    /// Disk is behind memory by construction - it is written on a debounce -
    /// so a message the app is still streaming must not be settled out from
    /// under it.
    #[test]
    fn a_message_still_in_flight_is_left_alone() {
        let (_dir, store) = store();
        store
            .write_messages("thr_1", &[streaming_message("msg_1")])
            .unwrap();

        let read = store.read_messages("thr_1", &["msg_1".to_string()]);
        assert!(!read[0].stopped, "a live message was settled");
        assert_eq!(read[0].status, Some(Status::Streaming));
    }

    #[test]
    fn an_unknown_thread_has_no_messages() {
        let (_dir, store) = store();
        assert!(store.read_messages("thr_missing", &[]).is_empty());
    }

    #[test]
    fn threads_list_newest_first() {
        let (_dir, store) = store();
        for (id, updated) in [
            ("thr_old", "2026-01-01T00:00:00.000Z"),
            ("thr_new", "2026-03-01T00:00:00.000Z"),
            ("thr_mid", "2026-02-01T00:00:00.000Z"),
        ] {
            store
                .write_thread(&Thread {
                    id: id.into(),
                    updated_at: updated.into(),
                    ..Default::default()
                })
                .unwrap();
        }

        let ids: Vec<String> = store.list_threads().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, vec!["thr_new", "thr_mid", "thr_old"]);
    }

    /// One unreadable conversation must not hide every other one.
    #[test]
    fn a_damaged_thread_does_not_hide_the_rest() {
        let (dir, store) = store();
        store
            .write_thread(&Thread {
                id: "thr_good".into(),
                updated_at: "2026-01-01T00:00:00.000Z".into(),
                ..Default::default()
            })
            .unwrap();
        std::fs::write(
            dir.path().join("conversations/threads/thr_bad.json"),
            "{ not json",
        )
        .unwrap();

        let threads = store.list_threads();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "thr_good");
        // And the damaged bytes were preserved rather than discarded.
        assert!(dir
            .path()
            .join("conversations/threads/thr_bad.json.damaged")
            .exists());
    }

    #[test]
    fn deleting_removes_both_files() {
        let (dir, store) = store();
        store
            .write_thread(&Thread {
                id: "thr_1".into(),
                ..Default::default()
            })
            .unwrap();
        store.write_messages("thr_1", &[]).unwrap();

        store.delete_thread("thr_1").unwrap();

        assert!(!dir.path().join("conversations/threads/thr_1.json").exists());
        assert!(!dir
            .path()
            .join("conversations/messages/thr_1.json")
            .exists());
        assert!(store.list_threads().is_empty());
    }

    // ── on-disk shape ───────────────────────────────────────────────────

    /// The file is one document holding the whole array, keyed the way the
    /// existing workspaces are.
    #[test]
    fn the_message_file_keeps_its_documented_shape() {
        let (dir, store) = store();
        store
            .write_messages(
                "thr_1",
                &[Message {
                    id: "msg_1".into(),
                    role: "user".into(),
                    content: "hi".into(),
                    ..Default::default()
                }],
            )
            .unwrap();

        let raw =
            std::fs::read_to_string(dir.path().join("conversations/messages/thr_1.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(value["id"], "thr_1");
        assert_eq!(value["threadId"], "thr_1");
        assert!(value["messages"].is_array());
    }

    #[test]
    fn thread_fields_are_camel_case_on_disk() {
        let (dir, store) = store();
        store
            .write_thread(&Thread {
                id: "thr_1".into(),
                agent_id: Some("atlas".into()),
                message_count: 3,
                updated_at: "2026-01-01T00:00:00.000Z".into(),
                ..Default::default()
            })
            .unwrap();

        let raw =
            std::fs::read_to_string(dir.path().join("conversations/threads/thr_1.json")).unwrap();
        assert!(raw.contains("\"agentId\""), "got {raw}");
        assert!(raw.contains("\"messageCount\""), "got {raw}");
        assert!(raw.contains("\"updatedAt\""), "got {raw}");
    }

    /// Shaped exactly like a thread the Electron build wrote. A file like this
    /// was read as damaged and quarantined, and every field below `mode` would
    /// have been deleted by the next save.
    const REAL_THREAD: &str = r#"{
      "id": "thr-local-mta1b2c3-1",
      "agentId": "agent-inertia-dev",
      "title": "Compare three note-taking apps",
      "mode": "group",
      "approval": "auto",
      "draft": false,
      "pinned": false,
      "unread": 0,
      "updatedAt": "2026-09-06T17:19:33.869Z",
      "preview": "@researcher hello",
      "messageCount": 1,
      "computerAttached": null,
      "temporary": false,
      "room": { "primary": "agent-inertia-dev", "roster": ["agent-inertia-dev", "agent-researcher"], "hops": 1 },
      "createdAt": "2026-09-06T16:49:20.572Z",
      "context": { "used": 215460, "window": 1000000, "at": 1788715172478 }
    }"#;

    /// A mode the window shipped and this enum never named must not turn a
    /// conversation into a `.damaged` file.
    #[test]
    fn a_thread_in_a_mode_this_build_does_not_know_still_reads() {
        let thread: Thread = serde_json::from_str(REAL_THREAD).unwrap();
        assert_eq!(thread.id, "thr-local-mta1b2c3-1");
        assert_eq!(thread.mode, Mode::Other("group".into()));
        assert_eq!(thread.message_count, 1);
    }

    /// The half that actually destroyed data: reading and writing a group
    /// conversation used to delete its roster.
    #[test]
    fn a_round_trip_keeps_the_fields_rust_does_not_model() {
        let (dir, store) = store();
        let thread: Thread = serde_json::from_str(REAL_THREAD).unwrap();
        store.write_thread(&thread).unwrap();

        let raw =
            std::fs::read_to_string(dir.path().join("conversations/threads/thr-local-mta1b2c3-1.json"))
                .unwrap();
        let written: serde_json::Value = serde_json::from_str(&raw).unwrap();

        assert_eq!(written["mode"], "group");
        assert_eq!(written["room"]["roster"][1], "agent-researcher");
        assert_eq!(written["context"]["window"], 1_000_000);
        assert_eq!(written["temporary"], false);
        assert_eq!(written["draft"], false);
    }
}

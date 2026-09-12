//! The two stored messages a turn produces, built as it streams.
//!
//! A turn started from a window does not need this. The window holds the
//! transcript it is drawing, writes it through the `messages` collection as the
//! reply arrives, and a second writer on this side would be two answers to
//! "what was said" with no rule for which wins.
//!
//! A turn nobody started from a window does. A routine's turn runs on a tick
//! with no author but the scheduler, and without this its conversation would be
//! empty: nothing to open under Routines, and nothing for the run's own outcome
//! to be read out of - which is decided by looking at the last reply on disk.
//!
//! It lives here rather than in the agent crate because it is a *storage*
//! concern. The agent already reports a canonical transcript; this is the shape
//! the screens read, derived from the same events.

use inertia_agent::{AgentEvent, StopReason};
use inertia_core::message::Entry;
use inertia_store::conversations::{Message, Part, Status, ToolState};

/// A user message and the reply being written into it.
#[derive(Debug)]
pub struct Assembling {
    user: Message,
    reply: Message,
}

impl Assembling {
    /// `reply_id` is minted by the caller, because the caller has to be able to
    /// name the message before the turn starts - that id is what a window
    /// attaches to when it wants to watch a run that is already going.
    pub fn new(model: &str, agent_id: Option<String>, prompt: &str, reply_id: String) -> Self {
        let now = jiff::Timestamp::now().to_string();
        Self {
            user: Message {
                id: format!("msg_{}", uuid::Uuid::now_v7().simple()),
                role: "user".into(),
                content: prompt.to_string(),
                created_at: now.clone(),
                status: Some(Status::Sent),
                ..Default::default()
            },
            reply: Message {
                id: reply_id,
                role: "agent".into(),
                agent_id,
                created_at: now,
                status: Some(Status::Streaming),
                model: Some(model.to_string()),
                ..Default::default()
            },
        }
    }

    pub fn observe(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::Delta { text } => {
                self.reply.content.push_str(text);
                match self.reply.parts.last_mut() {
                    Some(Part::Text { text: existing }) => existing.push_str(text),
                    _ => self.reply.parts.push(Part::Text { text: text.clone() }),
                }
            }
            AgentEvent::ToolStarted { call, .. } => {
                self.reply.parts.push(Part::Tool {
                    call_id: Some(call.id.to_string()),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                    state: ToolState::Running,
                    output: None,
                    ok: None,
                });
            }
            AgentEvent::ToolFinished { result } => {
                // Matched by id rather than by position: calls in one batch
                // finish in whatever order they finish.
                for part in self.reply.parts.iter_mut() {
                    if let Part::Tool {
                        call_id,
                        state,
                        output,
                        ok,
                        ..
                    } = part
                    {
                        if call_id.as_deref() == Some(result.call_id.as_str()) {
                            *state = if result.ok {
                                ToolState::Done
                            } else {
                                ToolState::Failed
                            };
                            *output = Some(result.output.clone());
                            *ok = Some(result.ok);
                        }
                    }
                }
            }
            AgentEvent::Done {
                stopped, history, ..
            } => {
                self.reply.status = Some(Status::Sent);
                self.reply.stopped = matches!(
                    stopped,
                    StopReason::Cancelled | StopReason::MaxSteps | StopReason::Refused
                );
                if let StopReason::Error { message } = stopped {
                    self.reply.error = Some(message.clone());
                }
                // The signed reasoning is only recoverable from the canonical
                // transcript, and it has to be stored to be replayed.
                if let Some(Entry::Assistant(last)) = history
                    .iter()
                    .rev()
                    .find(|e| matches!(e, Entry::Assistant(_)))
                {
                    self.reply.thinking = last
                        .thinking
                        .iter()
                        .filter_map(|b| serde_json::to_value(b).ok())
                        .collect();
                }
            }
            _ => {}
        }
    }

    /// The turn was stopped before it reported `Done`.
    pub fn interrupted(&mut self) {
        self.reply.status = Some(Status::Sent);
        self.reply.stopped = true;
        for part in self.reply.parts.iter_mut() {
            if let Part::Tool { state, output, .. } = part {
                if *state == ToolState::Running {
                    *state = ToolState::Failed;
                    *output = Some("This call was interrupted and never finished.".into());
                }
            }
        }
    }

    /// Appends both messages to what the thread already holds.
    ///
    /// Read-then-write rather than an append, because that is the whole of the
    /// conversations API - and the read happens here, at the end, so a message
    /// written by something else while the turn was running is still there
    /// afterwards.
    pub fn save(self, conversations: &inertia_store::conversations::Conversations, thread_id: &str) {
        let mut messages = conversations.read_messages(thread_id, &[]);
        messages.push(self.user);
        messages.push(self.reply);
        if let Err(error) = conversations.write_messages(thread_id, &messages) {
            tracing::error!(%error, thread = thread_id, "the conversation could not be saved");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::message::ToolCall;
    use inertia_core::tool::ToolResult;
    use inertia_core::ToolCallId;

    fn started(id: &str, name: &str) -> AgentEvent {
        AgentEvent::ToolStarted {
            call: ToolCall {
                id: ToolCallId::from_existing(id.to_string()),
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
            title: None,
        }
    }

    fn finished(id: &str, name: &str, ok: bool) -> AgentEvent {
        AgentEvent::ToolFinished {
            result: ToolResult {
                call_id: ToolCallId::from_existing(id.to_string()),
                tool: name.to_string(),
                title: None,
                ok,
                output: "done".to_string(),
                metadata: None,
                images: Vec::new(),
                duration_ms: 1,
            },
        }
    }

    #[test]
    fn the_reply_keeps_the_id_it_was_given() {
        let assembling = Assembling::new("anthropic/x", None, "go", "msg_fixed".into());
        assert_eq!(assembling.reply.id, "msg_fixed");
        assert_eq!(assembling.user.content, "go");
    }

    #[test]
    fn prose_accumulates_into_one_part() {
        let mut assembling = Assembling::new("m", None, "go", "msg_1".into());
        assembling.observe(&AgentEvent::Delta { text: "Hel".into() });
        assembling.observe(&AgentEvent::Delta { text: "lo".into() });
        assert_eq!(assembling.reply.content, "Hello");
        assert_eq!(assembling.reply.parts.len(), 1);
    }

    #[test]
    fn a_call_is_settled_by_id_not_by_order() {
        let mut assembling = Assembling::new("m", None, "go", "msg_1".into());
        assembling.observe(&started("a", "read"));
        assembling.observe(&started("b", "write"));
        // The second call answers first.
        assembling.observe(&finished("b", "write", true));

        let states: Vec<_> = assembling
            .reply
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Tool { state, .. } => Some(*state),
                _ => None,
            })
            .collect();
        assert_eq!(states, vec![ToolState::Running, ToolState::Done]);
    }

    #[test]
    fn an_interrupted_turn_fails_the_call_that_never_answered() {
        let mut assembling = Assembling::new("m", None, "go", "msg_1".into());
        assembling.observe(&started("a", "shell"));
        assembling.interrupted();

        assert!(assembling.reply.stopped);
        assert_eq!(assembling.reply.status, Some(Status::Sent));
        match &assembling.reply.parts[0] {
            Part::Tool { state, output, .. } => {
                assert_eq!(*state, ToolState::Failed);
                assert!(output.as_deref().unwrap_or_default().contains("interrupted"));
            }
            _ => panic!("expected a tool part"),
        }
    }

    /// The round trip a routine's own outcome is read out of: the run is
    /// judged by finding the last message on disk that is not the user's, and
    /// reading its error, its `stopped` flag and its text. A turn that wrote
    /// nothing reports "the turn left nothing behind", so this is the test that
    /// says a routine can report success at all.
    #[test]
    fn what_was_said_is_on_disk_after_the_turn_and_what_was_there_stays() {
        let dir = tempfile::tempdir().unwrap();
        let layout = inertia_store::layout::Layout::new(dir.path());
        let conversations = inertia_store::conversations::Conversations::new(layout);

        // Yesterday's run.
        let earlier = Message {
            id: "msg_old".into(),
            role: "agent".into(),
            content: "ran yesterday".into(),
            created_at: jiff::Timestamp::now().to_string(),
            status: Some(Status::Sent),
            ..Default::default()
        };
        conversations
            .write_messages("routine-morning", &[earlier])
            .unwrap();

        let mut assembling =
            Assembling::new("anthropic/x", Some("atlas".into()), "check the build", "msg_new".into());
        assembling.observe(&AgentEvent::Delta { text: "It is green.".into() });
        assembling.observe(&AgentEvent::Done {
            stopped: StopReason::Complete,
            history: Vec::new(),
            usage: None,
        });
        assembling.save(&conversations, "routine-morning");

        let stored = conversations.read_messages("routine-morning", &[]);
        assert_eq!(stored.len(), 3, "yesterday's reply is still there");

        let last = stored.iter().rev().find(|m| m.role != "user").unwrap();
        assert_eq!(last.id, "msg_new");
        assert_eq!(last.content, "It is green.");
        assert_eq!(last.agent_id.as_deref(), Some("atlas"));
        assert!(last.error.is_none());
        assert!(!last.stopped);

        let asked = stored.iter().find(|m| m.role == "user").unwrap();
        assert_eq!(asked.content, "check the build");
    }

    #[test]
    fn a_failed_turn_carries_the_reason_onto_the_message() {
        let mut assembling = Assembling::new("m", None, "go", "msg_1".into());
        assembling.observe(&AgentEvent::Done {
            stopped: StopReason::Error {
                message: "the provider refused".into(),
            },
            history: Vec::new(),
            usage: None,
        });
        assert_eq!(assembling.reply.error.as_deref(), Some("the provider refused"));
        assert_eq!(assembling.reply.status, Some(Status::Sent));
    }
}

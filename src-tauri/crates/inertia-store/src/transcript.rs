//! Converting between what is stored and what the model is sent.
//!
//! These are two different shapes on purpose. The stored [`Message`] is what
//! the UI renders: one record per bubble, with tool calls nested inside the
//! reply they belong to, which is how a person reads a conversation. The
//! [`Entry`] transcript is what a provider accepts: a flat sequence where a
//! tool call and its result are separate turns.
//!
//! The conversion has to be lossless in one direction - a conversation reloaded
//! from disk must reproduce a request the provider will accept. That is why the
//! stored tool part carries its raw argument string: without it the `tool_use`
//! block cannot be rebuilt, and the result that follows becomes an orphan that
//! strict providers reject outright.

use inertia_core::message::{AssistantEntry, Entry, ThinkingBlock, ToolCall, ToolEntry};
use inertia_core::ToolCallId;

use crate::conversations::{Message, Part, ToolState};

/// Builds the provider-facing transcript from stored messages.
pub fn to_entries(messages: &[Message]) -> Vec<Entry> {
    let mut entries = Vec::with_capacity(messages.len());

    for message in messages {
        if message.role == "user" {
            if !message.content.trim().is_empty() {
                entries.push(Entry::user(&message.content));
            }
            continue;
        }

        // Prose comes from the text parts when there are any, and from
        // `content` otherwise - a message that never streamed parts still has
        // its body there.
        let text: String = {
            let from_parts: String = message
                .parts
                .iter()
                .filter_map(|p| match p {
                    Part::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            if from_parts.is_empty() {
                message.content.clone()
            } else {
                from_parts
            }
        };

        let tool_calls: Vec<ToolCall> = message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Tool {
                    call_id,
                    name,
                    arguments,
                    ..
                } => Some(ToolCall {
                    id: ToolCallId::from_existing(call_id.clone().unwrap_or_default()),
                    name: name.clone(),
                    arguments: if arguments.is_empty() {
                        "{}".to_string()
                    } else {
                        arguments.clone()
                    },
                }),
                _ => None,
            })
            // A call with no id cannot be answered, and sending it guarantees
            // a rejected request.
            .filter(|call| !call.id.as_str().is_empty())
            .collect();

        let thinking: Vec<ThinkingBlock> = message
            .thinking
            .iter()
            .filter_map(|value| serde_json::from_value(value.clone()).ok())
            .collect();

        if text.trim().is_empty() && tool_calls.is_empty() && thinking.is_empty() {
            continue;
        }

        entries.push(Entry::Assistant(AssistantEntry {
            content: (!text.trim().is_empty()).then_some(text),
            tool_calls: tool_calls.clone(),
            thinking,
            // Read back from a stored transcript, where the author is a fact
            // about the message record rather than about this entry. The one
            // caller that needs it - a group turn being seen from another
            // agent's seat - is handed history by the window, which labels it.
            ..Default::default()
        }));

        // Each call's result becomes its own turn, in the order the calls were
        // made.
        for part in &message.parts {
            let Part::Tool {
                call_id,
                state,
                output,
                ok,
                ..
            } = part
            else {
                continue;
            };
            let Some(id) = call_id.as_ref().filter(|id| !id.is_empty()) else {
                continue;
            };
            // A call still marked running has no result to report. Leaving it
            // unanswered is handled by the provider translation, which
            // synthesises one rather than sending a broken request.
            if *state == ToolState::Running {
                continue;
            }

            entries.push(Entry::Tool(ToolEntry {
                tool_call_id: ToolCallId::from_existing(id.clone()),
                content: output.clone().unwrap_or_default(),
                ok: ok.or(Some(*state == ToolState::Done)),
                pruned: false,
            }));
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::Status;

    fn user(text: &str) -> Message {
        Message {
            id: "msg_u".into(),
            role: "user".into(),
            content: text.into(),
            status: Some(Status::Sent),
            ..Default::default()
        }
    }

    fn agent(parts: Vec<Part>) -> Message {
        Message {
            id: "msg_a".into(),
            role: "agent".into(),
            status: Some(Status::Sent),
            parts,
            ..Default::default()
        }
    }

    #[test]
    fn a_plain_exchange_converts() {
        let entries = to_entries(&[
            user("hello"),
            agent(vec![Part::Text {
                text: "hi there".into(),
            }]),
        ]);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text(), "hello");
        assert_eq!(entries[1].text(), "hi there");
    }

    /// The property the whole module exists for: a reloaded conversation must
    /// reproduce a request the provider accepts, which means every call is
    /// rebuilt with its arguments and paired with its result.
    #[test]
    fn a_tool_round_trip_is_rebuilt_with_its_arguments() {
        let entries = to_entries(&[
            user("read it"),
            agent(vec![
                Part::Text {
                    text: "looking".into(),
                },
                Part::Tool {
                    call_id: Some("toolu_1".into()),
                    name: "read".into(),
                    arguments: r#"{"filePath":"a.txt"}"#.into(),
                    state: ToolState::Done,
                    output: Some("file contents".into()),
                    ok: Some(true),
                },
            ]),
        ]);

        assert_eq!(entries.len(), 3);

        let Entry::Assistant(reply) = &entries[1] else {
            panic!("expected an assistant turn, got {:?}", entries[1]);
        };
        assert_eq!(reply.content.as_deref(), Some("looking"));
        assert_eq!(reply.tool_calls.len(), 1);
        assert_eq!(reply.tool_calls[0].parsed_arguments()["filePath"], "a.txt");

        let Entry::Tool(result) = &entries[2] else {
            panic!("expected a tool result, got {:?}", entries[2]);
        };
        // The pairing that keeps the request valid.
        assert_eq!(result.tool_call_id, reply.tool_calls[0].id);
        assert_eq!(result.content, "file contents");
        assert_eq!(result.ok, Some(true));
    }

    #[test]
    fn a_failed_call_is_recorded_as_failed() {
        let entries = to_entries(&[
            user("go"),
            agent(vec![Part::Tool {
                call_id: Some("toolu_1".into()),
                name: "read".into(),
                arguments: "{}".into(),
                state: ToolState::Failed,
                output: Some("no such file".into()),
                ok: Some(false),
            }]),
        ]);

        let Entry::Tool(result) = entries.last().unwrap() else {
            panic!("expected a tool result");
        };
        assert_eq!(result.ok, Some(false));
    }

    /// A call still running has no result. The provider translation
    /// synthesises one rather than sending a broken request, so emitting a
    /// half-finished result here would be worse than emitting none.
    #[test]
    fn a_still_running_call_contributes_no_result() {
        let entries = to_entries(&[
            user("go"),
            agent(vec![Part::Tool {
                call_id: Some("toolu_1".into()),
                name: "read".into(),
                arguments: "{}".into(),
                state: ToolState::Running,
                output: None,
                ok: None,
            }]),
        ]);

        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[1], Entry::Assistant(_)));
    }

    /// Signed reasoning has to come back byte-identical or the provider
    /// refuses to continue its own chain of thought.
    #[test]
    fn signed_thinking_survives_the_round_trip() {
        let message = Message {
            id: "msg_a".into(),
            role: "agent".into(),
            content: "done".into(),
            thinking: vec![serde_json::json!({
                "type": "thinking",
                "thinking": "let me see",
                "signature": "sig-abc"
            })],
            ..Default::default()
        };

        let entries = to_entries(&[user("go"), message]);
        let Entry::Assistant(reply) = &entries[1] else {
            panic!("expected an assistant turn");
        };
        assert_eq!(reply.thinking.len(), 1);
        assert!(reply.thinking[0].is_replayable());
    }

    #[test]
    fn empty_messages_are_skipped() {
        let entries = to_entries(&[user("   "), agent(vec![]), user("real")]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text(), "real");
    }

    /// A message that never streamed parts still has its body in `content`.
    #[test]
    fn a_reply_without_parts_uses_its_content() {
        let message = Message {
            id: "msg_a".into(),
            role: "agent".into(),
            content: "straight from content".into(),
            ..Default::default()
        };
        let entries = to_entries(&[user("go"), message]);
        assert_eq!(entries[1].text(), "straight from content");
    }

    /// A call with no id cannot be answered, so sending it guarantees a
    /// rejected request.
    #[test]
    fn a_call_without_an_id_is_dropped() {
        let entries = to_entries(&[
            user("go"),
            agent(vec![Part::Tool {
                call_id: None,
                name: "read".into(),
                arguments: "{}".into(),
                state: ToolState::Done,
                output: Some("x".into()),
                ok: Some(true),
            }]),
        ]);

        // The assistant turn has no calls, and no orphan result follows it.
        assert_eq!(entries.len(), 1);
    }
}

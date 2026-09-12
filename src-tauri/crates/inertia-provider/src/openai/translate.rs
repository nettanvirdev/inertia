//! Transcript → OpenAI chat-completions messages.
//!
//! Far less involved than the Anthropic translation, because this API is
//! forgiving about structure. Two rules still matter, and both exist because
//! providers reject the alternative:
//!
//!   - An assistant message carrying `tool_calls` with no prose sends
//!     `content: null`, not `""`. Several providers reject the empty string
//!     here and accept null.
//!   - A `tool` message whose call is no longer in the window is dropped. Its
//!     request was trimmed away, and an answer to a call the model cannot see
//!     is a 400.

use inertia_core::message::{Entry, Part};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub role: &'static str,
    /// `None` serialises as an explicit `null`, which is deliberate - see the
    /// module note.
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    fn simple(role: &'static str, text: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(Value::String(text.into())),
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

/// Builds the message array, system prompt first.
pub fn to_messages(system: &str, history: &[Entry]) -> Vec<Message> {
    let mut messages = vec![Message::simple("system", system)];

    // Which calls the model can still see. A result whose request has been
    // trimmed out of the window is dropped rather than sent orphaned.
    let mut answerable: Vec<String> = Vec::new();
    for entry in history {
        if let Entry::Assistant(entry) = entry {
            answerable.extend(entry.tool_calls.iter().map(|c| c.id.to_string()));
        }
    }

    for entry in history {
        match entry {
            Entry::Tool(result) => {
                let id = result.tool_call_id.to_string();
                if !answerable.contains(&id) {
                    continue;
                }
                messages.push(Message {
                    role: "tool",
                    content: Some(Value::String(result.content.clone())),
                    tool_calls: None,
                    tool_call_id: Some(id),
                });
            }
            Entry::Assistant(entry) => {
                let text = entry.content.as_deref().unwrap_or_default();
                let tool_calls: Vec<Value> = entry
                    .tool_calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id.to_string(),
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments,
                            }
                        })
                    })
                    .collect();

                let content = if text.is_empty() && !tool_calls.is_empty() {
                    // Explicitly null, not "". Several providers reject the
                    // empty string alongside tool_calls.
                    None
                } else {
                    Some(Value::String(text.to_string()))
                };

                messages.push(Message {
                    role: "assistant",
                    content,
                    tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                    tool_call_id: None,
                });
            }
            Entry::User(entry) => {
                if let Some(parts) = &entry.parts {
                    let content: Vec<Value> = parts
                        .iter()
                        .map(|part| match part {
                            Part::Text { text } => json!({ "type": "text", "text": text }),
                            Part::ImageUrl { image_url } => json!({
                                "type": "image_url",
                                "image_url": { "url": image_url.url() },
                            }),
                        })
                        .collect();
                    messages.push(Message {
                        role: "user",
                        content: Some(Value::Array(content)),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                    continue;
                }

                let text = entry.content.as_deref().unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                messages.push(Message::simple("user", text));
            }
        }
    }

    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::message::{AssistantEntry, ImageUrl, ToolCall, ToolEntry, UserEntry};
    use inertia_core::ToolCallId;

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: ToolCallId::from_existing(id),
            name: "read".into(),
            arguments: r#"{"path":"a.txt"}"#.into(),
        }
    }

    #[test]
    fn the_system_prompt_is_message_zero() {
        let messages = to_messages("be helpful", &[Entry::user("hi")]);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content, Some(json!("be helpful")));
    }

    /// Even an empty system prompt keeps its slot, so the array shape does not
    /// change between requests.
    #[test]
    fn an_empty_system_prompt_still_occupies_slot_zero() {
        let messages = to_messages("", &[Entry::user("hi")]);
        assert_eq!(messages[0].role, "system");
    }

    /// The empty string is rejected here by several providers; null is not.
    #[test]
    fn a_tool_only_assistant_turn_sends_null_content() {
        let history = vec![
            Entry::user("go"),
            Entry::Assistant(AssistantEntry {
                content: None,
                tool_calls: vec![call("tc_1")],
                ..Default::default()
            }),
            Entry::Tool(ToolEntry {
                tool_call_id: ToolCallId::from_existing("tc_1"),
                content: "done".into(),
                ok: Some(true),
                pruned: false,
            }),
        ];
        let messages = to_messages("", &history);

        let assistant = messages.iter().find(|m| m.role == "assistant").unwrap();
        assert!(assistant.content.is_none());

        let json = serde_json::to_value(assistant).unwrap();
        assert_eq!(json["content"], Value::Null, "content must be explicit null");
        assert_eq!(json["tool_calls"][0]["function"]["name"], "read");
        assert_eq!(json["tool_calls"][0]["type"], "function");
    }

    #[test]
    fn an_assistant_turn_with_prose_and_calls_keeps_both() {
        let history = vec![
            Entry::user("go"),
            Entry::Assistant(AssistantEntry {
                content: Some("let me look".into()),
                tool_calls: vec![call("tc_1")],
                ..Default::default()
            }),
        ];
        let messages = to_messages("", &history);
        let assistant = messages.iter().find(|m| m.role == "assistant").unwrap();
        assert_eq!(assistant.content, Some(json!("let me look")));
        assert!(assistant.tool_calls.is_some());
    }

    /// An answer to a call the model can no longer see is a 400.
    #[test]
    fn an_orphaned_tool_result_is_dropped() {
        let history = vec![
            Entry::user("go"),
            Entry::Tool(ToolEntry {
                tool_call_id: ToolCallId::from_existing("tc_gone"),
                content: "answer to nothing".into(),
                ok: Some(true),
                pruned: false,
            }),
        ];
        let messages = to_messages("", &history);
        assert!(messages.iter().all(|m| m.role != "tool"));
    }

    #[test]
    fn a_tool_result_carries_its_call_id() {
        let history = vec![
            Entry::user("go"),
            Entry::Assistant(AssistantEntry {
                tool_calls: vec![call("tc_1")],
                ..Default::default()
            }),
            Entry::Tool(ToolEntry {
                tool_call_id: ToolCallId::from_existing("tc_1"),
                content: "file contents".into(),
                ok: Some(true),
                pruned: false,
            }),
        ];
        let messages = to_messages("", &history);
        let tool = messages.iter().find(|m| m.role == "tool").unwrap();
        assert_eq!(tool.tool_call_id.as_deref(), Some("tc_1"));
        assert_eq!(tool.content, Some(json!("file contents")));
    }

    #[test]
    fn blank_user_messages_are_dropped() {
        let messages = to_messages("", &[Entry::user("  "), Entry::user("real")]);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, Some(json!("real")));
    }

    #[test]
    fn image_parts_become_content_parts() {
        let history = vec![Entry::User(UserEntry {
            parts: Some(vec![
                Part::Text {
                    text: "look".into(),
                },
                Part::ImageUrl {
                    image_url: ImageUrl::Bare("data:image/png;base64,AAA".into()),
                },
            ]),
            ..Default::default()
        })];
        let messages = to_messages("", &history);
        let content = messages[1].content.clone().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAA");
    }
}

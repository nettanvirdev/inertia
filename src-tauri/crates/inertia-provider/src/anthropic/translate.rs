//! Transcript → Anthropic Messages.
//!
//! Materially more involved than the OpenAI translation, because the Messages
//! API enforces structural rules the chat-completions API does not. Every rule
//! below was a production failure before it became a rule:
//!
//!   - The system prompt is a top-level field. Sending `role: "system"` is
//!     rejected outright.
//!   - A tool result is a `user` message, and several results answering one
//!     assistant turn are blocks inside a *single* message. Sent separately
//!     they read as several human turns, and adjacent same-role messages are
//!     refused.
//!   - Every `tool_use` must be answered or the whole request is refused. A
//!     turn stopped mid-tool-round is exactly that shape, so it is patched
//!     rather than left to poison every later message in the session.
//!   - Empty text blocks are rejected, and an assistant turn that was pure
//!     tool calls has no prose block at all.
//!   - Signed thinking blocks must come back verbatim or the model refuses to
//!     continue its own reasoning across a tool call.

use inertia_core::message::{Entry, Part, ThinkingBlock};

use super::wire::{Block, ImageSource, Message, Role};

/// The message a synthesised answer carries.
///
/// Phrased for the model rather than for a log: it explains why there is no
/// result without implying the tool failed, which would send it down a
/// debugging path that does not exist.
const UNFINISHED: &str = "This call did not finish. The turn was stopped before it returned.";

/// Translates a transcript into a system prompt and a message list.
pub fn to_messages(system: &str, history: &[Entry]) -> (Vec<Block>, Vec<Message>) {
    let system_blocks = if system.trim().is_empty() {
        Vec::new()
    } else {
        vec![Block::Text {
            text: system.to_string(),
        }]
    };

    let mut messages = merge_adjacent(stage(history));

    // Trimming can expose a new leading assistant message, and pairing can
    // drop a leading answer that exposes another - so both run until the list
    // is stable. Each pass removes at least one message, so this terminates.
    //
    // The order matters and was arrived at the hard way: pairing first let an
    // orphaned assistant tool-call survive at position zero, and trimming it
    // afterwards left its answer as the very first message - a `tool_result`
    // with no matching `tool_use` anywhere, rejected outright, which ended a
    // real nineteen-step session mid-turn.
    loop {
        let before = messages.len();
        messages = open_with_user(messages);
        messages = pair_tool_calls(&messages);
        if messages.is_empty() || messages[0].role == Role::User || messages.len() == before {
            break;
        }
    }

    strip_trailing_whitespace(&mut messages);

    (system_blocks, messages)
}

/// Turns each transcript entry into a message, dropping ones with no content.
fn stage(history: &[Entry]) -> Vec<Message> {
    let mut staged = Vec::new();

    for entry in history {
        match entry {
            Entry::Tool(result) => {
                let content = if result.content.trim().is_empty() {
                    // An empty tool result is still an answer, and the call
                    // must be answered. A placeholder is better than a
                    // rejected request.
                    "(no output)".to_string()
                } else {
                    result.content.clone()
                };
                staged.push(Message::user(vec![Block::ToolResult {
                    tool_use_id: result.tool_call_id.to_string(),
                    content,
                    is_error: result.ok == Some(false),
                }]));
            }
            Entry::Assistant(entry) => {
                let blocks = assistant_blocks(entry);
                if !blocks.is_empty() {
                    staged.push(Message::assistant(blocks));
                }
            }
            Entry::User(entry) => {
                let blocks = match &entry.parts {
                    Some(parts) => parts_to_blocks(parts),
                    None => {
                        let text = entry.content.as_deref().unwrap_or_default();
                        if text.trim().is_empty() {
                            Vec::new()
                        } else {
                            vec![Block::Text {
                                text: text.to_string(),
                            }]
                        }
                    }
                };
                if !blocks.is_empty() {
                    staged.push(Message::user(blocks));
                }
            }
        }
    }

    staged
}

/// Assistant content, in the order the API requires: thinking, then prose,
/// then tool calls.
fn assistant_blocks(entry: &inertia_core::message::AssistantEntry) -> Vec<Block> {
    let mut blocks = Vec::new();

    for block in &entry.thinking {
        match block {
            ThinkingBlock::RedactedThinking { data } => {
                blocks.push(Block::RedactedThinking { data: data.clone() });
            }
            ThinkingBlock::Thinking {
                thinking,
                signature,
            } => {
                // An unsigned block cannot be replayed - the API verifies the
                // signature - and sending it unsigned fails the whole request.
                // The model does not need it back either way.
                if let Some(signature) = signature.as_ref().filter(|s| !s.is_empty()) {
                    blocks.push(Block::Thinking {
                        thinking: thinking.clone(),
                        signature: signature.clone(),
                    });
                }
            }
        }
    }

    if let Some(text) = entry.content.as_deref().filter(|t| !t.trim().is_empty()) {
        blocks.push(Block::Text {
            text: text.to_string(),
        });
    }

    for call in &entry.tool_calls {
        if call.name.is_empty() {
            continue;
        }
        blocks.push(Block::ToolUse {
            id: call.id.to_string(),
            name: call.name.clone(),
            // Malformed JSON becomes an empty-object call rather than
            // aborting the request; the tool then reports what is missing,
            // which is something the model can act on.
            input: call.parsed_arguments(),
        });
    }

    blocks
}

fn parts_to_blocks(parts: &[Part]) -> Vec<Block> {
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } if !text.trim().is_empty() => {
                Some(Block::Text { text: text.clone() })
            }
            Part::Text { .. } => None,
            // An unusable image is dropped, never faked into something the
            // provider will reject.
            Part::ImageUrl { image_url } => image_block(image_url.url()),
        })
        .collect()
}

fn image_block(url: &str) -> Option<Block> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (media_type, data) = rest.split_once(";base64,")?;
        if media_type.is_empty() || data.is_empty() {
            return None;
        }
        return Some(Block::Image {
            source: ImageSource::Base64 {
                media_type: media_type.to_string(),
                data: data.to_string(),
            },
        });
    }

    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(Block::Image {
            source: ImageSource::Url {
                url: url.to_string(),
            },
        });
    }

    None
}

/// Merges consecutive same-role messages, which staging produces constantly -
/// two tool results in a row, say. Adjacent same-role messages are refused.
fn merge_adjacent(staged: Vec<Message>) -> Vec<Message> {
    let mut merged: Vec<Message> = Vec::with_capacity(staged.len());

    for message in staged {
        match merged.last_mut() {
            Some(previous) if previous.role == message.role => {
                previous.content.extend(message.content);
            }
            _ => merged.push(message),
        }
    }

    merged
}

/// Drops any leading run of assistant messages. The conversation must open
/// with a user turn.
fn open_with_user(messages: Vec<Message>) -> Vec<Message> {
    let first_user = messages.iter().position(|m| m.role == Role::User);
    match first_user {
        Some(0) => messages,
        Some(at) => messages[at..].to_vec(),
        None => Vec::new(),
    }
}

/// Makes every `tool_use` have exactly one answer.
///
/// Unmatched and duplicate results are dropped; missing ones are synthesised.
/// Both directions matter: an unanswered call is refused outright, and an
/// answer to a call that is no longer present is refused just as hard.
fn pair_tool_calls(messages: &[Message]) -> Vec<Message> {
    let mut paired: Vec<Message> = Vec::with_capacity(messages.len());

    for (index, message) in messages.iter().enumerate() {
        if message.role != Role::User {
            paired.push(message.clone());
            continue;
        }

        let asked: Vec<String> = index
            .checked_sub(1)
            .and_then(|i| messages.get(i))
            .filter(|previous| previous.role == Role::Assistant)
            .map(|previous| previous.asked_tool_ids())
            .unwrap_or_default();

        let mut answered: Vec<String> = Vec::new();
        let mut kept: Vec<Block> = Vec::new();

        for block in &message.content {
            match block {
                Block::ToolResult { tool_use_id, .. } => {
                    if asked.contains(tool_use_id) && !answered.contains(tool_use_id) {
                        answered.push(tool_use_id.clone());
                        kept.push(block.clone());
                    }
                    // Otherwise dropped: it answers a call that is not here.
                }
                other => kept.push(other.clone()),
            }
        }

        // Synthesised answers go first, before any real content, because the
        // API wants a message's tool results at its head.
        let mut content: Vec<Block> = asked
            .iter()
            .filter(|id| !answered.contains(id))
            .map(|id| Block::ToolResult {
                tool_use_id: id.clone(),
                content: UNFINISHED.to_string(),
                is_error: true,
            })
            .collect();
        content.extend(kept);

        if !content.is_empty() {
            paired.push(Message::user(content));
        }
    }

    // An assistant message at the very end with nothing after it has unanswered
    // calls by definition. This is what a turn stopped mid-tool-round looks
    // like, and it would make every subsequent request in the session fail.
    if let Some(last) = paired.last() {
        if last.role == Role::Assistant {
            let unanswered = last.asked_tool_ids();
            if !unanswered.is_empty() {
                paired.push(Message::user(
                    unanswered
                        .into_iter()
                        .map(|id| Block::ToolResult {
                            tool_use_id: id,
                            content: UNFINISHED.to_string(),
                            is_error: true,
                        })
                        .collect(),
                ));
            }
        }
    }

    paired
}

/// The API rejects a final assistant message whose text ends in whitespace.
fn strip_trailing_whitespace(messages: &mut [Message]) {
    let Some(last) = messages.last_mut() else {
        return;
    };
    if last.role != Role::Assistant {
        return;
    }
    if let Some(Block::Text { text }) = last.content.last_mut() {
        let trimmed = text.trim_end();
        if trimmed.len() != text.len() {
            *text = trimmed.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::message::{AssistantEntry, ImageUrl, ToolCall, ToolEntry, UserEntry};
    use inertia_core::ToolCallId;

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: ToolCallId::from_existing(id),
            name: name.into(),
            arguments: "{}".into(),
        }
    }

    fn assistant_calling(ids: &[&str]) -> Entry {
        Entry::Assistant(AssistantEntry {
            content: None,
            tool_calls: ids.iter().map(|id| call(id, "echo")).collect(),
            ..Default::default()
        })
    }

    fn tool_result(id: &str, output: &str) -> Entry {
        Entry::Tool(ToolEntry {
            tool_call_id: ToolCallId::from_existing(id),
            content: output.into(),
            ok: Some(true),
            pruned: false,
        })
    }

    // ── the system prompt ───────────────────────────────────────────────

    #[test]
    fn the_system_prompt_is_never_a_message() {
        let (system, messages) = to_messages("be helpful", &[Entry::user("hi")]);
        assert_eq!(
            system,
            vec![Block::Text {
                text: "be helpful".into()
            }]
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);
    }

    #[test]
    fn an_empty_system_prompt_is_an_empty_array() {
        let (system, _) = to_messages("   ", &[Entry::user("hi")]);
        assert!(system.is_empty());
    }

    // ── structure ───────────────────────────────────────────────────────

    #[test]
    fn the_conversation_must_open_with_a_user_turn() {
        let history = vec![Entry::assistant("I spoke first"), Entry::user("hello")];
        let (_, messages) = to_messages("", &history);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages.len(), 1);
    }

    /// Two results in a row is the normal shape after a parallel tool batch,
    /// and sending them as two messages reads as two human turns.
    #[test]
    fn consecutive_tool_results_merge_into_one_message() {
        let history = vec![
            Entry::user("go"),
            assistant_calling(&["tc_1", "tc_2"]),
            tool_result("tc_1", "first"),
            tool_result("tc_2", "second"),
        ];
        let (_, messages) = to_messages("", &history);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].role, Role::User);
        assert_eq!(messages[2].content.len(), 2);
    }

    #[test]
    fn an_empty_tool_result_still_answers_its_call() {
        let history = vec![
            Entry::user("go"),
            assistant_calling(&["tc_1"]),
            tool_result("tc_1", "   "),
        ];
        let (_, messages) = to_messages("", &history);

        let Block::ToolResult { content, .. } = &messages[2].content[0] else {
            panic!("expected a tool result");
        };
        assert_eq!(content, "(no output)");
    }

    // ── pairing ─────────────────────────────────────────────────────────

    /// A turn stopped mid-tool-round leaves an unanswered call, which would
    /// otherwise make every later request in the session fail.
    #[test]
    fn an_unanswered_call_gets_a_synthetic_answer() {
        let history = vec![Entry::user("go"), assistant_calling(&["tc_1"])];
        let (_, messages) = to_messages("", &history);

        assert_eq!(messages.len(), 3);
        let Block::ToolResult {
            tool_use_id,
            content,
            is_error,
        } = &messages[2].content[0]
        else {
            panic!("expected a synthesised answer, got {:?}", messages[2]);
        };
        assert_eq!(tool_use_id, "tc_1");
        assert!(content.contains("did not finish"));
        assert!(is_error);
    }

    #[test]
    fn only_some_calls_answered_means_the_rest_are_synthesised() {
        let history = vec![
            Entry::user("go"),
            assistant_calling(&["tc_1", "tc_2"]),
            tool_result("tc_1", "done"),
        ];
        let (_, messages) = to_messages("", &history);

        let ids: Vec<&str> = messages[2]
            .content
            .iter()
            .filter_map(|b| match b {
                Block::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"tc_1"));
        assert!(ids.contains(&"tc_2"));
        // The synthesised one comes first.
        assert_eq!(ids[0], "tc_2");
    }

    /// An answer to a call that is no longer in the window is refused as hard
    /// as a missing answer.
    #[test]
    fn an_orphaned_result_is_dropped() {
        let history = vec![
            Entry::user("go"),
            tool_result("tc_missing", "answer to nothing"),
        ];
        let (_, messages) = to_messages("", &history);

        let has_orphan = messages.iter().flat_map(|m| &m.content).any(
            |b| matches!(b, Block::ToolResult { tool_use_id, .. } if tool_use_id == "tc_missing"),
        );
        assert!(!has_orphan, "an orphaned result survived: {messages:?}");
    }

    /// The exact failure the pass ordering exists to prevent: an orphaned
    /// leading assistant call whose answer would otherwise end up as the first
    /// message in the request.
    #[test]
    fn a_leading_orphan_and_its_answer_are_both_removed() {
        let history = vec![
            assistant_calling(&["tc_1"]),
            tool_result("tc_1", "done"),
            Entry::user("now do this"),
        ];
        let (_, messages) = to_messages("", &history);

        assert_eq!(messages[0].role, Role::User);
        let first_is_a_result =
            matches!(messages[0].content.first(), Some(Block::ToolResult { .. }));
        assert!(
            !first_is_a_result,
            "the request opens with an unmatched tool_result: {messages:?}"
        );
    }

    #[test]
    fn a_duplicate_answer_is_dropped() {
        let history = vec![
            Entry::user("go"),
            assistant_calling(&["tc_1"]),
            tool_result("tc_1", "first"),
            tool_result("tc_1", "again"),
        ];
        let (_, messages) = to_messages("", &history);
        assert_eq!(messages[2].content.len(), 1);
    }

    // ── assistant block ordering ────────────────────────────────────────

    #[test]
    fn assistant_blocks_are_thinking_then_prose_then_calls() {
        let entry = AssistantEntry {
            content: Some("here goes".into()),
            tool_calls: vec![call("tc_1", "echo")],
            thinking: vec![ThinkingBlock::Thinking {
                thinking: "hmm".into(),
                signature: Some("sig".into()),
            }],
            ..Default::default()
        };
        let blocks = assistant_blocks(&entry);

        assert!(matches!(blocks[0], Block::Thinking { .. }));
        assert!(matches!(blocks[1], Block::Text { .. }));
        assert!(matches!(blocks[2], Block::ToolUse { .. }));
    }

    /// Unsigned thinking cannot be replayed, and sending it fails the request.
    #[test]
    fn unsigned_thinking_is_dropped() {
        let entry = AssistantEntry {
            content: Some("hi".into()),
            thinking: vec![ThinkingBlock::Thinking {
                thinking: "hmm".into(),
                signature: None,
            }],
            ..Default::default()
        };
        let blocks = assistant_blocks(&entry);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], Block::Text { .. }));
    }

    /// An assistant turn that was pure tool calls has no prose, and an empty
    /// text block is rejected.
    #[test]
    fn a_tool_only_turn_has_no_text_block() {
        let entry = AssistantEntry {
            content: Some("   ".into()),
            tool_calls: vec![call("tc_1", "echo")],
            ..Default::default()
        };
        let blocks = assistant_blocks(&entry);
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], Block::ToolUse { .. }));
    }

    #[test]
    fn malformed_call_arguments_become_an_empty_object() {
        let entry = AssistantEntry {
            tool_calls: vec![ToolCall {
                id: ToolCallId::from_existing("tc_1"),
                name: "echo".into(),
                arguments: "{oops".into(),
            }],
            ..Default::default()
        };
        let blocks = assistant_blocks(&entry);
        let Block::ToolUse { input, .. } = &blocks[0] else {
            panic!("expected a tool_use");
        };
        assert_eq!(input, &serde_json::json!({}));
    }

    // ── images ──────────────────────────────────────────────────────────

    #[test]
    fn a_data_url_becomes_a_base64_image() {
        let block = image_block("data:image/png;base64,AAAA").unwrap();
        let Block::Image {
            source: ImageSource::Base64 { media_type, data },
        } = block
        else {
            panic!("expected a base64 image");
        };
        assert_eq!(media_type, "image/png");
        assert_eq!(data, "AAAA");
    }

    /// Passed through rather than fetched and re-encoded - the provider can
    /// fetch it itself.
    #[test]
    fn an_http_url_is_passed_through() {
        let block = image_block("https://example.com/a.png").unwrap();
        assert!(matches!(
            block,
            Block::Image {
                source: ImageSource::Url { .. }
            }
        ));
    }

    #[test]
    fn an_unusable_image_is_dropped_rather_than_faked() {
        assert!(image_block("file:///local/a.png").is_none());
        assert!(image_block("data:image/png,notbase64").is_none());
        assert!(image_block("").is_none());
    }

    #[test]
    fn image_parts_and_text_parts_both_survive() {
        let history = vec![Entry::User(UserEntry {
            parts: Some(vec![
                Part::Text {
                    text: "look".into(),
                },
                Part::ImageUrl {
                    image_url: ImageUrl::Bare("data:image/png;base64,AAAA".into()),
                },
            ]),
            ..Default::default()
        })];
        let (_, messages) = to_messages("", &history);
        assert_eq!(messages[0].content.len(), 2);
    }

    // ── trailing whitespace ─────────────────────────────────────────────

    #[test]
    fn a_trailing_assistant_message_loses_its_trailing_whitespace() {
        let history = vec![Entry::user("hi"), Entry::assistant("bye   \n")];
        let (_, messages) = to_messages("", &history);
        let Block::Text { text } = messages.last().unwrap().content.last().unwrap() else {
            panic!("expected text");
        };
        assert_eq!(text, "bye");
    }

    // ── degenerate input ────────────────────────────────────────────────

    #[test]
    fn an_empty_history_produces_no_messages() {
        let (_, messages) = to_messages("be helpful", &[]);
        assert!(messages.is_empty());
    }

    #[test]
    fn a_history_of_only_assistant_turns_produces_nothing() {
        let history = vec![Entry::assistant("one"), Entry::assistant("two")];
        let (_, messages) = to_messages("", &history);
        assert!(messages.is_empty());
    }

    #[test]
    fn blank_user_messages_are_dropped() {
        let history = vec![Entry::user("   "), Entry::user("real")];
        let (_, messages) = to_messages("", &history);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].content[0],
            Block::Text {
                text: "real".into()
            }
        );
    }

    /// The invariant the whole module exists to guarantee. Whatever goes in,
    /// what comes out is a request the API will accept.
    #[test]
    fn the_output_is_always_well_formed() {
        let histories = vec![
            vec![Entry::user("hi")],
            vec![Entry::assistant("orphan"), Entry::user("hi")],
            vec![Entry::user("go"), assistant_calling(&["tc_1"])],
            vec![Entry::user("go"), tool_result("tc_x", "orphan")],
            vec![
                assistant_calling(&["tc_1"]),
                tool_result("tc_1", "done"),
                Entry::user("hi"),
            ],
        ];

        for history in histories {
            let (_, messages) = to_messages("system", &history);
            if messages.is_empty() {
                continue;
            }

            assert_eq!(
                messages[0].role,
                Role::User,
                "must open with a user turn: {messages:?}"
            );

            for pair in messages.windows(2) {
                assert_ne!(
                    pair[0].role, pair[1].role,
                    "adjacent same-role messages: {messages:?}"
                );
            }

            for message in &messages {
                assert!(!message.content.is_empty(), "empty message: {messages:?}");
                for block in &message.content {
                    if let Block::Text { text } = block {
                        assert!(!text.trim().is_empty(), "empty text block: {messages:?}");
                    }
                }
            }

            // Every call answered, every answer matched.
            for (i, message) in messages.iter().enumerate() {
                if message.role != Role::Assistant {
                    continue;
                }
                for id in message.asked_tool_ids() {
                    let answered = messages
                        .get(i + 1)
                        .map(|next| {
                            next.content.iter().any(|b| {
                                matches!(b, Block::ToolResult { tool_use_id, .. } if *tool_use_id == id)
                            })
                        })
                        .unwrap_or(false);
                    assert!(answered, "call {id} went unanswered: {messages:?}");
                }
            }
        }
    }
}

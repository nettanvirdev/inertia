//! One shared transcript, seen from one agent's seat.
//!
//! A group conversation has one transcript and several agents reading it, and
//! the mistake this module exists to stop is showing every agent's words to the
//! model as if the model had said them. Do that and a model in a room with two
//! colleagues sees three voices under its own role, learns that "this is what I
//! write", and starts writing all three - a whole scripted dialogue in one
//! reply, complete with the others' names in front of the lines. That is what
//! the first version of this feature did in the other shell, and it is exactly
//! what the weaker models produced.
//!
//! From any one seat the conversation has two sides: what I said (assistant)
//! and what was said to me (user). The person is on the second side and so is
//! every other agent, each line labelled with who said it, which is how a
//! messenger group looks to any one member of it. Tool calls another agent made
//! come across as what they did and what came back, in words, because that is
//! the work the room does not want done twice.
//!
//! A port of the main process's `team/perspective.cjs`.

use std::collections::{HashMap, HashSet};

use inertia_core::message::{AssistantEntry, Entry, Part, UserEntry};

/// How much of a colleague's tool output is worth carrying into another seat.
pub const RESULT_LIMIT: usize = 4000;

fn clip(text: &str, limit: usize) -> String {
    // Counted in characters rather than bytes: a cut that lands inside a
    // multi-byte character would panic, and the scripts this app sets are full
    // of them.
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= limit {
        return text.to_string();
    }
    let head: String = chars[..limit].iter().collect();
    format!(
        "{head}\n… ({} more characters not shown)",
        chars.len() - limit
    )
}

/// Who is who, for the labels.
#[derive(Debug, Clone, Default)]
pub struct Seat {
    /// The agent whose seat this is. Replies by anybody else become labelled
    /// lines addressed to them.
    pub me: Option<String>,
    /// Agent id to display name.
    pub names: HashMap<String, String>,
    /// What to call the human. Theirs is the one voice in the room whose word
    /// is final, and the label is how a model tells it from a colleague.
    pub person: String,
}

/// The one string a colleague's reply becomes.
fn said(name: &str, entry: &AssistantEntry, results: &HashMap<String, String>) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let text = entry.content.as_deref().unwrap_or_default().trim();
    if !text.is_empty() {
        lines.push(text.to_string());
    }
    for call in &entry.tool_calls {
        let args = call.arguments.trim();
        let args = if args.is_empty() || args == "{}" {
            String::new()
        } else {
            format!(" {}", clip(args, 300))
        };
        lines.push(format!("(ran {}{args})", call.name));
        if let Some(output) = results.get(call.id.as_str()) {
            let output = clip(output, RESULT_LIMIT);
            if !output.trim().is_empty() {
                lines.push(output);
            }
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(format!("{name}: {}", lines.join("\n")))
}

/// `history` as the agent in `seat` would have lived it.
///
/// Replies by the seat keep their role and their tools; every other reply
/// becomes a labelled user line with its tool results folded in, and runs of
/// user lines are joined into one message so no provider is asked to accept two
/// user turns in a row.
///
/// A reply with no agent id is treated as the seat's own - that is what a
/// single-agent transcript looks like, and a group that grew out of one should
/// read on.
pub fn perspective(history: Vec<Entry>, seat: &Seat) -> Vec<Entry> {
    // Results are looked up by call id, so a colleague's tool output can sit
    // next to the call that produced it rather than dangling after a message
    // that no longer has tool calls.
    let mut results: HashMap<String, String> = HashMap::new();
    for entry in &history {
        if let Entry::Tool(tool) = entry {
            results.insert(tool.tool_call_id.as_str().to_string(), tool.content.clone());
        }
    }

    let person = if seat.person.trim().is_empty() {
        "The person"
    } else {
        seat.person.trim()
    };

    let mut out: Vec<Entry> = Vec::new();
    let mut foreign: HashSet<String> = HashSet::new();
    let mut run: Vec<String> = Vec::new();

    macro_rules! flush {
        () => {
            if !run.is_empty() {
                out.push(Entry::user(run.join("\n\n")));
                run.clear();
            }
        };
    }

    for entry in history {
        match entry {
            Entry::User(user) => {
                // A message that is images or files as well as words keeps its
                // parts; the label goes on the text so the model still knows
                // who is talking.
                if let Some(parts) = user.parts.clone() {
                    flush!();
                    out.push(Entry::User(UserEntry {
                        parts: Some(label_parts(parts, person)),
                        ..user
                    }));
                    continue;
                }
                run.push(format!(
                    "{person}: {}",
                    user.content.as_deref().unwrap_or_default()
                ));
            }

            Entry::Assistant(reply) => {
                let who = reply.agent_id.clone().or_else(|| seat.me.clone());
                if seat.me.is_none() || who == seat.me {
                    flush!();
                    // The seat's own words, with the labels the window attached
                    // taken off: they were for this transform, not for the
                    // model, and a reply prefixed with its own author's name
                    // teaches the model to prefix its next one.
                    out.push(Entry::Assistant(AssistantEntry {
                        agent_id: None,
                        name: None,
                        ..reply
                    }));
                    continue;
                }
                for call in &reply.tool_calls {
                    foreign.insert(call.id.as_str().to_string());
                }
                let name = who
                    .as_ref()
                    .and_then(|id| seat.names.get(id))
                    .cloned()
                    .or_else(|| reply.name.clone())
                    .or(who)
                    .unwrap_or_else(|| "Someone".into());
                if let Some(line) = said(&name, &reply, &results) {
                    run.push(line);
                }
            }

            Entry::Tool(tool) => {
                // A colleague's result has already been folded into their line.
                if foreign.contains(tool.tool_call_id.as_str()) {
                    continue;
                }
                flush!();
                out.push(Entry::Tool(tool));
            }
        }
    }
    flush!();
    out
}

/// The person's label, on the first piece of text in a multi-part message.
fn label_parts(parts: Vec<Part>, person: &str) -> Vec<Part> {
    let first = parts
        .iter()
        .position(|part| matches!(part, Part::Text { .. }));

    match first {
        None => {
            let mut labelled = vec![Part::Text {
                text: format!("{person}:"),
            }];
            labelled.extend(parts);
            labelled
        }
        Some(index) => parts
            .into_iter()
            .enumerate()
            .map(|(i, part)| match part {
                Part::Text { text } if i == index => Part::Text {
                    text: format!("{person}: {text}"),
                },
                other => other,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::ToolCallId;
    use inertia_core::message::ToolCall;

    fn seat(me: &str) -> Seat {
        Seat {
            me: Some(me.into()),
            names: [
                ("a".to_string(), "Iris".to_string()),
                ("b".to_string(), "Atlas".to_string()),
            ]
            .into_iter()
            .collect(),
            person: "Tanvir (the person)".into(),
        }
    }

    fn by(agent: &str, text: &str) -> Entry {
        Entry::Assistant(AssistantEntry {
            content: Some(text.into()),
            agent_id: Some(agent.into()),
            name: None,
            ..Default::default()
        })
    }

    fn text_of(entry: &Entry) -> &str {
        entry.text()
    }

    #[test]
    fn my_own_replies_stay_mine_and_lose_the_label() {
        let out = perspective(vec![Entry::user("hi"), by("a", "Hello")], &seat("a"));
        assert!(matches!(out[1], Entry::Assistant(_)));
        let Entry::Assistant(reply) = &out[1] else {
            panic!("expected an assistant entry")
        };
        assert_eq!(reply.agent_id, None);
        assert_eq!(reply.content.as_deref(), Some("Hello"));
    }

    #[test]
    fn a_colleagues_reply_becomes_something_that_was_said_to_me() {
        let out = perspective(vec![Entry::user("hi"), by("b", "Hello")], &seat("a"));
        // One user message, not two: no provider takes two user turns in a row.
        assert_eq!(out.len(), 1);
        assert_eq!(
            text_of(&out[0]),
            "Tanvir (the person): hi\n\nAtlas: Hello"
        );
    }

    #[test]
    fn a_colleagues_tool_call_comes_across_as_what_they_did_and_what_came_back() {
        let call = ToolCall {
            id: ToolCallId::from_existing("tc_1"),
            name: "grep".into(),
            arguments: r#"{"pattern":"TODO"}"#.into(),
        };
        let history = vec![
            Entry::Assistant(AssistantEntry {
                content: Some("Let me look.".into()),
                tool_calls: vec![call],
                agent_id: Some("b".into()),
                ..Default::default()
            }),
            Entry::tool_result(ToolCallId::from_existing("tc_1"), "three matches", true),
        ];

        let out = perspective(history, &seat("a"));
        assert_eq!(out.len(), 1);
        let line = text_of(&out[0]);
        assert!(line.starts_with("Atlas: Let me look."), "{line}");
        assert!(line.contains(r#"(ran grep {"pattern":"TODO"})"#), "{line}");
        assert!(line.contains("three matches"), "{line}");
    }

    #[test]
    fn my_own_tool_results_are_left_where_they_are() {
        let call = ToolCall {
            id: ToolCallId::from_existing("tc_1"),
            name: "read".into(),
            arguments: "{}".into(),
        };
        let history = vec![
            Entry::Assistant(AssistantEntry {
                tool_calls: vec![call],
                agent_id: Some("a".into()),
                ..Default::default()
            }),
            Entry::tool_result(ToolCallId::from_existing("tc_1"), "file body", true),
        ];

        let out = perspective(history, &seat("a"));
        assert_eq!(out.len(), 2);
        assert!(matches!(out[1], Entry::Tool(_)));
    }

    #[test]
    fn the_person_is_named_on_every_line_they_said() {
        let out = perspective(vec![Entry::user("do it")], &seat("a"));
        assert_eq!(text_of(&out[0]), "Tanvir (the person): do it");
    }

    #[test]
    fn a_reply_with_no_author_reads_as_mine() {
        let out = perspective(vec![Entry::assistant("from before the room")], &seat("a"));
        assert!(matches!(out[0], Entry::Assistant(_)));
    }

    #[test]
    fn a_message_with_images_keeps_its_parts_and_gains_the_label() {
        let history = vec![Entry::User(UserEntry {
            parts: Some(vec![
                Part::Text {
                    text: "look at this".into(),
                },
                Part::ImageUrl {
                    image_url: inertia_core::message::ImageUrl::Bare("data:,x".into()),
                },
            ]),
            ..Default::default()
        })];

        let out = perspective(history, &seat("a"));
        let Entry::User(user) = &out[0] else {
            panic!("expected a user entry")
        };
        let parts = user.parts.as_ref().expect("parts were dropped");
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], Part::Text { text } if text == "Tanvir (the person): look at this"));
        assert!(matches!(parts[1], Part::ImageUrl { .. }));
    }

    #[test]
    fn a_long_colleague_result_is_clipped_rather_than_carried_whole() {
        let call = ToolCall {
            id: ToolCallId::from_existing("tc_1"),
            name: "read".into(),
            arguments: "{}".into(),
        };
        let history = vec![
            Entry::Assistant(AssistantEntry {
                tool_calls: vec![call],
                agent_id: Some("b".into()),
                ..Default::default()
            }),
            Entry::tool_result(
                ToolCallId::from_existing("tc_1"),
                "x".repeat(RESULT_LIMIT + 500),
                true,
            ),
        ];

        let out = perspective(history, &seat("a"));
        assert!(text_of(&out[0]).contains("500 more characters not shown"));
    }
}

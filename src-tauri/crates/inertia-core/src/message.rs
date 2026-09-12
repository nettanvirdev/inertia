//! The transcript: one message format, regardless of provider.
//!
//! The app keeps a single canonical shape and each provider translates to and
//! from its own wire format at the edge. That is what lets a conversation
//! started against Anthropic continue against OpenAI without rewriting
//! history.
//!
//! The field names here are the ones already on disk in existing workspaces,
//! so they are not up for redesign - `toolCallId`, not `tool_call_id`. Where
//! the original shape permits states that should not exist, the fields stay
//! faithful and the constructors and accessors enforce the invariant instead.

use serde::{Deserialize, Serialize};

use crate::id::ToolCallId;

fn is_false(b: &bool) -> bool {
    !*b
}

/// One entry in a conversation transcript.
///
/// Discriminated on `role`. `agent` is accepted as a synonym for `assistant`:
/// older transcripts wrote it and it never meant anything different.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Entry {
    User(UserEntry),
    #[serde(alias = "agent")]
    Assistant(AssistantEntry),
    Tool(ToolEntry),
}

impl Entry {
    /// A plain text message from the person using the app.
    pub fn user(text: impl Into<String>) -> Self {
        Self::User(UserEntry {
            content: Some(text.into()),
            parts: None,
            pinned: false,
            compacted: false,
        })
    }

    /// A message from the model.
    pub fn assistant(text: impl Into<String>) -> Self {
        Self::Assistant(AssistantEntry {
            content: Some(text.into()),
            ..Default::default()
        })
    }

    /// The answer to exactly one tool call.
    pub fn tool_result(id: ToolCallId, output: impl Into<String>, ok: bool) -> Self {
        Self::Tool(ToolEntry {
            tool_call_id: id,
            content: output.into(),
            ok: Some(ok),
            pruned: false,
        })
    }

    /// The prose in this entry, if any. Ignores images, tool calls and
    /// thinking - this is what a plain-text preview or a title generator
    /// wants.
    pub fn text(&self) -> &str {
        match self {
            Self::User(e) => e.text(),
            Self::Assistant(e) => e.content.as_deref().unwrap_or_default(),
            Self::Tool(e) => &e.content,
        }
    }

    /// Whether compaction is allowed to drop or shorten this entry.
    ///
    /// Pinned entries survive compaction; the summary that *replaces* compacted
    /// history is itself pinned, which is what stops the summary being
    /// summarised away on the next pass.
    pub fn is_pinned(&self) -> bool {
        matches!(self, Self::User(e) if e.pinned)
    }
}

/// A turn from the user.
///
/// Exactly one of `content` or `parts` carries the body. `parts` appears only
/// when there are images, which in practice means a tool produced them and the
/// model asked to look.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<Part>>,
    /// Survives context compaction.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// This entry *is* a compaction summary, not something the user typed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub compacted: bool,
}

impl UserEntry {
    /// The prose, whether it arrived as `content` or as text parts.
    pub fn text(&self) -> &str {
        if let Some(content) = &self.content {
            return content;
        }
        self.parts
            .as_ref()
            .and_then(|parts| {
                parts.iter().find_map(|p| match p {
                    Part::Text { text } => Some(text.as_str()),
                    Part::ImageUrl { .. } => None,
                })
            })
            .unwrap_or_default()
    }

    pub fn has_images(&self) -> bool {
        self.parts.as_ref().is_some_and(|parts| {
            parts.iter().any(|p| matches!(p, Part::ImageUrl { .. }))
        })
    }
}

/// A piece of a multi-part user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    ImageUrl {
        image_url: ImageUrl,
    },
}

/// An image reference, which older transcripts wrote as a bare string and
/// newer ones write as an object. Both must keep loading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImageUrl {
    Object { url: String },
    Bare(String),
}

impl ImageUrl {
    pub fn url(&self) -> &str {
        match self {
            Self::Object { url } => url,
            Self::Bare(url) => url,
        }
    }
}

/// A turn from the model.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Which agent wrote this, in a conversation several of them share.
    ///
    /// Absent everywhere else, and absent is the honest answer there: a
    /// single-agent transcript has one voice and labelling it would only teach
    /// the model to write the label. Carried on the entry rather than written
    /// into the text so the turn can be seen from any seat without the previous
    /// seat's name being parsed back out of the prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// That agent's display name, as the window had it. Sent alongside the id
    /// so a reply by an agent since deleted still reads as somebody.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Carried purely so it can be replayed verbatim on the next request.
    /// Anthropic signs these and refuses to continue its own reasoning across
    /// a tool call if they come back altered or missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking: Vec<ThinkingBlock>,
}

/// The model asking for a tool to run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    /// Kept as the raw JSON string the model emitted rather than a parsed
    /// value. It is not guaranteed to be valid JSON, and the point at which
    /// that is discovered should be the point at which the tool can complain
    /// about it - which is something the model can then correct.
    pub arguments: String,
}

impl ToolCall {
    /// Parses the arguments, falling back to an empty object.
    ///
    /// A model's malformed JSON becomes a call with no arguments rather than
    /// an aborted request: the tool then reports what it is missing, and the
    /// model gets a chance to fix it. Failing the whole turn here would just
    /// show the user a parse error they cannot act on.
    pub fn parsed_arguments(&self) -> serde_json::Value {
        serde_json::from_str(&self.arguments)
            .ok()
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()))
    }
}

/// A block of model reasoning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThinkingBlock {
    Thinking {
        thinking: String,
        /// Absent on blocks that were never signed. An unsigned block cannot
        /// be replayed - the API verifies the signature - so it is dropped at
        /// translation time rather than failing the request.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    RedactedThinking {
        data: String,
    },
}

impl ThinkingBlock {
    /// Whether this block can be sent back to the provider.
    pub fn is_replayable(&self) -> bool {
        match self {
            Self::Thinking { signature, .. } => {
                signature.as_ref().is_some_and(|s| !s.is_empty())
            }
            Self::RedactedThinking { .. } => true,
        }
    }
}

/// The result of one tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolEntry {
    pub tool_call_id: ToolCallId,
    pub content: String,
    /// `Some(false)` marks a failure. **Absent means unknown, not success** -
    /// transcripts written before this field existed have no opinion, and
    /// treating them as successes would silently relabel old errors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    /// The body was shortened by context compaction.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pruned: bool,
}

impl ToolEntry {
    /// True only for a result explicitly recorded as a failure.
    pub fn is_error(&self) -> bool {
        self.ok == Some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_entry_round_trips() {
        let entry = Entry::user("hello");
        let json = serde_json::to_string(&entry).unwrap();
        assert_eq!(json, r#"{"role":"user","content":"hello"}"#);
        assert_eq!(serde_json::from_str::<Entry>(&json).unwrap(), entry);
    }

    // Old transcripts wrote `agent`; it was never a distinct role.
    #[test]
    fn agent_reads_as_assistant() {
        let entry: Entry =
            serde_json::from_str(r#"{"role":"agent","content":"hi"}"#).unwrap();
        assert!(matches!(entry, Entry::Assistant(_)));
        assert_eq!(entry.text(), "hi");
        // ...and is rewritten under the canonical name.
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""role":"assistant""#));
    }

    #[test]
    fn tool_entries_use_the_on_disk_field_name() {
        let entry = Entry::tool_result(ToolCallId::from_existing("tc_1"), "done", true);
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains(r#""toolCallId":"tc_1""#), "got {json}");
    }

    // The distinction that stops old errors being relabelled as successes.
    #[test]
    fn an_absent_ok_is_not_a_success() {
        let legacy: ToolEntry =
            serde_json::from_str(r#"{"toolCallId":"tc_1","content":"out"}"#).unwrap();
        assert_eq!(legacy.ok, None);
        assert!(!legacy.is_error());

        let failed = ToolEntry {
            ok: Some(false),
            ..legacy.clone()
        };
        assert!(failed.is_error());
    }

    #[test]
    fn malformed_tool_arguments_become_an_empty_object() {
        let call = ToolCall {
            id: ToolCallId::from_existing("tc_1"),
            name: "read".into(),
            arguments: "{not json".into(),
        };
        assert_eq!(call.parsed_arguments(), serde_json::json!({}));
    }

    // A bare JSON array is valid JSON but not a valid argument object.
    #[test]
    fn non_object_tool_arguments_become_an_empty_object() {
        let call = ToolCall {
            id: ToolCallId::from_existing("tc_1"),
            name: "read".into(),
            arguments: "[1,2,3]".into(),
        };
        assert_eq!(call.parsed_arguments(), serde_json::json!({}));
    }

    #[test]
    fn well_formed_tool_arguments_parse() {
        let call = ToolCall {
            id: ToolCallId::from_existing("tc_1"),
            name: "read".into(),
            arguments: r#"{"path":"a.txt"}"#.into(),
        };
        assert_eq!(call.parsed_arguments()["path"], "a.txt");
    }

    #[test]
    fn unsigned_thinking_is_not_replayable() {
        let unsigned = ThinkingBlock::Thinking {
            thinking: "hmm".into(),
            signature: None,
        };
        assert!(!unsigned.is_replayable());

        let signed = ThinkingBlock::Thinking {
            thinking: "hmm".into(),
            signature: Some("sig".into()),
        };
        assert!(signed.is_replayable());

        // Redacted blocks carry no readable text but are still replayable.
        assert!(ThinkingBlock::RedactedThinking { data: "x".into() }.is_replayable());
    }

    #[test]
    fn images_load_from_both_spellings() {
        let object: Part =
            serde_json::from_str(r#"{"type":"image_url","image_url":{"url":"data:x"}}"#)
                .unwrap();
        let bare: Part =
            serde_json::from_str(r#"{"type":"image_url","image_url":"data:x"}"#).unwrap();
        for part in [object, bare] {
            match part {
                Part::ImageUrl { image_url } => assert_eq!(image_url.url(), "data:x"),
                Part::Text { .. } => panic!("expected an image part"),
            }
        }
    }

    #[test]
    fn a_parts_entry_reports_its_text_and_images() {
        let entry = UserEntry {
            parts: Some(vec![
                Part::Text {
                    text: "look at this".into(),
                },
                Part::ImageUrl {
                    image_url: ImageUrl::Bare("data:x".into()),
                },
            ]),
            ..Default::default()
        };
        assert_eq!(entry.text(), "look at this");
        assert!(entry.has_images());
    }

    // The summary that replaces compacted history is pinned so that the next
    // compaction pass cannot summarise the summary.
    #[test]
    fn a_compaction_summary_is_pinned() {
        let summary = Entry::User(UserEntry {
            content: Some("earlier: ...".into()),
            pinned: true,
            compacted: true,
            ..Default::default()
        });
        assert!(summary.is_pinned());
    }

    // Absent optional fields must not be written back, or every load-and-save
    // cycle grows the file with nulls and false.
    #[test]
    fn empty_fields_are_omitted_on_write() {
        let json = serde_json::to_string(&Entry::assistant("hi")).unwrap();
        assert_eq!(json, r#"{"role":"assistant","content":"hi"}"#);
    }
}

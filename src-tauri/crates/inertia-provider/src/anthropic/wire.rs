//! The Anthropic Messages wire shapes.
//!
//! Typed rather than `serde_json::Value` built by hand: the block ordering
//! rules and the optional-field rules below are enforced by the API, and a
//! typo in a string key is a 400 at runtime instead of a compile error.

use serde::{Deserialize, Serialize};

fn is_false(b: &bool) -> bool {
    !*b
}

/// One content block. The API is strict about which of these may appear where,
/// and in what order - see `assistant_blocks` in the translator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    /// Replayed verbatim from a previous turn. The signature is checked, so a
    /// block cannot be edited or synthesised.
    Thinking {
        thinking: String,
        signature: String,
    },
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "is_false")]
        is_error: bool,
    },
    Image {
        source: ImageSource,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImageSource {
    Base64 { media_type: String, data: String },
    /// A URL the provider fetches itself, saving a round trip through us.
    Url { url: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user(content: Vec<Block>) -> Self {
        Self {
            role: Role::User,
            content,
        }
    }

    pub fn assistant(content: Vec<Block>) -> Self {
        Self {
            role: Role::Assistant,
            content,
        }
    }

    /// The `tool_use` ids this message asks to have answered.
    pub fn asked_tool_ids(&self) -> Vec<String> {
        self.content
            .iter()
            .filter_map(|b| match b {
                Block::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect()
    }
}

/// A tool as Anthropic describes it. Note `input_schema`, not `parameters`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Thinking {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub budget_tokens: u32,
}

/// The request body.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub model: String,
    /// Required by this API, unlike OpenAI's optional field.
    pub max_tokens: u32,
    /// An array rather than a string, so a cache breakpoint has somewhere to
    /// hang. Empty when there is no system prompt.
    pub system: Vec<Block>,
    pub messages: Vec<Message>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Thinking>,
    // Both are omitted entirely when thinking is enabled - the API refuses the
    // combination rather than ignoring them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_serialise_with_their_tag() {
        let json = serde_json::to_value(Block::Text {
            text: "hi".into(),
        })
        .unwrap();
        assert_eq!(json["type"], "text");

        let json = serde_json::to_value(Block::RedactedThinking { data: "x".into() }).unwrap();
        assert_eq!(json["type"], "redacted_thinking");
    }

    /// `is_error` is absent on a successful result, not `false`. The API
    /// tolerates both, but matching the documented shape keeps request diffs
    /// readable when debugging a rejection.
    #[test]
    fn a_successful_tool_result_omits_is_error() {
        let json = serde_json::to_string(&Block::ToolResult {
            tool_use_id: "tc_1".into(),
            content: "done".into(),
            is_error: false,
        })
        .unwrap();
        assert!(!json.contains("is_error"), "got {json}");
    }

    #[test]
    fn a_tool_uses_input_schema_not_parameters() {
        let json = serde_json::to_value(Tool {
            name: "read".into(),
            description: "reads".into(),
            input_schema: serde_json::json!({"type": "object"}),
        })
        .unwrap();
        assert!(json.get("input_schema").is_some());
        assert!(json.get("parameters").is_none());
    }

    #[test]
    fn an_image_carries_its_source_kind() {
        let json = serde_json::to_value(Block::Image {
            source: ImageSource::Base64 {
                media_type: "image/png".into(),
                data: "abc".into(),
            },
        })
        .unwrap();
        assert_eq!(json["source"]["type"], "base64");
        assert_eq!(json["source"]["media_type"], "image/png");
    }
}

//! The seam the turn loop calls at the named moments of its work.
//!
//! This exists so that lifecycle hooks - the user's own programs, run before a
//! tool, after one, or when the model wants to stop - can change what a turn
//! does without the agent loop knowing that hooks exist. The loop calls these
//! methods; whether anything is listening, and what it runs, is the app's
//! business.
//!
//! **Nothing here returns an error.** A hook is the user's own program and its
//! failure is theirs to see, not the turn's to die of; an implementation that
//! cannot run reports [`Reaction::default`], which is "carry on".

use crate::{ToolCall, ToolResult};
use serde_json::Value;
use std::fmt::Debug;

/// What a listener wants the turn to do differently.
#[derive(Debug, Clone, Default)]
pub struct Reaction {
    /// Do not proceed. On a tool call this refuses it; on a stop it sends the
    /// model back round. Honoured only where the loop says it can be.
    pub block: bool,
    /// Why, written for the model - it is what the model reads next.
    pub reason: Option<String>,
    /// Text to hand the model without refusing anything.
    pub context: Vec<String>,
    /// Text for the person watching, shown in the transcript.
    pub message: Option<String>,
    /// Replacement arguments for a tool call that is about to run.
    pub updated_input: Option<Value>,
}

impl Reaction {
    /// Whether this reaction asks the loop to do anything at all. The common
    /// case - no listener, or nothing matched - is that it does not.
    pub fn is_quiet(&self) -> bool {
        !self.block
            && self.context.is_empty()
            && self.message.is_none()
            && self.updated_input.is_none()
    }
}

/// Something that watches a turn and may interrupt it.
#[async_trait::async_trait]
pub trait Lifecycle: Send + Sync + Debug {
    /// A tool is about to run. May refuse it, or rewrite its arguments.
    async fn pre_tool(&self, _call: &ToolCall) -> Reaction {
        Reaction::default()
    }

    /// A tool has run. May hand the model more context, or tell it the result
    /// is not acceptable and to try again.
    async fn post_tool(&self, _result: &ToolResult) -> Reaction {
        Reaction::default()
    }

    /// The model wants to stop. May send it back round with a reason - which
    /// is what "do not stop until the tests pass" is made of.
    async fn stopping(&self) -> Reaction {
        Reaction::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_reaction_asks_for_nothing() {
        assert!(Reaction::default().is_quiet());
    }

    #[test]
    fn any_of_the_four_makes_it_loud() {
        let loud = [
            Reaction {
                block: true,
                ..Default::default()
            },
            Reaction {
                context: vec!["x".into()],
                ..Default::default()
            },
            Reaction {
                message: Some("x".into()),
                ..Default::default()
            },
            Reaction {
                updated_input: Some(Value::Null),
                ..Default::default()
            },
        ];
        assert!(loud.iter().all(|reaction| !reaction.is_quiet()));
    }
}

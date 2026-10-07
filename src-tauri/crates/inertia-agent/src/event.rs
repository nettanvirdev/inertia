//! What a turn reports while it runs.
//!
//! This is the contract the UI renders from, so the tag spellings are load
//! bearing. It is deliberately a superset of the provider's own event stream:
//! the loop forwards prose and reasoning through unchanged, and adds the
//! things only it knows about - which step we are on, which tool is running,
//! and how the turn ended.

use inertia_core::message::{Entry, ToolCall};
use inertia_core::provider::Usage;
use inertia_core::tool::ToolResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentEvent {
    /// The turn has begun.
    Start { model: String },

    /// A step is one provider call plus any tools it asked for. Steps are
    /// 1-based, because they are shown to a person.
    Step { step: u32 },

    /// Prose, as it arrives.
    Delta { text: String },

    /// Reasoning the user may watch.
    Reasoning { text: String },

    /// A tool is about to run. `title` is the human label, already resolved.
    ToolStarted {
        call: ToolCall,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },

    /// A tool finished, successfully or not.
    ToolFinished { result: ToolResult },

    /// Something was worked around silently. Worth showing; not a failure.
    Notice { message: String },

    /// Something the person said while the turn was still running, at the
    /// moment it reached the model. Reported so the transcript can show it
    /// where it belongs - part of this reply, not a message of its own that
    /// would have to sort itself against a reply still being written.
    Steered { text: String },

    /// The turn is over.
    Done {
        stopped: StopReason,
        /// The full transcript including everything this turn added, for the
        /// caller to persist. Handed over rather than written here because the
        /// loop has no opinion about storage.
        history: Vec<Entry>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
}

/// Why a turn ended. Every path out of the loop names itself here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "camelCase")]
pub enum StopReason {
    /// The model finished and asked for nothing further. The ordinary ending.
    Complete,
    /// The step budget ran out. The model was given one final tool-less turn
    /// to say where it got to, so the user is not left mid-sentence.
    MaxSteps,
    /// The provider failed in a way retrying would not fix.
    Error { message: String },
    /// The user stopped it.
    Cancelled,
    /// A tool call was refused and the model had nothing else to do.
    Refused,
}

impl StopReason {
    /// Whether this ending is one to report as a failure. Cancellation is not:
    /// the user did it on purpose.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Error { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reasons_carry_their_tag() {
        let json = serde_json::to_value(StopReason::Complete).unwrap();
        assert_eq!(json["reason"], "complete");

        let json = serde_json::to_value(StopReason::Error {
            message: "boom".into(),
        })
        .unwrap();
        assert_eq!(json["reason"], "error");
        assert_eq!(json["message"], "boom");
    }

    #[test]
    fn cancelling_is_not_a_failure() {
        assert!(!StopReason::Cancelled.is_failure());
        assert!(!StopReason::Complete.is_failure());
        assert!(StopReason::Error {
            message: "x".into()
        }
        .is_failure());
    }

    #[test]
    fn events_carry_camel_case_fields() {
        let event = AgentEvent::ToolStarted {
            call: ToolCall {
                id: inertia_core::ToolCallId::from_existing("tc_1"),
                name: "read".into(),
                arguments: "{}".into(),
            },
            title: Some("Reading a.txt".into()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "toolStarted");
        assert_eq!(json["call"]["name"], "read");
    }
}

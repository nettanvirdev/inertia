//! The shared vocabulary of the Inertia backend.
//!
//! Two things live here and nothing else:
//!
//!   - **Domain types** - messages, content blocks, tool specs, sessions. The
//!     nouns every other crate passes around.
//!   - **Trait seams** - `Provider`, `Tool`, `ToolProvider`, `Store`,
//!     `PermissionGate`, `Clock`. The verbs, each with a real implementation
//!     somewhere in `crates/` and a fake in `inertia-mock`.
//!
//! There is no I/O here: no HTTP client, no filesystem, no async runtime. That
//! is not austerity for its own sake - every dependency this crate takes on is
//! inherited by the entire workspace, and every type from one of them that
//! appears in a signature becomes part of the seam. Keeping it thin is what
//! keeps the seams swappable.

// `unwrap` in a test is a readable assertion, not a latent panic. The lint is
// aimed at the code that ships.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod clock;
pub mod error;
pub mod id;
pub mod lifecycle;
pub mod message;
pub mod permission;
pub mod provider;
pub mod tool;

pub use clock::{Clock, FixedClock, SystemClock};
pub use error::{Error, Result};
pub use lifecycle::{Lifecycle, Reaction};
pub use id::{MessageId, SessionId, ToolCallId, TurnId};
pub use message::{
    AssistantEntry, Entry, ImageUrl, Part, ThinkingBlock, ToolCall, ToolEntry, UserEntry,
};
pub use permission::{Action, Rule, Verdict};
pub use provider::{
    ChatRequest, FinishReason, ModelInfo, Provider, StreamEvent, ToolSpec, Usage,
};
pub use tool::{
    Decision, PermissionGate, PermissionRequest, ProviderProblem, Tool, ToolContext, ToolOutcome,
    ToolProvider, ToolRegistry, ToolResult, ToolSource,
};

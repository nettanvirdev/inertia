//! The tool seams.
//!
//! Three traits, because there are three genuinely different jobs:
//!
//!   - [`Tool`] - one callable thing. Implemented by builtins, and by adapters
//!     wrapping an MCP server's tool, an OpenAPI operation, a Composio action.
//!   - [`ToolProvider`] - a *source* of tools that vary at runtime. An MCP
//!     server that reconnects offers a different list than it did a minute
//!     ago; a builtin never does.
//!   - [`ToolRegistry`] - what the agent loop actually holds. It flattens
//!     every source into one list, and owns the call pipeline: validate,
//!     ask permission, execute, cap the output.
//!
//! The agent loop sees only [`ToolRegistry`]. It cannot tell an MCP tool from
//! a builtin, which is the entire point - and it cannot skip the permission
//! check, because it never gets a handle on a [`Tool`] to call directly.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::id::{SessionId, ToolCallId};
use crate::message::ToolCall;
use crate::permission::{Action, Shape};
use crate::provider::ToolSpec;

/// Where a tool came from. Reported to the UI, which badges them differently,
/// and used to group them in settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolSource {
    Builtin,
    Mcp,
    OpenApi,
    Composio,
}

impl ToolSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Mcp => "mcp",
            Self::OpenApi => "openapi",
            Self::Composio => "composio",
        }
    }
}

/// What a tool wants permission to do, for this particular set of arguments.
///
/// `target` is the string permission rules are written against, so it has to
/// be the thing a person would naturally write a rule about: the command for a
/// shell tool, the path for a file tool. Getting this wrong makes rules
/// unwritable even though the engine works perfectly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    /// Which permission key this falls under, e.g. `"shell"`, `"edit"`.
    /// Several tools can share one key when a person would think of them as
    /// one capability.
    pub key: String,
    /// The specific thing being acted on.
    pub target: String,
    /// What an "always allow" click should remember. Usually broader than
    /// `target` - approving `git status` once should not mean approving only
    /// that exact string forever - and `None` when the action is too varied to
    /// generalise safely.
    pub always: Option<String>,
    /// What `target` is made of, when it is more than one action. Every part
    /// of a chained shell line needs a rule of its own.
    pub shape: Shape,
}

impl PermissionRequest {
    pub fn new(key: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            target: target.into(),
            always: None,
            shape: Shape::Whole,
        }
    }

    pub fn with_always(mut self, always: impl Into<String>) -> Self {
        self.always = Some(always.into());
        self
    }

    pub fn with_shape(mut self, shape: Shape) -> Self {
        self.shape = shape;
        self
    }
}

/// What a tool produced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutcome {
    /// A short label for the UI, e.g. the file that was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The text handed back to the model.
    pub output: String,
    /// Structured extras for the UI - a diff, a row count, an exit code.
    /// Never shown to the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Images produced by the tool, as `data:` or `http(s)` URLs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

impl ToolOutcome {
    pub fn text(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            ..Default::default()
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// A finished tool call, successful or not.
///
/// There is no `Err` variant of this at the registry boundary, and that is
/// deliberate: a tool that fails has *produced a result* - an error message
/// the model should read and react to. Propagating it as an error would end
/// the turn over something the model could have fixed by trying a different
/// path. The exception is cancellation, which is an error, because there is
/// nobody left to read the result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: ToolCallId,
    pub tool: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    pub duration_ms: u64,
}

/// Marker written into [`ToolResult::metadata`] when a call was refused by the
/// permission gate, as opposed to failing on its own merits.
///
/// The distinction matters to the agent loop: a tool that failed is something
/// the model can try differently, while a tool that was refused will be
/// refused again, and looping on it just asks the user the same question
/// repeatedly.
pub const ERROR_DENIED: &str = "denied";

/// Marker written into [`ToolResult::metadata`] when arguments failed schema
/// validation and the tool never ran.
pub const ERROR_INVALID_ARGUMENTS: &str = "invalid-arguments";

impl ToolResult {
    /// Whether this call was refused rather than attempted.
    pub fn was_denied(&self) -> bool {
        self.metadata
            .as_ref()
            .and_then(|m| m.get("error"))
            .and_then(|v| v.as_str())
            == Some(ERROR_DENIED)
    }

    /// A failure the model should read and respond to.
    pub fn failed(
        call_id: ToolCallId,
        tool: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            call_id,
            tool: tool.into(),
            ok: false,
            title: None,
            output: message.into(),
            metadata: None,
            images: Vec::new(),
            duration_ms: 0,
        }
    }
}

/// What a tool is given when it runs.
#[derive(Clone)]
pub struct ToolContext {
    /// The workspace root. Tools resolve relative paths against this and must
    /// refuse to escape it unless explicitly permitted.
    pub root: PathBuf,
    /// The conversation this call belongs to.
    pub session: SessionId,
    /// The call being executed.
    pub call_id: ToolCallId,
    /// Asks a person. Already consulted by the registry before `execute` runs;
    /// held here for the rare tool that needs a second decision partway
    /// through.
    pub permissions: Arc<dyn PermissionGate>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("root", &self.root)
            .field("session", &self.session)
            .field("call_id", &self.call_id)
            .finish_non_exhaustive()
    }
}

/// One callable tool.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique across every source. This is the name the model calls.
    fn id(&self) -> &str;

    /// Read by the model to decide whether to call this. Product-critical
    /// text, not documentation.
    fn description(&self) -> &str;

    /// JSON Schema for the arguments. Always an object schema.
    fn parameters(&self) -> serde_json::Value;

    fn source(&self) -> ToolSource;

    /// What this call needs permission for. Takes the arguments because the
    /// answer depends on them: reading one file is not reading another.
    fn permission(&self, args: &serde_json::Value) -> PermissionRequest;

    /// A label shown while the call runs, e.g. `Reading src/main.rs`.
    fn render(&self, _args: &serde_json::Value) -> Option<String> {
        None
    }

    /// A chance to repair arguments before schema validation.
    ///
    /// Models get close but not exact - a string where an array was asked
    /// for, a path with the workspace prefix already attached. Fixing it here
    /// costs nothing; rejecting it costs a whole round trip. Must never fail:
    /// anything it cannot repair it returns unchanged, for the validator to
    /// report properly.
    fn normalize(&self, args: serde_json::Value) -> serde_json::Value {
        args
    }

    /// Does the work.
    ///
    /// Returning `Err` is for failures the model cannot act on. Anything it
    /// *can* act on - a bad path, a command that exited non-zero - is a
    /// successful call with unhappy output, and belongs in [`ToolOutcome`].
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> crate::Result<ToolOutcome>;
}

/// A source of tools whose membership changes at runtime.
#[async_trait]
pub trait ToolProvider: Send + Sync + std::fmt::Debug {
    /// Name used when reporting that this source is broken, e.g. `"mcp"`.
    fn name(&self) -> &'static str;

    /// The tools currently on offer.
    ///
    /// Called on roughly every turn, so it must be cheap: read cached state,
    /// never touch the network or spawn a process. Refreshing is the
    /// provider's own business, done in the background.
    async fn tools(&self, root: &Path) -> crate::Result<Vec<Arc<dyn Tool>>>;

    /// Drops cached state, after a connection or import changes.
    fn invalidate(&self);
}

/// Why a tool source is not contributing.
///
/// A broken MCP server must not take the turn down with it. The failure is
/// recorded here and surfaced in the UI; the other sources carry on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProblem {
    pub source: String,
    pub message: String,
}

/// What the agent loop holds.
#[async_trait]
pub trait ToolRegistry: Send + Sync {
    /// Every tool the model may see this turn, already filtered by permission
    /// rules and **sorted by id**.
    ///
    /// The sort is not cosmetic. Two turns whose tool lists differ only in
    /// order share no cached prompt prefix, so an unstable order silently
    /// doubles the cost of every conversation.
    async fn specs(&self) -> Vec<ToolSpec>;

    /// Runs one call, start to finish: normalize, validate, ask permission,
    /// execute, cap the output.
    ///
    /// Only cancellation comes back as `Err`. Every other failure is a
    /// [`ToolResult`] with `ok: false`, for the model to read.
    async fn run(&self, call: &ToolCall, ctx: &ToolContext) -> crate::Result<ToolResult>;

    /// Sources that failed to contribute, for the UI to show.
    async fn problems(&self) -> Vec<ProviderProblem>;
}

/// The decision on one permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Allow, and remember this for next time.
    AllowAlways,
    Deny,
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow | Self::AllowAlways)
    }
}

/// Decides whether a tool call may proceed.
///
/// The real implementation consults the rules, and shows the user a card when
/// they say `ask`. Test implementations answer from a script. Either way the
/// registry cannot run a tool without going through here.
#[async_trait]
pub trait PermissionGate: Send + Sync {
    /// Resolves a request, blocking until a person answers if it comes to
    /// that.
    ///
    /// Returns `Err(Error::Cancelled)` if the turn is abandoned while a prompt
    /// is still open - the user closed the conversation rather than answering.
    async fn ask(&self, request: &PermissionRequest) -> crate::Result<Decision>;

    /// What the rules say, without asking anyone. Used to decide which tools
    /// to offer the model at all.
    async fn verdict(&self, key: &str, target: &str) -> Action;

    /// The workspace folder behind these rules, if there is one.
    ///
    /// Part of that folder is the machinery that decides what an agent may do
    /// - its secrets, its permission rules, its hooks - and the file tools ask
    /// for it here to keep out of that machinery whatever the rules say. A
    /// gate with no workspace behind it, as in a test, has nothing to guard.
    fn workspace(&self) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_decision_to_always_allow_is_still_an_allow() {
        assert!(Decision::Allow.is_allowed());
        assert!(Decision::AllowAlways.is_allowed());
        assert!(!Decision::Deny.is_allowed());
    }

    #[test]
    fn a_permission_request_defaults_to_not_generalising() {
        let request = PermissionRequest::new("shell", "git status");
        assert_eq!(request.always, None);
        let broader = request.with_always("git *");
        assert_eq!(broader.always.as_deref(), Some("git *"));
    }

    #[test]
    fn tool_sources_have_stable_names() {
        // These strings reach the UI and stored permission rules.
        assert_eq!(ToolSource::Builtin.as_str(), "builtin");
        assert_eq!(ToolSource::OpenApi.as_str(), "openapi");
        assert_eq!(
            serde_json::to_value(ToolSource::Mcp).unwrap(),
            serde_json::json!("mcp")
        );
    }

    // A failed call is a result, not an error: the model reads the message and
    // gets a chance to correct itself.
    #[test]
    fn a_failure_is_still_a_result() {
        let result = ToolResult::failed(
            ToolCallId::from_existing("tc_1"),
            "read",
            "no such file: a.txt",
        );
        assert!(!result.ok);
        assert!(result.output.contains("no such file"));
    }

    #[test]
    fn a_tool_outcome_omits_empty_extras() {
        let json = serde_json::to_string(&ToolOutcome::text("done")).unwrap();
        assert_eq!(json, r#"{"output":"done"}"#);
    }
}

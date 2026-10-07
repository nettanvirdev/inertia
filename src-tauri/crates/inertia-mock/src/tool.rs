//! A tool that records what it was asked to do.
//!
//! Plugs into the *real* registry, so tests exercising it are testing the
//! actual validate/ask/execute/truncate pipeline rather than a second
//! implementation of it that could drift.

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use parking_lot::Mutex;

/// What the fake does when called.
#[derive(Debug, Clone)]
pub enum Behaviour {
    /// Succeeds, echoing the arguments back. Enough for most tests.
    Echo,
    /// Succeeds with fixed output.
    Output(String),
    /// Fails in a way the model can act on - a result with `ok: false`.
    Failure(String),
    /// Fails in a way it cannot - ends the turn.
    Fatal,
}

/// A configurable stand-in for a real tool.
#[derive(Debug)]
pub struct MockTool {
    id: String,
    description: String,
    parameters: serde_json::Value,
    behaviour: Behaviour,
    permission_key: String,
    calls: Mutex<Vec<serde_json::Value>>,
}

impl MockTool {
    /// A tool taking a single optional `input` string.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: "a mock tool".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "input": { "type": "string" } },
            }),
            behaviour: Behaviour::Echo,
            permission_key: "mock".into(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Sets the argument schema. Use a schema with `required` to test that
    /// validation actually rejects bad calls before the tool ever runs.
    pub fn with_parameters(mut self, parameters: serde_json::Value) -> Self {
        self.parameters = parameters;
        self
    }

    pub fn with_behaviour(mut self, behaviour: Behaviour) -> Self {
        self.behaviour = behaviour;
        self
    }

    pub fn with_permission_key(mut self, key: impl Into<String>) -> Self {
        self.permission_key = key.into();
        self
    }

    /// The arguments of every call that reached `execute`.
    ///
    /// A call rejected by validation or refused by the permission gate must
    /// **not** appear here - that is how those tests prove the tool never ran.
    pub fn calls(&self) -> Vec<serde_json::Value> {
        self.calls.lock().clone()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().len()
    }
}

#[async_trait]
impl Tool for MockTool {
    fn id(&self) -> &str {
        &self.id
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> serde_json::Value {
        self.parameters.clone()
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &serde_json::Value) -> PermissionRequest {
        // Mirrors what a real tool does: the target is the thing a person
        // would write a rule about, taken from the arguments.
        let target = args
            .get("input")
            .and_then(|v| v.as_str())
            .unwrap_or(inertia_core::permission::ANY);
        PermissionRequest::new(self.permission_key.clone(), target)
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> inertia_core::Result<ToolOutcome> {
        self.calls.lock().push(args.clone());

        match &self.behaviour {
            Behaviour::Echo => Ok(ToolOutcome::text(args.to_string())),
            Behaviour::Output(text) => Ok(ToolOutcome::text(text.clone())),
            Behaviour::Failure(message) => Err(inertia_core::Error::InvalidInput(message.clone())),
            Behaviour::Fatal => Err(inertia_core::Error::Auth("no credentials".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::{SessionId, ToolCallId};
    use std::sync::Arc;

    fn context() -> ToolContext {
        ToolContext {
            root: std::path::PathBuf::from("."),
            session: SessionId::new(),
            call_id: ToolCallId::new(),
            permissions: Arc::new(crate::permission::MockGate::allow_all()),
        }
    }

    #[tokio::test]
    async fn echo_returns_its_arguments() {
        let tool = MockTool::new("echo");
        let out = tool
            .execute(serde_json::json!({"input": "hi"}), &context())
            .await
            .unwrap();
        assert!(out.output.contains("hi"));
    }

    #[tokio::test]
    async fn it_records_every_call() {
        let tool = MockTool::new("echo");
        let ctx = context();
        tool.execute(serde_json::json!({"input": "a"}), &ctx)
            .await
            .unwrap();
        tool.execute(serde_json::json!({"input": "b"}), &ctx)
            .await
            .unwrap();
        assert_eq!(tool.call_count(), 2);
        assert_eq!(tool.calls()[1]["input"], "b");
    }

    #[tokio::test]
    async fn a_failing_tool_reports_something_the_model_can_fix() {
        let tool = MockTool::new("bad").with_behaviour(Behaviour::Failure("no such path".into()));
        let err = tool
            .execute(serde_json::json!({}), &context())
            .await
            .unwrap_err();
        assert!(err.is_model_correctable());
    }

    #[tokio::test]
    async fn a_fatal_tool_reports_something_it_cannot() {
        let tool = MockTool::new("bad").with_behaviour(Behaviour::Fatal);
        let err = tool
            .execute(serde_json::json!({}), &context())
            .await
            .unwrap_err();
        assert!(!err.is_model_correctable());
    }

    #[tokio::test]
    async fn the_permission_target_comes_from_the_arguments() {
        let tool = MockTool::new("echo");
        let request = tool.permission(&serde_json::json!({"input": "delete everything"}));
        assert_eq!(request.target, "delete everything");
        // With nothing to name, it falls back to the catch-all.
        let bare = tool.permission(&serde_json::json!({}));
        assert_eq!(bare.target, inertia_core::permission::ANY);
    }
}

//! The registry is the chokepoint every tool call passes through, so these
//! tests are less about tools working and more about the guarantees the rest
//! of the system is allowed to assume: that a call cannot run without asking,
//! that a malformed call cannot raise a prompt, and that one broken source
//! cannot take down a turn.

// An `unwrap` in a test is a readable assertion, not a latent panic.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_pass_by_value)]

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::message::ToolCall;
use inertia_core::tool::{PermissionGate, Tool, ToolContext, ToolProvider, ToolRegistry};
use inertia_core::{SessionId, ToolCallId};
use inertia_mock::{Behaviour, MockGate, MockTool, Policy};
use inertia_tools::{Limits, Registry};
use serde_json::json;

fn context(gate: Arc<dyn PermissionGate>) -> ToolContext {
    ToolContext {
        root: PathBuf::from("."),
        session: SessionId::new(),
        call_id: ToolCallId::new(),
        permissions: gate,
    }
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    }
}

#[tokio::test]
async fn a_permitted_call_runs_the_tool() {
    let tool = Arc::new(MockTool::new("echo"));
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone()).with_tool(tool.clone());

    let result = registry
        .run(&call("echo", json!({"input": "hi"})), &context(gate))
        .await
        .unwrap();

    assert!(result.ok);
    assert_eq!(tool.call_count(), 1);
}

/// The guarantee the whole permission system rests on.
#[tokio::test]
async fn a_refused_call_never_reaches_the_tool() {
    let tool = Arc::new(MockTool::new("echo"));
    let gate = Arc::new(MockGate::deny_all());
    let registry = Registry::new(gate.clone()).with_tool(tool.clone());

    let result = registry
        .run(&call("echo", json!({"input": "hi"})), &context(gate))
        .await
        .unwrap();

    assert!(!result.ok);
    assert_eq!(tool.call_count(), 0, "the tool ran despite being refused");
    assert!(result.output.contains("refused"), "got {}", result.output);
}

/// Validation precedes the permission ask deliberately: a model sending
/// nonsense must not be able to interrupt the user with a prompt about it.
#[tokio::test]
async fn a_malformed_call_neither_runs_nor_asks() {
    let tool = Arc::new(
        MockTool::new("echo").with_parameters(json!({
            "type": "object",
            "properties": { "input": { "type": "string" } },
            "required": ["input"],
            "additionalProperties": false,
        })),
    );
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone()).with_tool(tool.clone());

    let result = registry
        .run(&call("echo", json!({})), &context(gate.clone()))
        .await
        .unwrap();

    assert!(!result.ok);
    assert_eq!(tool.call_count(), 0);
    assert!(gate.asked().is_empty(), "a malformed call raised a prompt");
    assert!(
        result.output.contains("Invalid arguments for echo"),
        "got {}",
        result.output
    );
    assert_eq!(result.metadata.unwrap()["error"], "invalid-arguments");
}

#[tokio::test]
async fn arguments_are_repaired_before_validation() {
    let tool = Arc::new(MockTool::new("echo").with_parameters(json!({
        "type": "object",
        "properties": { "limit": { "type": "integer" } },
        "additionalProperties": false,
    })));
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone()).with_tool(tool.clone());

    // A stringified number, which models produce constantly.
    let result = registry
        .run(&call("echo", json!({"limit": "5"})), &context(gate))
        .await
        .unwrap();

    assert!(result.ok, "got {}", result.output);
    assert_eq!(tool.calls()[0]["limit"], 5);
}

/// A tool failure is a result the model reads, not an error that ends the
/// turn - it may well be able to fix it and try again.
#[tokio::test]
async fn a_tool_failure_is_a_result_not_an_error() {
    let tool = Arc::new(
        MockTool::new("bad").with_behaviour(Behaviour::Failure("no such file: a.txt".into())),
    );
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone()).with_tool(tool);

    let result = registry
        .run(&call("bad", json!({})), &context(gate))
        .await
        .expect("a tool failure must not propagate as an error");

    assert!(!result.ok);
    assert!(result.output.contains("no such file"));
}

#[tokio::test]
async fn an_unknown_tool_is_reported_to_the_model() {
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone());

    let result = registry
        .run(&call("nonexistent", json!({})), &context(gate))
        .await
        .unwrap();

    assert!(!result.ok);
    assert!(result.output.contains("nonexistent"));
}

#[tokio::test]
async fn the_permission_target_is_the_thing_a_rule_would_name() {
    let tool = Arc::new(MockTool::new("shell").with_permission_key("shell"));
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone()).with_tool(tool);

    registry
        .run(&call("shell", json!({"input": "git push"})), &context(gate.clone()))
        .await
        .unwrap();

    assert_eq!(gate.targets(), vec!["git push"]);
}

#[tokio::test]
async fn oversized_output_is_capped_and_says_so() {
    let huge = "x".repeat(50_000);
    let tool = Arc::new(MockTool::new("big").with_behaviour(Behaviour::Output(huge)));
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone())
        .with_tool(tool)
        .with_limits(Limits { output_bytes: 1_000 });

    let result = registry
        .run(&call("big", json!({})), &context(gate))
        .await
        .unwrap();

    assert!(result.ok);
    assert!(result.output.len() < 3_000);
    assert!(result.output.contains("omitted"));
    assert_eq!(result.metadata.unwrap()["truncated"], true);
}

// ── the tool list offered to the model ──────────────────────────────────

#[tokio::test]
async fn specs_are_sorted_by_id() {
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate)
        .with_tool(Arc::new(MockTool::new("zebra")))
        .with_tool(Arc::new(MockTool::new("apple")))
        .with_tool(Arc::new(MockTool::new("mango")));

    let names: Vec<String> = registry.specs().await.into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["apple", "mango", "zebra"]);
}

#[tokio::test]
async fn an_unconditionally_denied_tool_is_not_offered() {
    let gate = Arc::new(MockGate::new(Policy::Rules(vec![
        inertia_core::permission::Rule::for_any("secret", inertia_core::permission::Action::Deny),
        inertia_core::permission::Rule::for_any("mock", inertia_core::permission::Action::Allow),
    ])));
    let registry = Registry::new(gate)
        .with_tool(Arc::new(MockTool::new("visible")))
        .with_tool(Arc::new(MockTool::new("hidden").with_permission_key("secret")));

    let names: Vec<String> = registry.specs().await.into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["visible"]);
}

#[tokio::test]
async fn the_first_registration_of_an_id_wins() {
    let first = Arc::new(MockTool::new("dup").with_description("the real one"));
    let second = Arc::new(MockTool::new("dup").with_description("an impostor"));
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate).with_tool(first).with_tool(second);

    let specs = registry.specs().await;
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].description, "the real one");
}

// ── dynamic sources ─────────────────────────────────────────────────────

#[derive(Debug)]
struct BrokenProvider;

#[async_trait]
impl ToolProvider for BrokenProvider {
    fn name(&self) -> &'static str {
        "mcp"
    }
    async fn tools(&self, _root: &std::path::Path) -> inertia_core::Result<Vec<Arc<dyn Tool>>> {
        Err(inertia_core::Error::Network("connection refused".into()))
    }
    fn invalidate(&self) {}
}

#[derive(Debug)]
struct WorkingProvider;

#[async_trait]
impl ToolProvider for WorkingProvider {
    fn name(&self) -> &'static str {
        "openapi"
    }
    async fn tools(&self, _root: &std::path::Path) -> inertia_core::Result<Vec<Arc<dyn Tool>>> {
        Ok(vec![Arc::new(MockTool::new("remote"))])
    }
    fn invalidate(&self) {}
}

/// A misconfigured MCP server must not take down a turn that was never going
/// to use it.
#[tokio::test]
async fn a_broken_source_is_isolated_not_fatal() {
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate)
        .with_tool(Arc::new(MockTool::new("local")))
        .with_provider(Arc::new(BrokenProvider))
        .with_provider(Arc::new(WorkingProvider));

    let names: Vec<String> = registry.specs().await.into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["local", "remote"]);

    let problems = registry.problems().await;
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].source, "mcp");
    assert!(problems[0].message.contains("connection refused"));
}

#[tokio::test]
async fn tools_from_a_provider_are_callable() {
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone()).with_provider(Arc::new(WorkingProvider));

    let result = registry
        .run(&call("remote", json!({"input": "x"})), &context(gate))
        .await
        .unwrap();

    assert!(result.ok, "got {}", result.output);
}

#[tokio::test]
async fn a_tool_source_that_reports_a_type_the_model_sent_wrong_explains_itself() {
    let tool = Arc::new(MockTool::new("echo").with_parameters(json!({
        "type": "object",
        "properties": { "input": { "type": "string" } },
        "additionalProperties": false,
    })));
    let gate = Arc::new(MockGate::allow_all());
    let registry = Registry::new(gate.clone()).with_tool(tool);

    let result = registry
        .run(&call("echo", json!({"input": 42})), &context(gate))
        .await
        .unwrap();

    // The message is written for the model to act on, not for a developer.
    assert!(
        result.output.contains("`input` must be a string, got number."),
        "got {}",
        result.output
    );
}

//! The MCP client against a real server process.
//!
//! Spawned, over real pipes, with real framing. The unit tests cover the
//! parsing; this covers everything that only goes wrong once a process is
//! involved.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;

use inertia_core::tool::{ToolContext, ToolProvider};
use inertia_core::{SessionId, ToolCallId};
use inertia_mcp::client::{Connection, ServerRecord};
use inertia_mcp::provider::McpProvider;

/// Cargo builds this binary for us and hands over its path.
fn fake_server() -> ServerRecord {
    ServerRecord {
        id: "fake".into(),
        name: "Fake".into(),
        enabled: true,
        command: env!("CARGO_BIN_EXE_fake-mcp-server").to_string(),
        ..Default::default()
    }
}

#[tokio::test]
async fn connecting_completes_the_handshake_and_lists_tools() {
    let connection = Connection::connect_stdio(fake_server())
        .await
        .expect("the handshake should complete");

    let names: Vec<&str> = connection.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["echo", "fails"]);
}

/// The server prints a banner to stdout before any protocol. A client that
/// treats stdout as pure JSON chokes on it.
#[tokio::test]
async fn a_banner_on_stdout_does_not_break_the_connection() {
    // The fake always prints one, so simply getting here proves it.
    let connection = Connection::connect_stdio(fake_server()).await.unwrap();
    assert!(!connection.tools.is_empty());
}

#[tokio::test]
async fn calling_a_tool_returns_its_text() {
    let connection = Connection::connect_stdio(fake_server()).await.unwrap();

    let result = connection
        .call_tool("echo", serde_json::json!({ "text": "hello" }))
        .await
        .unwrap();

    let output = inertia_mcp::protocol::flatten_content(&result);
    assert!(output.contains("echoed: hello"), "got {output}");
}

#[tokio::test]
async fn an_unknown_method_comes_back_as_an_error() {
    let connection = Connection::connect_stdio(fake_server()).await.unwrap();
    // `tools/call` for a tool the server reports an error for.
    let result = connection
        .call_tool("fails", serde_json::json!({}))
        .await
        .unwrap();

    assert!(inertia_mcp::protocol::is_error_result(&result));
    assert!(inertia_mcp::protocol::flatten_content(&result).contains("it went wrong"));
}

/// A server's `$schema` and other validator metadata must not reach a model
/// provider, which would reject the whole tool list.
#[tokio::test]
async fn the_advertised_schema_is_sanitised() {
    let provider = McpProvider::new();
    provider.add(fake_server()).await.unwrap();

    let tools = provider.tools(Path::new(".")).await.unwrap();
    let echo = tools.iter().find(|t| t.id() == "fake_echo").unwrap();

    let schema = echo.parameters().to_string();
    assert!(!schema.contains("$schema"), "got {schema}");
    assert!(schema.contains("\"text\""));
}

#[tokio::test]
async fn tools_are_namespaced_by_server() {
    let provider = McpProvider::new();
    provider.add(fake_server()).await.unwrap();

    let ids: Vec<String> = provider
        .tools(Path::new("."))
        .await
        .unwrap()
        .iter()
        .map(|t| t.id().to_string())
        .collect();

    assert_eq!(ids, vec!["fake_echo", "fake_fails"]);
}

/// The server is one process for the whole app, so it cannot know which
/// project this turn is about unless it is told.
#[tokio::test]
async fn a_blank_project_folder_is_filled_from_the_working_directory() {
    let provider = McpProvider::new();
    provider.add(fake_server()).await.unwrap();

    let tools = provider.tools(Path::new(".")).await.unwrap();
    let echo = tools.iter().find(|t| t.id() == "fake_echo").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        root: dir.path().to_path_buf(),
        session: SessionId::new(),
        call_id: ToolCallId::new(),
        permissions: Arc::new(inertia_mock::MockGate::allow_all()),
    };

    // The model supplied no folder, so `normalize` marks it and `execute`
    // fills it in.
    let args = echo.normalize(serde_json::json!({ "text": "hi" }));
    let outcome = echo.execute(args, &ctx).await.unwrap();

    assert!(
        outcome.output.contains(&dir.path().display().to_string()),
        "the working folder was not passed through: {}",
        outcome.output
    );
}

/// A folder the model chose is never replaced.
#[tokio::test]
async fn a_supplied_project_folder_survives() {
    let provider = McpProvider::new();
    provider.add(fake_server()).await.unwrap();

    let tools = provider.tools(Path::new(".")).await.unwrap();
    let echo = tools.iter().find(|t| t.id() == "fake_echo").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        root: dir.path().to_path_buf(),
        session: SessionId::new(),
        call_id: ToolCallId::new(),
        permissions: Arc::new(inertia_mock::MockGate::allow_all()),
    };

    let args = echo.normalize(serde_json::json!({
        "text": "hi",
        "projectPath": "/chosen/by/the/model"
    }));
    let outcome = echo.execute(args, &ctx).await.unwrap();

    assert!(outcome.output.contains("/chosen/by/the/model"));
    assert!(!outcome.output.contains(&dir.path().display().to_string()));
}

/// A tool reporting failure has produced a result the model should read, and
/// it comes back as something it can act on rather than as a transport error.
#[tokio::test]
async fn a_tool_error_is_model_correctable() {
    let provider = McpProvider::new();
    provider.add(fake_server()).await.unwrap();

    let tools = provider.tools(Path::new(".")).await.unwrap();
    let failing = tools.iter().find(|t| t.id() == "fake_fails").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext {
        root: dir.path().to_path_buf(),
        session: SessionId::new(),
        call_id: ToolCallId::new(),
        permissions: Arc::new(inertia_mock::MockGate::allow_all()),
    };

    let error = failing
        .execute(serde_json::json!({}), &ctx)
        .await
        .unwrap_err();

    assert!(error.is_model_correctable());
    assert!(error.to_string().contains("it went wrong"));
}

#[tokio::test]
async fn a_server_that_cannot_start_reports_why() {
    let record = ServerRecord {
        id: "missing".into(),
        command: "this-command-does-not-exist-anywhere".into(),
        ..Default::default()
    };

    let error = Connection::connect_stdio(record).await.unwrap_err();
    assert!(
        error.to_string().contains("this-command-does-not-exist"),
        "got {error}"
    );
}

#[tokio::test]
async fn removing_a_server_removes_its_tools() {
    let provider = McpProvider::new();
    provider.add(fake_server()).await.unwrap();
    assert_eq!(provider.tools(Path::new(".")).await.unwrap().len(), 2);

    provider.remove("fake");
    assert!(provider.tools(Path::new(".")).await.unwrap().is_empty());
}

/// Two servers offering the same tool name must not collide silently.
#[tokio::test]
async fn two_servers_get_distinct_tool_ids() {
    let provider = McpProvider::new();
    provider.add(fake_server()).await.unwrap();
    provider
        .add(ServerRecord {
            id: "second".into(),
            name: "Second".into(),
            ..fake_server()
        })
        .await
        .unwrap();

    let ids: Vec<String> = provider
        .tools(Path::new("."))
        .await
        .unwrap()
        .iter()
        .map(|t| t.id().to_string())
        .collect();

    assert!(ids.contains(&"fake_echo".to_string()));
    assert!(ids.contains(&"second_echo".to_string()));
    assert_eq!(ids.len(), 4);
}

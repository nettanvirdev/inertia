//! MCP servers as a source of ordinary tools.
//!
//! Once a server is connected, its tools are indistinguishable to the agent
//! loop from a builtin: same trait, same registry, same permission pipeline.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{
    PermissionRequest, Tool, ToolContext, ToolOutcome, ToolProvider, ToolSource,
};
use inertia_core::{Error, Result};
use parking_lot::RwLock;
use serde_json::Value;

use crate::client::{Connection, ServerRecord, ToolDescriptor};
use crate::protocol;

/// Property names filled from the conversation's working folder when the model
/// leaves them blank.
///
/// A connected server is one process for the whole app, so the directory it was
/// spawned in has nothing to do with which project this turn is about. Without
/// this, a server that wants a project path gets whichever folder the app
/// happened to start in.
const FOLDER_PROPERTIES: &[&str] = &["projectPath", "project_path", "projectRoot", "project_root"];

/// Lowercases and replaces anything outside `[a-z0-9_-]`.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// The id a server's tool is known by.
///
/// Namespacing is mandatory, not cosmetic: two servers both offering `search`
/// would otherwise collide silently, and the model would call whichever
/// happened to load last with no error anywhere.
pub fn tool_id(server_name: &str, tool_name: &str) -> String {
    format!("{}_{}", sanitize(server_name), sanitize(tool_name))
}

/// Marks a blank project-folder argument as fillable.
///
/// Split out from the tool so it can be tested without a live connection, and
/// because the decision it makes is the interesting part: only these specific
/// names, only when the schema declares them as strings, and only when the
/// model supplied nothing. A value the model chose is never overridden - this
/// fills gaps, it does not correct intent.
///
/// The actual path is substituted at execute time, where the working folder is
/// known; this only decides *which* arguments get one.
fn mark_fillable_folders(schema: &Value, mut args: Value) -> Value {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return args;
    };
    let Some(map) = args.as_object_mut() else {
        return args;
    };

    for name in FOLDER_PROPERTIES {
        let Some(declared) = properties.get(*name) else {
            continue;
        };
        let is_string = match declared.get("type") {
            Some(Value::String(t)) => t == "string",
            // A union like `["string", "null"]` still accepts a path.
            Some(Value::Array(types)) => types.iter().any(|t| t == "string"),
            _ => false,
        };
        if !is_string {
            continue;
        }

        let missing = match map.get(*name) {
            None | Some(Value::Null) => true,
            Some(Value::String(existing)) => existing.trim().is_empty(),
            _ => false,
        };
        if missing {
            map.insert((*name).to_string(), Value::String(String::new()));
        }
    }

    args
}

/// One tool on one connected server.
pub struct McpTool {
    id: String,
    description: String,
    schema: Value,
    server: String,
    tool_name: String,
    connection: Arc<Connection>,
}

impl std::fmt::Debug for McpTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpTool").field("id", &self.id).finish()
    }
}

impl McpTool {
    pub fn new(
        server_name: &str,
        descriptor: &ToolDescriptor,
        connection: Arc<Connection>,
    ) -> Self {
        Self {
            id: tool_id(server_name, &descriptor.name),
            description: descriptor.description.clone(),
            schema: protocol::sanitize_schema(&descriptor.input_schema),
            server: server_name.to_string(),
            tool_name: descriptor.name.clone(),
            connection,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn id(&self) -> &str {
        &self.id
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        self.schema.clone()
    }

    fn source(&self) -> ToolSource {
        ToolSource::Mcp
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        // Keyed on `mcp` with the tool as the target, so a rule can name one
        // tool (`github_create_issue`), a whole server (`github_*`), or every
        // MCP tool at once - all with the same glob syntax used everywhere
        // else.
        PermissionRequest::new("mcp", self.id.clone()).with_always(self.id.clone())
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some(format!("{} · {}", self.server, self.tool_name))
    }

    /// Fills a project-folder argument the model left blank.
    ///
    /// Only the specific property names above, only when the schema declares
    /// them as strings, and only when the model supplied nothing. A value the
    /// model chose is never overridden - this fills gaps, it does not correct
    /// intent.
    fn normalize(&self, args: Value) -> Value {
        mark_fillable_folders(&self.schema, args)
    }

    async fn execute(&self, mut args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        // Fill the folder arguments now that the working directory is known.
        if let Some(map) = args.as_object_mut() {
            for name in FOLDER_PROPERTIES {
                if let Some(Value::String(value)) = map.get(*name) {
                    if value.is_empty() {
                        map.insert(
                            (*name).to_string(),
                            Value::String(ctx.root.display().to_string()),
                        );
                    }
                }
            }
        }

        let result = self
            .connection
            .call_tool(&self.tool_name, args)
            .await
            .map_err(|e| match e {
                crate::client::McpError::Timeout(secs) => {
                    Error::Timeout(std::time::Duration::from_secs(secs))
                }
                // A server-reported error is something the model can often fix
                // by calling differently.
                crate::client::McpError::Rpc(error) => Error::InvalidInput(error.message),
                other => Error::Other(other.to_string()),
            })?;

        let output = protocol::flatten_content(&result);

        // A tool that reports failure through `isError` has still produced a
        // result the model should read, so it is surfaced as unhappy output
        // rather than as a transport failure.
        if protocol::is_error_result(&result) {
            return Err(Error::InvalidInput(if output.is_empty() {
                format!("{} reported an error.", self.tool_name)
            } else {
                output
            }));
        }

        Ok(ToolOutcome {
            title: Some(format!("{} · {}", self.server, self.tool_name)),
            output,
            metadata: Some(serde_json::json!({
                "server": self.server,
                "tool": self.tool_name,
            })),
            images: Vec::new(),
        })
    }
}

/// Every connected server, as one source of tools.
#[derive(Default)]
pub struct McpProvider {
    /// Live connections. Process state, never persisted - the on-disk record
    /// only remembers configuration and the last status seen.
    connections: RwLock<Vec<(ServerRecord, Arc<Connection>)>>,
    /// Set when something changed and the tool list should be rebuilt.
    stale: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for McpProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpProvider")
            .field("connections", &self.connections.read().len())
            .finish()
    }
}

impl McpProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Connects a server and keeps it.
    ///
    /// Which transport is the record's business, not the caller's. Routing on
    /// `type` here rather than at every call site is why a workspace full of
    /// HTTP servers used to report `'""' is not recognized as an internal or
    /// external command`: every record went to the stdio path and was asked to
    /// spawn a `command` that HTTP records leave empty.
    pub async fn add(&self, record: ServerRecord) -> std::result::Result<usize, String> {
        let connection = match record.kind {
            crate::client::Kind::Http => Connection::connect_http(record.clone()).await,
            crate::client::Kind::Stdio => Connection::connect_stdio(record.clone()).await,
        }
        .map_err(|e| e.to_string())?;
        let count = connection.tools.len();

        self.connections
            .write()
            .push((record, Arc::new(connection)));
        self.invalidate();
        Ok(count)
    }

    /// Disconnects a server, stopping its process.
    pub fn remove(&self, id: &str) {
        self.connections
            .write()
            .retain(|(record, _)| record.id != id);
        self.invalidate();
    }

    /// What one connected server provides, as the settings screen shows it.
    ///
    /// Live state only: a server that is not connected has no tools, which is
    /// different from a server that connected and offered none.
    pub fn tools_for(&self, id: &str) -> Vec<crate::client::ToolDescriptor> {
        self.connections
            .read()
            .iter()
            .find(|(record, _)| record.id == id)
            .map(|(_, connection)| connection.tools.clone())
            .unwrap_or_default()
    }

    pub fn connected(&self) -> Vec<ServerRecord> {
        self.connections
            .read()
            .iter()
            .map(|(record, _)| record.clone())
            .collect()
    }
}

#[async_trait]
impl ToolProvider for McpProvider {
    fn name(&self) -> &'static str {
        "mcp"
    }

    async fn tools(&self, _root: &Path) -> Result<Vec<Arc<dyn Tool>>> {
        // Reads live state only - no network, no process spawning. This is
        // called on roughly every turn.
        let connections = self.connections.read();
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for (record, connection) in connections.iter() {
            let server_name = if record.name.is_empty() {
                &record.id
            } else {
                &record.name
            };

            for descriptor in &connection.tools {
                let tool = McpTool::new(server_name, descriptor, connection.clone());
                // First registration wins, so the winner is the same across
                // reconnects rather than depending on connection order.
                if seen.insert(tool.id().to_string()) {
                    tools.push(Arc::new(tool));
                }
            }
        }

        Ok(tools)
    }

    fn invalidate(&self) {
        self.stale.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn descriptor(name: &str, schema: Value) -> ToolDescriptor {
        serde_json::from_value(json!({
            "name": name,
            "description": "does a thing",
            "inputSchema": schema
        }))
        .unwrap()
    }

    /// Without namespacing, two servers offering `search` collide silently and
    /// the model calls whichever loaded last.
    #[test]
    fn tool_ids_are_namespaced_by_server() {
        assert_eq!(tool_id("GitHub", "create_issue"), "github_create_issue");
        assert_eq!(tool_id("My Server", "search"), "my_server_search");
    }

    #[test]
    fn anything_unusual_in_a_name_becomes_an_underscore() {
        assert_eq!(tool_id("a.b/c", "d:e"), "a_b_c_d_e");
    }

    #[test]
    fn a_permission_rule_can_name_one_tool_or_a_whole_server() {
        use inertia_core::permission::{evaluate, Action, Rule};

        let rules = vec![
            Rule::for_any("mcp", Action::Ask),
            Rule::new("mcp", Action::Allow, "github_*"),
            Rule::new("mcp", Action::Deny, "github_delete_repo"),
        ];

        assert_eq!(
            evaluate(&rules, "mcp", "github_create_issue").action,
            Action::Allow
        );
        assert_eq!(
            evaluate(&rules, "mcp", "github_delete_repo").action,
            Action::Deny
        );
        assert_eq!(evaluate(&rules, "mcp", "other_search").action, Action::Ask);
    }

    // ── folder auto-fill ────────────────────────────────────────────────

    fn folder_schema(declared_type: Value) -> Value {
        json!({
            "type": "object",
            "properties": { "projectPath": { "type": declared_type } }
        })
    }

    #[test]
    fn a_blank_folder_argument_is_marked_for_filling() {
        let schema = folder_schema(json!("string"));

        for supplied in [
            json!({}),
            json!({ "projectPath": null }),
            json!({ "projectPath": "  " }),
        ] {
            let filled = mark_fillable_folders(&schema, supplied.clone());
            assert_eq!(
                filled["projectPath"],
                json!(""),
                "not marked for {supplied}"
            );
        }
    }

    /// The rule that keeps auto-fill from overriding intent.
    #[test]
    fn a_supplied_folder_is_never_overridden() {
        let schema = folder_schema(json!("string"));
        let filled = mark_fillable_folders(&schema, json!({ "projectPath": "/specific" }));
        assert_eq!(filled["projectPath"], "/specific");
    }

    #[test]
    fn a_union_typed_folder_is_still_fillable() {
        let schema = folder_schema(json!(["string", "null"]));
        let filled = mark_fillable_folders(&schema, json!({}));
        assert_eq!(filled["projectPath"], json!(""));
    }

    #[test]
    fn a_folder_property_of_the_wrong_type_is_left_alone() {
        let schema = folder_schema(json!("number"));
        let filled = mark_fillable_folders(&schema, json!({}));
        assert!(filled.get("projectPath").is_none());
    }

    /// Only the specific names, so an unrelated string argument is untouched.
    #[test]
    fn unrelated_arguments_are_untouched() {
        let schema = json!({
            "type": "object",
            "properties": { "query": { "type": "string" } }
        });
        let filled = mark_fillable_folders(&schema, json!({}));
        assert_eq!(filled, json!({}));
    }

    #[test]
    fn a_schema_from_a_server_is_sanitised_before_the_model_sees_it() {
        let tool = descriptor(
            "x",
            json!({ "type": "object", "$schema": "https://json-schema.org/", "properties": {} }),
        );
        let cleaned = protocol::sanitize_schema(&tool.input_schema);
        assert!(!cleaned.to_string().contains("$schema"));
    }

    #[tokio::test]
    async fn a_provider_with_no_servers_offers_no_tools() {
        let provider = McpProvider::new();
        let tools = provider.tools(Path::new(".")).await.unwrap();
        assert!(tools.is_empty());
    }
}

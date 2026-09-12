//! Connected Composio apps as a source of tools.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{
    PermissionRequest, Tool, ToolContext, ToolOutcome, ToolProvider, ToolSource,
};
use inertia_core::{Error, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::Composio;

/// Descriptions longer than this crowd the tool list without adding anything.
const MAX_DESCRIPTION_CHARS: usize = 1_024;

/// Per-app and overall caps on how many tools reach the model.
///
/// Much lower than the MCP or OpenAPI ceilings, and deliberately: a model
/// handed every operation of every connected app stops choosing well. Gmail
/// alone exposes dozens, and the twentieth variant of "list messages" makes the
/// first nineteen harder to pick between.
const MAX_TOOLS_PER_TOOLKIT: usize = 40;
const MAX_TOOLS_TOTAL: usize = 96;

/// One connected app, as stored in the workspace.
///
/// Every name here is the name already on disk, and that is the whole of the
/// contract: these records are written and read by two front ends over one
/// workspace folder, so a field spelled differently in Rust is not a style
/// choice, it is a record that reads back empty. `toolkit` and
/// `connectedAccountId` were exactly that - the screen asked a connection for
/// its `toolkitSlug`, got nothing, and called the tools command with no app
/// named.
///
/// `extra` is here for the same reason. A record carries fields this struct has
/// no opinion about - the cached logo, whatever is added next - and a save that
/// round-trips through a struct without them deletes them from the user's file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConnectionRecord {
    pub id: String,
    /// Composio's slug for the app, e.g. `gmail`.
    pub toolkit_slug: String,
    pub name: String,
    /// Whose account it is, when the app will say. Often empty.
    pub label: String,
    pub enabled: bool,
    /// Composio's connected-account id, used at execute time.
    pub account_id: String,
    /// The user id this connection was linked under, so a reconnect refers to
    /// the same person on Composio's side.
    pub composio_user_id: String,
    /// `"ACTIVE"` once the OAuth handshake has completed.
    pub status: String,
    /// Which of the toolkit's tools to offer. Empty means all of them: the
    /// user already made a deliberate choice by connecting the app, unlike an
    /// OpenAPI import where every operation arrives at once.
    pub enabled_tools: Vec<String>,
    /// Everything else the record carries, kept so a save does not drop it.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// `enabled` defaults to true, and the rest to nothing.
///
/// Not derived: a record written before `enabled` existed, or by a version that
/// leaves it off, would otherwise read as switched off, and the user would find
/// every app they had connected silently contributing no tools.
impl Default for ConnectionRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            toolkit_slug: String::new(),
            name: String::new(),
            label: String::new(),
            enabled: true,
            account_id: String::new(),
            composio_user_id: String::new(),
            status: String::new(),
            enabled_tools: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}

impl ConnectionRecord {
    /// Whether this connection should contribute tools.
    pub fn is_usable(&self) -> bool {
        self.enabled && self.status.eq_ignore_ascii_case("ACTIVE")
    }
}

/// Composio may describe a tool's inputs as a full JSON Schema or as a bare
/// bag of property schemas. Both occur; only the first is usable as-is.
fn parameters_of(tool: &Value) -> Value {
    let raw = tool
        .get("input_parameters")
        .or_else(|| tool.get("inputParameters"))
        .or_else(|| tool.get("parameters"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    let wrapped = if raw.get("type").is_some() || raw.get("properties").is_some() {
        raw
    } else {
        serde_json::json!({ "type": "object", "properties": raw })
    };

    relax(&wrapped)
}

/// Strips validator-oriented constraints at every depth.
///
/// `required` and `additionalProperties: false` are written for a validator,
/// not for a model, and a strict provider rejects the entire tool list over
/// either of them. The tool still validates its own arguments, so nothing is
/// actually loosened - only what the model is shown.
fn relax(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut cleaned = serde_json::Map::new();
            for (key, child) in map {
                match key.as_str() {
                    "required" => continue,
                    "additionalProperties" if child == &Value::Bool(false) => continue,
                    _ => {
                        cleaned.insert(key.clone(), relax(child));
                    }
                }
            }
            cleaned
                .entry("type")
                .or_insert(Value::String("object".into()));
            Value::Object(cleaned)
        }
        Value::Array(items) => Value::Array(items.iter().map(relax).collect()),
        other => other.clone(),
    }
}

/// One Composio operation.
pub struct ComposioTool {
    id: String,
    /// Composio's own spelling. Its execute endpoint answers to nothing else.
    slug: String,
    toolkit: String,
    description: String,
    schema: Value,
    connected_account_id: String,
    client: Composio,
}

impl std::fmt::Debug for ComposioTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposioTool").field("id", &self.id).finish()
    }
}

impl ComposioTool {
    pub fn new(record: &ConnectionRecord, tool: &Value, client: Composio) -> Option<Self> {
        let slug = tool.get("slug").and_then(Value::as_str)?.to_string();

        let mut description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if description.chars().count() > MAX_DESCRIPTION_CHARS {
            let cut: String = description.chars().take(MAX_DESCRIPTION_CHARS).collect();
            description = cut;
        }

        Some(Self {
            // Lower-cased because every other tool id in the app is lowercase,
            // and a model shown both conventions in one list guesses wrong
            // about which a given tool follows.
            id: slug.to_ascii_lowercase(),
            slug,
            toolkit: record.toolkit_slug.clone(),
            description,
            schema: parameters_of(tool),
            connected_account_id: record.account_id.clone(),
            client,
        })
    }
}

#[async_trait]
impl Tool for ComposioTool {
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
        ToolSource::Composio
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        // Scoped to the whole app rather than the individual operation, unlike
        // MCP and OpenAPI. A rule that reads "allow composio gmail" is one a
        // person can write and mean; "allow composio gmail_fetch_emails" is
        // not a decision anyone wants to make forty times.
        PermissionRequest::new("composio", self.toolkit.clone())
            .with_always(self.toolkit.clone())
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some(format!("{} · {}", self.toolkit, self.id))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let account = (!self.connected_account_id.is_empty())
            .then_some(self.connected_account_id.as_str());

        // Composio's own spelling, not the lower-cased id.
        let result = self
            .client
            .execute(&self.slug, args, account)
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        // Composio reports a failed action in the envelope rather than as a
        // non-2xx, so this is where it has to be noticed.
        let succeeded = result
            .get("successful")
            .or_else(|| result.get("successfull"))
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let payload = result
            .get("data")
            .cloned()
            .unwrap_or_else(|| result.clone());

        let output = match &payload {
            Value::String(text) => text.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        };

        if !succeeded {
            let reason = result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or(&output)
                .to_string();
            // Something the model can often fix by calling differently, so it
            // goes back as a correctable failure rather than a fatal one.
            return Err(Error::InvalidInput(reason));
        }

        Ok(ToolOutcome {
            title: Some(format!("{} · {}", self.toolkit, self.id)),
            output,
            metadata: Some(serde_json::json!({
                "toolkit": self.toolkit,
                "composioSlug": self.slug,
            })),
            images: Vec::new(),
        })
    }
}

/// Every connected app, as one source of tools.
#[derive(Default)]
pub struct ComposioProvider {
    client: RwLock<Option<Composio>>,
    /// Connection records with their discovered tools, cached. Discovery is on
    /// the turn path and must never touch the network there.
    connections: RwLock<Vec<(ConnectionRecord, Vec<Value>)>>,
}

impl std::fmt::Debug for ComposioProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposioProvider")
            .field("connections", &self.connections.read().len())
            .finish()
    }
}

impl ComposioProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the API key. Without one this provider offers nothing, which is
    /// not an error - an app with no Composio account simply has no Composio
    /// tools.
    pub fn set_key(&self, api_key: Option<String>) {
        *self.client.write() = api_key
            .filter(|key| !key.trim().is_empty())
            .map(Composio::new);
        self.connections.write().clear();
    }

    pub fn has_key(&self) -> bool {
        self.client.read().is_some()
    }

    /// Discovers and caches one connection's tools.
    pub async fn connect(&self, record: ConnectionRecord) -> std::result::Result<usize, String> {
        let client = self
            .client
            .read()
            .clone()
            .ok_or_else(|| "No Composio API key is configured.".to_string())?;

        let discovered = client
            .tools(&record.toolkit_slug)
            .await
            .map_err(|e| e.to_string())?;

        let selected = select(&discovered, &record.enabled_tools);
        let count = selected.len();

        self.connections
            .write()
            .retain(|(existing, _)| existing.id != record.id);
        self.connections.write().push((record, selected));

        Ok(count)
    }

    pub fn disconnect(&self, id: &str) {
        self.connections
            .write()
            .retain(|(record, _)| record.id != id);
    }

    pub fn connections(&self) -> Vec<ConnectionRecord> {
        self.connections
            .read()
            .iter()
            .map(|(record, _)| record.clone())
            .collect()
    }

    pub fn client(&self) -> Option<Composio> {
        self.client.read().clone()
    }
}

/// Picks which of a toolkit's tools to offer.
///
/// An empty selection means all of them, capped. Unlike an OpenAPI import,
/// connecting an app was already a deliberate choice about that app, so the
/// default is on rather than off.
fn select(all: &[Value], enabled: &[String]) -> Vec<Value> {
    let mut chosen: Vec<Value> = if enabled.is_empty() {
        all.to_vec()
    } else {
        all.iter()
            .filter(|tool| {
                tool.get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|slug| enabled.iter().any(|e| e.eq_ignore_ascii_case(slug)))
            })
            .cloned()
            .collect()
    };

    // Composio marks some tools as important; those are the ones worth keeping
    // when the cap bites.
    chosen.sort_by_key(|tool| {
        let important = tool
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|tags| tags.iter().any(|t| t.as_str() == Some("important")));
        !important
    });

    chosen.truncate(MAX_TOOLS_PER_TOOLKIT);
    chosen
}

#[async_trait]
impl ToolProvider for ComposioProvider {
    fn name(&self) -> &'static str {
        "composio"
    }

    async fn tools(&self, _root: &Path) -> Result<Vec<Arc<dyn Tool>>> {
        let Some(client) = self.client.read().clone() else {
            // No key configured. Not an error.
            return Ok(Vec::new());
        };

        let connections = self.connections.read();
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

        for (record, discovered) in connections.iter() {
            if !record.is_usable() {
                continue;
            }
            for tool in discovered {
                if tools.len() >= MAX_TOOLS_TOTAL {
                    break;
                }
                if let Some(tool) = ComposioTool::new(record, tool, client.clone()) {
                    tools.push(Arc::new(tool));
                }
            }
        }

        Ok(tools)
    }

    fn invalidate(&self) {
        self.drop_cached_tools();
    }
}

impl ComposioProvider {
    /// Drops the discovered tool lists, so the next turn asks Composio again.
    ///
    /// Public as well as available through the trait, because the settings
    /// screen invalidates after an edit that went around the tool pipeline
    /// entirely - and reaching it through `ToolProvider` there would mean
    /// importing the trait to call one method on a concrete type.
    pub fn drop_cached_tools(&self) {
        self.connections.write().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(slug: &str) -> Value {
        json!({
            "slug": slug,
            "description": "does a thing",
            "input_parameters": {
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
                "additionalProperties": false
            }
        })
    }

    fn record() -> ConnectionRecord {
        ConnectionRecord {
            id: "conn_1".into(),
            toolkit_slug: "gmail".into(),
            name: "Gmail".into(),
            account_id: "acc_1".into(),
            status: "ACTIVE".into(),
            ..ConnectionRecord::default()
        }
    }

    #[test]
    fn a_connection_needs_to_be_enabled_and_active() {
        assert!(record().is_usable());

        assert!(!ConnectionRecord {
            enabled: false,
            ..record()
        }
        .is_usable());

        assert!(!ConnectionRecord {
            status: "INITIATED".into(),
            ..record()
        }
        .is_usable());
    }

    /// The names on disk, asserted rather than assumed.
    ///
    /// These records are written and read by two front ends over one workspace
    /// folder. A field renamed on this side is not a refactor, it is a record
    /// that reads back empty - which is what happened when this struct called
    /// them `toolkit` and `connectedAccountId`.
    #[test]
    fn a_record_is_read_and_written_by_the_names_already_on_disk() {
        let stored = serde_json::json!({
            "id": "conn_1",
            "toolkitSlug": "gmail",
            "name": "Gmail",
            "logo": "https://logos.composio.dev/api/gmail",
            "accountId": "acc_1",
            "composioUserId": "gmail-abc",
            "status": "ACTIVE",
            "label": "someone@example.com",
            "enabled": true,
            "enabledTools": ["GMAIL_FETCH_EMAILS"],
        });

        let record: ConnectionRecord = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(record.toolkit_slug, "gmail");
        assert_eq!(record.account_id, "acc_1");
        assert_eq!(record.composio_user_id, "gmail-abc");
        assert_eq!(record.label, "someone@example.com");
        assert_eq!(record.enabled_tools, ["GMAIL_FETCH_EMAILS"]);

        // Including the fields this struct has no opinion about: a save that
        // round-trips through it must not delete them from the user's file.
        assert_eq!(serde_json::to_value(&record).unwrap(), stored);
    }

    /// A record from before `enabled` was written reads as switched on.
    ///
    /// The other way round, every app the user had connected would quietly
    /// contribute no tools, with nothing on screen saying why.
    #[test]
    fn a_record_without_enabled_is_still_enabled() {
        let record: ConnectionRecord = serde_json::from_value(serde_json::json!({
            "id": "conn_1",
            "toolkitSlug": "gmail",
            "status": "ACTIVE",
        }))
        .unwrap();
        assert!(record.enabled);
        assert!(record.is_usable());
    }

    /// A model shown both `GMAIL_FETCH` and `read` in one list guesses wrong
    /// about which convention applies.
    #[test]
    fn the_tool_id_is_lower_cased_but_the_slug_is_not() {
        let built =
            ComposioTool::new(&record(), &tool("GMAIL_FETCH_EMAILS"), Composio::new("k")).unwrap();
        assert_eq!(built.id(), "gmail_fetch_emails");
        assert_eq!(built.slug, "GMAIL_FETCH_EMAILS");
    }

    /// A rule that reads "allow composio gmail" is one a person can write and
    /// mean.
    #[test]
    fn permission_is_scoped_to_the_whole_app() {
        let built = ComposioTool::new(&record(), &tool("GMAIL_SEND"), Composio::new("k")).unwrap();
        let request = built.permission(&json!({}));
        assert_eq!(request.key, "composio");
        assert_eq!(request.target, "gmail");
        assert_eq!(request.always.as_deref(), Some("gmail"));
    }

    /// A strict provider rejects the entire tool list over these.
    #[test]
    fn validator_constraints_are_stripped_from_the_schema() {
        let built = ComposioTool::new(&record(), &tool("X"), Composio::new("k")).unwrap();
        let schema = built.parameters().to_string();

        assert!(!schema.contains("required"), "got {schema}");
        assert!(!schema.contains("additionalProperties"), "got {schema}");
        assert!(schema.contains("query"));
    }

    /// Composio returns either a real schema or a bare bag of properties.
    #[test]
    fn a_bare_property_bag_is_wrapped_into_a_schema() {
        let bag = json!({
            "slug": "X",
            "input_parameters": { "query": { "type": "string" } }
        });
        let built = ComposioTool::new(&record(), &bag, Composio::new("k")).unwrap();
        let schema = built.parameters();

        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["query"]["type"], "string");
    }

    #[test]
    fn an_enormous_description_is_capped() {
        let long = json!({ "slug": "X", "description": "x".repeat(5_000) });
        let built = ComposioTool::new(&record(), &long, Composio::new("k")).unwrap();
        assert_eq!(built.description().chars().count(), MAX_DESCRIPTION_CHARS);
    }

    #[test]
    fn a_tool_without_a_slug_is_skipped() {
        assert!(ComposioTool::new(&record(), &json!({ "name": "x" }), Composio::new("k")).is_none());
    }

    // ── selection ───────────────────────────────────────────────────────

    /// Connecting the app was already the deliberate choice.
    #[test]
    fn an_empty_selection_means_every_tool() {
        let all = vec![tool("A"), tool("B")];
        assert_eq!(select(&all, &[]).len(), 2);
    }

    #[test]
    fn a_named_selection_is_honoured() {
        let all = vec![tool("A"), tool("B")];
        let chosen = select(&all, &["a".to_string()]);
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0]["slug"], "A");
    }

    /// When the cap bites, the ones Composio marks important are the ones
    /// worth keeping.
    #[test]
    fn important_tools_are_kept_first() {
        let mut all: Vec<Value> = (0..50).map(|i| tool(&format!("T{i}"))).collect();
        all.push(json!({
            "slug": "IMPORTANT_ONE",
            "description": "the useful one",
            "tags": ["important"]
        }));

        let chosen = select(&all, &[]);
        assert_eq!(chosen.len(), MAX_TOOLS_PER_TOOLKIT);
        assert_eq!(chosen[0]["slug"], "IMPORTANT_ONE");
    }

    // ── the provider ────────────────────────────────────────────────────

    /// An app with no Composio account simply has no Composio tools. That is
    /// not a failure state.
    #[tokio::test]
    async fn no_key_means_no_tools_and_no_error() {
        let provider = ComposioProvider::new();
        assert!(!provider.has_key());
        assert!(provider.tools(Path::new(".")).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn connecting_without_a_key_says_so() {
        let provider = ComposioProvider::new();
        let error = provider.connect(record()).await.unwrap_err();
        assert!(error.contains("API key"), "got {error}");
    }

    #[test]
    fn setting_a_blank_key_is_the_same_as_none() {
        let provider = ComposioProvider::new();
        provider.set_key(Some("   ".into()));
        assert!(!provider.has_key());
    }
}

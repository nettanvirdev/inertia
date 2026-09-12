//! Inertia, setting itself up.
//!
//! Everything this app shows is a file in the workspace folder: an agent is
//! `agents/<id>.json`, a routine is `routines/<id>.json`, a skill is a folder
//! with a SKILL.md in it. Which means an agent with the file tools could already
//! make one - and did, badly. It had to know the field names, the id rules and
//! the shape of a cron schedule; it had to be told the folder was not "outside
//! the project"; and when it got one wrong it wrote a file that loaded as a
//! record with no name, which is the kind of broken nobody notices for a week.
//!
//! So the app's own setup gets tools of its own. Not a wrapper around `write`:
//! a named list of the things Inertia is made of, each with the fields that
//! matter and validation that answers in the user's terms rather than leaving a
//! malformed record on disk. The model reads the schema instead of a document
//! about JSON, which is the difference between an agent that can add a routine
//! and one that has to be walked through it.
//!
//! This is a port of the Electron build's `src/main/tools/builtin/inertia.cjs`.
//! The tool ids, the parameter names, the permission keys, the validation
//! messages and the metadata keys are copied deliberately: the renderer already
//! renders these shapes, and the rules in a real workspace's
//! `settings/permissions.json` are already written against `inertia` and
//! `inertia_guarded`.
//!
//! ## Why the window updates by itself
//!
//! Every write here goes through the same `collections` layer the screens write
//! through, and then says so on the same channel. The window is already
//! listening - that is how a second window, or the user's own editor, shows up
//! in a list - so a routine an agent writes appears under Routines while the
//! agent is still talking, with nothing to refresh and nothing to restart.
//!
//! The emitter arrives as a plain closure rather than an `AppHandle` for two
//! reasons. A test has no window to tell, and constructing anything Tauri-shaped
//! inside the app crate's test binary takes the whole binary down on Windows
//! before a single test runs. So the coordinator passes the real `app.emit`
//! wrapper and a test passes something that records what it was handed.
//!
//! The three live sources are different and are handled as such. An MCP server,
//! an API import and a Composio connection are not only records: something has
//! to start the process, fetch the spec, or begin an OAuth handshake. Those go
//! through [`Apps`], which the app implements over the same `Workspace` the
//! Integrations screen drives, so an agent that connects a server gets a server
//! that is actually running - and a test gets a fake rather than the network.
//!
//! ## What it may not do
//!
//! Removing is a separate permission from creating, and it removes one named
//! thing per call - there is no sweep here, deliberately. Conversations,
//! history, secrets and the manifest are not reachable through these tools at
//! all: the record of what happened is the user's, and a key belongs in the one
//! file that holds keys.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use inertia_store::collections;
use inertia_store::layout::{Collection, Document};
use inertia_store::Layout;
use serde_json::{json, Map, Value};

/// The key every read and every create asks under.
///
/// Not a new name: a real workspace's `settings/permissions.json` already has
/// rules written against this exact string, and renaming it would silently
/// un-grant every one of them.
pub const PERMISSION_KEY: &str = "inertia";

/// Destroying and re-writing the rules ask under their own key, which defaults
/// to `ask`. Creating things and destroying them are not the same decision and
/// should not share one switch.
pub const GUARDED_KEY: &str = "inertia_guarded";

/// Pictures live beside the agents they belong to. One file per agent.
pub const PICTURE_DIR: &str = "agents/pictures";

/// Big enough for any avatar, small enough that it stays a portable folder.
const MAX_PICTURE_BYTES: usize = 2 * 1024 * 1024;

const IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
];

/// Tells every open window that something in the folder changed.
///
/// `Arc<dyn Fn>` rather than an `AppHandle` so the tools can be built in a test.
/// The payload is exactly what `ws.rs` emits on `workspace:changed`, because the
/// renderer's `onChanged` subscribers already read that shape.
pub type Emitter = Arc<dyn Fn(Value) + Send + Sync>;

/// An emitter that tells nobody. For a code path with no window around it.
pub fn silent() -> Emitter {
    Arc::new(|_| {})
}

/* -- the things Inertia is made of ---------------------------------------- */

/// One kind, and adding a kind is adding a variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Agent,
    Routine,
    Skill,
    Memory,
    Computer,
    Mcp,
    Api,
    App,
}

/// The order the model sees in every `kind` enum. Stable on purpose: it is part
/// of the prompt prefix every turn pays for.
const KINDS: &[Kind] = &[
    Kind::Agent,
    Kind::Routine,
    Kind::Skill,
    Kind::Memory,
    Kind::Computer,
    Kind::Mcp,
    Kind::Api,
    Kind::App,
];

/// Which live source a kind needs started, fetched or connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Live {
    Mcp,
    OpenApi,
    Composio,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Routine => "routine",
            Self::Skill => "skill",
            Self::Memory => "memory",
            Self::Computer => "computer",
            Self::Mcp => "mcp",
            Self::Api => "api",
            Self::App => "app",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        let wanted = name.trim().to_lowercase();
        KINDS.iter().copied().find(|k| k.as_str() == wanted)
    }

    fn collection(self) -> Collection {
        match self {
            Self::Agent => Collection::Agents,
            Self::Routine => Collection::Routines,
            Self::Skill => Collection::Skills,
            Self::Memory => Collection::Memories,
            Self::Computer => Collection::Computers,
            Self::Mcp => Collection::Mcp,
            Self::Api => Collection::OpenApi,
            Self::App => Collection::Composio,
        }
    }

    /// What a person calls one of these. Used in every message, so it is the
    /// user's word and not the collection's.
    fn label(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Routine => "routine",
            Self::Skill => "skill",
            Self::Memory => "memory",
            Self::Computer => "computer",
            Self::Mcp => "MCP server",
            Self::Api => "API import",
            Self::App => "connected app",
        }
    }

    /// Where it turns up on screen, so a model can tell the person where to
    /// look rather than telling them to restart.
    fn where_(self) -> &'static str {
        match self {
            Self::Agent => "the sidebar, under Agents",
            Self::Routine => "the Routines screen",
            Self::Skill => "Integrations, under Skills",
            Self::Memory => "the Memory screen",
            Self::Computer => "the Computers screen",
            Self::Mcp => "Integrations, under MCP",
            Self::Api => "Integrations, under APIs",
            Self::App => "Integrations, under Composio",
        }
    }

    /// What the model may set - deliberately not everything on the record,
    /// because `lastRun`, `stats` and `runHistory` are written by the app and a
    /// model that can set them can lie about what happened.
    fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Agent => &[
                "name",
                "handle",
                "role",
                "description",
                "systemPrompt",
                "model",
                "cwd",
                "computerId",
                "tags",
                "icon",
                "avatarColor",
                "status",
                "thinkingBudget",
                "reasoningEffort",
            ],
            Self::Routine => &[
                "name",
                "description",
                "agentId",
                "markdown",
                "schedule",
                "enabled",
                "icon",
                "tags",
                "mode",
                "approval",
            ],
            Self::Skill => &["name", "description", "instructions", "enabled", "tags", "version"],
            Self::Memory => &["title", "body", "kind", "agentId", "tags", "pinned"],
            Self::Computer => &[],
            Self::Mcp => &[
                "name", "type", "command", "args", "cwd", "env", "url", "headers", "enabled",
                "timeoutMs",
            ],
            Self::Api => &["name", "url", "text", "baseUrl", "enabled"],
            Self::App => &[],
        }
    }

    /// Why `inertia_save` will not touch this kind, when it will not.
    fn read_only(self) -> Option<&'static str> {
        match self {
            Self::Computer => Some(
                "Making a machine means provisioning it with Docker or Daytona, which these tools \
                 do not do. Ask the user to make one on the Computers screen, then assign it with \
                 inertia_save on the agent's computerId.",
            ),
            Self::App => Some(
                "Connecting an app is an OAuth handshake, so it has its own tool: \
                 inertia_connect_app. Use inertia_remove to disconnect one.",
            ),
            _ => None,
        }
    }

    fn live(self) -> Option<Live> {
        match self {
            Self::Mcp => Some(Live::Mcp),
            Self::Api => Some(Live::OpenApi),
            Self::App => Some(Live::Composio),
            _ => None,
        }
    }
}

fn kind_names() -> Vec<&'static str> {
    KINDS.iter().map(|k| k.as_str()).collect()
}

/// The sentence a model gets for a kind nobody has heard of.
fn unknown_kind(name: &str) -> Error {
    Error::Other(format!(
        "\"{name}\" is not something Inertia is made of. The kinds are: {}.",
        kind_names().join(", ")
    ))
}

fn kind_of(args: &Value) -> Result<Kind> {
    let raw = args.get("kind").and_then(Value::as_str).unwrap_or_default();
    Kind::parse(raw).ok_or_else(|| unknown_kind(raw))
}

/* -- record helpers ------------------------------------------------------- */

fn object(value: &Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    }
}

fn text(record: &Value, key: &str) -> String {
    record
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The name a person would recognise: `name`, then `title`, then the id.
fn display_name(record: &Value, id: &str) -> String {
    for key in ["name", "title"] {
        let found = text(record, key);
        if !found.is_empty() {
            return found;
        }
    }
    id.to_string()
}

/// Is this record protected from being changed by an agent?
///
/// Protection is a statement about who may change a thing, not about what it
/// is. The agent that ships with the app carries it so a model cannot delete
/// the thing that knows how the app works. Editing counts as well as deleting:
/// being able to rewrite the system prompt of the agent you may not delete is
/// the same hole with an extra step.
fn is_protected(record: &Value) -> bool {
    record.get("protected").and_then(Value::as_bool) == Some(true)
}

/// Why a change was refused, and what to do about it. Word for word the
/// Electron build's `protectedReason`.
fn protected_reason(record: &Value, what: &str) -> String {
    let name = display_name(record, "").trim().to_string();
    let name = if name.is_empty() {
        format!("That {what}")
    } else {
        name
    };
    format!(
        "{name} is protected, so it cannot be changed or removed from here. Open it in Inertia \
         and turn off Protected first, if that is really what you want."
    )
}

/// Insert a default only where the incoming record is silent.
///
/// This is the Rust of `{ default, ...record }`: a key the record carries wins,
/// even when it carries an empty string. Reproduced rather than improved,
/// because `handle: ""` staying `""` is behaviour a workspace on disk may be
/// relying on.
fn default_to(record: &mut Map<String, Value>, key: &str, value: Value) {
    record.entry(key.to_string()).or_insert(value);
}

/* -- schedules ------------------------------------------------------------ */

/// When a routine is next due, and whether its schedule can be read at all.
///
/// A thin front on [`crate::routines`], which owns the cron parser, the ISO
/// duration reader and the search forward in minutes. Two cron parsers in one
/// binary is two answers to "does this schedule ever come round", and the one
/// `inertia_save` refuses on has to be the one the scheduler will later obey -
/// or a routine is accepted here and never fires.
mod schedule {
    use serde_json::Value;

    /// `(when, why not)`, mirroring the Electron build's `{at, reason}`.
    ///
    /// `(None, None)` means "not on a clock" - manual, triggered or disabled.
    /// `(None, Some(reason))` is a schedule that cannot be read, which is what
    /// `inertia_save` refuses on: a schedule that never comes round is a
    /// routine that looks scheduled and never runs.
    pub(super) fn next_run(routine: &Value) -> (Option<String>, Option<String>) {
        let next = crate::routines::next_run(routine, jiff::Timestamp::now());
        (next.at, next.reason)
    }

    /// A schedule in words, for a screen that should not have to parse cron.
    pub(super) fn describe(routine: &Value) -> String {
        crate::routines::describe(routine)
    }
}

/* -- the live sources ----------------------------------------------------- */

/// What starting, fetching and connecting looks like from in here.
///
/// An MCP server has to be launched, an OpenAPI document has to be fetched and
/// parsed, and a Composio app is an OAuth handshake with a third party. None of
/// those belong inside a tool's `execute`, and all three have to be absent from
/// a unit test, so they sit behind one trait the app implements over the same
/// `Workspace` the Integrations screen drives.
///
/// Every method answers `Result<_, String>` rather than an `Error`: the string
/// is what the model reads, so it has to be a sentence written for the model.
#[async_trait]
pub trait Apps: Send + Sync {
    /// Composio's catalogue, as `[{slug, name}]` rows.
    async fn catalogue(&self, search: Option<&str>) -> std::result::Result<Vec<Value>, String>;

    /// Begins the OAuth handshake. `{ id, redirectUrl, status }`.
    async fn connect(&self, toolkit: &str) -> std::result::Result<Value, String>;

    /// Whether a started connection has landed.
    /// `{ name, status, label, toolkitSlug }`.
    async fn status(&self, id: &str) -> std::result::Result<Value, String>;

    /// Revokes the account and forgets the record.
    async fn disconnect(&self, id: &str) -> std::result::Result<(), String>;

    /// Saves an MCP server record and starts it. Answers the stored record.
    async fn save_mcp(
        &self,
        id: Option<&str>,
        fields: &Value,
    ) -> std::result::Result<Value, String>;

    async fn remove_mcp(&self, id: &str) -> std::result::Result<(), String>;

    /// Fetches and imports an OpenAPI document. `(record, warnings)`.
    async fn import_api(
        &self,
        fields: &Value,
    ) -> std::result::Result<(Value, Vec<String>), String>;

    /// Forget what the running import was built from, after its record changed.
    fn invalidate_api(&self);

    async fn remove_api(&self, id: &str) -> std::result::Result<(), String>;
}

/// The answer when this build has no integrations wired up.
///
/// Not a silent no-op: an agent that is told "connected apps are not available"
/// stops trying, while one whose call succeeded and did nothing will go on
/// telling the user their app is connected.
#[derive(Debug, Default)]
pub struct NoApps;

const NO_LIVE: &str = "This build cannot start servers or connect apps. Ask the user to do it on \
                       the Integrations screen.";

#[async_trait]
impl Apps for NoApps {
    async fn catalogue(&self, _search: Option<&str>) -> std::result::Result<Vec<Value>, String> {
        Err(NO_LIVE.into())
    }
    async fn connect(&self, _toolkit: &str) -> std::result::Result<Value, String> {
        Err(NO_LIVE.into())
    }
    async fn status(&self, _id: &str) -> std::result::Result<Value, String> {
        Err(NO_LIVE.into())
    }
    async fn disconnect(&self, _id: &str) -> std::result::Result<(), String> {
        Err(NO_LIVE.into())
    }
    async fn save_mcp(
        &self,
        _id: Option<&str>,
        _fields: &Value,
    ) -> std::result::Result<Value, String> {
        Err(NO_LIVE.into())
    }
    async fn remove_mcp(&self, _id: &str) -> std::result::Result<(), String> {
        Err(NO_LIVE.into())
    }
    async fn import_api(
        &self,
        _fields: &Value,
    ) -> std::result::Result<(Value, Vec<String>), String> {
        Err(NO_LIVE.into())
    }
    fn invalidate_api(&self) {}
    async fn remove_api(&self, _id: &str) -> std::result::Result<(), String> {
        Err(NO_LIVE.into())
    }
}

/// The real thing, over the workspace the Integrations screen drives.
///
/// Deliberately not built on `AppState`: it holds only the `Workspace`, so the
/// same wiring works from a routine run with no window, and so nothing here can
/// reach for Tauri's window chrome.
pub struct LiveApps {
    workspace: Arc<crate::state::Workspace>,
}

impl std::fmt::Debug for LiveApps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveApps").finish_non_exhaustive()
    }
}

/// What the user is told when Composio has no key. The same sentence the
/// Integrations screen shows, so the two do not disagree.
const NO_COMPOSIO_KEY: &str =
    "Composio is not set up yet. Add a Composio API key under Integrations first.";

impl LiveApps {
    pub fn new(workspace: Arc<crate::state::Workspace>) -> Self {
        Self { workspace }
    }

    /// The Composio key, read the way `integrations::composio_key` reads it:
    /// the name is a setting, the value is in the secret store.
    fn composio(&self) -> std::result::Result<inertia_composio::api::Composio, String> {
        let app = collections::read_document(&self.workspace.layout, Document::App, json!({}));
        let name = app
            .get("composioKeySecret")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or("COMPOSIO_API_KEY")
            .to_string();

        let key = inertia_store::secrets::get(&self.workspace.layout, &name)
            .map_err(|e| e.to_string())?
            .filter(|key| !key.trim().is_empty())
            .ok_or(NO_COMPOSIO_KEY)?;

        // Kept in step with the provider, so the tools a turn can call are
        // backed by the same account the settings pane is showing.
        if !self.workspace.composio.has_key() {
            self.workspace.composio.set_key(Some(key.clone()));
        }
        Ok(inertia_composio::api::Composio::new(key))
    }
}

#[async_trait]
impl Apps for LiveApps {
    async fn catalogue(&self, search: Option<&str>) -> std::result::Result<Vec<Value>, String> {
        let client = self.composio()?;
        let configured = client.auth_config_slugs().await.unwrap_or_default();
        let wanted = search.unwrap_or_default();
        Ok(client
            .toolkits()
            .await
            .map_err(|e| e.to_string())?
            .iter()
            .map(|item| inertia_composio::toolkit::normalise(item, &configured))
            .filter(|row| !row["slug"].as_str().unwrap_or_default().is_empty())
            .filter(|row| inertia_composio::toolkit::matches_search(row, wanted))
            .map(|row| json!({ "slug": row["slug"], "name": row["name"] }))
            .collect())
    }

    async fn connect(&self, toolkit: &str) -> std::result::Result<Value, String> {
        let client = self.composio()?;
        let auth_config = client
            .auth_config_for(toolkit)
            .await
            .map_err(|e| e.to_string())?;

        let composio_user_id = format!("{toolkit}-{}", uuid::Uuid::new_v4().simple());
        let connection = client
            .start_connection(&auth_config, Some(&composio_user_id))
            .await
            .map_err(|e| e.to_string())?;

        // The display name is read now rather than at render time, so a list of
        // connected apps stays readable with the network off, and it is not
        // worth failing a connection over.
        let described = client.toolkit(toolkit).await.unwrap_or_default();
        let name = described
            .get("name")
            .or_else(|| described.pointer("/meta/name"))
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(toolkit)
            .to_string();

        // Saved as pending, so a connection abandoned halfway is visible and can
        // be cleaned up rather than lingering only in Composio's records.
        let record = inertia_composio::provider::ConnectionRecord {
            id: connection.id.clone(),
            toolkit_slug: toolkit.to_string(),
            name,
            account_id: connection.id.clone(),
            composio_user_id,
            status: "INITIATED".into(),
            ..Default::default()
        };
        self.workspace.save_composio(&record)?;

        Ok(json!({
            "id": connection.id,
            "redirectUrl": connection.redirect_url,
            "status": record.status,
        }))
    }

    async fn status(&self, id: &str) -> std::result::Result<Value, String> {
        let record = self
            .workspace
            .composio_records()
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("No Composio connection called `{id}`."))?;
        let client = self.composio()?;
        let account = client
            .connected_account(&record.account_id)
            .await
            .map_err(|e| e.to_string())?;
        let status = account
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_string();

        Ok(json!({
            "name": record.name,
            "status": status,
            "label": record.label,
            "toolkitSlug": record.toolkit_slug,
        }))
    }

    async fn disconnect(&self, id: &str) -> std::result::Result<(), String> {
        let record = self
            .workspace
            .composio_records()
            .into_iter()
            .find(|r| r.id == id);
        // Revoking is best-effort: an account Composio has already forgotten
        // must not leave a row the user cannot delete from here.
        if let Some(record) = &record {
            if let Ok(client) = self.composio() {
                let _ = client.revoke(&record.account_id).await;
            }
        }
        self.workspace.composio.disconnect(id);
        self.workspace.delete_composio(id)
    }

    async fn save_mcp(
        &self,
        id: Option<&str>,
        fields: &Value,
    ) -> std::result::Result<Value, String> {
        let layout = &self.workspace.layout;
        let previous = match id {
            Some(id) => collections::get(layout, Collection::Mcp, id)
                .map_err(|e| e.to_string())?
                .unwrap_or_else(|| json!({})),
            None => json!({}),
        };

        let mut merged = object(&previous);
        for (key, value) in object(fields) {
            merged.insert(key, value);
        }
        default_to(&mut merged, "type", json!("stdio"));
        default_to(&mut merged, "enabled", json!(true));

        // Written through `collections` rather than the typed record, so a key
        // the window added and this build does not model survives the save.
        let saved = collections::put(layout, Collection::Mcp, Value::Object(merged))
            .map_err(|e| e.to_string())?;
        let saved_id = text(&saved, "id");

        self.workspace.mcp.remove(&saved_id);
        let enabled = saved.get("enabled").and_then(Value::as_bool) != Some(false);

        let (status, error, tool_count) = if !enabled {
            ("disabled".to_string(), String::new(), 0usize)
        } else {
            let record: inertia_mcp::client::ServerRecord =
                serde_json::from_value(saved.clone()).map_err(|e| e.to_string())?;
            match self
                .workspace
                .mcp
                .add(crate::integrations::with_secrets(layout, &record))
                .await
            {
                Ok(count) => ("connected".to_string(), String::new(), count),
                Err(message) => ("failed".to_string(), message, 0),
            }
        };

        let mut with_status = object(&saved);
        with_status.insert("status".into(), json!(status));
        with_status.insert("error".into(), json!(error));
        with_status.insert("toolCount".into(), json!(tool_count));
        collections::put(layout, Collection::Mcp, Value::Object(with_status))
            .map_err(|e| e.to_string())
    }

    async fn remove_mcp(&self, id: &str) -> std::result::Result<(), String> {
        self.workspace.mcp.remove(id);
        self.workspace.delete_mcp_record(id)
    }

    async fn import_api(
        &self,
        fields: &Value,
    ) -> std::result::Result<(Value, Vec<String>), String> {
        let url = text(fields, "url");
        let pasted = fields
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let raw = if !pasted.trim().is_empty() {
            pasted
        } else {
            reqwest::get(&url)
                .await
                .map_err(|e| format!("Could not fetch {url}: {e}"))?
                .text()
                .await
                .map_err(|e| format!("Could not read {url}: {e}"))?
        };

        let document = inertia_openapi::spec::parse(&raw).map_err(|e| e.to_string())?;

        // The id and the timestamps come from `collections` rather than being
        // invented here, so an import is named the way every other record is.
        let seed = collections::put(
            &self.workspace.layout,
            Collection::OpenApi,
            json!({
                "name": text(fields, "name"),
                "source": url,
                "baseUrl": text(fields, "baseUrl"),
                "enabled": true,
            }),
        )
        .map_err(|e| e.to_string())?;

        let mut record: inertia_openapi::provider::ImportRecord =
            serde_json::from_value(seed.clone()).map_err(|e| e.to_string())?;
        let count = self
            .workspace
            .openapi
            .add(record.clone(), &document, Default::default())
            .map_err(|e| e.to_string())?;
        record.tool_count = count;

        self.workspace.save_openapi(&record, &document)?;
        let stored = collections::get(&self.workspace.layout, Collection::OpenApi, &record.id)
            .map_err(|e| e.to_string())?
            .unwrap_or(seed);
        Ok((stored, Vec::new()))
    }

    fn invalidate_api(&self) {
        use inertia_core::tool::ToolProvider;
        self.workspace.openapi.invalidate();
    }

    async fn remove_api(&self, id: &str) -> std::result::Result<(), String> {
        self.workspace.openapi.remove(id);
        self.workspace.delete_openapi(id)
    }
}

/* -- summaries ------------------------------------------------------------ */

/// One row, small enough to list a hundred of.
///
/// A full record is a system prompt, a playbook and a permission array, and a
/// model that asked "what agents are there" wanted none of that. `inertia_get`
/// is one call away when it does.
fn summarise(kind: Kind, record: &Value) -> Value {
    let id = text(record, "id");
    let name = display_name(record, &id);
    let field = |key: &str| record.get(key).cloned().unwrap_or(Value::Null);
    let enabled = record.get("enabled").and_then(Value::as_bool) != Some(false);

    match kind {
        Kind::Agent => json!({
            "id": id,
            "name": name,
            "role": field("role"),
            "model": match text(record, "model") {
                m if m.is_empty() => "workspace default".to_string(),
                m => m,
            },
            "computerId": field("computerId"),
            "hasPicture": record.get("avatarFile").is_some_and(|v| !v.is_null()),
            "status": field("status"),
        }),
        Kind::Routine => json!({
            "id": id,
            "name": name,
            "agentId": field("agentId"),
            "enabled": enabled,
            "schedule": match text(record.get("schedule").unwrap_or(&Value::Null), "humanLabel") {
                label if label.is_empty() => schedule::describe(record),
                label => label,
            },
            "lastRun": record.pointer("/lastRun/at").cloned().unwrap_or(Value::Null),
            "lastStatus": record.pointer("/lastRun/status").cloned().unwrap_or(Value::Null),
        }),
        Kind::Skill => {
            let mut row = json!({
                "id": id, "name": name,
                "description": field("description"),
                "enabled": enabled,
            });
            // Only when there is something wrong: an empty array on every row
            // is noise the model pays for on every list.
            if let Some(problems) = record.get("problems").and_then(Value::as_array) {
                if !problems.is_empty() {
                    row["problems"] = json!(problems);
                }
            }
            row
        }
        Kind::Memory => json!({
            "id": id,
            "name": name,
            "kind": field("kind"),
            "agentId": field("agentId"),
            "pinned": record.get("pinned").and_then(Value::as_bool).unwrap_or(false),
        }),
        Kind::Computer => json!({
            "id": id,
            "name": name,
            "provider": field("provider"),
            "status": field("status"),
            "assignedAgentIds": record
                .get("assignedAgentIds")
                .cloned()
                .unwrap_or_else(|| json!([])),
        }),
        Kind::Mcp => {
            let mut row = json!({
                "id": id, "name": name,
                "type": field("type"),
                "enabled": enabled,
                "status": field("status"),
                "toolCount": record.get("toolCount").cloned().unwrap_or(json!(0)),
            });
            let error = text(record, "error");
            if !error.is_empty() {
                row["error"] = json!(error);
            }
            row
        }
        Kind::Api => json!({
            "id": id,
            "name": name,
            "baseUrl": field("baseUrl"),
            "enabled": enabled,
            "operations": record
                .get("operations")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter(|op| op.get("enabled").and_then(Value::as_bool) == Some(true))
                        .count()
                })
                .unwrap_or(0),
        }),
        Kind::App => {
            let mut row = json!({
                "id": id, "name": name,
                "toolkitSlug": field("toolkitSlug"),
                "status": field("status"),
                "enabled": enabled,
            });
            let account = text(record, "label");
            if !account.is_empty() {
                row["account"] = json!(account);
            }
            row
        }
    }
}

/// The whole record, minus what nobody can act on.
fn detail(kind: Kind, record: &Value) -> Value {
    let mut out = object(record);
    out.remove("runHistory");
    if kind == Kind::App {
        out.remove("composioUserId");
    }
    Value::Object(out)
}

/* -- shared state --------------------------------------------------------- */

/// Everything all seven tools need. Cloned into each of them, so a tool can be
/// built without the others.
#[derive(Clone)]
struct Setup {
    layout: Layout,
    emit: Emitter,
    apps: Arc<dyn Apps>,
}

impl std::fmt::Debug for Setup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Setup").field("layout", &self.layout).finish_non_exhaustive()
    }
}

impl Setup {
    /// Announce a write so every open screen redraws without being asked.
    ///
    /// The collection name is the key the renderer asks for - `plugins.mcp`,
    /// not `plugins/mcp` - because that is what `ws.rs` emits and what the
    /// store's subscribers match on.
    fn announce(&self, collection: Collection, id: &str, op: &str) {
        (self.emit)(json!({ "collection": collection.key(), "id": id, "op": op }));
    }

    fn announce_document(&self, key: &str) {
        (self.emit)(json!({ "document": key, "op": "set" }));
    }

    fn get(&self, kind: Kind, id: &str) -> Result<Option<Value>> {
        collections::get(&self.layout, kind.collection(), id)
            .map_err(|e| Error::Other(e.to_string()))
    }

    /// The record, or the sentence saying there is no such thing.
    fn require(&self, kind: Kind, id: &str) -> Result<Value> {
        self.get(kind, id)?.ok_or_else(|| {
            Error::Other(format!("There is no {} with the id {id}.", kind.label()))
        })
    }
}

/* -- inertia_list --------------------------------------------------------- */

#[derive(Debug)]
pub struct ListTool(Setup);

#[async_trait]
impl Tool for ListTool {
    fn id(&self) -> &str {
        "inertia_list"
    }

    fn description(&self) -> &str {
        "List what this Inertia workspace is made of: agents, routines, skills, memories,\n\
         computers, MCP servers, API imports and connected apps.\n\
         \n\
         Read this before changing anything. Ids are what every other tool here takes, and\n\
         a routine needs the id of the agent that will run it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": kind_names(),
                    "description": "What to list. Omit to get a count of everything."
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("*")
            .to_string();
        PermissionRequest::new(PERMISSION_KEY, target)
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(match args.get("kind").and_then(Value::as_str) {
            Some(kind) => format!("list {kind}s"),
            None => "list the workspace".to_string(),
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let Some(wanted) = args.get("kind").and_then(Value::as_str) else {
            let mut counts = Map::new();
            for kind in KINDS {
                let rows = collections::list(&self.0.layout, kind.collection());
                counts.insert(kind.as_str().to_string(), json!(rows.len()));
            }
            let counts = Value::Object(counts);
            return Ok(ToolOutcome {
                title: Some("the workspace".into()),
                output: serde_json::to_string_pretty(&counts).unwrap_or_default(),
                metadata: Some(json!({ "counts": counts })),
                images: Vec::new(),
            });
        };

        let kind = Kind::parse(wanted).ok_or_else(|| unknown_kind(wanted))?;
        let rows = collections::list(&self.0.layout, kind.collection());
        let summaries: Vec<Value> = rows.iter().map(|row| summarise(kind, row)).collect();

        Ok(ToolOutcome {
            title: Some(format!(
                "{} {}{}",
                rows.len(),
                kind.label(),
                if rows.len() == 1 { "" } else { "s" }
            )),
            output: if summaries.is_empty() {
                format!("There are no {}s yet.", kind.label())
            } else {
                serde_json::to_string_pretty(&summaries).unwrap_or_default()
            },
            metadata: Some(json!({ "kind": kind.as_str(), "count": rows.len() })),
            images: Vec::new(),
        })
    }
}

/* -- inertia_get ---------------------------------------------------------- */

#[derive(Debug)]
pub struct GetTool(Setup);

#[async_trait]
impl Tool for GetTool {
    fn id(&self) -> &str {
        "inertia_get"
    }

    fn description(&self) -> &str {
        "Read one agent, routine, skill, memory, computer, server, import or app in full. Use it \
         before changing something, because a save replaces the fields it is given and you \
         should know what is there."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": kind_names(),
                    "description": "What kind of thing it is"
                },
                "id": { "type": "string", "description": "Its id, from inertia_list" }
            },
            "required": ["kind", "id"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        PermissionRequest::new(
            PERMISSION_KEY,
            format!("{}:{}", text(args, "kind"), text(args, "id")),
        )
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("{} {}", text(args, "kind"), text(args, "id")))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let kind = kind_of(&args)?;
        let id = text(&args, "id");
        let record = self.0.require(kind, &id)?;

        Ok(ToolOutcome {
            title: Some(display_name(&record, &id)),
            output: serde_json::to_string_pretty(&detail(kind, &record)).unwrap_or_default(),
            metadata: Some(json!({ "kind": kind.as_str(), "id": id })),
            images: Vec::new(),
        })
    }
}

/* -- inertia_save --------------------------------------------------------- */

/// Fill in what the app owns, and only where the record is silent.
fn complete(kind: Kind, merged: &Map<String, Value>) -> Map<String, Value> {
    let mut record = merged.clone();
    match kind {
        Kind::Agent => {
            // A new agent is idle rather than whatever the model felt like, and
            // its handle follows its name unless one was given.
            default_to(&mut record, "status", json!("idle"));
            let handle = format!(
                "@{}",
                Value::Object(record.clone())
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase()
                    .split_whitespace()
                    .collect::<String>()
            );
            default_to(&mut record, "handle", json!(handle));
            default_to(&mut record, "role", json!("Teammate"));
            if !record.get("tags").is_some_and(Value::is_array) {
                record.insert("tags".into(), json!([]));
            }
            default_to(
                &mut record,
                "stats",
                json!({ "messages": 0, "routinesRun": 0, "tokensUsed": 0 }),
            );
        }
        Kind::Routine => {
            default_to(&mut record, "icon", json!("Repeat"));
            default_to(&mut record, "enabled", json!(true));
            default_to(&mut record, "tags", json!([]));

            let as_value = Value::Object(record.clone());
            let mut schedule = object(record.get("schedule").unwrap_or(&Value::Null));
            default_to(&mut schedule, "kind", json!("manual"));
            default_to(&mut schedule, "expression", Value::Null);
            let label = schedule
                .get("humanLabel")
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| schedule::describe(&as_value));
            schedule.insert("humanLabel".into(), json!(label));
            // The scheduler owns when it next runs. A value here would be a plan
            // the model invented for a clock it cannot see.
            schedule.insert("nextRunAt".into(), Value::Null);
            record.insert("schedule".into(), Value::Object(schedule));
        }
        Kind::Skill => {
            default_to(&mut record, "enabled", json!(true));
            default_to(&mut record, "tags", json!([]));
        }
        Kind::Memory => {
            default_to(&mut record, "kind", json!("fact"));
            default_to(&mut record, "tags", json!([]));
            default_to(&mut record, "pinned", json!(false));
            default_to(&mut record, "source", json!("agent"));
            default_to(&mut record, "confidence", json!(1));
            default_to(&mut record, "useCount", json!(0));
        }
        _ => {}
    }
    record
}

/// What is wrong with this record, in the user's terms. Every sentence here is
/// copied from the Electron build: a model that has learnt to fix one of these
/// should not meet a reworded version of it.
fn validate(kind: Kind, record: &Value) -> Option<String> {
    match kind {
        Kind::Agent => {
            if text(record, "name").is_empty() {
                return Some("An agent needs a name.".into());
            }
            None
        }
        Kind::Routine => {
            if text(record, "name").is_empty() {
                return Some("A routine needs a name.".into());
            }
            if text(record, "agentId").is_empty() {
                return Some(
                    "A routine needs an agentId: the agent that runs it. List the agents first."
                        .into(),
                );
            }
            if text(record, "markdown").is_empty() {
                return Some(
                    "A routine with no playbook does nothing. Put the instructions in `markdown`."
                        .into(),
                );
            }
            if let Some(mode) = record.get("mode") {
                let named = mode.as_str().unwrap_or_default();
                if !["chat", "plan", "autonomous", "group"].contains(&named) {
                    return Some(format!(
                        "\"{named}\" is not a mode. Use chat, plan or autonomous."
                    ));
                }
            }
            if let Some(approval) = record.get("approval") {
                let named = approval.as_str().unwrap_or_default();
                if !["ask", "edits", "auto"].contains(&named) {
                    return Some(format!(
                        "\"{named}\" is not an approval. Use ask, edits or auto."
                    ));
                }
            }

            let schedule = record.get("schedule").cloned().unwrap_or(Value::Null);
            let schedule_kind = schedule
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("manual")
                .to_string();
            if !["cron", "interval", "once", "manual", "trigger"].contains(&schedule_kind.as_str())
            {
                return Some(format!(
                    "\"{schedule_kind}\" is not a schedule kind. Use cron, interval, once, manual \
                     or trigger."
                ));
            }
            // A schedule that never comes round is a routine that looks
            // scheduled and never runs, which is the failure this whole check
            // exists for.
            if ["cron", "interval", "once"].contains(&schedule_kind.as_str()) {
                let mut probe = object(record);
                probe.insert("enabled".into(), json!(true));
                let (at, reason) = schedule::next_run(&Value::Object(probe));
                if at.is_none() {
                    let expression = schedule
                        .get("expression")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    return Some(match reason {
                        Some(reason) => format!("That schedule was not usable: {reason}"),
                        None => format!(
                            "\"{expression}\" is not a usable {schedule_kind} expression."
                        ),
                    });
                }
            }
            None
        }
        Kind::Skill => {
            if text(record, "name").is_empty() {
                return Some("A skill needs a name.".into());
            }
            if text(record, "description").is_empty() {
                return Some(
                    "A skill needs a description, and it is the only thing another agent sees \
                     before deciding to load it. Say what it covers AND when to use it."
                        .into(),
                );
            }
            if text(record, "instructions").is_empty() {
                return Some(
                    "A skill with no instructions has nothing to load. Put them in \
                     `instructions`."
                        .into(),
                );
            }
            None
        }
        Kind::Memory => {
            if text(record, "title").is_empty() {
                return Some("A memory needs a title.".into());
            }
            if text(record, "body").is_empty() {
                return Some("A memory with no body records nothing.".into());
            }
            None
        }
        _ => None,
    }
}

#[derive(Debug)]
pub struct SaveTool(Setup);

const SAVE_DESCRIPTION: &str = concat!(
    "Create or change an agent, a routine, a skill, a memory, an MCP server or an API import.\n",
    "It appears on screen immediately - there is nothing to restart and nothing to refresh.\n",
    "\n",
    "Give `id` to change something that exists, and leave it out to make a new one. Fields\n",
    "you do not send are left as they are, so changing one line of a system prompt is one\n",
    "field and not the whole record.\n",
    "\n",
    "What each kind needs:\n",
    "- agent: name. Optionally role, description, systemPrompt, model (`provider/model`),\n",
    "  cwd, computerId, icon, avatarColor, tags. Use inertia_set_picture for a photo.\n",
    "- routine: name, agentId, markdown (the playbook), schedule. Optionally mode and\n",
    "  approval.\n",
    "  schedule is {\"kind\":\"cron\",\"expression\":\"0 9 * * 1-5\"} - five fields, local time -\n",
    "  or {\"kind\":\"interval\",\"expression\":\"PT30M\"}, {\"kind\":\"once\",\"expression\":\"<ISO time>\"}\n",
    "  for a single run at a time, or {\"kind\":\"manual\"}. A routine runs\n",
    "  unattended, so write a playbook that decides rather than one that asks.\n",
    "  mode is chat, plan or autonomous (default autonomous: every tool). approval is\n",
    "  auto, edits or ask (default auto). Unattended, a call that would ask a person is\n",
    "  refused instead, so auto is what lets a routine send the email or run the\n",
    "  command; choose edits or ask only for a routine the person wants held back.\n",
    "- skill: name, description, instructions. The description is the only thing an agent\n",
    "  sees before loading it, so say what it covers AND when to use it.\n",
    "- memory: title, body.\n",
    "- mcp: name, type (stdio or http), then command+args, or url. Use {secret:NAME} in env\n",
    "  or headers rather than a real key. It is started as soon as it is saved.\n",
    "- api: name and url (an OpenAPI spec), or text. Reads are enabled, writes are not.\n",
    "\n",
    "Skills, agents and routines are files in the workspace folder. You can also read them\n",
    "with the file tools, but write them through here: this validates them first."
);

#[async_trait]
impl Tool for SaveTool {
    fn id(&self) -> &str {
        "inertia_save"
    }

    fn description(&self) -> &str {
        SAVE_DESCRIPTION
    }

    fn parameters(&self) -> Value {
        let writable: Vec<&str> = KINDS
            .iter()
            .filter(|kind| kind.read_only().is_none())
            .map(|kind| kind.as_str())
            .collect();
        json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": writable,
                    "description": "What to create or change"
                },
                "id": {
                    "type": "string",
                    "description": "The id of an existing one. Leave it out to create."
                },
                "fields": {
                    "type": "object",
                    // Open, because the keys differ per kind and dropping the
                    // ones this schema did not name would save a record with
                    // the system prompt silently missing.
                    "additionalProperties": true,
                    "description": "The fields to set, as an object. See the list above for each kind."
                }
            },
            "required": ["kind", "fields"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    /// Writing is one permission; the model may not widen its own rules with it.
    fn permission(&self, args: &Value) -> PermissionRequest {
        let kind = text(args, "kind");
        let id = match text(args, "id") {
            id if id.is_empty() => "new".to_string(),
            id => id,
        };
        PermissionRequest::new(PERMISSION_KEY, format!("{kind}:{id}"))
            .with_always(format!("{kind}:*"))
    }

    fn render(&self, args: &Value) -> Option<String> {
        let verb = if text(args, "id").is_empty() { "create" } else { "update" };
        let fields = args.get("fields").cloned().unwrap_or(Value::Null);
        let what = [text(&fields, "name"), text(&fields, "title"), text(args, "id")]
            .into_iter()
            .find(|one| !one.is_empty())
            .unwrap_or_default();
        Some(format!("{verb} {} {what}", text(args, "kind")).trim().to_string())
    }

    /// Models send `fields` as a JSON string about as often as they send an
    /// object, and refusing that is a turn spent on punctuation.
    fn normalize(&self, args: Value) -> Value {
        let mut map = object(&args);
        if let Some(Value::String(raw)) = map.get("fields") {
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                map.insert("fields".into(), parsed);
            }
        }
        Value::Object(map)
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let kind = kind_of(&args)?;
        if let Some(reason) = kind.read_only() {
            return Err(Error::Other(reason.to_string()));
        }

        let incoming = args.get("fields").cloned().unwrap_or_else(|| json!({}));
        let incoming_map = object(&incoming);
        let unknown: Vec<&str> = incoming_map
            .keys()
            .map(String::as_str)
            .filter(|key| *key != "id" && !kind.fields().contains(key))
            .collect();
        if !unknown.is_empty() {
            return Err(Error::Other(format!(
                "A {} has no {}. The fields you can set are: {}.",
                kind.label(),
                unknown.join(", "),
                kind.fields().join(", ")
            )));
        }

        let id = match text(&args, "id") {
            id if id.is_empty() => None,
            id => Some(id),
        };
        let previous = match &id {
            Some(id) => self.0.get(kind, id)?,
            None => None,
        };
        if let (Some(id), None) = (&id, &previous) {
            return Err(Error::Other(format!(
                "There is no {} with the id {id}. Leave the id out to create one, or list them \
                 first.",
                kind.label()
            )));
        }
        // Protection covers editing as well as deleting.
        if let Some(record) = &previous {
            if is_protected(record) {
                return Err(Error::Other(protected_reason(record, kind.label())));
            }
        }

        // The live sources are not only records: something has to start, fetch
        // or connect. They go through the same seam the screens call.
        match kind.live() {
            Some(Live::Mcp) => {
                let saved = self
                    .0
                    .apps
                    .save_mcp(id.as_deref(), &incoming)
                    .await
                    .map_err(Error::Other)?;
                let saved_id = text(&saved, "id");
                let name = display_name(&saved, &saved_id);
                self.0.announce(
                    kind.collection(),
                    &saved_id,
                    if previous.is_some() { "patch" } else { "put" },
                );
                return Ok(ToolOutcome {
                    title: Some(name.clone()),
                    output: format!(
                        "{} the MCP server {name}. It is starting now; its tools appear as \
                         {saved_id}_* once it is up.",
                        if previous.is_some() { "Updated" } else { "Added" }
                    ),
                    metadata: Some(json!({ "kind": kind.as_str(), "id": saved_id })),
                    images: Vec::new(),
                });
            }
            Some(Live::OpenApi) => {
                if let Some(id) = &id {
                    let saved = collections::patch(
                        &self.0.layout,
                        kind.collection(),
                        id,
                        incoming.clone(),
                    )
                    .map_err(|e| Error::Other(e.to_string()))?;
                    self.0.apps.invalidate_api();
                    self.0.announce(kind.collection(), id, "patch");
                    let name = display_name(&saved, id);
                    return Ok(ToolOutcome {
                        title: Some(name.clone()),
                        output: format!("Updated the API import {name}."),
                        metadata: Some(json!({ "kind": kind.as_str(), "id": id })),
                        images: Vec::new(),
                    });
                }
                if text(&incoming, "url").is_empty()
                    && incoming
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    return Err(Error::Other(
                        "An API import needs `url` (a link to an OpenAPI spec) or `text` (the \
                         spec itself)."
                            .into(),
                    ));
                }
                let (record, warnings) = self
                    .0
                    .apps
                    .import_api(&incoming)
                    .await
                    .map_err(Error::Other)?;
                let record_id = text(&record, "id");
                let name = display_name(&record, &record_id);
                self.0.announce(kind.collection(), &record_id, "put");

                let operations = record
                    .get("operations")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                let mut lines = vec![
                    format!("Imported {name} with {operations} operations."),
                    "Reads are enabled and writes are not; the user turns those on under \
                     Integrations."
                        .to_string(),
                ];
                lines.extend(warnings.into_iter().take(5));
                return Ok(ToolOutcome {
                    title: Some(name),
                    output: lines.join("\n"),
                    metadata: Some(json!({ "kind": kind.as_str(), "id": record_id })),
                    images: Vec::new(),
                });
            }
            _ => {}
        }

        let mut merged = object(previous.as_ref().unwrap_or(&Value::Null));
        for (key, value) in incoming_map {
            merged.insert(key, value);
        }
        let mut complete = complete(kind, &merged);
        if let Some(problem) = validate(kind, &Value::Object(complete.clone())) {
            return Err(Error::Other(problem));
        }
        if let Some(id) = &id {
            complete.insert("id".into(), json!(id));
        }

        let saved = collections::put(&self.0.layout, kind.collection(), Value::Object(complete))
            .map_err(|e| Error::Other(e.to_string()))?;
        let saved_id = text(&saved, "id");
        self.0.announce(
            kind.collection(),
            &saved_id,
            if previous.is_some() { "patch" } else { "put" },
        );

        let name = display_name(&saved, &saved_id);
        let mut output = format!(
            "{} the {} {name} (id {saved_id}).",
            if previous.is_some() { "Updated" } else { "Created" },
            kind.label()
        );
        if previous.is_none() {
            output.push_str(&format!(" It is on screen now, in {}.", kind.where_()));
        }

        Ok(ToolOutcome {
            title: Some(name),
            output,
            metadata: Some(json!({
                "kind": kind.as_str(),
                "id": saved_id,
                "created": previous.is_none(),
            })),
            images: Vec::new(),
        })
    }
}

/* -- inertia_remove ------------------------------------------------------- */

#[derive(Debug)]
pub struct RemoveTool(Setup);

#[async_trait]
impl Tool for RemoveTool {
    fn id(&self) -> &str {
        "inertia_remove"
    }

    fn description(&self) -> &str {
        "Delete one agent, routine, skill, memory, MCP server or API import, or disconnect one\n\
         app. One named thing per call - there is no way to clear a whole folder here.\n\
         \n\
         Deleting an agent does not delete its conversations, and disconnecting an app revokes\n\
         the account with Composio. Neither can be undone from inside Inertia, so say what you\n\
         are about to remove before you do it."
    }

    fn parameters(&self) -> Value {
        let removable: Vec<&str> = KINDS
            .iter()
            .filter(|kind| **kind != Kind::Computer)
            .map(|kind| kind.as_str())
            .collect();
        json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": removable,
                    "description": "What kind of thing to remove"
                },
                "id": { "type": "string", "description": "Its id, from inertia_list" }
            },
            "required": ["kind", "id"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    /// Its own key, and it asks by default.
    fn permission(&self, args: &Value) -> PermissionRequest {
        let kind = text(args, "kind");
        PermissionRequest::new(GUARDED_KEY, format!("{kind}:{}", text(args, "id")))
            .with_always(format!("{kind}:*"))
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("remove {} {}", text(args, "kind"), text(args, "id")))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let kind = kind_of(&args)?;
        let id = text(&args, "id");
        let record = self.0.require(kind, &id)?;
        if is_protected(&record) {
            return Err(Error::Other(protected_reason(&record, kind.label())));
        }
        let name = display_name(&record, &id);
        let metadata = Some(json!({ "kind": kind.as_str(), "id": id }));

        match kind.live() {
            Some(Live::Mcp) => {
                self.0.apps.remove_mcp(&id).await.map_err(Error::Other)?;
                self.0.announce(kind.collection(), &id, "remove");
                return Ok(ToolOutcome {
                    title: Some(name.clone()),
                    output: format!("Removed the MCP server {name}."),
                    metadata,
                    images: Vec::new(),
                });
            }
            Some(Live::OpenApi) => {
                self.0.apps.remove_api(&id).await.map_err(Error::Other)?;
                self.0.announce(kind.collection(), &id, "remove");
                return Ok(ToolOutcome {
                    title: Some(name.clone()),
                    output: format!("Removed the API import {name}."),
                    metadata,
                    images: Vec::new(),
                });
            }
            Some(Live::Composio) => {
                self.0.apps.disconnect(&id).await.map_err(Error::Other)?;
                self.0.announce(kind.collection(), &id, "remove");
                return Ok(ToolOutcome {
                    title: Some(name.clone()),
                    output: format!("Disconnected {name}."),
                    metadata,
                    images: Vec::new(),
                });
            }
            None => {}
        }

        // An agent's picture is its own file and goes with it, or the folder
        // fills with portraits of teammates nobody has.
        if kind == Kind::Agent {
            if let Some(file) = record.get("avatarFile").and_then(Value::as_str) {
                if let Ok(path) = inertia_store::fsx::resolve_inside(self.0.layout.root(), file) {
                    let _ = inertia_store::fsx::remove(&path);
                }
            }
        }

        collections::remove(&self.0.layout, kind.collection(), &id)
            .map_err(|e| Error::Other(e.to_string()))?;
        self.0.announce(kind.collection(), &id, "remove");

        Ok(ToolOutcome {
            title: Some(name.clone()),
            output: format!("Removed the {} {name}.", kind.label()),
            metadata,
            images: Vec::new(),
        })
    }
}

/* -- inertia_set_picture -------------------------------------------------- */

/// Where an agent's picture lives, as a path built one component at a time.
///
/// Never `join("agents/pictures")`: on Windows the embedded slash survives into
/// the `PathBuf` and compares unequal to the same path built properly, which
/// makes "did I already write this file" answer wrong.
fn picture_path(root: &std::path::Path, id: &str, extension: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in PICTURE_DIR.split('/') {
        path.push(part);
    }
    path.push(format!("{id}.{extension}"));
    path
}

/// The relative path stored on the record. Always forward slashes: the renderer
/// builds a URL out of it, and the folder is meant to be portable.
fn picture_reference(id: &str, extension: &str) -> String {
    format!("{PICTURE_DIR}/{id}.{extension}")
}

#[derive(Debug)]
pub struct SetPictureTool(Setup);

#[async_trait]
impl Tool for SetPictureTool {
    fn id(&self) -> &str {
        "inertia_set_picture"
    }

    fn description(&self) -> &str {
        "Give an agent a picture, the way the user's own profile has one.\n\
         \n\
         Pass `url` to fetch an image, or `path` to a PNG, JPEG, WebP or GIF on this machine -\n\
         one you generated is fine. Pass `clear: true` to take it away and go back to the\n\
         letters. The picture is copied into the workspace folder, so it travels with it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agentId": { "type": "string", "description": "The agent's id, from inertia_list" },
                "url": { "type": "string", "description": "An https link to the image" },
                "path": { "type": "string", "description": "A path to an image file on this machine" },
                "clear": { "type": "boolean", "description": "Remove the picture instead of setting one" }
            },
            "required": ["agentId"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        PermissionRequest::new(PERMISSION_KEY, format!("agent:{}", text(args, "agentId")))
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(
            if args.get("clear").and_then(Value::as_bool) == Some(true) {
                "clear a picture".into()
            } else {
                "set an agent's picture".to_string()
            },
        )
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let agent_id = text(&args, "agentId");
        let agent = self.0.get(Kind::Agent, &agent_id)?.ok_or_else(|| {
            Error::Other(format!("There is no agent with the id {agent_id}."))
        })?;
        let id = text(&agent, "id");
        let name = display_name(&agent, &id);
        let now = jiff::Timestamp::now().as_millisecond();

        if args.get("clear").and_then(Value::as_bool) == Some(true) {
            if let Some(file) = agent.get("avatarFile").and_then(Value::as_str) {
                if let Ok(path) = inertia_store::fsx::resolve_inside(self.0.layout.root(), file) {
                    let _ = inertia_store::fsx::remove(&path);
                }
            }
            collections::patch(
                &self.0.layout,
                Collection::Agents,
                &id,
                json!({ "avatarFile": Value::Null, "avatarUpdatedAt": now }),
            )
            .map_err(|e| Error::Other(e.to_string()))?;
            self.0.announce(Collection::Agents, &id, "patch");

            return Ok(ToolOutcome {
                title: Some(name.clone()),
                output: format!("{name} is back to its initials."),
                metadata: Some(json!({ "agentId": id, "cleared": true })),
                images: Vec::new(),
            });
        }

        let url = text(&args, "url");
        let source_path = text(&args, "path");
        let (bytes, mime) = if !url.is_empty() {
            let response = reqwest::get(&url)
                .await
                .map_err(|e| Error::Other(format!("That image could not be fetched: {e}.")))?;
            if !response.status().is_success() {
                let status = response.status();
                return Err(Error::Other(format!(
                    "That image could not be fetched: {} {}.",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                )));
            }
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            let bytes = response
                .bytes()
                .await
                .map_err(|e| Error::Other(format!("That image could not be read: {e}.")))?
                .to_vec();
            (bytes, mime)
        } else if !source_path.is_empty() {
            // Resolved against the workspace root, which is what a tool running
            // with no project folder has; an absolute path is left alone.
            let candidate = PathBuf::from(&source_path);
            let file = if candidate.is_absolute() {
                candidate
            } else {
                self.0.layout.root().join(&candidate)
            };
            let bytes = std::fs::read(&file).map_err(|_| {
                Error::Other(format!("There is no readable file at {}.", file.display()))
            })?;
            let mime = match file
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or_default()
                .to_lowercase()
                .as_str()
            {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "gif" => "image/gif",
                _ => "",
            };
            (bytes, mime.to_string())
        } else {
            return Err(Error::Other(
                "Give either `url` or `path`, or `clear: true` to remove the picture.".into(),
            ));
        };

        let Some((_, extension)) = IMAGE_TYPES.iter().find(|(kind, _)| *kind == mime) else {
            return Err(Error::Other(format!(
                "That is a {}, and a picture has to be a PNG, JPEG, WebP or GIF.",
                if mime.is_empty() { "file of unknown type" } else { &mime }
            )));
        };
        if bytes.len() > MAX_PICTURE_BYTES {
            return Err(Error::Other(format!(
                "That image is {}KB and the limit is 2MB. Use a smaller one.",
                bytes.len().div_ceil(1024)
            )));
        }

        // One file per agent, overwritten, so replacing a picture does not leave
        // the old one behind. The extension is part of the name because the
        // window reads the type back off it.
        for (_, ext) in IMAGE_TYPES {
            let _ = inertia_store::fsx::remove(&picture_path(self.0.layout.root(), &id, ext));
        }
        let target = picture_path(self.0.layout.root(), &id, extension);
        if let Some(parent) = target.parent() {
            inertia_store::fsx::ensure_dir(parent).map_err(|e| Error::Other(e.to_string()))?;
        }
        std::fs::write(&target, &bytes).map_err(|e| {
            Error::Other(format!("The picture could not be saved: {e}."))
        })?;

        let reference = picture_reference(&id, extension);
        collections::patch(
            &self.0.layout,
            Collection::Agents,
            &id,
            json!({ "avatarFile": reference, "avatarUpdatedAt": now }),
        )
        .map_err(|e| Error::Other(e.to_string()))?;
        self.0.announce(Collection::Agents, &id, "patch");

        Ok(ToolOutcome {
            title: Some(name.clone()),
            output: format!(
                "{name} now has a picture, saved as {reference}. It is showing already."
            ),
            metadata: Some(json!({ "agentId": id, "file": reference, "bytes": bytes.len() })),
            images: Vec::new(),
        })
    }
}

/* -- inertia_connect_app -------------------------------------------------- */

#[derive(Debug)]
pub struct ConnectAppTool(Setup);

#[async_trait]
impl Tool for ConnectAppTool {
    fn id(&self) -> &str {
        "inertia_connect_app"
    }

    fn description(&self) -> &str {
        "Connect one of the user's apps - Gmail, Slack, Notion, GitHub and several hundred\n\
         others - through Composio, so its actions become tools.\n\
         \n\
         Pass no `toolkit` to search the catalogue. Pass one to start the connection: you get\n\
         back a link, and the user has to open it and sign in. Give them the link, then check\n\
         `status` with the id you were handed until it says ACTIVE.\n\
         \n\
         The tools appear as <toolkit>_* on the NEXT turn, not this one."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "toolkit": {
                    "type": "string",
                    "description": "The app to connect, as its slug: \"gmail\", \"slack\", \"notion\""
                },
                "search": { "type": "string", "description": "Search the catalogue instead of connecting" },
                "status": {
                    "type": "string",
                    "description": "A connection id from an earlier call, to ask whether it landed"
                }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = [text(args, "toolkit"), text(args, "status")]
            .into_iter()
            .find(|one| !one.is_empty())
            .unwrap_or_else(|| "search".to_string());
        PermissionRequest::new(PERMISSION_KEY, target)
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(if !text(args, "status").is_empty() {
            "check a connection".to_string()
        } else if !text(args, "toolkit").is_empty() {
            format!("connect {}", text(args, "toolkit"))
        } else {
            "search apps".to_string()
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let status_id = text(&args, "status");
        if !status_id.is_empty() {
            let row = self
                .0
                .apps
                .status(&status_id)
                .await
                .map_err(Error::Other)?;
            let state = text(&row, "status").to_uppercase();
            let name = display_name(&row, &status_id);
            let label = text(&row, "label");
            let output = if state == "ACTIVE" {
                format!(
                    "{name} is connected{}. Its tools are {}_* and they are available from your \
                     next turn.",
                    if label.is_empty() { String::new() } else { format!(" as {label}") },
                    text(&row, "toolkitSlug").to_lowercase()
                )
            } else {
                format!(
                    "{name} is still {}. The user has not finished signing in yet.",
                    if state.is_empty() { "pending" } else { &state }
                )
            };
            return Ok(ToolOutcome {
                title: Some(name),
                output,
                metadata: Some(json!({ "id": status_id, "status": row.get("status") })),
                images: Vec::new(),
            });
        }

        let toolkit = text(&args, "toolkit");
        if toolkit.is_empty() {
            let search = text(&args, "search");
            let rows = self
                .0
                .apps
                .catalogue(if search.is_empty() { None } else { Some(&search) })
                .await
                .map_err(Error::Other)?;
            let rows: Vec<Value> = rows.into_iter().take(40).collect();
            return Ok(ToolOutcome {
                title: Some(if search.is_empty() {
                    "the app catalogue".to_string()
                } else {
                    format!("apps matching {search}")
                }),
                output: if rows.is_empty() {
                    format!("Nothing in the catalogue matches {search}.")
                } else {
                    serde_json::to_string_pretty(&rows).unwrap_or_default()
                },
                metadata: Some(json!({ "count": rows.len() })),
                images: Vec::new(),
            });
        }

        let started = self
            .0
            .apps
            .connect(&toolkit.to_lowercase())
            .await
            .map_err(Error::Other)?;
        let connection_id = text(&started, "id");
        let redirect = text(&started, "redirectUrl");
        self.0.announce(Collection::Composio, &connection_id, "put");

        Ok(ToolOutcome {
            title: Some(toolkit.clone()),
            output: [
                format!("Started connecting {toolkit}."),
                String::new(),
                "Ask the user to open this link and sign in:".to_string(),
                redirect.clone(),
                String::new(),
                format!("Then call this tool again with status: \"{connection_id}\" to see whether it landed."),
            ]
            .join("\n"),
            metadata: Some(json!({
                "id": connection_id,
                "redirectUrl": redirect,
                "status": started.get("status"),
            })),
            images: Vec::new(),
        })
    }
}

/* -- inertia_set_rules ---------------------------------------------------- */

/// Every permission key a rule may be written about.
///
/// The twin of `TOOL_KEYS` in `src/shared/tools.js`, in the same order. A key
/// missing here is a rule the model cannot write even though the settings screen
/// offers the switch, so the two lists have to say the same thing.
pub const TOOL_KEYS: &[&str] = &[
    "read",
    "edit",
    "glob",
    "grep",
    "shell",
    "computer",
    "browser",
    "terminal_read",
    "delete_everything",
    "external_directory",
    "task",
    "skill",
    "memory",
    "load_tools",
    "todowrite",
    "question",
    "doom_loop",
    "failures",
    "inertia",
    "inertia_guarded",
    "mcp",
    "openapi",
    "composio",
];

const ACTIONS: &[&str] = &["allow", "ask", "deny"];

#[derive(Debug)]
pub struct SetRulesTool(Setup);

#[async_trait]
impl Tool for SetRulesTool {
    fn id(&self) -> &str {
        "inertia_set_rules"
    }

    fn description(&self) -> &str {
        // Built once, because the key list is data and a hand-written copy of it
        // in prose is the copy that goes stale.
        static TEXT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        TEXT.get_or_init(|| {
            [
                "Change what an agent is allowed to do, or what the whole workspace is allowed to do.",
                "",
                "A rule is {tool, pattern, action}: the tool key, what it is written about (`*` for",
                "everything), and allow, ask or deny. The most specific matching rule wins, so the order",
                "does not matter. Rules you do not mention are left alone; a rule for a tool and pattern",
                "that already has one replaces it.",
                "",
                &format!("The tool keys are: {}.", TOOL_KEYS.join(", ")),
                "",
                "This widens what an agent may do, including what it may do to Inertia itself, so it",
                "asks every time unless the user has said otherwise.",
            ]
            .join("\n")
        })
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agentId": {
                    "type": "string",
                    "description": "The agent these rules are for. Leave it out for the whole workspace."
                },
                "rules": {
                    "type": "array",
                    "description": "The rules to set",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tool": { "type": "string", "description": "The tool key" },
                            "pattern": { "type": "string", "description": "What the rule is about. `*` means everything." },
                            "action": { "type": "string", "enum": ACTIONS, "description": "allow, ask or deny" }
                        },
                        "required": ["tool", "action"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["rules"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = match text(args, "agentId") {
            id if id.is_empty() => "rules:workspace".to_string(),
            id => format!("rules:{id}"),
        };
        PermissionRequest::new(GUARDED_KEY, target)
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(if text(args, "agentId").is_empty() {
            "change the workspace permissions".into()
        } else {
            "change an agent's permissions".to_string()
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let incoming: Vec<Value> = args
            .get("rules")
            .and_then(Value::as_array)
            .map(|rules| {
                rules
                    .iter()
                    .map(|rule| {
                        json!({
                            "tool": text(rule, "tool"),
                            "pattern": match text(rule, "pattern") {
                                p if p.is_empty() => "*".to_string(),
                                p => p,
                            },
                            "action": text(rule, "action"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if incoming.is_empty() {
            return Err(Error::Other(
                "No rules were given, so there is nothing to change.".into(),
            ));
        }

        let unknown: Vec<String> = incoming
            .iter()
            .map(|rule| text(rule, "tool"))
            .filter(|tool| !TOOL_KEYS.contains(&tool.as_str()))
            .collect();
        if !unknown.is_empty() {
            return Err(Error::Other(format!(
                "No tool is called {}. The keys are: {}.",
                unknown.join(", "),
                TOOL_KEYS.join(", ")
            )));
        }

        let agent_id = text(&args, "agentId");
        // Read and written as a document rather than through the typed
        // `Permissions` struct, because a real workspace's permissions file
        // carries a top-level `rules` array this build does not model, and a
        // typed round trip would delete it.
        let doc = collections::read_document(&self.0.layout, Document::Permissions, json!({}));
        let mut doc = object(&doc);

        let current: Vec<Value> = if agent_id.is_empty() {
            doc.get("workspace").and_then(Value::as_array).cloned().unwrap_or_default()
        } else {
            doc.get("agents")
                .and_then(|agents| agents.get(&agent_id))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };

        // Replace by (tool, pattern) rather than appending: two rules about the
        // same thing is a ruleset nobody can read, and the merge would keep the
        // first one anyway.
        let mut next: Vec<Value> = current
            .into_iter()
            .filter(|rule| {
                let tool = text(rule, "tool");
                let pattern = match text(rule, "pattern") {
                    p if p.is_empty() => "*".to_string(),
                    p => p,
                };
                !incoming
                    .iter()
                    .any(|one| text(one, "tool") == tool && text(one, "pattern") == pattern)
            })
            .collect();
        next.extend(incoming.iter().cloned());

        if agent_id.is_empty() {
            doc.insert("workspace".into(), Value::Array(next));
        } else {
            let mut agents = object(doc.get("agents").unwrap_or(&Value::Null));
            agents.insert(agent_id.clone(), Value::Array(next));
            doc.insert("agents".into(), Value::Object(agents));
        }

        collections::write_document(&self.0.layout, Document::Permissions, &Value::Object(doc))
            .map_err(|e| Error::Other(e.to_string()))?;
        self.0.announce_document(Document::Permissions.key());

        let spelled: Vec<String> = incoming
            .iter()
            .map(|rule| {
                format!(
                    "{} {} -> {}",
                    text(rule, "tool"),
                    text(rule, "pattern"),
                    text(rule, "action")
                )
            })
            .collect();

        Ok(ToolOutcome {
            title: Some(if agent_id.is_empty() {
                "workspace rules".to_string()
            } else {
                format!("rules for {agent_id}")
            }),
            output: format!(
                "Set {} rule{}: {}.",
                incoming.len(),
                if incoming.len() == 1 { "" } else { "s" },
                spelled.join(", ")
            ),
            metadata: Some(json!({
                "agentId": if agent_id.is_empty() { Value::Null } else { json!(agent_id) },
                "rules": incoming,
            })),
            images: Vec::new(),
        })
    }
}

/* -- the set ------------------------------------------------------------- */

/// The seven tools, built for one workspace.
///
/// `layout` is the workspace folder these write into; `emit` is what tells the
/// open windows a record changed, and is called with the same payload `ws.rs`
/// emits on `workspace:changed`; `apps` is the seam onto the live sources, which
/// is [`LiveApps`] in the app and a fake in a test.
///
/// Returned as a flat list so the caller drops them into the registry alongside
/// every other builtin. Mode filtering is the caller's job: `inertia_save` and
/// the rest are already in `inertia_agent::prompt::MUTATING`, so a Chat or Plan
/// turn withholds them without anything here knowing about modes.
pub fn inertia_tools(layout: Layout, emit: Emitter, apps: Arc<dyn Apps>) -> Vec<Arc<dyn Tool>> {
    let setup = Setup { layout, emit, apps };
    vec![
        Arc::new(ListTool(setup.clone())),
        Arc::new(GetTool(setup.clone())),
        Arc::new(SaveTool(setup.clone())),
        Arc::new(RemoveTool(setup.clone())),
        Arc::new(SetPictureTool(setup.clone())),
        Arc::new(ConnectAppTool(setup.clone())),
        Arc::new(SetRulesTool(setup)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::permission::Action;
    use inertia_core::tool::{Decision, PermissionGate};
    use parking_lot::Mutex;

    /// The gate is not what these tests are about: these tools ask for
    /// permission the way every other tool does, and the registry runs that
    /// pipeline.
    #[derive(Debug)]
    struct Allow;

    #[async_trait]
    impl PermissionGate for Allow {
        async fn ask(&self, _request: &PermissionRequest) -> Result<Decision> {
            Ok(Decision::Allow)
        }
        async fn verdict(&self, _key: &str, _target: &str) -> Action {
            Action::Allow
        }
    }

    /// Everything a test needs: the folder, and what the window was told.
    struct Bench {
        _dir: tempfile::TempDir,
        layout: Layout,
        events: Arc<Mutex<Vec<Value>>>,
        tools: Vec<Arc<dyn Tool>>,
    }

    impl Bench {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("a temp dir");
            let layout = Layout::new(dir.path());
            layout.scaffold().expect("a scaffolded workspace");
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink = events.clone();
            let emit: Emitter = Arc::new(move |payload| sink.lock().push(payload));
            let tools = inertia_tools(layout.clone(), emit, Arc::new(NoApps));
            Self { _dir: dir, layout, events, tools }
        }

        fn tool(&self, id: &str) -> Arc<dyn Tool> {
            self.tools
                .iter()
                .find(|tool| tool.id() == id)
                .cloned()
                .unwrap_or_else(|| panic!("no tool called {id}"))
        }

        /// Drives the real `execute`, through the tool's own `normalize` so a
        /// test exercises the repair pass the app runs.
        async fn call(&self, id: &str, args: Value) -> std::result::Result<ToolOutcome, String> {
            let tool = self.tool(id);
            let args = tool.normalize(args);
            tool.execute(args, &ctx()).await.map_err(|e| e.to_string())
        }

        fn record(&self, collection: Collection, id: &str) -> Value {
            collections::get(&self.layout, collection, id)
                .expect("the collection was readable")
                .unwrap_or_else(|| panic!("no record {id}"))
        }
    }

    fn ctx() -> ToolContext {
        ToolContext {
            root: PathBuf::from("."),
            session: SessionId::from_existing("t1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(Allow),
        }
    }

    fn id_of(outcome: &ToolOutcome) -> String {
        outcome
            .metadata
            .as_ref()
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /* -- listing --------------------------------------------------------- */

    #[tokio::test]
    async fn listing_agents_answers_the_records_that_are_on_disk() {
        let bench = Bench::new();
        for (id, name) in [("agent-a", "Mercury"), ("agent-b", "Iris")] {
            collections::put(
                &bench.layout,
                Collection::Agents,
                json!({ "id": id, "name": name, "role": "Teammate" }),
            )
            .expect("the agent was written");
        }

        let out = bench.call("inertia_list", json!({ "kind": "agent" })).await.expect("listed");
        assert_eq!(out.title.as_deref(), Some("2 agents"));
        let rows: Vec<Value> = serde_json::from_str(&out.output).expect("rows");
        let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap_or("")).collect();
        assert!(names.contains(&"Mercury") && names.contains(&"Iris"), "{names:?}");
        // A model that asked what agents there are did not ask for the system
        // prompt of each one.
        assert_eq!(rows[0]["model"], json!("workspace default"));
    }

    #[tokio::test]
    async fn an_empty_kind_says_so_plainly_rather_than_answering_an_empty_array() {
        let bench = Bench::new();
        let out = bench.call("inertia_list", json!({ "kind": "routine" })).await.expect("listed");
        assert_eq!(out.output, "There are no routines yet.");
    }

    #[tokio::test]
    async fn no_kind_counts_everything_under_the_names_the_model_writes() {
        let bench = Bench::new();
        collections::put(&bench.layout, Collection::Agents, json!({ "name": "Mercury" }))
            .expect("an agent");
        let out = bench.call("inertia_list", json!({})).await.expect("counted");
        let counts = &out.metadata.expect("metadata")["counts"];
        assert_eq!(counts["agent"], json!(1));
        assert_eq!(counts["routine"], json!(0));
        // Every kind is present, including the ones nothing can create.
        for name in kind_names() {
            assert!(counts.get(name).is_some(), "{name} was not counted");
        }
    }

    #[tokio::test]
    async fn a_kind_nobody_has_heard_of_is_answered_with_the_list_of_kinds() {
        let bench = Bench::new();
        let message = bench
            .call("inertia_list", json!({ "kind": "widget" }))
            .await
            .expect_err("a refusal");
        assert!(message.contains("not something Inertia is made of"), "{message}");
        assert!(message.contains("agent, routine, skill"), "{message}");
    }

    /* -- getting --------------------------------------------------------- */

    #[tokio::test]
    async fn one_record_comes_back_in_full_by_the_id_it_was_saved_under() {
        let bench = Bench::new();
        let made = bench
            .call(
                "inertia_save",
                json!({ "kind": "agent", "fields": { "name": "Mercury", "systemPrompt": "Be brief." } }),
            )
            .await
            .expect("saved");
        let id = id_of(&made);

        let out = bench
            .call("inertia_get", json!({ "kind": "agent", "id": id }))
            .await
            .expect("read back");
        assert_eq!(out.title.as_deref(), Some("Mercury"));
        let record: Value = serde_json::from_str(&out.output).expect("a record");
        assert_eq!(record["systemPrompt"], json!("Be brief."));
        assert_eq!(record["handle"], json!("@mercury"));
    }

    /// The id is the filename, and the filename follows the name: an agent
    /// called Mercury is reachable as `mercury` without a list call first.
    #[tokio::test]
    async fn the_id_a_record_gets_is_built_from_its_name() {
        let bench = Bench::new();
        let made = bench
            .call("inertia_save", json!({ "kind": "agent", "fields": { "name": "Mercury" } }))
            .await
            .expect("saved");
        assert_eq!(id_of(&made), "mercury");

        let out = bench
            .call("inertia_get", json!({ "kind": "agent", "id": "mercury" }))
            .await
            .expect("read by the name-derived id");
        assert_eq!(out.title.as_deref(), Some("Mercury"));
    }

    #[tokio::test]
    async fn reading_something_that_is_not_there_says_so_rather_than_answering_null() {
        let bench = Bench::new();
        let message = bench
            .call("inertia_get", json!({ "kind": "agent", "id": "agent-nope" }))
            .await
            .expect_err("a refusal");
        assert_eq!(message, "There is no agent with the id agent-nope.");
    }

    /* -- saving ---------------------------------------------------------- */

    #[tokio::test]
    async fn a_new_agent_lands_on_disk_and_the_window_is_told() {
        let bench = Bench::new();
        let out = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "agent",
                    "fields": { "name": "Mercury", "role": "Inbox triage", "systemPrompt": "Be brief." }
                }),
            )
            .await
            .expect("saved");

        assert!(out.output.contains("on screen now"), "{}", out.output);
        let id = id_of(&out);

        // On disk, with the fields the app owns filled in.
        let agent = bench.record(Collection::Agents, &id);
        assert_eq!(agent["name"], json!("Mercury"));
        assert_eq!(agent["handle"], json!("@mercury"));
        assert_eq!(agent["status"], json!("idle"));
        assert_eq!(agent["stats"], json!({ "messages": 0, "routinesRun": 0, "tokensUsed": 0 }));

        // And the window heard about it, on the channel it is already listening
        // on, with the collection key the renderer matches against.
        let events = bench.events.lock().clone();
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0], json!({ "collection": "agents", "id": id, "op": "put" }));
    }

    #[tokio::test]
    async fn changing_one_field_leaves_the_rest_of_the_record_alone() {
        let bench = Bench::new();
        let made = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "agent",
                    "fields": { "name": "Mercury", "systemPrompt": "Be brief.", "role": "Inbox triage" }
                }),
            )
            .await
            .expect("saved");
        let id = id_of(&made);

        bench
            .call(
                "inertia_save",
                json!({ "kind": "agent", "id": id, "fields": { "role": "Triage and drafting" } }),
            )
            .await
            .expect("updated");

        let agent = bench.record(Collection::Agents, &id);
        assert_eq!(agent["role"], json!("Triage and drafting"));
        assert_eq!(agent["systemPrompt"], json!("Be brief."));
        // An update is a patch to the window, not a create.
        assert_eq!(bench.events.lock().last().expect("an event")["op"], json!("patch"));
    }

    #[tokio::test]
    async fn an_agent_with_no_name_is_refused_in_the_electron_builds_own_words() {
        let bench = Bench::new();
        let message = bench
            .call("inertia_save", json!({ "kind": "agent", "fields": { "role": "Nobody" } }))
            .await
            .expect_err("a refusal");
        assert_eq!(message, "An agent needs a name.");
        // Nothing was written, and nothing was announced.
        assert!(collections::list(&bench.layout, Collection::Agents).is_empty());
        assert!(bench.events.lock().is_empty());
    }

    /// A model that invents `prompt` should be told the field is `systemPrompt`,
    /// not have its call silently saved with the prompt dropped.
    #[tokio::test]
    async fn a_field_nobody_has_names_the_fields_that_exist() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_save",
                json!({ "kind": "agent", "fields": { "name": "Mercury", "prompt": "Be brief." } }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("has no prompt"), "{message}");
        assert!(message.contains("systemPrompt"), "{message}");
    }

    #[tokio::test]
    async fn updating_something_that_does_not_exist_is_refused_rather_than_creating_it() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_save",
                json!({ "kind": "agent", "id": "agent-nope", "fields": { "name": "Ghost" } }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("There is no agent with the id agent-nope"), "{message}");
        assert!(message.contains("Leave the id out"), "{message}");
    }

    /// Protection covers editing as well as deleting: rewriting the system
    /// prompt of the agent you may not delete is the same hole with a step.
    #[tokio::test]
    async fn a_protected_record_cannot_be_edited_or_removed() {
        let bench = Bench::new();
        collections::put(
            &bench.layout,
            Collection::Agents,
            json!({ "id": "agent-inertia-dev", "name": "Inertia Dev", "protected": true }),
        )
        .expect("the shipped agent");

        for (tool, args) in [
            (
                "inertia_save",
                json!({ "kind": "agent", "id": "agent-inertia-dev", "fields": { "role": "Mine now" } }),
            ),
            ("inertia_remove", json!({ "kind": "agent", "id": "agent-inertia-dev" })),
        ] {
            let message = bench.call(tool, args).await.expect_err("a refusal");
            assert!(message.contains("Inertia Dev is protected"), "{message}");
        }
        assert!(collections::get(&bench.layout, Collection::Agents, "agent-inertia-dev")
            .expect("readable")
            .is_some());
    }

    #[tokio::test]
    async fn fields_sent_as_a_json_string_are_repaired_rather_than_refused() {
        let bench = Bench::new();
        let out = bench
            .call(
                "inertia_save",
                json!({ "kind": "agent", "fields": "{\"name\":\"Mercury\"}" }),
            )
            .await
            .expect("saved");
        assert_eq!(bench.record(Collection::Agents, &id_of(&out))["name"], json!("Mercury"));
    }

    /* -- routines -------------------------------------------------------- */

    #[tokio::test]
    async fn a_routine_is_written_with_its_schedule_in_words_for_the_screen() {
        let bench = Bench::new();
        let out = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "routine",
                    "fields": {
                        "name": "Morning brief",
                        "agentId": "agent-1",
                        "markdown": "Check the build and report.",
                        "schedule": { "kind": "cron", "expression": "0 9 * * 1-5" }
                    }
                }),
            )
            .await
            .expect("saved");

        let saved = bench.record(Collection::Routines, &id_of(&out));
        assert_eq!(saved["name"], json!("Morning brief"));
        assert_eq!(saved["schedule"]["humanLabel"], json!("On the schedule 0 9 * * 1-5"));
        // The scheduler decides when it next runs; a value here would be
        // invented for a clock the model cannot see.
        assert_eq!(saved["schedule"]["nextRunAt"], Value::Null);
        assert_eq!(saved["enabled"], json!(true));
    }

    /// `0 0 30 2 *` is the classic: the thirtieth of February.
    #[tokio::test]
    async fn a_cron_expression_that_never_comes_round_is_refused() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "routine",
                    "fields": {
                        "name": "Never", "agentId": "agent-1", "markdown": "Do a thing.",
                        "schedule": { "kind": "cron", "expression": "0 0 30 2 *" }
                    }
                }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("never comes round"), "{message}");
        assert!(collections::list(&bench.layout, Collection::Routines).is_empty());
    }

    #[tokio::test]
    async fn a_cron_expression_that_is_not_one_at_all_is_refused() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "routine",
                    "fields": {
                        "name": "Bad", "agentId": "agent-1", "markdown": "Do a thing.",
                        "schedule": { "kind": "cron", "expression": "every morning" }
                    }
                }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("not usable"), "{message}");
    }

    #[tokio::test]
    async fn the_field_a_model_forgets_is_the_one_it_is_told_about() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_save",
                json!({ "kind": "routine", "fields": { "name": "Orphan", "markdown": "Do a thing." } }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("needs an agentId"), "{message}");

        let message = bench
            .call(
                "inertia_save",
                json!({ "kind": "routine", "fields": { "name": "Empty", "agentId": "agent-1" } }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("no playbook"), "{message}");
    }

    #[tokio::test]
    async fn an_interval_shorter_than_a_minute_is_a_runaway_bill_and_is_refused() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "routine",
                    "fields": {
                        "name": "Hot loop", "agentId": "agent-1", "markdown": "Go.",
                        "schedule": { "kind": "interval", "expression": "PT10S" }
                    }
                }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("shortest interval is one minute"), "{message}");
    }

    #[tokio::test]
    async fn a_mode_nobody_has_is_named_back_with_the_ones_that_exist() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "routine",
                    "fields": {
                        "name": "Odd", "agentId": "agent-1", "markdown": "Go.", "mode": "telepathy"
                    }
                }),
            )
            .await
            .expect_err("a refusal");
        assert_eq!(message, "\"telepathy\" is not a mode. Use chat, plan or autonomous.");
    }

    /* -- skills ---------------------------------------------------------- */

    #[tokio::test]
    async fn a_skill_is_written_as_a_folder_with_a_skill_md_in_it() {
        let bench = Bench::new();
        let out = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "skill",
                    "fields": {
                        "name": "deploy",
                        "description": "How to ship a release. Use when the user asks to deploy.",
                        "instructions": "Run the pipeline."
                    }
                }),
            )
            .await
            .expect("saved");

        let mut file = bench.layout.root().to_path_buf();
        file.push("skills");
        file.push(id_of(&out));
        file.push("SKILL.md");
        let text = std::fs::read_to_string(&file).expect("a SKILL.md");
        assert!(text.contains("Run the pipeline."), "{text}");
        assert!(text.contains("description: How to ship a release."), "{text}");
    }

    #[tokio::test]
    async fn a_skill_with_no_description_is_refused_because_that_is_all_another_agent_reads() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_save",
                json!({
                    "kind": "skill",
                    "fields": { "name": "deploy", "instructions": "Run the pipeline." }
                }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("only thing another agent sees"), "{message}");
    }

    /* -- removing -------------------------------------------------------- */

    #[tokio::test]
    async fn removing_takes_the_record_off_disk_and_tells_the_window() {
        let bench = Bench::new();
        let made = bench
            .call("inertia_save", json!({ "kind": "agent", "fields": { "name": "Mercury" } }))
            .await
            .expect("saved");
        let id = id_of(&made);
        bench.events.lock().clear();

        let out = bench
            .call("inertia_remove", json!({ "kind": "agent", "id": id }))
            .await
            .expect("removed");
        assert_eq!(out.output, "Removed the agent Mercury.");
        assert!(collections::list(&bench.layout, Collection::Agents).is_empty());
        assert_eq!(
            bench.events.lock().clone(),
            vec![json!({ "collection": "agents", "id": id, "op": "remove" })]
        );
    }

    #[tokio::test]
    async fn removing_something_that_is_not_there_says_so_rather_than_pretending() {
        let bench = Bench::new();
        let message = bench
            .call("inertia_remove", json!({ "kind": "agent", "id": "agent-nope" }))
            .await
            .expect_err("a refusal");
        assert_eq!(message, "There is no agent with the id agent-nope.");
    }

    /// Creating things and destroying them are not the same decision, and the
    /// rules in a real workspace are written against these two exact keys.
    #[test]
    fn destroying_asks_under_its_own_permission_key() {
        let bench = Bench::new();
        let args = json!({ "kind": "agent", "id": "agent-a" });

        let save = bench.tool("inertia_save").permission(&json!({ "kind": "agent" }));
        assert_eq!(save.key, "inertia");
        assert_eq!(save.target, "agent:new");
        assert_eq!(save.always.as_deref(), Some("agent:*"));

        let remove = bench.tool("inertia_remove").permission(&args);
        assert_eq!(remove.key, "inertia_guarded");
        assert_eq!(remove.target, "agent:agent-a");

        let rules = bench.tool("inertia_set_rules").permission(&json!({}));
        assert_eq!(rules.key, "inertia_guarded");
        assert_eq!(rules.target, "rules:workspace");

        for id in ["inertia_list", "inertia_get", "inertia_set_picture", "inertia_connect_app"] {
            assert_eq!(bench.tool(id).permission(&args).key, "inertia", "{id}");
        }
    }

    /* -- pictures -------------------------------------------------------- */

    /// One PNG, small enough to write inline. Only the magic number matters:
    /// the type is read off the extension, the way Electron read it.
    const PNG: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

    #[tokio::test]
    async fn a_picture_is_copied_into_the_workspace_and_the_record_points_at_it() {
        let bench = Bench::new();
        let made = bench
            .call("inertia_save", json!({ "kind": "agent", "fields": { "name": "Mercury" } }))
            .await
            .expect("saved");
        let id = id_of(&made);

        let source = bench.layout.root().join("face.png");
        std::fs::write(&source, PNG).expect("a source image");

        bench
            .call(
                "inertia_set_picture",
                json!({ "agentId": id, "path": source.to_string_lossy() }),
            )
            .await
            .expect("a picture");

        let agent = bench.record(Collection::Agents, &id);
        assert_eq!(agent["avatarFile"], json!(format!("agents/pictures/{id}.png")));
        assert!(agent["avatarUpdatedAt"].as_i64().unwrap_or(0) > 0);
        assert!(picture_path(bench.layout.root(), &id, "png").is_file());
    }

    #[tokio::test]
    async fn something_that_is_not_an_image_is_refused_by_name() {
        let bench = Bench::new();
        let made = bench
            .call("inertia_save", json!({ "kind": "agent", "fields": { "name": "Mercury" } }))
            .await
            .expect("saved");
        let source = bench.layout.root().join("notes.txt");
        std::fs::write(&source, "hello").expect("a source file");

        let message = bench
            .call(
                "inertia_set_picture",
                json!({ "agentId": id_of(&made), "path": source.to_string_lossy() }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("PNG, JPEG, WebP or GIF"), "{message}");
    }

    #[tokio::test]
    async fn clearing_a_picture_takes_the_file_and_the_reference_both() {
        let bench = Bench::new();
        let made = bench
            .call("inertia_save", json!({ "kind": "agent", "fields": { "name": "Mercury" } }))
            .await
            .expect("saved");
        let id = id_of(&made);
        let source = bench.layout.root().join("face.png");
        std::fs::write(&source, PNG).expect("a source image");
        bench
            .call(
                "inertia_set_picture",
                json!({ "agentId": id, "path": source.to_string_lossy() }),
            )
            .await
            .expect("a picture");

        bench
            .call("inertia_set_picture", json!({ "agentId": id, "clear": true }))
            .await
            .expect("cleared");

        assert_eq!(bench.record(Collection::Agents, &id)["avatarFile"], Value::Null);
        assert!(!picture_path(bench.layout.root(), &id, "png").exists());
    }

    /// A folder that fills with portraits of teammates nobody has is a folder
    /// nobody can tidy, so the picture goes with the agent.
    #[tokio::test]
    async fn removing_an_agent_takes_its_picture_with_it() {
        let bench = Bench::new();
        let made = bench
            .call("inertia_save", json!({ "kind": "agent", "fields": { "name": "Mercury" } }))
            .await
            .expect("saved");
        let id = id_of(&made);
        let source = bench.layout.root().join("face.png");
        std::fs::write(&source, PNG).expect("a source image");
        bench
            .call(
                "inertia_set_picture",
                json!({ "agentId": id, "path": source.to_string_lossy() }),
            )
            .await
            .expect("a picture");
        assert!(picture_path(bench.layout.root(), &id, "png").exists());

        bench
            .call("inertia_remove", json!({ "kind": "agent", "id": id }))
            .await
            .expect("removed");
        assert!(!picture_path(bench.layout.root(), &id, "png").exists());
    }

    /* -- rules ----------------------------------------------------------- */

    fn permissions(bench: &Bench) -> Value {
        collections::read_document(&bench.layout, Document::Permissions, json!({}))
    }

    #[tokio::test]
    async fn a_rule_for_a_tool_and_pattern_replaces_the_one_that_was_there() {
        let bench = Bench::new();
        for action in ["allow", "ask"] {
            bench
                .call(
                    "inertia_set_rules",
                    json!({ "rules": [{ "tool": "shell", "pattern": "git *", "action": action }] }),
                )
                .await
                .expect("rules set");
        }

        let doc = permissions(&bench);
        let matching: Vec<&Value> = doc["workspace"]
            .as_array()
            .expect("workspace rules")
            .iter()
            .filter(|rule| rule["tool"] == json!("shell") && rule["pattern"] == json!("git *"))
            .collect();
        assert_eq!(matching.len(), 1, "{doc}");
        assert_eq!(matching[0]["action"], json!("ask"));
        assert_eq!(
            bench.events.lock().last().expect("an event"),
            &json!({ "document": "settings.permissions", "op": "set" })
        );
    }

    #[tokio::test]
    async fn an_agents_rules_are_kept_apart_from_the_workspaces() {
        let bench = Bench::new();
        bench
            .call(
                "inertia_set_rules",
                json!({
                    "agentId": "agent-1",
                    "rules": [{ "tool": "edit", "pattern": "*", "action": "deny" }]
                }),
            )
            .await
            .expect("rules set");

        let doc = permissions(&bench);
        assert_eq!(
            doc["agents"]["agent-1"],
            json!([{ "tool": "edit", "pattern": "*", "action": "deny" }])
        );
        assert!(doc.get("workspace").is_none() || doc["workspace"].as_array().is_none());
    }

    /// The file `commands::permissions_save` writes is a typed `Permissions`
    /// with only `workspace` and `agents` on it, and a real workspace's file
    /// also carries a top-level `rules` array. Writing this one as a document
    /// is what keeps that array from being deleted by a tool call.
    #[tokio::test]
    async fn a_key_this_build_does_not_model_survives_a_rule_change() {
        let bench = Bench::new();
        collections::write_document(
            &bench.layout,
            Document::Permissions,
            &json!({ "rules": [{ "id": "perm-run-terminal" }], "workspace": [] }),
        )
        .expect("a hand-written file");

        bench
            .call(
                "inertia_set_rules",
                json!({ "rules": [{ "tool": "shell", "pattern": "*", "action": "ask" }] }),
            )
            .await
            .expect("rules set");

        let doc = permissions(&bench);
        assert_eq!(doc["rules"], json!([{ "id": "perm-run-terminal" }]));
        assert_eq!(doc["workspace"][0]["tool"], json!("shell"));
    }

    #[tokio::test]
    async fn a_tool_key_that_does_not_exist_is_refused_with_the_keys_that_do() {
        let bench = Bench::new();
        let message = bench
            .call(
                "inertia_set_rules",
                json!({ "rules": [{ "tool": "telepathy", "pattern": "*", "action": "allow" }] }),
            )
            .await
            .expect_err("a refusal");
        assert!(message.contains("No tool is called telepathy"), "{message}");
        assert!(message.contains("inertia_guarded"), "{message}");
    }

    #[tokio::test]
    async fn no_rules_at_all_is_refused_rather_than_written_as_an_empty_change() {
        let bench = Bench::new();
        let message = bench
            .call("inertia_set_rules", json!({ "rules": [] }))
            .await
            .expect_err("a refusal");
        assert_eq!(message, "No rules were given, so there is nothing to change.");
    }

    /* -- the set --------------------------------------------------------- */

    #[test]
    fn every_tool_the_prompt_promises_is_in_the_set() {
        let bench = Bench::new();
        let mut ids: Vec<&str> = bench.tools.iter().map(|tool| tool.id()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec![
                "inertia_connect_app",
                "inertia_get",
                "inertia_list",
                "inertia_remove",
                "inertia_save",
                "inertia_set_picture",
                "inertia_set_rules",
            ]
        );
    }

    /// The mode filter is keyed on tool ids, and a tool absent from `MUTATING`
    /// is one a Chat turn would still be handed.
    #[test]
    fn everything_that_writes_is_withheld_from_a_turn_that_may_not_write() {
        for id in [
            "inertia_save",
            "inertia_remove",
            "inertia_set_picture",
            "inertia_set_rules",
            "inertia_connect_app",
        ] {
            assert!(
                inertia_agent::prompt::MUTATING.contains(&id),
                "{id} is not withheld from Chat and Plan"
            );
        }
        for id in ["inertia_list", "inertia_get"] {
            assert!(!inertia_agent::prompt::MUTATING.contains(&id), "{id} was withheld");
        }
    }

    /// A build with no integrations wired up must say so rather than succeeding
    /// and doing nothing: an agent told nothing happened stops, and one told it
    /// worked goes on telling the user their app is connected.
    #[tokio::test]
    async fn a_live_source_with_nothing_behind_it_refuses_out_loud() {
        let bench = Bench::new();
        let message = bench
            .call("inertia_connect_app", json!({ "toolkit": "gmail" }))
            .await
            .expect_err("a refusal");
        assert!(message.contains("Integrations screen"), "{message}");
    }

    #[tokio::test]
    async fn a_kind_that_cannot_be_created_says_what_to_do_instead() {
        let bench = Bench::new();
        let message = bench
            .call("inertia_save", json!({ "kind": "computer", "fields": { "name": "box" } }))
            .await
            .expect_err("a refusal");
        assert!(message.contains("Computers screen"), "{message}");

        let message = bench
            .call("inertia_save", json!({ "kind": "app", "fields": { "name": "gmail" } }))
            .await
            .expect_err("a refusal");
        assert!(message.contains("inertia_connect_app"), "{message}");
    }

    /* -- schedules ------------------------------------------------------- */

    #[test]
    fn a_schedule_in_words_is_what_the_routines_screen_shows() {
        assert_eq!(schedule::describe(&json!({})), "Runs when you ask");
        assert_eq!(
            schedule::describe(&json!({ "schedule": { "kind": "interval", "expression": "PT2H" } })),
            "Every 2 hour(s)"
        );
        assert_eq!(
            schedule::describe(&json!({ "schedule": { "kind": "cron", "expression": "0 9 * * 1-5" } })),
            "On the schedule 0 9 * * 1-5"
        );
    }
}

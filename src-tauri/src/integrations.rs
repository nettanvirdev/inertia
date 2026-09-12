//! The three tool sources, as the renderer asks for them.
//!
//! MCP servers, OpenAPI imports and Composio apps each have a full
//! implementation in `crates/`. What was missing was the command surface: the
//! Integrations screen is gated on all three namespaces existing, so a single
//! absent one blanked the whole pane and reported it as needing a desktop app
//! the user was already running.
//!
//! These are per-item operations - connect *this* server, test *that* record -
//! which is what a settings screen does. The bulk operations in `commands.rs`
//! are what the app does on startup, and they stay there.

use inertia_composio::api::Composio;
use inertia_composio::provider::ConnectionRecord;
use serde::Deserialize;
use inertia_mcp::client::ServerRecord;
use inertia_openapi::provider::ImportRecord;
use serde_json::{json, Value};
use tauri::State;

use crate::state::AppState;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/* -- MCP ----------------------------------------------------------------- */

/// Connects one stored server.
#[tauri::command]
pub async fn mcp_connect(
    state: State<'_, AppState>,
    id: String,
) -> Result<ServerRecord, String> {
    let workspace = state.workspace()?;
    let mut record = workspace
        .mcp_records()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("No MCP server called `{id}` is configured."))?;

    // Dropped first: whatever is running is running on the old configuration,
    // and reconnecting without stopping it would leave two processes.
    workspace.mcp.remove(&id);

    match workspace
        .mcp
        .add(with_secrets(&workspace.layout, &record))
        .await
    {
        Ok(tool_count) => {
            record.status = "connected".into();
            record.error = String::new();
            record.tool_count = tool_count;
        }
        Err(message) => {
            record.status = "failed".into();
            record.error = message;
            record.tool_count = 0;
        }
    }
    workspace.save_mcp_record(&record)?;
    Ok(record)
}

#[tauri::command]
pub fn mcp_disconnect(state: State<'_, AppState>, id: String) -> Result<ServerRecord, String> {
    let workspace = state.workspace()?;
    workspace.mcp.remove(&id);

    let mut record = workspace
        .mcp_records()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("No MCP server called `{id}` is configured."))?;
    record.status = "disconnected".into();
    record.tool_count = 0;
    workspace.save_mcp_record(&record)?;
    Ok(record)
}

/// Starts a server, reads its tools, and stops it again.
///
/// Never leaves it running: this is the Test button on a record the user may be
/// in the middle of editing, and a half-configured server left connected would
/// offer its tools to the next turn.
#[tauri::command]
pub async fn mcp_test(
    state: State<'_, AppState>,
    record: ServerRecord,
) -> Result<Value, String> {
    let workspace = state.workspace()?;

    // A draft is tested under a name nothing else uses, so testing an edit
    // cannot disturb the connection the saved record already has.
    let mut probe = record.clone();
    probe.id = format!("__test__{}", record.id);

    let started = std::time::Instant::now();
    let outcome = workspace
        .mcp
        .add(with_secrets(&workspace.layout, &probe))
        .await;
    let latency = started.elapsed().as_millis() as u64;

    let tools: Vec<Value> = workspace
        .mcp
        .tools_for(&probe.id)
        .into_iter()
        .map(|tool| json!({ "name": tool.name, "description": tool.description }))
        .collect();

    // Always, including on failure: a server that started and then errored has
    // still left a process behind.
    workspace.mcp.remove(&probe.id);

    match outcome {
        Ok(tool_count) => Ok(json!({
            "ok": true,
            "toolCount": tool_count,
            "latencyMs": latency,
            "tools": tools,
        })),
        Err(message) => Ok(json!({ "ok": false, "error": message, "latencyMs": latency })),
    }
}

/// Changes a stored server and restarts it.
///
/// A patch rather than a whole record, because the settings screen edits one
/// field at a time and sending back a record assembled in the window would
/// overwrite whatever a connect had just written to `status` and `toolCount`.
#[tauri::command]
pub async fn mcp_save_patch(
    state: State<'_, AppState>,
    id: String,
    changes: Value,
) -> Result<ServerRecord, String> {
    {
        let workspace = state.workspace()?;
        let layout = workspace.layout.clone();
        inertia_store::collections::patch(&layout, inertia_store::Collection::Mcp, &id, changes)
            .map_err(err)?;
    }
    // Reconnected rather than merely saved: whatever is running is running on
    // the old configuration, and an edit nobody applied is an edit that did
    // not happen.
    mcp_connect(state, id).await
}

/// Which servers are up right now.
#[tauri::command]
pub fn mcp_status(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let workspace = state.workspace()?;
    Ok(workspace
        .mcp
        .connected()
        .into_iter()
        .map(|r| json!({ "id": r.id, "status": "connected", "toolCount": r.tool_count }))
        .collect())
}

/// What one connected server provides. Empty when it is not connected.
#[tauri::command]
pub fn mcp_tools(state: State<'_, AppState>, id: String) -> Result<Vec<Value>, String> {
    let workspace = state.workspace()?;
    Ok(workspace
        .mcp
        .tools_for(&id)
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            })
        })
        .collect())
}

/* -- OpenAPI ------------------------------------------------------------- */

#[tauri::command]
pub fn openapi_update(
    state: State<'_, AppState>,
    id: String,
    changes: Value,
) -> Result<ImportRecord, String> {
    let workspace = state.workspace()?;
    let layout = workspace.layout.clone();
    let saved = inertia_store::collections::patch(
        &layout,
        inertia_store::Collection::OpenApi,
        &id,
        changes,
    )
    .map_err(err)?;
    serde_json::from_value(saved).map_err(err)
}

/// Which operations this import offers, and which are turned on.
#[tauri::command]
pub fn openapi_operations(state: State<'_, AppState>, id: String) -> Result<Vec<Value>, String> {
    let workspace = state.workspace()?;
    let Some(document) = workspace.read_openapi_spec(&id) else {
        return Err("The stored document for this import could not be read.".into());
    };

    let record = workspace
        .openapi_records()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("No OpenAPI import called `{id}`."))?;
    let namespace = if record.name.is_empty() { &record.id } else { &record.name };
    let enabled = &record.operations;

    Ok(inertia_openapi::operation::extract(&document, namespace)
        .into_iter()
        .map(|op| {
            json!({
                "operationId": op.id,
                "method": op.method,
                "path": op.path,
                "summary": op.description,
                // An import with no explicit list has everything on, which is
                // what the importer wrote and what the tools already reflect.
                "enabled": enabled.is_empty() || enabled.contains(&op.id),
                // The schema pointed outside the document, so the tool this
                // becomes takes looser arguments than the spec describes.
                "unresolved": op.unresolved,
            })
        })
        .collect())
}

#[tauri::command]
pub fn openapi_set_operations(
    state: State<'_, AppState>,
    id: String,
    operations: Vec<String>,
) -> Result<ImportRecord, String> {
    openapi_update(state, id, json!({ "operations": operations }))
}

/// Calls one operation with arguments the user typed.
///
/// The point is proof. An import that looks right and is missing a trailing
/// `/v1` fails later, inside an agent turn, as a confusing tool error; here it
/// fails immediately, in front of the person who can fix it, with the API's own
/// words on screen.
///
/// It answers `{ output }` because that is the field the panel renders, and the
/// output is the status line and the body exactly as the agent would see them -
/// a 404 with a message in it is the useful answer, not a failure.
#[tauri::command]
pub async fn openapi_test(
    state: State<'_, AppState>,
    id: String,
    operation_id: String,
    args: Option<Value>,
) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let Some(document) = workspace.read_openapi_spec(&id) else {
        return Err("The stored document for this import could not be read.".into());
    };
    let record = workspace
        .openapi_records()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("No OpenAPI import called `{id}`."))?;

    let credential = crate::commands::resolve_openapi_credential(&workspace, &record);
    let outcome = workspace
        .openapi
        .call_operation(
            &record,
            &document,
            credential,
            &operation_id,
            args.unwrap_or_else(|| json!({})),
        )
        .await?;

    Ok(json!({
        "output": outcome.output,
        "title": outcome.title,
        "metadata": outcome.metadata,
    }))
}

/// What the document itself says: which servers it offers, and which of its
/// authentication schemes this app can configure.
///
/// Read on demand rather than carried on the record, because both live in the
/// stored document and neither is small.
#[tauri::command]
pub fn openapi_details(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let Some(document) = workspace.read_openapi_spec(&id) else {
        return Err("The stored document for this import could not be read.".into());
    };
    let record = workspace
        .openapi_records()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("No OpenAPI import called `{id}`."))?;
    let namespace = if record.name.is_empty() { &record.id } else { &record.name };

    Ok(json!({
        "title": inertia_openapi::spec::title(&document),
        // What the document says it is served from, which the record may be
        // overriding. Both are shown, because a mismatch is the usual reason an
        // import reaches the wrong host.
        "declaredBaseUrl": inertia_openapi::spec::base_url(&document),
        "baseUrl": record.base_url,
        "operationCount": inertia_openapi::operation::extract(&document, namespace).len(),
    }))
}

/* -- Composio ------------------------------------------------------------ */

const NO_COMPOSIO_KEY: &str = "No Composio API key is stored. Add one under Settings, Secrets.";

/// The Composio key, read from the secret store.
///
/// Read on every call rather than held, and this is the fix for a real bug: the
/// provider's key was only ever set by `composio_set_key`, which nothing calls
/// at startup, so a workspace with a perfectly good key in it reported "not
/// configured" until the user went and re-pointed it by hand. The same key that
/// worked in the Electron app was sitting in the same file the whole time.
///
/// Which secret holds it is the workspace's business - `composioKeySecret` in
/// the app settings renames it - so a person keeping two accounts is not forced
/// to overwrite one to test the other.
fn composio_key(state: &AppState) -> Result<Option<String>, String> {
    let workspace = state.workspace()?;
    let app = inertia_store::collections::read_document(
        &workspace.layout,
        inertia_store::layout::Document::App,
        json!({}),
    );
    let name = app
        .get("composioKeySecret")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("COMPOSIO_API_KEY")
        .to_string();

    Ok(inertia_store::secrets::get(&workspace.layout, &name)
        .map_err(err)?
        .filter(|key| !key.trim().is_empty()))
}

/// The key, or the message that says what to do about not having one.
pub fn composio_client(state: &AppState) -> Result<Composio, String> {
    let key = composio_key(state)?.ok_or(NO_COMPOSIO_KEY)?;
    // Kept in step with the provider so the tools a turn can call are backed by
    // the same account the settings pane is showing.
    let workspace = state.workspace()?;
    if !workspace.composio.has_key() {
        workspace.composio.set_key(Some(key.clone()));
    }
    Ok(Composio::new(key))
}

#[tauri::command]
pub fn composio_configured(state: State<'_, AppState>) -> Result<bool, String> {
    composio_configured_for(&state)
}

/// The same question, asked from a command that answers something wider.
pub fn composio_configured_for(state: &AppState) -> Result<bool, String> {
    Ok(composio_key(state)?.is_some())
}

#[tauri::command]
pub fn composio_connections(
    state: State<'_, AppState>,
) -> Result<Vec<ConnectionRecord>, String> {
    let workspace = state.workspace()?;
    Ok(workspace.composio_records())
}

#[tauri::command]
pub fn composio_permissions(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let record = workspace
        .composio_records()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("No Composio connection called `{id}`."))?;
    Ok(json!({ "id": record.id, "enabledTools": record.enabled_tools }))
}

#[tauri::command]
pub fn composio_set_permissions(
    state: State<'_, AppState>,
    id: String,
    patch: Value,
) -> Result<ConnectionRecord, String> {
    let workspace = state.workspace()?;
    let layout = workspace.layout.clone();
    let saved = inertia_store::collections::patch(
        &layout,
        inertia_store::Collection::Composio,
        &id,
        patch,
    )
    .map_err(err)?;
    serde_json::from_value(saved).map_err(err)
}

/// What the catalogue pane asks for.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogueQuery {
    pub search: String,
    pub category: String,
    pub refresh: bool,
}

/// How long a stored catalogue is served without asking Composio again.
///
/// The list of apps changes when Composio ships, which is not several times an
/// afternoon, and Refresh is how a person says otherwise. The ceiling only
/// exists so a machine left running for a week eventually notices.
const CATALOGUE_MAX_AGE_MS: i64 = 6 * 60 * 60 * 1000;

fn millis_now() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

/// Composio's catalogue of apps.
///
/// Three things this owes the window, and it used to do none of them. It
/// answers `{items, fetchedAt, nextCursor}` rather than a bare array, because
/// the pane caches the answer and has to be able to say how old what it is
/// showing is - a cache that cannot answer that is one a person has to trust
/// blindly. It normalises each row, because the picker reads `logo`,
/// `connectable` and `setupUrl`, none of which Composio's payload spells that
/// way. And it keeps the whole list in `cache/`, because collecting several
/// hundred rows over paged requests at every launch puts the wait in exactly
/// the moment somebody is watching an empty screen.
///
/// A narrowed read is never served from the cache and never written to it: it
/// is somebody asking a different question, and answering it out of a cache of
/// everything would hand them rows they filtered out.
#[tauri::command]
pub async fn composio_toolkits(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    query: Option<CatalogueQuery>,
) -> Result<Value, String> {
    let query = query.unwrap_or_default();
    let narrowed = !query.search.trim().is_empty() || !query.category.trim().is_empty();
    let layout = state.workspace()?.layout.clone();

    if !query.refresh && !narrowed {
        let stored = inertia_store::collections::read_document(
            &layout,
            inertia_store::layout::Document::CacheComposio,
            Value::Null,
        );
        match aged_catalogue(&stored) {
            Aged::Fresh(fresh) => return Ok(fresh),
            // Stale is still an answer. Almost every row in a day-old
            // catalogue is still true, and handing it over now beats holding
            // somebody in front of a spinner to confirm it. The re-read runs
            // behind them and the screen redraws when it lands, which is what
            // the refresh event was always for.
            Aged::Stale(stale) => {
                let client = composio_client(&state)?;
                let behind = app.clone();
                let layout = layout.clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(answer) = fetch_catalogue(&client, &CatalogueQuery::default()).await {
                        store_catalogue(&layout, &answer);
                        announce_refresh(&behind, None);
                    }
                });
                return Ok(stale);
            }
            Aged::Missing => {}
        }
    }

    let client = composio_client(&state)?;
    let answer = fetch_catalogue(&client, &query).await?;

    if !narrowed {
        store_catalogue(&layout, &answer);
    }

    Ok(answer)
}

/// Ask Composio for the catalogue, narrowed as the caller asked.
///
/// The two reads are independent: nothing in the toolkit walk needs to know
/// which slugs already have an auth config until the rows are being
/// classified, so they run together. One after the other, the auth-config walk
/// was pure added wait in front of the list somebody is actually looking at.
async fn fetch_catalogue(
    client: &inertia_composio::api::Composio,
    query: &CatalogueQuery,
) -> Result<Value, String> {
    // Failing to read the project's auth configs is not a missing catalogue:
    // every row is still there, a few of them classified more pessimistically
    // than they deserve.
    let (configured, toolkits) = tokio::join!(client.auth_config_slugs(), client.toolkits());
    let configured = configured.unwrap_or_default();

    let items: Vec<Value> = toolkits
        .map_err(err)?
        .iter()
        .map(|item| inertia_composio::toolkit::normalise(item, &configured))
        .filter(|row| !row["slug"].as_str().unwrap_or_default().is_empty())
        .filter(|row| inertia_composio::toolkit::matches_search(row, &query.search))
        .filter(|row| inertia_composio::toolkit::matches_category(row, &query.category))
        .collect();

    Ok(json!({
        "items": items,
        "fetchedAt": millis_now(),
        "nextCursor": Value::Null,
    }))
}

/// Keep a whole catalogue for the next launch.
///
/// An empty one is not worth keeping, and a failed write costs one fetch and
/// nothing else - never the read somebody is waiting on.
fn store_catalogue(layout: &inertia_store::layout::Layout, answer: &Value) {
    if answer["items"].as_array().is_none_or(Vec::is_empty) {
        return;
    }
    let _ = inertia_store::collections::write_document(
        layout,
        inertia_store::layout::Document::CacheComposio,
        answer,
    );
}

/// What a stored catalogue is worth right now.
#[derive(Debug, PartialEq)]
enum Aged {
    /// Recent enough to serve and stop there.
    Fresh(Value),
    /// Worth showing, and worth re-reading behind it.
    Stale(Value),
    /// Nothing usable stored. Somebody has to wait.
    Missing,
}

/// A stored catalogue, and how far it can be trusted.
///
/// The age ceiling used to be the line between serving and fetching. It is now
/// the line between serving quietly and serving while checking: a list of apps
/// six hours old is not wrong, it is unconfirmed, and confirming it is not
/// something to make anybody watch.
fn aged_catalogue(stored: &Value) -> Aged {
    let Some(items) = stored.get("items").and_then(Value::as_array) else {
        return Aged::Missing;
    };
    if items.is_empty() {
        return Aged::Missing;
    }

    let mut answer = stored.clone();
    answer["fromCache"] = Value::Bool(true);

    let age = millis_now() - stored.get("fetchedAt").and_then(Value::as_i64).unwrap_or(0);
    // A negative age is a clock that moved backwards, which says nothing about
    // the rows. Old rather than absent.
    if (0..CATALOGUE_MAX_AGE_MS).contains(&age) {
        Aged::Fresh(answer)
    } else {
        Aged::Stale(answer)
    }
}

/// What one app can do.
#[tauri::command]
pub async fn composio_tools(
    state: State<'_, AppState>,
    toolkit_slug: String,
) -> Result<Vec<Value>, String> {
    let client = composio_client(&state)?;
    client.tools(&toolkit_slug).await.map_err(err)
}

/// One connection, as Composio currently sees it.
///
/// Separate from `composio_status`, which answers for the whole integration.
/// This is the poll a pending OAuth handshake runs until the account goes
/// active, so it has to ask Composio rather than read the stored record.
#[tauri::command]
pub async fn composio_connection_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<Value, String> {
    let (client, record) = {
        let record = state
            .workspace()?
            .composio_records()
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("No Composio connection called `{id}`."))?;
        (composio_client(&state)?, record)
    };

    let account = client
        .connected_account(&record.account_id)
        .await
        .map_err(err)?;
    let status = account
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN")
        .to_string();

    // The dialog polls this while the person finishes the handshake in their
    // browser; the moment it turns ACTIVE the rest of the screen needs to know
    // too, and it is not the one doing the polling.
    if status.eq_ignore_ascii_case("ACTIVE") {
        announce_connection(&app, &record.id);
    }

    Ok(json!({ "id": record.id, "status": status, "active": status.eq_ignore_ascii_case("ACTIVE") }))
}

/// The channel the Integrations screen listens on.
///
/// The window subscribed to this from the first build and nothing ever sent on
/// it, so a connection that went ACTIVE while its dialog was open sat there
/// saying it was still waiting until the person closed and reopened the screen.
pub const COMPOSIO_EVENT: &str = "composio:event";

/// Tell the Integrations screen a connection has changed.
pub fn announce_connection(app: &tauri::AppHandle, id: &str) {
    use tauri::Emitter;
    let _ = app.emit(COMPOSIO_EVENT, json!({ "type": "connection", "id": id }));
}

/// Tell it a catalogue was re-read, for one app or for all of them.
pub fn announce_refresh(app: &tauri::AppHandle, toolkit_slug: Option<&str>) {
    use tauri::Emitter;
    let _ = app.emit(
        COMPOSIO_EVENT,
        json!({ "type": "refresh", "toolkitSlug": toolkit_slug }),
    );
}

/// An app's logo, as a data URL.
///
/// A bare string, and `""` when there is not one: the row hands this straight
/// to an `img` element. It used to forward to `app_fetch_image`, which answers
/// `{ url, bytes }` - so every logo in the catalogue was set as the literal
/// text `[object Object]` and every row fell back to its initial.
#[tauri::command]
pub async fn composio_logo(app: tauri::AppHandle, url: String) -> String {
    crate::logos::logo(&app, &url).await
}

/// Forget the tool lists Composio was asked for.
///
/// The cache lives for the process, which is right for a catalogue that changes
/// when Composio ships and wrong the one afternoon somebody adds a tool to a
/// toolkit and cannot work out why Inertia will not offer it. A slug narrows it
/// to one app; nothing clears the lot.
///
/// Separate from `composio_reconnect` because the two are separate acts - the
/// bridge pointed both at one command, so pressing Refresh in the tool picker
/// called the reconnect path with no `id` and failed.
#[tauri::command]
pub fn composio_invalidate(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    toolkit_slug: Option<String>,
) -> Value {
    if let Ok(workspace) = state.workspace() {
        workspace.composio.drop_cached_tools();
    }
    let which = toolkit_slug.filter(|slug| !slug.trim().is_empty());
    announce_refresh(&app, which.as_deref());
    json!({ "refreshed": which.unwrap_or_else(|| "all".into()) })
}

/// A server record with its `{secret:NAME}` placeholders filled in.
///
/// Headers and environment both, because either can carry a credential: an HTTP
/// server authenticates with `Authorization: Bearer {secret:...}` and a stdio
/// one usually wants an API key in its environment.
///
/// Resolved at the moment of connecting rather than stored resolved, which is
/// the whole point of the placeholder: the record on disk names a secret and
/// can be exported or screenshotted without carrying one. Nothing was doing
/// this for MCP, so every server authenticating by header was sending the
/// literal text `{secret:MINI_LLM_API_KEY}` and being refused.
///
/// Both maps go through `resolve` in a single call because it re-reads the
/// secret file each time it is asked, and a server with eight headers has no
/// reason to read it eight times.
pub fn with_secrets(layout: &inertia_store::Layout, record: &ServerRecord) -> ServerRecord {
    let resolved = inertia_store::secrets::resolve(
        layout,
        &json!({ "headers": record.headers, "env": record.env }),
    );

    let take = |field: &str, original: &std::collections::HashMap<String, String>| {
        let Some(map) = resolved.get(field).and_then(Value::as_object) else {
            return original.clone();
        };
        map.iter()
            .map(|(key, value)| {
                let text = value.as_str().unwrap_or_default();
                (key.clone(), text.to_string())
            })
            .collect()
    };

    let mut out = record.clone();
    out.headers = take("headers", &record.headers);
    out.env = take("env", &record.env);
    out
}

//! The command surface the frontend calls.
//!
//! Thin by design. Every command resolves the workspace, delegates to a crate
//! that knows nothing about Tauri, and converts the result. Where a command
//! here starts to contain logic, that logic belongs in a crate where it can be
//! tested without a window.
//!
//! Errors are returned as `String` because that is what reaches the frontend as
//! a rejected promise. The messages are written for a person to read, not for a
//! developer to grep.
//!
//! What is left here is the three tool sources: MCP servers, OpenAPI imports
//! and Composio connections.
//!
//! The conversation, settings and permission commands that used to live here
//! are gone. The window reads all three through the generic workspace surface
//! in `ws.rs`, and had done for long enough that `permissions_save` was writing
//! a differently shaped file than the tool path read - two writers of one
//! document, one of them unreachable.
//!
//! `agent_send` is gone too, and it was the larger of the two. It was a second
//! turn loop, kept alive by one caller - the routine scheduler - and every tool
//! added to the real one since had passed it by. Routines now run through
//! `turn::agent_run`, the same command the composer calls.

use inertia_composio::provider::ConnectionRecord;
use inertia_mcp::client::ServerRecord;
use inertia_openapi::provider::{Auth, Credential, ImportRecord};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::state::AppState;

// ── MCP servers ─────────────────────────────────────────────────────────

/// Every configured server, with its last-known status.
#[tauri::command]
pub fn mcp_list(state: State<'_, AppState>) -> Result<Vec<ServerRecord>, String> {
    let workspace = state.workspace()?;
    let connected: Vec<String> = workspace
        .mcp
        .connected()
        .into_iter()
        .map(|r| r.id)
        .collect();

    Ok(workspace
        .mcp_records()
        .into_iter()
        .map(|mut record| {
            // The stored status is only a memory of the last session; what is
            // actually running is process state and is the truth here.
            record.status = if connected.contains(&record.id) {
                "connected".into()
            } else {
                "disconnected".into()
            };
            record
        })
        .collect())
}

/// Adds or updates a server and connects it.
///
/// The record is saved first, so a server that fails to start is still
/// configured and can be corrected rather than having to be re-entered.
#[tauri::command]
pub async fn mcp_save(
    state: State<'_, AppState>,
    record: ServerRecord,
) -> Result<ServerRecord, String> {
    let workspace = state.workspace()?;
    let mut record = record;

    workspace.save_mcp_record(&record)?;
    workspace.mcp.remove(&record.id);

    if !record.enabled {
        record.status = "disabled".into();
        record.error = String::new();
        workspace.save_mcp_record(&record)?;
        return Ok(record);
    }

    match workspace
        .mcp
        .add(crate::integrations::with_secrets(&workspace.layout, &record))
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
pub fn mcp_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let workspace = state.workspace()?;
    workspace.mcp.remove(&id);
    workspace.delete_mcp_record(&id)
}

// ── OpenAPI imports ─────────────────────────────────────────────────────

/// Imports a document from a URL or from pasted text.
///
/// The spec is stored beside its record, so an import keeps working when the
/// URL it came from goes away - and so the exact document that produced the
/// current tools can be inspected.
#[tauri::command]
pub async fn openapi_import(
    state: State<'_, AppState>,
    record: ImportRecord,
    text: Option<String>,
) -> Result<ImportRecord, String> {
    let workspace = state.workspace()?;
    let mut record = record;

    let raw = match text {
        Some(text) if !text.trim().is_empty() => text,
        _ => {
            if record.source.trim().is_empty() {
                return Err("Give a URL to fetch the document from, or paste it in.".into());
            }
            reqwest::get(&record.source)
                .await
                .map_err(|e| format!("Could not fetch {}: {e}", record.source))?
                .text()
                .await
                .map_err(|e| format!("Could not read {}: {e}", record.source))?
        }
    };

    let document = inertia_openapi::spec::parse(&raw).map_err(|e| e.to_string())?;

    let credential = resolve_openapi_credential(&workspace, &record);
    let count = workspace
        .openapi
        .add(record.clone(), &document, credential)
        .inspect_err(|e| {
            record.error = e.clone();
        })?;

    record.tool_count = count;
    record.error = String::new();

    workspace.save_openapi(&record, &document)?;
    Ok(record)
}

#[tauri::command]
pub fn openapi_list(state: State<'_, AppState>) -> Result<Vec<ImportRecord>, String> {
    Ok(state.workspace()?.openapi_records())
}

#[tauri::command]
pub fn openapi_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let workspace = state.workspace()?;
    workspace.openapi.remove(&id);
    workspace.delete_openapi(&id)
}

/// Looks up the key an import's auth refers to, at the last possible moment.
pub(crate) fn resolve_openapi_credential(
    workspace: &crate::state::Workspace,
    record: &ImportRecord,
) -> Credential {
    let name = match &record.auth {
        Auth::None => return Credential::default(),
        Auth::Bearer { secret } => secret,
        Auth::ApiKey { secret, .. } => secret,
    };
    Credential {
        value: workspace
            .settings
            .secrets()
            .get(name)
            .unwrap_or_default()
            .to_string(),
    }
}

// ── Composio ────────────────────────────────────────────────────────────

/// Begins connecting an app, returning the URL the user must visit.
#[tauri::command]
pub async fn composio_start_connection(
    state: State<'_, AppState>,
    toolkit: String,
) -> Result<ConnectionStarted, String> {
    let workspace = state.workspace()?;
    let client = crate::integrations::composio_client(&state)?;

    let auth_config = client
        .auth_config_for(&toolkit)
        .await
        .map_err(|e| e.to_string())?;

    let composio_user_id = new_composio_user_id(&toolkit);
    let connection = client
        .start_connection(&auth_config, Some(&composio_user_id))
        .await
        .map_err(|e| e.to_string())?;

    // The display name and logo are read now rather than at render time, so a
    // list of connected apps stays readable with the network off. Neither is
    // worth failing a connection over.
    let described = client.toolkit(&toolkit).await.unwrap_or_default();
    let text = |key: &str| {
        described
            .get(key)
            .or_else(|| described.pointer(&format!("/meta/{key}")))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let name = match text("name") {
        found if found.is_empty() => toolkit.clone(),
        found => found,
    };

    // Saved as pending, so a connection abandoned halfway is visible and can
    // be cleaned up rather than lingering only in Composio's records.
    let mut record = ConnectionRecord {
        id: connection.id.clone(),
        toolkit_slug: toolkit,
        name,
        account_id: connection.id.clone(),
        composio_user_id,
        status: "INITIATED".into(),
        ..ConnectionRecord::default()
    };
    let logo = text("logo");
    if !logo.is_empty() {
        record.extra.insert("logo".into(), Value::String(logo));
    }
    workspace.save_composio(&record)?;

    Ok(ConnectionStarted {
        id: connection.id,
        redirect_url: connection.redirect_url,
        status: record.status,
    })
}

/// A fresh Composio user id, which is only ever a label they can group by.
fn new_composio_user_id(toolkit: &str) -> String {
    format!("{toolkit}-{}", uuid::Uuid::new_v4().simple())
}

/// Start the handshake again for a connection that already exists.
///
/// The alternative the user has is disconnect and reconnect, which throws away
/// the row - and with it the list of operations they chose to enable, which for
/// a big toolkit is a real amount of work. Reusing the record keeps that, and
/// keeps the Composio user id, so anything scoped to it there goes on referring
/// to the same person.
#[tauri::command]
pub async fn composio_reconnect(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectionStarted, String> {
    let workspace = state.workspace()?;
    let client = crate::integrations::composio_client(&state)?;

    let mut record = workspace
        .composio_records()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("No connection called `{id}`."))?;
    if record.toolkit_slug.is_empty() {
        return Err("That connection does not name an app to reconnect.".into());
    }

    let auth_config = client
        .auth_config_for(&record.toolkit_slug)
        .await
        .map_err(|e| e.to_string())?;

    if record.composio_user_id.is_empty() {
        record.composio_user_id = new_composio_user_id(&record.toolkit_slug);
    }
    let link = client
        .start_connection(&auth_config, Some(&record.composio_user_id))
        .await
        .map_err(|e| e.to_string())?;

    // The old account is revoked only after the new link exists. Doing it first
    // would leave the user with nothing at all if the link then failed, and an
    // account left dangling in Composio's dashboard is not worth failing over.
    if !record.account_id.is_empty() && record.account_id != link.id {
        let _ = client.revoke(&record.account_id).await;
    }

    record.account_id = link.id.clone();
    record.status = "INITIATED".into();
    record.label = String::new();
    workspace.save_composio(&record)?;
    workspace.composio.drop_cached_tools();

    Ok(ConnectionStarted {
        id: record.id,
        redirect_url: link.redirect_url,
        status: record.status,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStarted {
    pub id: String,
    pub redirect_url: Option<String>,
    /// `"INITIATED"` until the handshake lands. The screen shows the row as
    /// waiting rather than as connected, and polls.
    pub status: String,
}

/// Checks whether a pending connection has completed, and loads its tools if
/// it has.
#[tauri::command]
pub async fn composio_refresh(
    state: State<'_, AppState>,
    id: String,
) -> Result<ConnectionRecord, String> {
    let workspace = state.workspace()?;
    let client = workspace
        .composio
        .client()
        .ok_or_else(|| "No Composio API key is configured.".to_string())?;

    let mut record = workspace
        .composio_records()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("No connection called `{id}`."))?;

    let account = client
        .connected_account(&id)
        .await
        .map_err(|e| e.to_string())?;

    if let Some(status) = account.get("status").and_then(|s| s.as_str()) {
        record.status = status.to_string();
    }

    if record.is_usable() {
        let _ = workspace.composio.connect(record.clone()).await;
    }

    workspace.save_composio(&record)?;
    Ok(record)
}

#[tauri::command]
pub fn composio_disconnect(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let workspace = state.workspace()?;
    workspace.composio.disconnect(&id);
    let removed = workspace.delete_composio(&id);
    // The row has to leave every list that was showing it, not just the one
    // the person clicked in.
    crate::integrations::announce_connection(&app, &id);
    removed
}

/// Reconnects every active app. Called after a workspace opens.
///
/// All at once, not one after another. Each connect is its own walk of that
/// app's paged tool list, so ten connected apps done in turn is ten waits
/// stacked end to end - on every visit to the Integrations screen, which is
/// where this is called from. They do not depend on each other, so they should
/// not queue behind each other.
#[tauri::command]
pub async fn composio_load_all(state: State<'_, AppState>) -> Result<Vec<ConnectionRecord>, String> {
    let workspace = state.workspace()?;
    let records = workspace.composio_records();

    // One app failing to load must not stop the others, which is what
    // discarding each result is for.
    futures::future::join_all(
        records
            .iter()
            .filter(|record| record.is_usable())
            .map(|record| workspace.composio.connect(record.clone())),
    )
    .await;

    Ok(records)
}

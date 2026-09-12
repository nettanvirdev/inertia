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



use futures::StreamExt;
use inertia_core::tool::ToolRegistry;
use inertia_agent::{Agent, AgentConfig, AgentEvent, StopReason, Turn};
use inertia_core::message::Entry;
use inertia_core::SessionId;
use inertia_composio::provider::ConnectionRecord;
use inertia_mcp::client::ServerRecord;
use inertia_openapi::provider::{Auth, Credential, ImportRecord};
use inertia_store::conversations::{Message, Part, Status, Thread, ToolState};
use inertia_store::transcript::to_entries;
use inertia_store::{Models, Permissions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::permission::Answer;
use crate::state::{gate_for, provider_for, AppState, Running};

/// The channel every turn event is emitted on.
///
/// One channel carrying tagged events rather than a channel per event type:
/// the frontend subscribes once, and adding an event later needs no change on
/// either side of the boundary.
pub const AGENT_EVENT: &str = "agent:event";

// ── workspace ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn workspace_open(
    app: AppHandle,
    state: State<'_, AppState>,
    root: String,
) -> Result<String, String> {
    // Opened with a window attached, so a record written from inside a turn -
    // an agent saving another agent, a routine editing itself - redraws the
    // screen the same way the screen's own save does.
    state.open_workspace_for(&app, std::path::PathBuf::from(&root))?;
    Ok(root)
}

#[tauri::command]
pub fn workspace_current(state: State<'_, AppState>) -> Option<String> {
    state.workspace_root()
}

// ── conversations ───────────────────────────────────────────────────────

#[tauri::command]
pub fn threads_list(state: State<'_, AppState>) -> Result<Vec<Thread>, String> {
    Ok(state.workspace()?.conversations.list_threads())
}

#[tauri::command]
pub fn thread_save(state: State<'_, AppState>, thread: Thread) -> Result<(), String> {
    state
        .workspace()?
        .conversations
        .write_thread(&thread)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn thread_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .workspace()?
        .conversations
        .delete_thread(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn messages_read(
    state: State<'_, AppState>,
    thread_id: String,
    live: Option<Vec<String>>,
) -> Result<Vec<Message>, String> {
    // `live` names messages the window is still streaming. Anything else found
    // mid-stream was abandoned by a crash and is settled on the way out, so a
    // conversation never reopens stuck showing "Stop".
    Ok(state
        .workspace()?
        .conversations
        .read_messages(&thread_id, &live.unwrap_or_default()))
}

#[tauri::command]
pub fn messages_save(
    state: State<'_, AppState>,
    thread_id: String,
    messages: Vec<Message>,
) -> Result<(), String> {
    state
        .workspace()?
        .conversations
        .write_messages(&thread_id, &messages)
        .map_err(|e| e.to_string())
}

// ── settings ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn models_get(state: State<'_, AppState>) -> Result<Models, String> {
    Ok(state.workspace()?.settings.models())
}

#[tauri::command]
pub fn models_save(state: State<'_, AppState>, models: Models) -> Result<(), String> {
    state
        .workspace()?
        .settings
        .save_models(&models)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn permissions_get(state: State<'_, AppState>) -> Result<Permissions, String> {
    Ok(state.workspace()?.settings.permissions())
}

#[tauri::command]
pub fn permissions_save(
    state: State<'_, AppState>,
    permissions: Permissions,
) -> Result<(), String> {
    state
        .workspace()?
        .settings
        .save_permissions(&permissions)
        .map_err(|e| e.to_string())
}

/// Asks a provider what models it offers.
#[tauri::command]
pub async fn models_probe(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<Vec<inertia_core::ModelInfo>, String> {
    let workspace = state.workspace()?;
    // Any model id under the provider will do; only the provider half is used.
    let (provider, _) = provider_for(&workspace.settings, &format!("{provider_id}/probe"))?;
    provider.list_models().await.map_err(|e| e.to_string())
}

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

/// Connects every enabled server.
///
/// Called after a workspace opens. Failures are recorded against their records
/// rather than propagated: one misconfigured server must not stop the others
/// from coming up.
#[tauri::command]
pub async fn mcp_connect_all(state: State<'_, AppState>) -> Result<Vec<ServerRecord>, String> {
    let workspace = state.workspace()?;
    let mut results = Vec::new();

    for mut record in workspace.mcp_records() {
        if !record.enabled {
            record.status = "disabled".into();
            results.push(record);
            continue;
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

        let _ = workspace.save_mcp_record(&record);
        results.push(record);
    }

    Ok(results)
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

/// Re-reads every stored document. Called after a workspace opens.
#[tauri::command]
pub fn openapi_load_all(state: State<'_, AppState>) -> Result<Vec<ImportRecord>, String> {
    let workspace = state.workspace()?;
    let mut loaded = Vec::new();

    for mut record in workspace.openapi_records() {
        let Some(document) = workspace.read_openapi_spec(&record.id) else {
            record.error = "The stored document is missing.".into();
            loaded.push(record);
            continue;
        };

        let credential = resolve_openapi_credential(&workspace, &record);
        match workspace.openapi.add(record.clone(), &document, credential) {
            Ok(count) => {
                record.tool_count = count;
                record.error = String::new();
            }
            Err(message) => record.error = message,
        }
        loaded.push(record);
    }

    Ok(loaded)
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposioStatus {
    pub configured: bool,
    pub connections: Vec<ConnectionRecord>,
}

#[tauri::command]
pub fn composio_status(state: State<'_, AppState>) -> Result<ComposioStatus, String> {
    let workspace = state.workspace()?;
    Ok(ComposioStatus {
        configured: crate::integrations::composio_configured_for(&state)?,
        connections: workspace.composio_records(),
    })
}

/// Points the provider at a key held in the secret store.
#[tauri::command]
pub fn composio_set_key(state: State<'_, AppState>, secret: String) -> Result<bool, String> {
    let workspace = state.workspace()?;
    let key = workspace.settings.secrets().get(&secret).map(str::to_string);
    workspace.composio.set_key(key);
    Ok(workspace.composio.has_key())
}

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
#[tauri::command]
pub async fn composio_load_all(state: State<'_, AppState>) -> Result<Vec<ConnectionRecord>, String> {
    let workspace = state.workspace()?;
    let mut loaded = Vec::new();

    for record in workspace.composio_records() {
        if record.is_usable() {
            // One app failing to load must not stop the others.
            let _ = workspace.composio.connect(record.clone()).await;
        }
        loaded.push(record);
    }

    Ok(loaded)
}

// ── permission answers ──────────────────────────────────────────────────

#[tauri::command]
pub fn permission_respond(state: State<'_, AppState>, id: String, answer: Answer) {
    state.answer_permission(&id, answer);
}

// ── running a turn ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStarted {
    pub turn_id: String,
}

/// One event as the frontend receives it, tagged with the turn it belongs to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a> {
    turn_id: &'a str,
    thread_id: &'a str,
    #[serde(flatten)]
    event: &'a AgentEvent,
}

/// Starts a turn and returns immediately.
///
/// The turn runs in the background and reports on [`AGENT_EVENT`]. Returning
/// the id rather than awaiting the turn is what lets the UI stream, show tool
/// cards, and offer a Stop button - an awaited command would block the bridge
/// until the whole turn finished.
#[tauri::command]
// A Tauri command takes its arguments flat, one per field the window sends;
// grouping them into a struct would change the shape of the call.
#[allow(clippy::too_many_arguments)]
pub async fn agent_send(
    app: AppHandle,
    state: State<'_, AppState>,
    thread_id: String,
    text: String,
    model: Option<String>,
    agent_id: Option<String>,
    // The composer's pills, for a caller that has them. A routine does: it was
    // written to run in a mode, with an approval setting, and a turn started
    // with neither is a routine that does not do what its own screen says it
    // does. Absent for a plain send, which has no pills to carry.
    mode: Option<String>,
    approval: Option<String>,
) -> Result<TurnStarted, String> {
    let workspace = state.workspace()?;

    let reference = match model {
        Some(model) if !model.trim().is_empty() => model,
        _ => workspace.settings.models().default_model,
    };
    let (provider, model_id) = provider_for(&workspace.settings, &reference)?;

    // History first: if the conversation cannot be rebuilt, nothing should be
    // sent and nothing should be written.
    let stored = workspace.conversations.read_messages(&thread_id, &[]);
    let mut history: Vec<Entry> = to_entries(&stored);
    history.push(Entry::user(&text));

    let agent_id_for_hooks = agent_id.clone();
    let gate = gate_for(&app, &workspace, thread_id.clone(), agent_id);
    // What this turn's mode does not hold. A routine set to Chat must not be
    // handed `write` here any more than a Chat conversation is.
    let withheld = mode
        .as_deref()
        .map(inertia_agent::prompt::Mode::parse)
        .unwrap_or_default()
        .withheld();
    let registry = crate::state::registry_with(
        &workspace,
        gate.clone(),
        state.project(),
        Vec::new(),
        &withheld,
    );
    // The prompt describes what this turn can do, so it is asked rather than
    // assumed: a tool the rules deny is not in here.
    let held: Vec<String> = registry
        .specs()
        .await
        .into_iter()
        .map(|spec| spec.name)
        .collect();

    let turn_id = format!("turn_{}", uuid::Uuid::now_v7().simple());
    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();

    state.register(
        turn_id.clone(),
        Running {
            cancel,
            // Nothing steers this path: it is the plain "send a message to
            // this thread" command, with no composer behind it to type into a
            // turn that is already running.
            steer: inertia_agent::Steer::new(),
            session: thread_id.clone(),
            gate: gate.clone(),
        },
    );

    let mut agent = Agent::new(
        provider,
        registry,
        gate,
        AgentConfig {
            model: model_id.clone(),
            ..Default::default()
        },
    );

    // Only when the workspace actually has hooks: attaching a listener with
    // nothing to run would put three file reads and an await into every tool
    // call of every turn, for nothing.
    let recorder = app.state::<std::sync::Arc<inertia_hooks::Recorder>>().inner().clone();
    if let Some(hooks) = crate::hooks::TurnHooks::load(
        &workspace.layout,
        &workspace.layout.work_dir(),
        recorder,
        inertia_hooks::Context {
            session_id: Some(thread_id.clone()),
            thread_id: Some(thread_id.clone()),
            turn_id: Some(turn_id.clone()),
            agent: agent_id_for_hooks.clone(),
            cwd: Some(workspace.layout.work_dir()),
            root: Some(workspace.layout.root().to_path_buf()),
            model: Some(model_id.clone()),
            ..Default::default()
        },
    ) {
        agent = agent.with_lifecycle(std::sync::Arc::new(hooks));
    }

    // Ranked against what was just said, so the memories that answer the
    // question are the ones that make the budget - not the ones used most.
    let cwd = workspace.layout.work_dir();
    let memories = crate::memory::block_for(&state, &text);

    let turn = Turn {
        session: SessionId::from_existing(thread_id.clone()),
        // Assembled per turn: the workspace, the date and the tool list all
        // change between turns.
        system: crate::prompt::build(
            &workspace,
            &cwd,
            &memories,
            &crate::prompt::Turn {
                agent_id: agent_id_for_hooks.clone(),
                // Whatever the caller carried. A plain send has no pills and
                // passes neither, which reads as the defaults.
                mode: mode.clone(),
                approval: approval.clone(),
                model: reference.clone(),
                is_subagent: false,
                // This path does not enter worktrees either: it sends into the
                // workspace's own folder.
                worktree: None,
                previous_cwd: None,
                tools: held.clone(),
                // Nor a room: this path sends to one agent by id, and a
                // conversation whose floor is decided elsewhere is exactly what
                // it is not.
                room: None,
            },
        ),
        history,
        root: cwd,
    };

    // Everything below runs detached, reporting through events.
    let task_app = app.clone();
    let task_turn = turn_id.clone();
    let task_thread = thread_id.clone();
    let task_model = model_id;
    let task_model_ref = reference.clone();
    let task_prompt = text;
    // What the capture pass will read, taken now: by the time the turn ends it
    // owns the history, and the pass wants what was sent anyway.
    let task_history = turn.history.clone();

    tauri::async_runtime::spawn(async move {
        let mut events = agent.run(turn);
        let mut assembled = Assembling::new(&task_model, task_prompt);

        // `select!` rather than a token threaded through the loop: dropping the
        // stream is what cancels the provider request, and this is where the
        // drop happens.
        let stop = async move {
            let _ = cancelled.await;
        };
        tokio::pin!(stop);

        loop {
            tokio::select! {
                biased;

                _ = &mut stop => {
                    assembled.interrupted();
                    break;
                }

                next = events.next() => {
                    let Some(event) = next else { break };
                    assembled.observe(&event);

                    let _ = task_app.emit(
                        AGENT_EVENT,
                        Envelope {
                            turn_id: &task_turn,
                            thread_id: &task_thread,
                            event: &event,
                        },
                    );

                    if matches!(event, AgentEvent::Done { .. }) {
                        break;
                    }
                }
            }
        }

        // Persisted here rather than by the frontend, so a window that closes
        // mid-turn still leaves the conversation on disk.
        if let Some(state) = task_app.try_state::<AppState>() {
            if let Ok(workspace) = state.workspace() {
                let mut messages = workspace.conversations.read_messages(&task_thread, &[]);
                messages.push(assembled.user);
                messages.push(assembled.reply);
                if let Err(e) = workspace
                    .conversations
                    .write_messages(&task_thread, &messages)
                {
                    tracing::error!(error = %e, "could not save the conversation");
                }
            }
            state.finished(&task_turn);

            // A conversation does not end, it goes quiet. This arms a timer
            // that will mine it for what it was worth if nothing else is said;
            // the next turn stands the timer down again.
            crate::memory::arm(&state, &task_thread, &task_history, &task_model_ref);
        }
    });

    Ok(TurnStarted { turn_id })
}

#[tauri::command]
pub fn agent_stop(state: State<'_, AppState>, turn_id: String) -> bool {
    state.stop(&turn_id)
}

/// Builds the two stored messages a turn produces, as it streams.
///
/// Kept here rather than in the agent crate because it is a *storage* concern:
/// the agent already reports a canonical transcript, and this is the UI-facing
/// shape derived from it.
struct Assembling {
    user: Message,
    reply: Message,
}

impl Assembling {
    fn new(model: &str, prompt: String) -> Self {
        let now = jiff::Timestamp::now().to_string();
        Self {
            user: Message {
                id: format!("msg_{}", uuid::Uuid::now_v7().simple()),
                role: "user".into(),
                content: prompt,
                created_at: now.clone(),
                status: Some(Status::Sent),
                ..Default::default()
            },
            reply: Message {
                id: format!("msg_{}", uuid::Uuid::now_v7().simple()),
                role: "agent".into(),
                created_at: now,
                status: Some(Status::Streaming),
                model: Some(model.to_string()),
                ..Default::default()
            },
        }
    }

    fn observe(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::Delta { text } => {
                self.reply.content.push_str(text);
                match self.reply.parts.last_mut() {
                    Some(Part::Text { text: existing }) => existing.push_str(text),
                    _ => self.reply.parts.push(Part::Text { text: text.clone() }),
                }
            }
            AgentEvent::ToolStarted { call, .. } => {
                self.reply.parts.push(Part::Tool {
                    call_id: Some(call.id.to_string()),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                    state: ToolState::Running,
                    output: None,
                    ok: None,
                });
            }
            AgentEvent::ToolFinished { result } => {
                // Matched by id rather than by position: calls in one batch
                // finish in whatever order they finish.
                for part in self.reply.parts.iter_mut() {
                    if let Part::Tool {
                        call_id,
                        state,
                        output,
                        ok,
                        ..
                    } = part
                    {
                        if call_id.as_deref() == Some(result.call_id.as_str()) {
                            *state = if result.ok {
                                ToolState::Done
                            } else {
                                ToolState::Failed
                            };
                            *output = Some(result.output.clone());
                            *ok = Some(result.ok);
                        }
                    }
                }
            }
            AgentEvent::Done { stopped, history, .. } => {
                self.reply.status = Some(Status::Sent);
                self.reply.stopped = matches!(
                    stopped,
                    StopReason::Cancelled | StopReason::MaxSteps | StopReason::Refused
                );
                if let StopReason::Error { message } = stopped {
                    self.reply.error = Some(message.clone());
                }
                // The signed reasoning is only recoverable from the canonical
                // transcript, and it has to be stored to be replayed.
                if let Some(Entry::Assistant(last)) = history
                    .iter()
                    .rev()
                    .find(|e| matches!(e, Entry::Assistant(_)))
                {
                    self.reply.thinking = last
                        .thinking
                        .iter()
                        .filter_map(|b| serde_json::to_value(b).ok())
                        .collect();
                }
            }
            _ => {}
        }
    }

    /// The turn was stopped before it reported `Done`.
    fn interrupted(&mut self) {
        self.reply.status = Some(Status::Sent);
        self.reply.stopped = true;
        for part in self.reply.parts.iter_mut() {
            if let Part::Tool { state, output, .. } = part {
                if *state == ToolState::Running {
                    *state = ToolState::Failed;
                    *output = Some("This call was interrupted and never finished.".into());
                }
            }
        }
    }
}


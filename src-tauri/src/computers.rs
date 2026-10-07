//! Computers: the command surface over the three providers.
//!
//! A record in `computers/` says which provider a machine belongs to and what
//! that provider calls it (`handle`). Everything here is: read the record, pick
//! the provider, do the thing, write back what changed. No screen and no tool
//! knows which provider answered.
//!
//! Settings and keys are read per call rather than cached. A user can paste a
//! Daytona key into the Secrets pane while the computers screen is open, and a
//! cached "not configured" would mean a relaunch to pick it up.

use std::sync::Arc;

use inertia_computers::{
    daytona::DaytonaProvider, desktop, docker::DockerProvider, image, local::LocalProvider,
    ExecRequest, Provider, Spec, Status,
};
use inertia_store::layout::{Collection, Document};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

/// The channel a running command reports on.
const COMPUTER_EVENT: &str = "computer:event";

/// Commands still running, by the run id the caller was given.
///
/// The Stop button existed from the first build and stopped nothing: the
/// bridge answered `{cancelled: false}` without asking anybody. Dropping the
/// provider future is what actually ends a command - `spawn_docker` sets
/// `kill_on_drop`, and the local provider kills its child the same way - so
/// the whole of cancelling is holding onto something that can drop it.
///
/// A Daytona command is an HTTP request to a toolbox that has already been
/// handed the command, so aborting stops this app waiting and does not reach
/// into the sandbox. Said plainly in the answer rather than implied.
static RUNNING: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<String, tokio::task::AbortHandle>>,
> = std::sync::LazyLock::new(Default::default);

/// Runs a command so that [`computer_cancel`] can end it.
///
/// The work goes onto its own task and the handle into the map, so the
/// invoke's future is not the only thing holding it. `finally` style cleanup
/// is on both paths, because a run left in the map is a run the Stop button
/// would later claim to have cancelled.
async fn cancellable<F>(run_id: &str, work: F) -> Result<F::Output, String>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    // `tokio::spawn` rather than Tauri's wrapper, which hands back a handle
    // with no way to abort it - and abort is the entire point here.
    let task = tokio::spawn(work);
    RUNNING
        .lock()
        .insert(run_id.to_string(), task.abort_handle());

    let finished = task.await;
    RUNNING.lock().remove(run_id);

    finished.map_err(|failed| {
        if failed.is_cancelled() {
            "Stopped.".to_string()
        } else {
            format!("The command ended unexpectedly: {failed}")
        }
    })
}

/// Stop a command that is still running.
///
/// Answers whether there was one. A run that finished a moment ago is not an
/// error - the button and the answer raced, and the person got what they
/// wanted either way.
#[tauri::command]
pub fn computer_cancel(run_id: String) -> Value {
    match RUNNING.lock().remove(&run_id) {
        Some(handle) => {
            handle.abort();
            json!({ "cancelled": true })
        }
        None => json!({ "cancelled": false }),
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The settings document, with the defaults a fresh workspace gets.
fn settings(state: &AppState) -> Result<Value, String> {
    let workspace = state.workspace()?;
    Ok(inertia_store::collections::read_document(
        &workspace.layout,
        Document::Computers,
        json!({
            "defaultProvider": "docker",
            "daytonaApiUrl": "",
            "daytonaImage": "",
            "autoStopMinutes": 5,
            "defaultSpecs": { "cpu": 4, "memoryGb": 8, "diskGb": 40 },
            "pollSeconds": 10,
        }),
    ))
}

/// Builds one provider by name.
///
/// Every provider is constructible even when it cannot run - `available()` is
/// what says whether it can, and it answers with a reason. A registry that
/// omitted the ones that are not ready would make the settings screen unable to
/// explain why a provider is missing.
fn provider_for(state: &AppState, id: &str) -> Result<Arc<dyn Provider>, String> {
    let workspace = state.workspace()?;
    Ok(match id {
        "local" => Arc::new(LocalProvider::new(workspace.layout.root().to_path_buf())),
        "daytona" => Arc::new(daytona(state)?),
        "docker" => Arc::new(DockerProvider::new()),
        other => return Err(format!("There is no `{other}` provider.")),
    })
}

/// Daytona, pointed wherever the workspace says.
///
/// Read per call rather than cached, because a person can paste a key into the
/// secrets pane with the computers screen already open, and a remembered "not
/// configured" is the kind of small lie that reads as the feature being broken.
///
/// The endpoint is a secret first and a setting second: a self-hosted Daytona
/// is configured by putting `DAYTONA_API_URL` next to its key, which is where
/// someone would look for it.
fn daytona(state: &AppState) -> Result<DaytonaProvider, String> {
    let workspace = state.workspace()?;
    let secret = |name: &str| {
        inertia_store::secrets::get(&workspace.layout, name)
            .map_err(err)
            .map(|found| found.filter(|value| !value.trim().is_empty()))
    };

    let url = match secret("DAYTONA_API_URL")? {
        Some(from_secrets) => Some(from_secrets),
        None => settings(state)?
            .get("daytonaApiUrl")
            .and_then(Value::as_str)
            .map(str::to_string),
    };

    Ok(DaytonaProvider::new(
        secret(inertia_computers::daytona::KEY_SECRET)?.unwrap_or_default(),
        url,
    ))
}

const PROVIDER_IDS: &[&str] = &["docker", "daytona", "local"];

/// One machine's record, or an error naming it.
fn record_of(state: &AppState, id: &str) -> Result<Value, String> {
    let workspace = state.workspace()?;
    inertia_store::collections::get(&workspace.layout, Collection::Computers, id)
        .map_err(err)?
        .ok_or_else(|| format!("There is no computer called `{id}`."))
}

fn save(state: &AppState, record: Value) -> Result<Value, String> {
    let workspace = state.workspace()?;
    inertia_store::collections::put(&workspace.layout, Collection::Computers, record).map_err(err)
}

/// Applies a patch to a stored record and saves it.
fn patch(state: &AppState, id: &str, changes: Value) -> Result<Value, String> {
    let workspace = state.workspace()?;
    inertia_store::collections::patch(&workspace.layout, Collection::Computers, id, changes)
        .map_err(err)
}

fn field<'a>(record: &'a Value, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The provider and handle for a stored machine.
fn machine(state: &AppState, id: &str) -> Result<(Arc<dyn Provider>, Value), String> {
    let record = record_of(state, id)?;
    let provider_id = {
        let named = field(&record, "provider");
        if named.is_empty() {
            settings(state)?
                .get("defaultProvider")
                .and_then(Value::as_str)
                .unwrap_or("docker")
                .to_string()
        } else {
            named.to_string()
        }
    };
    Ok((provider_for(state, &provider_id)?, record))
}

fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/* -- what exists ---------------------------------------------------------- */

/// Every provider, and whether it can run right now.
///
/// The reason matters more than the flag: "Docker is not running" is something
/// a person can act on, and "unavailable" is not.
#[tauri::command]
pub async fn computer_providers(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for id in PROVIDER_IDS {
        let provider = provider_for(&state, id)?;
        let readiness = provider.available().await;
        // `ok`, not `available`: the picker disables a row on `!row.ok`, so
        // spelling it the other way left every provider greyed out with no
        // reason shown next to it.
        let mut row = json!({
            "id": provider.id(),
            "label": provider.label(),
            "blurb": provider.blurb(),
            "ok": readiness.ready,
            "reason": readiness.reason,
            "warning": readiness.warning,
            "hint": readiness.hint,
        });
        if *id == "daytona" {
            row["apiUrl"] = json!(daytona(&state)?.api_url());
        }
        out.push(row);
    }
    Ok(out)
}

#[tauri::command]
pub fn computer_settings(state: State<'_, AppState>) -> Result<Value, String> {
    settings(&state)
}

#[tauri::command]
pub fn computer_save_settings(state: State<'_, AppState>, patch: Value) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let mut current = settings(&state)?;
    if let (Some(target), Value::Object(changes)) = (current.as_object_mut(), patch) {
        for (key, value) in changes {
            target.insert(key, value);
        }
    }
    inertia_store::collections::write_document(&workspace.layout, Document::Computers, &current)
        .map_err(err)?;
    Ok(current)
}

/// What every machine is doing, asked of its provider rather than remembered.
///
/// The stored status is only a memory of the last time anyone looked. A
/// container the user stopped by hand, a sandbox reaped for idleness - both are
/// ordinary, and a list that showed the remembered state would be confidently
/// wrong.
#[tauri::command]
pub async fn computer_list(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let workspace = state.workspace()?;
    let records = inertia_store::collections::list(&workspace.layout, Collection::Computers);

    let mut out = Vec::new();
    for record in records {
        let id = field(&record, "id").to_string();
        out.push(match refreshed(&state, &id).await {
            Ok(fresh) => fresh,
            // One machine whose provider is unreachable must not empty the
            // screen; it is shown as it was last seen.
            Err(_) => record,
        });
    }
    Ok(out)
}

/// Asks the provider what one machine is doing, and stores the answer.
async fn refreshed(state: &AppState, id: &str) -> Result<Value, String> {
    let (provider, record) = machine(state, id)?;
    let handle = field(&record, "handle").to_string();
    if handle.is_empty() {
        return Ok(record);
    }

    let health = provider.status(&handle).await;
    let stats = provider.stats(&handle).await;

    let mut changes = json!({
        "status": health.status,
        "usage": {
            "cpuPct": stats.cpu_pct,
            "memPct": stats.mem_pct,
            "diskPct": stats.disk_pct,
            "scope": stats.scope,
            "at": now(),
        },
    });
    if let Some(started) = health.started_at {
        changes["startedAt"] = json!(started);
    }
    patch(state, id, changes)
}

#[tauri::command]
pub async fn computer_refresh(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    refreshed(&state, &id).await
}

/* -- lifecycle ------------------------------------------------------------ */

#[tauri::command]
pub async fn computer_create(state: State<'_, AppState>, draft: Value) -> Result<Value, String> {
    let settings = settings(&state)?;
    let provider_id = draft
        .get("provider")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| {
            settings
                .get("defaultProvider")
                .and_then(Value::as_str)
                .unwrap_or("docker")
        })
        .to_string();

    let provider = provider_for(&state, &provider_id)?;
    let readiness = provider.available().await;
    if !readiness.ready {
        return Err(readiness.reason);
    }

    // Saved before it is built, so a machine that fails to come up is still a
    // record the user can see, correct and retry rather than having to re-enter.
    let mut record = draft;
    if let Value::Object(map) = &mut record {
        map.insert("provider".into(), json!(provider_id));
        map.insert("status".into(), json!(Status::Provisioning));
    }
    let record = save(&state, record)?;
    let id = field(&record, "id").to_string();

    let specs = record.get("specs").cloned().unwrap_or(json!({}));
    let number = |key: &str| specs.get(key).and_then(Value::as_f64);

    let spec = Spec {
        id: id.clone(),
        cpu: number("cpu"),
        memory_gb: number("memoryGb"),
        disk_gb: number("diskGb"),
        image: record
            .get("image")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string),
    };

    match provider.create(&spec).await {
        Ok(made) => patch(
            &state,
            &id,
            json!({
                "handle": made.handle,
                "machineName": made.name,
                "status": made.status,
                "os": made.os,
                "image": made.image,
                "workdir": made.workdir,
                "error": Value::Null,
                "lastUsedAt": now(),
            }),
        ),
        Err(failure) => {
            patch(
                &state,
                &id,
                json!({ "status": Status::Error, "error": failure.to_string() }),
            )?;
            Err(failure.to_string())
        }
    }
}

/// What the provider calls this machine.
pub(crate) fn handle_of(record: &Value) -> String {
    field(record, "handle").to_string()
}

/// One lifecycle verb, applied and recorded.
async fn lifecycle(state: &AppState, id: &str, action: &str) -> Result<Value, String> {
    let (provider, record) = machine(state, id)?;
    let handle = field(&record, "handle").to_string();
    let name = field(&record, "machineName").to_string();

    if handle.is_empty() && action != "remove" {
        return Err("That machine has not been built yet.".into());
    }

    let outcome = match action {
        "start" => provider.start(&handle).await,
        "stop" => provider.stop(&handle).await,
        "pause" => provider.pause(&handle).await,
        "resume" => provider.resume(&handle).await,
        "remove" => {
            let name = (!name.is_empty()).then_some(name.as_str());
            provider.remove(&handle, name).await
        }
        other => {
            return Err(format!(
                "`{other}` is not something a computer can be asked to do."
            ))
        }
    };

    if let Err(failure) = outcome {
        patch(state, id, json!({ "error": failure.to_string() }))?;
        return Err(failure.to_string());
    }

    if action == "remove" {
        let workspace = state.workspace()?;
        inertia_store::collections::remove(&workspace.layout, Collection::Computers, id)
            .map_err(err)?;
        return Ok(json!({ "id": id, "removed": true }));
    }

    patch(
        state,
        id,
        json!({ "error": Value::Null, "lastUsedAt": now() }),
    )?;
    refreshed(state, id).await
}

#[tauri::command]
pub async fn computer_start(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    lifecycle(&state, &id, "start").await
}

#[tauri::command]
pub async fn computer_stop(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    lifecycle(&state, &id, "stop").await
}

#[tauri::command]
pub async fn computer_pause(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    lifecycle(&state, &id, "pause").await
}

#[tauri::command]
pub async fn computer_resume(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    lifecycle(&state, &id, "resume").await
}

#[tauri::command]
pub async fn computer_remove(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    lifecycle(&state, &id, "remove").await
}

/* -- using one ------------------------------------------------------------ */

/// Runs a command in a machine.
///
/// A non-zero exit is a result, not an error: an agent reading a failing test
/// suite can decide what to do, where one handed an exception cannot tell that
/// from an unreachable machine.
#[tauri::command]
pub async fn computer_exec(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    request: Value,
) -> Result<Value, String> {
    // `running_machine`, not `machine`: a command sent to a stopped container
    // fails with a sentence about Docker rather than about this machine, and
    // the pane's own guard reads a record that may be a moment out of date.
    let (provider, record) = running_machine(&state, &id)?;
    let handle = field(&record, "handle").to_string();

    let text = |key: &str| request.get(key).and_then(Value::as_str).map(str::to_string);
    let run_id = text("runId").unwrap_or_else(|| format!("run-{}", uuid::Uuid::now_v7().simple()));

    let ask = ExecRequest {
        command: text("command").unwrap_or_default(),
        cwd: text("cwd"),
        env: request
            .get("env")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        timeout: request
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(std::time::Duration::from_millis),
    };

    // Said before the command runs, because it carries the run id and the id is
    // what makes Stop possible. The pane opens its subscription before it sends
    // the request and sets its id from this; nothing ever sent one, so
    // `runId` stayed null and the Stop button was a no-op for the life of the
    // app - against `cancellable` below, which was ready for it.
    let _ = app.emit(
        COMPUTER_EVENT,
        json!({ "runId": run_id, "computerId": id, "type": "start" }),
    );

    let result = {
        let provider = provider.clone();
        let handle = handle.clone();
        cancellable(&run_id, async move { provider.exec(&handle, &ask).await }).await?
    }
    .map_err(err)?;

    // The whole output arrives at once from every provider here, so there is
    // nothing to stream - but the pane paints on this event, and one that never
    // arrives is a pane that stays empty after a command it watched run.
    let _ = app.emit(
        COMPUTER_EVENT,
        json!({
            "runId": run_id,
            "computerId": id,
            "type": "done",
            "code": result.code,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }),
    );
    let _ = patch(&state, &id, json!({ "lastUsedAt": now() }));

    Ok(json!({
        "runId": run_id,
        "code": result.code,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "durationMs": result.duration_ms,
        "timedOut": result.timed_out,
        "ok": result.ok(),
    }))
}

#[tauri::command]
pub async fn computer_list_dir(
    state: State<'_, AppState>,
    id: String,
    path: Option<String>,
) -> Result<Vec<Value>, String> {
    let (provider, record) = machine(&state, &id)?;
    let entries = provider
        .list_dir(field(&record, "handle"), &path.unwrap_or_default())
        .await
        .map_err(err)?;
    entries
        .into_iter()
        .map(|entry| serde_json::to_value(entry).map_err(err))
        .collect()
}

#[tauri::command]
pub async fn computer_read_file(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<String, String> {
    let (provider, record) = machine(&state, &id)?;
    provider
        .read_file(field(&record, "handle"), &path)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn computer_read_file_bytes(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<String, String> {
    let (provider, record) = machine(&state, &id)?;
    provider
        .read_file_base64(field(&record, "handle"), &path)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn computer_write_file(
    state: State<'_, AppState>,
    id: String,
    path: String,
    content: Option<String>,
) -> Result<Value, String> {
    let (provider, record) = machine(&state, &id)?;
    provider
        .write_file(
            field(&record, "handle"),
            &path,
            &content.unwrap_or_default(),
        )
        .await
        .map_err(err)?;
    Ok(json!({ "path": path, "written": true }))
}

/// A file out of the machine and onto this disk, as base64 for the window to
/// save.
#[tauri::command]
pub async fn computer_download_file(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<Value, String> {
    let bytes = computer_read_file_bytes(state, id, path.clone()).await?;
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("download")
        .to_string();
    Ok(json!({ "name": name, "bytes": bytes }))
}

/* -- the tools an agent with a machine gets -------------------------------- */

/// The computer tools for one agent, or nothing.
///
/// Here rather than in `computer_tools` because this is the half that needs the
/// workspace: which machine an agent was assigned, and which provider owns it.
/// The tools themselves take a provider and a record and know nothing about
/// either.
///
/// Empty for an agent with no `computerId`, for one pointing at a machine that
/// has since been removed, and for a turn with no agent at all. Empty rather
/// than an error in every case: an agent that lost its machine should carry on
/// with `shell`, not fail the turn.
pub fn tools_for_agent(
    state: &AppState,
    agent_id: Option<&str>,
) -> Vec<Arc<dyn inertia_core::tool::Tool>> {
    let Ok(workspace) = state.workspace() else {
        return Vec::new();
    };
    let Some(agent_id) = agent_id else {
        return Vec::new();
    };

    let agent = inertia_store::collections::get(&workspace.layout, Collection::Agents, agent_id)
        .ok()
        .flatten();
    let computer_id = agent
        .as_ref()
        .and_then(|record| record.get("computerId"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty());
    let Some(computer_id) = computer_id else {
        return Vec::new();
    };

    // A machine the user removed while an agent still pointed at it is an
    // ordinary state of the world, and the turn has to run without it.
    let Ok((provider, record)) = machine(state, computer_id) else {
        return Vec::new();
    };
    crate::computer_tools::computer_tools(provider, record)
}

/* -- the image ------------------------------------------------------------ */

/// What the sandbox image is called and whether there is anything to build it
/// from.
///
/// Answered from the binary rather than from a folder on disk, so it is the
/// same answer under `tauri dev` and in an installed build, and `buildable` is
/// a fact rather than a filesystem question. See `inertia_computers::image` for
/// why the build context is compiled in rather than bundled as a resource.
#[tauri::command]
pub fn computer_image() -> Value {
    image::info()
}

/// Build the sandbox image now, streaming the output.
///
/// The first Docker machine builds it either way. Doing it here, from a screen
/// that can show the log, is the difference between a fifteen-minute build and
/// fifteen minutes of a dialog that looks hung - which is why the events go out
/// as they arrive rather than at the end.
#[tauri::command]
pub async fn computer_build_image(app: AppHandle) -> Result<Value, String> {
    // Asked before the build rather than left to `docker build` to fail:
    // "Docker is installed but not running" is a sentence a person can act on,
    // and whatever the CLI says when the daemon is down is not.
    //
    // Docker directly rather than through `provider_for`, because building the
    // image needs no workspace and a person can reasonably press Build on the
    // settings screen before they have chosen a folder.
    let readiness = DockerProvider::new().available().await;
    if !readiness.ready {
        return Err(readiness.reason);
    }

    let run_id = format!("build-{}", uuid::Uuid::now_v7().simple());
    let _ = app.emit(
        COMPUTER_EVENT,
        json!({ "type": "build-start", "runId": run_id }),
    );

    let emitter = app.clone();
    let streaming = run_id.clone();
    let sink = move |stream: &str, text: &str| {
        let _ = emitter.emit(
            COMPUTER_EVENT,
            json!({
                "type": "output",
                "runId": streaming,
                "stream": stream,
                "text": text,
            }),
        );
    };

    match inertia_computers::docker::ensure_image(&inertia_computers::docker::Cli, &sink).await {
        Ok(built) => {
            let answer = json!({ "built": built.built, "ref": built.reference });
            let _ = app.emit(
                COMPUTER_EVENT,
                json!({
                    "type": "build-end",
                    "runId": run_id,
                    "ok": true,
                    "built": built.built,
                    "ref": built.reference,
                }),
            );
            Ok(answer)
        }
        Err(failure) => {
            let message = failure.to_string();
            let _ = app.emit(
                COMPUTER_EVENT,
                json!({
                    "type": "build-end",
                    "runId": run_id,
                    "ok": false,
                    "error": message,
                }),
            );
            Err(message)
        }
    }
}

/// What a cloud machine can be made from.
///
/// Empty for every provider that has no such concept - Docker takes a real
/// `--cpus` and `--memory`, which is the ordinary way round. Daytona does have
/// one, and this answered an empty list for it regardless, so the New computer
/// dialog said "No snapshots on this account" to people whose account had
/// snapshots and then refused to let them press Create.
///
/// Empty rather than an error when the read fails, on purpose: a catalogue
/// that cannot be read is not a reason to block making a machine, and the
/// dialog falls back to the provider's own default.
#[tauri::command]
pub async fn computer_catalogue(
    state: State<'_, AppState>,
    provider_id: Option<String>,
) -> Result<Vec<Value>, String> {
    let provider_id = provider_id.unwrap_or_else(|| "docker".into());
    if provider_id != "daytona" {
        return Ok(Vec::new());
    }

    let provider = provider_for(&state, &provider_id)?;
    // Daytona's snapshot list belongs to the account, not to any one sandbox,
    // so the handle it takes is ignored - which is what lets the dialog ask
    // before there is a machine to ask about.
    let snapshots = provider.snapshots("").await.unwrap_or_default();

    Ok(snapshots
        .into_iter()
        .map(|snapshot| {
            json!({
                "id": snapshot.id,
                "name": snapshot.name,
                "createdAt": snapshot.created_at,
                "size": snapshot.size,
                // Which of them the dialog should land on. The one this app
                // publishes is the one it knows will work.
                "desktop": snapshot.name == image_snapshot_name(),
            })
        })
        .collect())
}

/// The snapshot name this build expects to find on a Daytona account.
fn image_snapshot_name() -> String {
    let manifest = inertia_computers::image::manifest();
    if manifest.daytona_snapshot.is_empty() {
        manifest.reference.clone()
    } else {
        manifest.daytona_snapshot.clone()
    }
}

/* -- the desktop ---------------------------------------------------------- */

/// A machine that is running, or the sentence saying why it is not.
///
/// Checked before anything that drives a screen, because every one of those
/// commands against a stopped container fails with a message about a container
/// rather than about the machine.
pub(crate) fn running_machine(
    state: &AppState,
    id: &str,
) -> Result<(Arc<dyn Provider>, Value), String> {
    let (provider, record) = machine(state, id)?;
    let status = field(&record, "status");
    if status != "running" {
        let name = field(&record, "name");
        let name = if name.is_empty() { id } else { name };
        let doing = if status.is_empty() {
            "not running"
        } else {
            status
        };
        return Err(format!("{name} is {doing}. Start it first."));
    }
    Ok((provider, record))
}

/// Drive the machine's desktop.
///
/// The Browser pane and the agent's computer tools go through the same command
/// builders in `inertia_computers::desktop`, so a fix to one is a fix to both.
/// When each carried its own launch command they were opening different
/// browsers - different profile directories, different flags - and that is how
/// one pane kept hitting an error weeks after the tool had stopped hitting it.
#[tauri::command]
pub async fn computer_drive(
    state: State<'_, AppState>,
    id: String,
    action: String,
    args: Option<Value>,
) -> Result<Value, String> {
    let args = args.unwrap_or_else(|| json!({}));
    let (provider, record) = running_machine(&state, &id)?;
    let command = drive_command(&action, &args)?;

    let timeout = args
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .unwrap_or(90_000);

    let result = provider
        .exec(
            field(&record, "handle"),
            &ExecRequest {
                command,
                timeout: Some(std::time::Duration::from_millis(timeout)),
                ..Default::default()
            },
        )
        .await
        .map_err(err)?;

    let said = said_by(&result.stdout, &result.stderr);

    // Exit 3 is the agreed "this machine cannot do that". Kept as a field
    // rather than raised, because the pane wants to render the reason and a
    // raised error arrives at the UI as a toast that vanishes.
    Ok(json!({
        "ok": result.code == 0,
        "code": result.code,
        "text": said,
        "unsupported": result.code == desktop::NO_DESKTOP_CODE,
    }))
}

/// One pane verb, as the shell line that performs it.
///
/// The pane speaks in single verbs because that is what its buttons are; the
/// machine speaks in batches. One action is a batch of one, so the pane and the
/// agent's `computer_act` still go through the same compiler and neither can
/// drift from the other.
///
/// Its own function so it can be tested without an `AppState`: this is the part
/// with a decision in it, and the rest of the command is plumbing.
pub(crate) fn drive_command(action: &str, args: &Value) -> Result<String, String> {
    let one = |kind: &str, fields: Value| -> Result<String, String> {
        let mut item = json!({ "kind": kind });
        if let (Some(target), Some(extra)) = (item.as_object_mut(), fields.as_object()) {
            for (key, value) in extra {
                if !value.is_null() {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
        let actions = desktop::parse_actions(Some(&json!([item])), &Value::Null)?;
        Ok(desktop::batch(
            &actions,
            args.get("settleMs").and_then(Value::as_i64),
            false,
        ))
    };

    let arg = |key: &str| args.get(key).cloned().unwrap_or(Value::Null);

    Ok(match action {
        "open" => desktop::open(args.get("url").and_then(Value::as_str).unwrap_or_default()),
        "close" => desktop::close_browser(),
        "pageText" => desktop::page_text(),
        "windows" => desktop::windows(),
        "observe" => desktop::observe_annotated(),
        "click" => one(
            "click",
            json!({
                "x": arg("x"),
                "y": arg("y"),
                "button": arg("button"),
                "double": arg("double"),
            }),
        )?,
        "type" => one("type", json!({ "text": arg("text") }))?,
        "key" => one(
            "key",
            json!({ "key": arg("key"), "modifiers": arg("modifiers") }),
        )?,
        "scroll" => one(
            "scroll",
            json!({ "direction": arg("direction"), "amount": arg("amount") }),
        )?,
        other => return Err(format!("Unknown desktop action: {other}")),
    })
}

/// What a command said, both streams together, in the order a person reads
/// them.
pub(crate) fn said_by(stdout: &str, stderr: &str) -> String {
    [stdout, stderr]
        .iter()
        .filter(|part| !part.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Where this machine's screen can be watched, and where it can be driven.
///
/// Asked of the provider every time rather than stored on the record, because
/// the answer is a host port Docker picks fresh on every `docker start`. A
/// cached one is right until the first restart and then points at nothing,
/// which is the worst of both - the pane would show a blank frame and no
/// reason.
///
/// Null for a machine with no such thing, which the pane draws by falling back
/// to still frames. Null rather than an error for a machine that is not running
/// too: the pane asks the moment a machine appears, and an error there would be
/// a toast for a perfectly ordinary state.
#[tauri::command]
pub async fn computer_screen(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Value>, String> {
    let Ok((provider, record)) = running_machine(&state, &id) else {
        return Ok(None);
    };
    let found = provider
        .screen(field(&record, "handle"))
        .await
        .unwrap_or(None);
    Ok(found.map(|screen| json!({ "view": screen.view, "control": screen.control })))
}

/* -- snapshots and the screen --------------------------------------------- */

#[tauri::command]
pub async fn computer_snapshot(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
) -> Result<Value, String> {
    let (provider, record) = machine(&state, &id)?;
    let label = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(now);
    let snapshot = provider
        .snapshot(field(&record, "handle"), &label)
        .await
        .map_err(err)?;

    let count = provider
        .snapshots(field(&record, "handle"))
        .await
        .map(|all| all.len())
        .unwrap_or(0);
    patch(&state, &id, json!({ "snapshotCount": count }))?;

    serde_json::to_value(snapshot).map_err(err)
}

#[tauri::command]
pub async fn computer_snapshots(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<Value>, String> {
    let (provider, record) = machine(&state, &id)?;
    let all = provider
        .snapshots(field(&record, "handle"))
        .await
        .map_err(err)?;
    all.into_iter()
        .map(|s| serde_json::to_value(s).map_err(err))
        .collect()
}

#[tauri::command]
pub async fn computer_restore(
    state: State<'_, AppState>,
    id: String,
    snapshot_id: String,
) -> Result<Value, String> {
    let (provider, record) = machine(&state, &id)?;
    let name = field(&record, "machineName").to_string();
    let name = if name.is_empty() {
        inertia_computers::machine_name(&id)
    } else {
        name
    };

    let restored = provider
        .restore(field(&record, "handle"), &snapshot_id, &name)
        .await
        .map_err(err)?;

    // The handle can change - Daytona restores by making a new sandbox - so it
    // is written back rather than assumed unchanged.
    patch(
        &state,
        &id,
        json!({
            "handle": restored.handle,
            "machineName": restored.name,
            "status": restored.status,
            "image": restored.image,
            "error": Value::Null,
        }),
    )
}

#[tauri::command]
pub async fn computer_screenshot(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<String>, String> {
    let (provider, record) = machine(&state, &id)?;
    provider
        .screenshot(field(&record, "handle"))
        .await
        .map_err(err)
}

/// Which agents may drive this machine.
#[tauri::command]
pub fn computer_assign(
    state: State<'_, AppState>,
    id: String,
    agent_ids: Vec<String>,
) -> Result<Value, String> {
    patch(&state, &id, json!({ "assignedAgentIds": agent_ids }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_provider_is_one_that_exists() {
        // The registry and the ids the settings screen offers are the same
        // list, so a default naming a provider nobody built is impossible.
        assert!(PROVIDER_IDS.contains(&"docker"));
        assert!(PROVIDER_IDS.contains(&"daytona"));
        assert!(PROVIDER_IDS.contains(&"local"));
        assert_eq!(PROVIDER_IDS.len(), 3);
    }

    /// The settings pane reads `ref` and `buildable` off this, and draws "No
    /// Dockerfile found" when `buildable` is false. It never can be: the
    /// context is compiled into the binary.
    #[test]
    fn the_image_answers_from_the_binary_rather_than_from_a_folder() {
        let info = computer_image();
        assert_eq!(info["ref"], json!("inertia-sandbox:1.1.0"));
        assert_eq!(info["buildable"], json!(true));
        assert_eq!(info["workdir"], json!("/workspace"));
    }

    /// The pane's back, forward and reload buttons are all one `key` verb with
    /// the chord in one string, and its address bar is `open`.
    #[test]
    fn every_verb_the_browser_pane_uses_compiles() {
        let open = drive_command("open", &json!({ "url": "https://example.com" })).unwrap();
        assert!(open.contains("xdg-open 'https://example.com'"));

        let back = drive_command("key", &json!({ "key": "alt+Left" })).unwrap();
        assert!(back.contains("xdotool key --clearmodifiers 'alt+Left'"));
        // A single verb is a batch of one that does not stop to take a picture:
        // the pane grabs its own frame afterwards.
        assert!(!back.contains("IMAGE="));

        let closed = drive_command("close", &json!({})).unwrap();
        assert!(closed.contains("pkill -x chromium"));
    }

    #[test]
    fn the_pointer_verbs_go_through_the_same_compiler_as_the_agents_batch() {
        let click =
            drive_command("click", &json!({ "x": 12, "y": 34, "button": "right" })).unwrap();
        assert!(click.contains("mousemove --sync 12 34"));
        assert!(click.contains("mousedown 3"));

        let typed = drive_command("type", &json!({ "text": "hello" })).unwrap();
        assert!(typed.contains("xclip -selection clipboard -i"));

        let scrolled = drive_command("scroll", &json!({ "direction": "up", "amount": 5 })).unwrap();
        assert!(scrolled.contains("xdotool click --repeat 5 4"));
    }

    /// A click with no coordinates is the model's or the pane's mistake, and it
    /// has to come back as a sentence rather than as a shell line with an empty
    /// argument in it.
    #[test]
    fn a_verb_that_cannot_be_compiled_says_why() {
        assert!(drive_command("click", &json!({})).is_err());
        let unknown = drive_command("teleport", &json!({})).unwrap_err();
        assert_eq!(unknown, "Unknown desktop action: teleport");
    }

    #[test]
    fn both_streams_are_reported_and_an_empty_one_is_left_out() {
        assert_eq!(said_by("out", "err"), "out\nerr");
        assert_eq!(said_by("", "err"), "err");
        assert_eq!(said_by("out\n", ""), "out");
        assert_eq!(said_by("  ", "\n"), "");
    }

    #[test]
    fn a_field_that_is_not_there_reads_as_empty_rather_than_panicking() {
        let record = json!({ "id": "box" });
        assert_eq!(field(&record, "id"), "box");
        assert_eq!(field(&record, "handle"), "");
        assert_eq!(field(&Value::Null, "handle"), "");
    }
}

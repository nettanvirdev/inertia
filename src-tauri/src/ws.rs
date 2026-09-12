//! The workspace command surface.
//!
//! These are a literal mirror of the `ws:*` IPC channels the renderer was
//! written against. The renderer names collections and documents; it never
//! names paths, which is what keeps `inertia_store::layout` the only place that
//! knows the folder's shape.
//!
//! Each command returns a value or an error string. The JavaScript bridge wraps
//! that into the `{ ok, data } | { ok, error }` envelope the renderer unwraps,
//! because a renderer that has to tell "empty list" from "the folder is gone"
//! needs the failure to be a value it can read rather than a thrown string.

use std::path::{Path, PathBuf};

use inertia_store::collections;
use inertia_store::layout::{Collection, Directory, Document, Layout, DIRECTORY_INFO, MANIFEST};
use inertia_store::{fsx, secrets};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;

/// Bumped when the on-disk shape changes in a way a migration must notice.
pub const WORKSPACE_VERSION: u32 = 1;

/// What the transcript will show as a picture, and how big it may be.
const IMAGE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
];
const MAX_IMAGE_BYTES: u64 = 12 * 1024 * 1024;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/* -- the pointer out of the app and into the folder ---------------------- */

/// Where the workspace folder is.
///
/// This one pointer is the only thing Inertia keeps outside the workspace, and
/// it has to be: something must survive a fresh install to say where everything
/// else went. It is cheap to delete - doing so just sends the app back through
/// first-run setup with the folder itself untouched.
fn pointer_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(err)?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir.join("workspace.json"))
}

fn stored_root(app: &AppHandle) -> Option<PathBuf> {
    let path = pointer_file(app).ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let root = value.get("root")?.as_str()?;
    (!root.trim().is_empty()).then(|| PathBuf::from(root))
}

fn store_root(app: &AppHandle, root: &Path) -> Result<(), String> {
    let body = json!({
        "root": root.to_string_lossy(),
        // Recorded for support: which install pointed here, and when.
        "appVersion": app.package_info().version.to_string(),
        "updatedAt": now(),
    });
    std::fs::write(
        pointer_file(app)?,
        serde_json::to_string_pretty(&body).map_err(err)?,
    )
    .map_err(err)
}

fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// A sensible place to offer on first run: Documents/Inertia.
fn suggested_root(app: &AppHandle) -> String {
    let base = app
        .path()
        .document_dir()
        .or_else(|_| app.path().home_dir())
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("Inertia").to_string_lossy().to_string()
}

/// The layout for the open workspace, opening it from the stored pointer if
/// this is the first call since launch.
///
/// Lazy rather than done at startup because the pointer may name a folder that
/// has been unplugged, and the honest place to report that is the screen that
/// asked for data, not a panic before the first frame.
fn layout(app: &AppHandle, state: &AppState) -> Result<Layout, String> {
    if let Ok(workspace) = state.workspace() {
        return Ok(workspace.layout.clone());
    }
    let root = stored_root(app).ok_or("No workspace folder is configured yet.")?;
    state.open_workspace(root.clone())?;
    Ok(Layout::new(root))
}

/// Tells every window a record changed.
///
/// The commands below announce their own writes rather than relying on a folder
/// watcher. A pane's own save must not depend on the filesystem agreeing, two
/// hundred milliseconds later, on a mechanism that reports nothing at all on a
/// network share.
fn announce(app: &AppHandle, payload: Value) {
    let _ = app.emit("workspace:changed", payload);
}

/* -- status, layout, tree ------------------------------------------------ */

fn read_manifest(root: &Path) -> Option<Value> {
    let (value, outcome) = fsx::read_json::<Value>(&root.join(MANIFEST));
    matches!(outcome, fsx::ReadOutcome::Loaded).then_some(value)
}

fn is_manifest(manifest: Option<&Value>) -> bool {
    manifest
        .and_then(|m| m.get("app"))
        .and_then(Value::as_str)
        .is_some_and(|app| app == "inertia")
}

fn version_of(manifest: Option<&Value>) -> u32 {
    manifest
        .and_then(|m| m.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32
}

/// The one call the app makes at startup, and after any change of folder.
#[tauri::command]
pub fn ws_status(app: AppHandle) -> Value {
    let suggested = suggested_root(&app);
    let Some(root) = stored_root(&app) else {
        return json!({
            "configured": false,
            "root": Value::Null,
            "suggested": suggested,
            "manifest": Value::Null,
        });
    };

    let present = root.is_dir();
    let manifest = read_manifest(&root);
    json!({
        "configured": true,
        "root": root.to_string_lossy(),
        "suggested": suggested,
        "manifest": manifest.clone().unwrap_or(Value::Null),
        // A folder that has been moved or unmounted since it was chosen: the
        // app must say so rather than quietly writing a fresh tree at the old
        // path.
        "missing": !present,
        // Present but no longer ours - a user emptied it, or picked a synced
        // folder that has not come down yet.
        "stale": present && !is_manifest(manifest.as_ref()),
        // Written by a newer Inertia than this one. Reading it with the old
        // rules is how records get quietly rewritten into a shape the newer app
        // then cannot read.
        "ahead": present && version_of(manifest.as_ref()) > WORKSPACE_VERSION,
        "formatVersion": version_of(manifest.as_ref()),
        "appFormatVersion": WORKSPACE_VERSION,
    })
}

/// The layout itself, so the UI can label folders without duplicating them.
#[tauri::command]
pub fn ws_layout(app: AppHandle) -> Value {
    json!({
        "version": WORKSPACE_VERSION,
        "appVersion": app.package_info().version.to_string(),
        "directories": DIRECTORY_INFO,
        "collections": Collection::ALL.iter().map(|c| c.key()).collect::<Vec<_>>(),
        "documents": Document::ALL.iter().map(|d| d.key()).collect::<Vec<_>>(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TreeRow {
    #[serde(flatten)]
    directory: Directory,
    entries: usize,
    bytes: u64,
    exists: bool,
}

/// How big a directory is, counting only what is directly inside it plus its
/// descendants. Errors read as zero: a folder we cannot stat is reported as
/// empty rather than taking the whole pane down.
fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&entry.path()),
            Ok(_) => entry.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Directory-by-directory counts and sizes, for the workspace settings pane.
#[tauri::command]
pub fn ws_tree(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    let rows: Vec<TreeRow> = DIRECTORY_INFO
        .iter()
        .map(|directory| {
            let full = layout.root().join(directory.path);
            let files = fsx::list_dir(&full, fsx::ListOptions::default());
            let dirs = fsx::list_dir(
                &full,
                fsx::ListOptions {
                    only_dirs: true,
                    ..Default::default()
                },
            );
            TreeRow {
                directory: *directory,
                entries: files.len() + dirs.len(),
                bytes: dir_size(&full),
                exists: full.is_dir(),
            }
        })
        .collect();

    Ok(json!({ "root": layout.root().to_string_lossy(), "directories": rows }))
}

/* -- choosing a folder --------------------------------------------------- */

/// Opens a folder picker and waits for it.
///
/// `blocking_pick_folder` would deadlock the command thread on some platforms,
/// so the async form is driven by a channel instead.
async fn pick_folder(app: &AppHandle, title: &str, start_at: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;

    let mut builder = app.dialog().file().set_title(title);
    if let Some(start) = start_at.filter(|s| !s.trim().is_empty()) {
        builder = builder.set_directory(PathBuf::from(start));
    }

    let (tx, rx) = tokio::sync::oneshot::channel();
    builder.pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    rx.await
        .ok()
        .flatten()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().to_string())
}

/// Pick a folder. `null` when the user cancels - not an error.
#[tauri::command]
pub async fn ws_browse(app: AppHandle, start_at: Option<String>) -> Option<String> {
    let start = start_at.or_else(|| Some(suggested_root(&app)));
    pick_folder(&app, "Choose your Inertia folder", start).await
}

/// Pick any folder, for anything.
///
/// Separate from [`ws_browse`], whose dialog says "Choose your Inertia folder" -
/// a title that is wrong and slightly alarming when what is being chosen is a
/// project to work in. Same dialog, different sentence.
#[tauri::command]
pub async fn ws_choose_folder(app: AppHandle, options: Option<Value>) -> Option<String> {
    let field = |key: &str| {
        options
            .as_ref()
            .and_then(|o| o.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let title = field("title").unwrap_or_else(|| "Choose a folder".to_string());
    pick_folder(&app, &title, field("startAt")).await
}

/// What is in a folder, and what using it would mean.
#[tauri::command]
pub fn ws_inspect(dir: String) -> Value {
    let target = PathBuf::from(&dir);
    let target = std::path::absolute(&target).unwrap_or(target);
    let exists = target.exists();

    if exists && !target.is_dir() {
        return json!({
            "path": target.to_string_lossy(),
            "exists": true,
            "isDirectory": false,
            "isEmpty": false,
            "hasWorkspace": false,
            "manifest": Value::Null,
            "entryCount": 0,
            "writable": false,
            "action": "blocked",
            "error": "That path is a file, not a folder.",
        });
    }

    let mut entry_count = 0usize;
    if exists {
        let files = fsx::list_dir(&target, fsx::ListOptions::default());
        let dirs = fsx::list_dir(
            &target,
            fsx::ListOptions {
                only_dirs: true,
                ..Default::default()
            },
        );
        entry_count = files.len() + dirs.len();

        let manifest = read_manifest(&target);
        if is_manifest(manifest.as_ref()) {
            return json!({
                "path": target.to_string_lossy(),
                "exists": true,
                "isDirectory": true,
                "isEmpty": entry_count == 0,
                "hasWorkspace": true,
                "manifest": manifest,
                "entryCount": entry_count,
                "writable": true,
                "action": "adopt",
                "error": Value::Null,
            });
        }
    }

    // A folder with unrelated files in it is not refused - people keep their
    // notes in Documents/Inertia already - but the UI needs to say so.
    let action = if exists && entry_count > 0 {
        "create-in-used"
    } else {
        "create"
    };
    json!({
        "path": target.to_string_lossy(),
        "exists": exists,
        "isDirectory": exists,
        "isEmpty": entry_count == 0,
        "hasWorkspace": false,
        "manifest": Value::Null,
        "entryCount": entry_count,
        "writable": true,
        "action": action,
        "error": Value::Null,
    })
}

/// Writes the manifest and the README, and creates every directory.
fn scaffold(root: &Path) -> Result<Value, String> {
    let layout = Layout::new(root);
    layout.scaffold().map_err(err)?;

    let existing = read_manifest(root);
    let manifest = if is_manifest(existing.as_ref()) {
        let mut manifest = existing.unwrap_or(json!({}));
        if let Value::Object(map) = &mut manifest {
            map.insert("openedAt".into(), json!(now()));
        }
        manifest
    } else {
        json!({
            "app": "inertia",
            "version": WORKSPACE_VERSION,
            "id": uuid::Uuid::now_v7().to_string(),
            "createdAt": now(),
            "openedAt": now(),
        })
    };

    fsx::write_json(&layout.manifest(), &manifest).map_err(err)?;
    let readme = root.join(inertia_store::layout::README);
    if !readme.exists() {
        fsx::write_text(&readme, README_BODY).map_err(err)?;
    }
    Ok(manifest)
}

/// Written once, for whoever opens the folder without the app.
const README_BODY: &str = r#"# Inertia workspace

Everything Inertia knows lives in this folder: settings, agents, skills,
plugins, conversations, run history, memory and secrets.

Back it up by copying the folder. Restore it by pointing a fresh install at the
copy - Inertia will recognise the manifest and adopt it instead of starting over.

Do not rename the directories. They are the addresses the app resolves by name.

- settings/     preferences, appearance, model providers
- agents/       one file per agent
- skills/       one folder per skill, each with a SKILL.md
- plugins/      mcp/, openapi/ and composio/ tool sources
- conversations/ threads and their transcripts
- history/      execution and activity logs
- memory/       long-lived facts
- secrets/      API keys and tokens, currently stored in the clear
- files/        attachments and generated output
- backups/      snapshots of this folder
"#;

/// Create or adopt, then remember. Both paths run the scaffold.
#[tauri::command]
pub fn ws_configure(
    app: AppHandle,
    state: State<'_, AppState>,
    dir: String,
) -> Result<Value, String> {
    let report = ws_inspect(dir);
    if report["action"] == "blocked" {
        return Err(report["error"]
            .as_str()
            .unwrap_or("That folder cannot be used.")
            .to_string());
    }
    let root = PathBuf::from(report["path"].as_str().unwrap_or_default());
    let manifest = scaffold(&root)?;
    store_root(&app, &root)?;
    state.open_workspace(root)?;

    let mut status = ws_status(app.clone());
    if let Value::Object(map) = &mut status {
        map.insert("adopted".into(), json!(report["action"] == "adopt"));
        map.insert("manifest".into(), manifest);
    }
    announce(&app, status.clone());
    Ok(status)
}

/// Forget the folder. The folder itself is never touched.
#[tauri::command]
pub fn ws_reset(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    let _ = std::fs::remove_file(pointer_file(&app)?);
    state.close_workspace();
    let status = ws_status(app.clone());
    announce(&app, status.clone());
    Ok(status)
}

/// Empty the workspace and start it again.
///
/// Every managed directory goes, the manifest with it, and the folder is then
/// scaffolded fresh. The folder itself and the pointer to it survive, because
/// the user asked to delete their data, not to be sent back through setup.
///
/// Anything they put in the folder that Inertia does not manage is left alone.
/// It is their folder.
#[tauri::command]
pub fn ws_wipe(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    for dir in inertia_store::layout::DIRECTORIES {
        fsx::remove_dir_all(&layout.root().join(dir)).map_err(err)?;
    }
    fsx::remove(&layout.manifest()).map_err(err)?;
    let manifest = scaffold(layout.root())?;
    announce(&app, json!({ "collection": "*", "op": "wipe" }));
    Ok(json!({ "wiped": true, "manifest": manifest }))
}

/* -- files inside the folder --------------------------------------------- */

/// What is actually in a folder inside the workspace.
///
/// Added for skills, where the folder IS the unit: a skill is a SKILL.md plus
/// the scripts and reference files beside it, and a screen that can only show
/// the one file it wrote misrepresents what the user has.
#[tauri::command]
pub fn ws_list_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: Option<String>,
) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    let dir = fsx::resolve_inside(layout.root(), rel_path.unwrap_or_default()).map_err(err)?;

    let dirs = fsx::list_dir(
        &dir,
        fsx::ListOptions {
            only_dirs: true,
            ..Default::default()
        },
    );
    let files = fsx::list_dir(&dir, fsx::ListOptions::default());

    let mut rows: Vec<Value> = dirs
        .into_iter()
        .map(|name| json!({ "name": name, "isDir": true }))
        .collect();

    // Sizes are worth the extra stat: the thing a person wants to spot in a
    // skill folder is the reference document that turned out to be four
    // megabytes, and a bare list of names cannot show that.
    rows.extend(files.into_iter().map(|name| match std::fs::metadata(dir.join(&name)) {
        Ok(meta) => json!({
            "name": name,
            "isDir": false,
            "size": meta.len(),
            "mtimeMs": meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64),
        }),
        Err(_) => json!({ "name": name, "isDir": false, "size": Value::Null, "mtimeMs": Value::Null }),
    }));

    Ok(Value::Array(rows))
}

#[tauri::command]
pub fn ws_reveal(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: Option<String>,
) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let layout = layout(&app, &state)?;
    let target = fsx::resolve_inside(layout.root(), rel_path.unwrap_or_default()).map_err(err)?;
    let path = target.to_string_lossy().to_string();
    app.opener().open_path(path.clone(), None::<&str>).map_err(err)?;
    Ok(path)
}

#[tauri::command]
pub fn ws_file_read(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: String,
) -> Result<Option<String>, String> {
    let layout = layout(&app, &state)?;
    let target = fsx::resolve_inside(layout.root(), rel_path).map_err(err)?;
    Ok(fsx::read_text(&target))
}

#[tauri::command]
pub fn ws_file_write(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: String,
    contents: Option<String>,
) -> Result<String, String> {
    let layout = layout(&app, &state)?;
    let target = fsx::resolve_inside(layout.root(), &rel_path).map_err(err)?;
    fsx::write_text(&target, &contents.unwrap_or_default()).map_err(err)?;
    Ok(rel_path)
}

/// The same two, for files that are not text.
///
/// Base64 across the bridge rather than raw bytes, because a byte array becomes
/// a plain object of numbered keys in JSON and the renderer would have to
/// reassemble it. The avatar is the reason this exists: keeping it in
/// `settings/avatar.png` means the user can replace their photo by dropping a
/// file into the folder, which they cannot do with a data URL wedged in JSON.
#[tauri::command]
pub fn ws_file_read_bytes(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: String,
) -> Result<Option<String>, String> {
    use base64::Engine;
    let layout = layout(&app, &state)?;
    let target = fsx::resolve_inside(layout.root(), rel_path).map_err(err)?;
    Ok(std::fs::read(target)
        .ok()
        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[tauri::command]
pub fn ws_file_write_bytes(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: String,
    base64_data: Option<String>,
) -> Result<String, String> {
    use base64::Engine;
    let layout = layout(&app, &state)?;
    let target = fsx::resolve_inside(layout.root(), &rel_path).map_err(err)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data.unwrap_or_default())
        .map_err(|_| "That attachment is not valid base64.".to_string())?;
    if let Some(parent) = target.parent() {
        fsx::ensure_dir(parent).map_err(err)?;
    }
    std::fs::write(&target, bytes).map_err(err)?;
    Ok(rel_path)
}

/// An image on this machine, as a data URL, for the transcript to show.
///
/// The path may be absolute, because the interesting images are screenshots and
/// mock-ups in the project folder rather than in the Inertia folder. Three
/// things keep that narrow: it must be an image extension, it must be under the
/// cap, and nothing is sent anywhere - the bytes go to the window on this
/// machine, which is the same file the person could open themselves. A path
/// that is missing or is not an image comes back as null and the transcript
/// falls back to showing a link.
#[tauri::command]
pub fn ws_file_read_image(
    app: AppHandle,
    state: State<'_, AppState>,
    file_path: String,
) -> Result<Option<Value>, String> {
    use base64::Engine;

    let extension = Path::new(&file_path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let Some((_, mime)) = IMAGE_TYPES.iter().find(|(ext, _)| *ext == extension) else {
        return Ok(None);
    };

    let candidate = PathBuf::from(&file_path);
    let target = if candidate.is_absolute() {
        candidate
    } else {
        let layout = layout(&app, &state)?;
        fsx::resolve_inside(layout.root(), &file_path).map_err(err)?
    };

    let Ok(meta) = std::fs::metadata(&target) else {
        return Ok(None);
    };
    if !meta.is_file() || meta.len() > MAX_IMAGE_BYTES {
        return Ok(None);
    }
    let Ok(bytes) = std::fs::read(&target) else {
        return Ok(None);
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(Some(json!({
        "url": format!("data:{mime};base64,{encoded}"),
        "bytes": meta.len(),
        "path": target.to_string_lossy(),
    })))
}

#[tauri::command]
pub fn ws_file_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    rel_path: String,
) -> Result<String, String> {
    let layout = layout(&app, &state)?;
    let target = fsx::resolve_inside(layout.root(), &rel_path).map_err(err)?;
    if target.is_dir() {
        fsx::remove_dir_all(&target).map_err(err)?;
    } else {
        fsx::remove(&target).map_err(err)?;
    }
    Ok(rel_path)
}

/* -- collections --------------------------------------------------------- */

fn resolve(name: &str) -> Result<Collection, String> {
    collections::collection(name).map_err(err)
}

/// Memories are the one collection that is not one folder.
///
/// What is about the person lives in the workspace; what is about a project
/// lives in that project's own `.inertia/memory/`. The screen asks for a
/// collection by name and has nowhere to put a folder, so these five commands
/// route it through the store, which knows about both. Every other collection
/// goes straight to the workspace layout as before.
fn is_memory(collection: Collection) -> bool {
    matches!(collection, Collection::Memories)
}

#[tauri::command]
pub fn ws_list(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<Vec<Value>, String> {
    let collection = resolve(&name)?;
    if is_memory(collection) {
        return Ok(state.memory()?.list());
    }
    let layout = layout(&app, &state)?;
    Ok(collections::list(&layout, collection))
}

#[tauri::command]
pub fn ws_get(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    id: String,
) -> Result<Option<Value>, String> {
    let layout = layout(&app, &state)?;
    collections::get(&layout, resolve(&name)?, &id).map_err(err)
}

#[tauri::command]
pub fn ws_put(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    record: Value,
) -> Result<Value, String> {
    let collection = resolve(&name)?;
    let saved = if is_memory(collection) {
        // `merge: false` - a person typing a memory into the screen means to
        // have that memory, and silently folding it into one that reads
        // similarly would look like the save failing.
        state.memory()?.remember(&record, false)?
    } else {
        let layout = layout(&app, &state)?;
        collections::put(&layout, collection, record).map_err(err)?
    };
    announce(
        &app,
        json!({ "collection": name, "id": saved["id"], "op": "put" }),
    );
    Ok(saved)
}

#[tauri::command]
pub fn ws_patch(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    id: String,
    changes: Value,
) -> Result<Value, String> {
    let collection = resolve(&name)?;
    let saved = if is_memory(collection) {
        state.memory()?.update(&id, changes)?
    } else {
        let layout = layout(&app, &state)?;
        collections::patch(&layout, collection, &id, changes).map_err(err)?
    };
    announce(&app, json!({ "collection": name, "id": id, "op": "patch" }));
    Ok(saved)
}

#[tauri::command]
pub fn ws_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    id: String,
) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    let collection = resolve(&name)?;

    // A protected record is refused here as well as in the tools, because the
    // window is a second door to the same folder: a stale button, a shortcut,
    // or a screen that has not been told about protection all arrive through
    // this one call.
    if let Some(existing) = collections::get(&layout, collection, &id).map_err(err)? {
        if let Some(reason) = protected_reason(&existing) {
            return Err(reason);
        }
    }

    // Written down, every time, and written after the fact: a record that
    // vanishes from the folder with no line in the log is a record nobody can
    // explain afterwards, and a line that says "removed" for a removal Windows
    // then refused is worse than no line - it sends an investigation the wrong
    // way.
    let removal = if is_memory(collection) {
        state.memory()?.forget(&id)
    } else {
        collections::remove(&layout, collection, &id).map_err(err)
    };
    if let Err(error) = removal {
        tracing::warn!(%name, %id, %error, "could not remove a record");
        return Err(error);
    }
    tracing::info!(%name, %id, "removed a record");
    announce(&app, json!({ "collection": name, "id": id, "op": "remove" }));
    Ok(json!({ "id": id, "removed": true }))
}

/// Why this record may not be deleted, if it may not be.
///
/// Records the app itself seeded carry `protected: true`; removing one leaves a
/// workspace that looks set up but has lost the agent every other default
/// refers to.
fn protected_reason(record: &Value) -> Option<String> {
    record
        .get("protected")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        .then(|| {
            let name = record
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| record.get("id").and_then(Value::as_str))
                .unwrap_or("That record");
            format!("{name} is part of Inertia itself and cannot be deleted.")
        })
}

#[tauri::command]
pub fn ws_rename(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    id: String,
    next_id: String,
) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    let saved = collections::rename(&layout, resolve(&name)?, &id, &next_id).map_err(err)?;
    announce(&app, json!({ "collection": name, "id": next_id, "op": "rename" }));
    Ok(saved)
}

/* -- documents ----------------------------------------------------------- */

fn document(key: &str) -> Result<Document, String> {
    Document::from_key(key).ok_or_else(|| format!("Unknown document: {key}"))
}

#[tauri::command]
pub fn ws_doc_get(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
    fallback: Option<Value>,
) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    Ok(collections::read_document(
        &layout,
        document(&key)?,
        fallback.unwrap_or_else(|| json!({})),
    ))
}

/// Write a document, and say so.
///
/// Every collection write above announces; this one must too. Providers are a
/// document, and when this was silent the symptom was a provider that had
/// plainly saved - the settings pane showed it, because that pane keeps its own
/// copy - and yet was missing from every model picker until the app restarted.
#[tauri::command]
pub fn ws_doc_set(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
    value: Value,
) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    collections::write_document(&layout, document(&key)?, &value).map_err(err)?;
    announce(&app, json!({ "document": key, "op": "set" }));
    Ok(value)
}

/* -- secrets ------------------------------------------------------------- */

#[tauri::command]
pub fn ws_secret_list(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let layout = layout(&app, &state)?;
    Ok(secrets::list(&layout))
}

#[tauri::command]
pub fn ws_secret_get(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<String>, String> {
    let layout = layout(&app, &state)?;
    secrets::get(&layout, &name).map_err(err)
}

#[tauri::command]
pub fn ws_secret_set(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    value: String,
    label: Option<String>,
) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    let saved = secrets::set(&layout, &name, &value, &label.unwrap_or_default()).map_err(err)?;
    announce(&app, json!({ "document": "secrets", "op": "set" }));
    Ok(saved)
}

#[tauri::command]
pub fn ws_secret_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<Value, String> {
    let layout = layout(&app, &state)?;
    let done = secrets::remove(&layout, &name).map_err(err)?;
    announce(&app, json!({ "document": "secrets", "op": "set" }));
    Ok(done)
}

//! Where memories are kept.
//!
//! Two places, and which one a record goes to is decided by the record's own
//! scope:
//!
//!   · **Global** - about the person - lives in the workspace, at
//!     `<workspace>/memory/`. It is theirs, it follows them between projects,
//!     and it has no business in a repository.
//!   · **Project** - about one folder - lives in that folder, at
//!     `<project>/.inertia/memory/`, beside `.inertia/rules/` and
//!     `.inertia/hooks.json`. It travels with the repository, so a teammate who
//!     clones it starts out knowing what the last person worked out, and a
//!     project deleted from disk takes its memories with it instead of leaving
//!     them behind in a workspace nobody prunes.
//!
//! Both are the same on-disk format - one JSON file per record, the collection
//! machinery in `inertia-store` - so the only thing that differs is the root.
//! That is why there is no second reader: a `Layout` pointed at `.inertia` is a
//! workspace layout as far as `collections` is concerned.
//!
//! Records written before this existed are still in the workspace folder, and
//! are still read from it. A project memory found there applies exactly as it
//! did - it says which folder it belongs to, and that has always been what
//! scopes it. Nothing is migrated: moving a user's files to tidy up an
//! implementation detail is not a trade worth making, and the old location goes
//! on working for as long as there is anything in it.
//!
//! ## The seam
//!
//! Those folders are one implementation of `Backend`, not the only one. The
//! other is an MCP memory server (`crate::mcp`), which is what lets Inertia and
//! another coding agent pointed at the same server know the same things: no
//! copy, no sync, one store.
//!
//! `Store` is what everything else holds, and it does not change shape when the
//! storage does. It owns the local folders unconditionally, because they are
//! what a remote backend falls back to: a server that is unreachable, offers no
//! tool this app recognises, or answers with something unreadable must cost a
//! turn nothing at all. It degrades to the workspace's own memories and keeps a
//! sentence saying why, which the settings screen shows until the server works
//! again. A memory backend is an improvement to a conversation, never a
//! precondition for one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use inertia_store::collections;
use inertia_store::layout::Collection;
use inertia_store::Layout;

use crate::record;

/// The folder inside a project that holds its own configuration.
pub const PROJECT_DIR: &str = ".inertia";

/// Where a project's memories live.
pub fn project_layout(folder: &Path) -> Layout {
    Layout::new(folder.join(PROJECT_DIR))
}

/// What a backend can actually do.
///
/// Backends genuinely differ - some memory servers cannot edit a stored memory,
/// some cannot delete one - and a screen that greys out a button it cannot
/// honour is better than one that offers it and fails at the click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub list: bool,
    pub remember: bool,
    pub update: bool,
    pub forget: bool,
}

impl Capabilities {
    /// Everything, which is what a folder of JSON files can do.
    pub const fn all() -> Self {
        Self { list: true, remember: true, update: true, forget: true }
    }
}

/// Storing a record and finding it again, wherever that happens.
///
/// Deliberately the narrow half. When to capture, what is worth keeping, what
/// reaches the prompt and within what budget all stay above this line and do not
/// change because the storage changed.
///
/// Synchronous, because `Store` is and every caller of it is. A backend that
/// talks to a server does the waiting behind this trait rather than turning
/// every call site in the app async for the sake of a setting most people never
/// change.
pub trait Backend: std::fmt::Debug + Send + Sync {
    /// What to call this backend on screen and in a fallback note.
    fn label(&self) -> String;

    fn capabilities(&self) -> Capabilities;

    /// Every memory this backend holds. An error here is not fatal: `Store`
    /// falls back to the local folders and reports the message.
    fn list(&self) -> Result<Vec<Value>, String>;

    fn remember(&self, input: &Value, merge: bool) -> Result<Value, String>;

    fn update(&self, id: &str, changes: Value) -> Result<Value, String>;

    fn forget(&self, id: &str) -> Result<(), String>;

    /// Say that these memories were useful. Bookkeeping, so it cannot fail: a
    /// backend with nowhere to put a use count simply does nothing.
    fn touch(&self, ids: &[String]);
}

/// Memories in folders on this machine.
///
/// The default, the fallback, and the only backend that needs nothing: records
/// are JSON files the Memory screen renders and the person can open in their own
/// editor. One store, two doors.
#[derive(Debug, Clone)]
pub struct Local {
    workspace: Layout,
    /// The project in front of the person right now, when there is one. Only
    /// this project's memories are ever read: a memory about another folder is
    /// not merely irrelevant here, it must not be reachable.
    project: Option<PathBuf>,
}

impl Local {
    pub fn new(workspace: Layout, project: Option<PathBuf>) -> Self {
        Self { workspace, project }
    }

    /// Where a record with this scope and folder belongs.
    ///
    /// A project memory with no folder, or one for a project that is not the
    /// one open, falls back to the workspace. Refusing to write it would lose
    /// the memory; writing it into whichever project happens to be open would
    /// file it under the wrong one, which is worse.
    fn layout_for(&self, scope: &str, folder: &str) -> Layout {
        if scope != "project" || folder.is_empty() {
            return self.workspace.clone();
        }
        match &self.project {
            Some(open) if record::same_folder(&open.to_string_lossy(), folder) => {
                project_layout(open)
            }
            _ => self.workspace.clone(),
        }
    }

    /// Every memory that could apply here, from both locations.
    ///
    /// The project's own folder is read second, and a record found in both wins
    /// from the project: that is the case where somebody copied a memory into a
    /// repository and the workspace still holds the original.
    pub fn list(&self) -> Vec<Value> {
        let mut rows = collections::list(&self.workspace, Collection::Memories);

        if let Some(folder) = &self.project {
            let here = collections::list(&project_layout(folder), Collection::Memories);
            let owned = folder.to_string_lossy().to_string();
            for mut row in here {
                // Stamped on read rather than on write. A repository cloned to
                // a different path would otherwise carry the path it was
                // written at, and every one of its memories would be invisible
                // to the person who cloned it.
                if let Some(map) = row.as_object_mut() {
                    map.insert("scope".into(), Value::String("project".into()));
                    map.insert("folder".into(), Value::String(owned.clone()));
                }
                let id = record::text(&row, "id");
                rows.retain(|held| record::text(held, "id") != id);
                rows.push(row);
            }
        }
        rows
    }

    /// The memories in force for a turn in this folder.
    pub fn in_scope(&self, rows: &[Value]) -> Vec<Value> {
        let folder = self.project.as_ref().map(|p| p.to_string_lossy().to_string());
        record::approved(record::applicable(rows, folder.as_deref()))
            .into_iter()
            .cloned()
            .collect()
    }

    /// Write one memory, merging it into the one it repeats.
    ///
    /// Both writers can produce a repeat: a background pass that ignored the
    /// list of what is already known, and an agent that saves something
    /// mid-conversation which it was already told at the top of the same
    /// conversation. Neither is a bug worth chasing at the source, because both
    /// are model judgement - so the store refuses to hold two copies instead.
    ///
    /// The newer text wins and the older record keeps its id, its pin and its
    /// history. That makes a repeat behave as a correction, which is what a
    /// repeat with different wording usually is.
    pub fn remember(&self, input: &Value, merge: bool) -> Result<Value, String> {
        if !record::is_storable(input) {
            return Err("A memory needs a title and a body, and may not carry a secret.".into());
        }

        let scope = record::scope_of(input).to_string();
        let folder = record::text(input, "folder");
        let layout = self.layout_for(&scope, &folder);

        if merge {
            let held = self.list();
            if let Some(twin) = record::find_duplicate(&held, input) {
                let id = record::text(twin, "id");
                let mut updated = record::merged(twin, input);
                stamp_used(&mut updated);
                // Written back where the twin already is, which may not be
                // where a new record with this scope would go: a memory that
                // predates the project folder stays where the user can find it.
                let home = self.layout_for(record::scope_of(twin), &record::text(twin, "folder"));
                return collections::patch(&home, Collection::Memories, &id, updated)
                    .map_err(|e| e.to_string());
            }
        }

        let mut fresh = input.clone();
        stamp_used(&mut fresh);
        strip_location(&mut fresh, &scope);
        collections::put(&layout, Collection::Memories, fresh).map_err(|e| e.to_string())
    }

    /// Change one memory, wherever it is.
    pub fn update(&self, id: &str, changes: Value) -> Result<Value, String> {
        let layout = self.home_of(id)?;
        collections::patch(&layout, Collection::Memories, id, changes).map_err(|e| e.to_string())
    }

    pub fn forget(&self, id: &str) -> Result<(), String> {
        let layout = self.home_of(id)?;
        collections::remove(&layout, Collection::Memories, id).map_err(|e| e.to_string())
    }

    /// Say that these memories were useful.
    ///
    /// `useCount` and `lastUsedAt` are what the Memory screen sorts and ages
    /// records by. They are bookkeeping, so a failed write is not worth failing
    /// a recall over - the memory was still found, and the count being one short
    /// is not a thing anyone can act on.
    pub fn touch(&self, ids: &[String]) {
        let now = now_iso();
        for id in ids {
            let Ok(layout) = self.home_of(id) else { continue };
            let used = collections::get(&layout, Collection::Memories, id)
                .ok()
                .flatten()
                .and_then(|row| row.get("useCount").and_then(Value::as_i64))
                .unwrap_or(0);
            let _ = collections::patch(
                &layout,
                Collection::Memories,
                id,
                json!({ "useCount": used + 1, "lastUsedAt": now }),
            );
        }
    }

    /// Which of the two locations holds this record.
    fn home_of(&self, id: &str) -> Result<Layout, String> {
        if let Some(folder) = &self.project {
            let layout = project_layout(folder);
            if collections::get(&layout, Collection::Memories, id)
                .ok()
                .flatten()
                .is_some()
            {
                return Ok(layout);
            }
        }
        Ok(self.workspace.clone())
    }

    /// The project's handover note, if one has been written.
    pub fn note(&self, rows: &[Value]) -> Option<Value> {
        let folder = self.project.as_ref()?.to_string_lossy().to_string();
        rows.iter()
            .find(|row| {
                record::text(row, "kind") == "handover"
                    && record::same_folder(&record::text(row, "folder"), &folder)
            })
            .cloned()
    }

    pub fn project(&self) -> Option<&Path> {
        self.project.as_deref()
    }
}

impl Backend for Local {
    fn label(&self) -> String {
        "This workspace".into()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::all()
    }

    fn list(&self) -> Result<Vec<Value>, String> {
        Ok(Local::list(self))
    }

    fn remember(&self, input: &Value, merge: bool) -> Result<Value, String> {
        Local::remember(self, input, merge)
    }

    fn update(&self, id: &str, changes: Value) -> Result<Value, String> {
        Local::update(self, id, changes)
    }

    fn forget(&self, id: &str) -> Result<(), String> {
        Local::forget(self, id)
    }

    fn touch(&self, ids: &[String]) {
        Local::touch(self, ids);
    }
}

/// Reading and writing memories, whichever backend is in force.
///
/// The local folders are held unconditionally even when a server is chosen,
/// because they are what the server falls back to. Every operation tries the
/// remote first and, when it will not answer, does the same thing locally and
/// records one sentence saying why - so a misconfigured or not-yet-started
/// server costs a turn nothing and is still visible to the person who
/// configured it.
#[derive(Debug, Clone)]
pub struct Store {
    local: Local,
    remote: Option<Arc<dyn Backend>>,
    /// Why the chosen server is not the one being used, when it is not. Shared
    /// rather than owned so a clone of the store reports the same trouble: the
    /// turn path and the settings screen hold different clones of one store.
    problem: Arc<RwLock<Option<String>>>,
}

impl Store {
    /// The default: memories in this workspace and in the open project.
    pub fn new(workspace: Layout, project: Option<PathBuf>) -> Self {
        Self {
            local: Local::new(workspace, project),
            remote: None,
            problem: Arc::new(RwLock::new(None)),
        }
    }

    /// The same store, keeping memories on a server instead.
    #[must_use]
    pub fn with_remote(mut self, remote: Arc<dyn Backend>) -> Self {
        self.remote = Some(remote);
        self
    }

    /// The same store, local, carrying why the server it was pointed at is not
    /// in use.
    ///
    /// This is the shape the caller reaches for when building the remote itself
    /// failed - a server that is not connected yet, or offers no tool this app
    /// recognises. Refusing to open the workspace over it would mean a typo in a
    /// server name stopping every conversation.
    #[must_use]
    pub fn fell_back(self, why: impl Into<String>) -> Self {
        *self.problem.write() = Some(why.into());
        self
    }

    /// What is wrong with the chosen backend, in a sentence worth showing.
    ///
    /// Kept until it works again rather than cleared at the end of a turn: a
    /// message that clears itself on the next turn is one nobody ever reads.
    pub fn problem(&self) -> Option<String> {
        self.problem.read().clone()
    }

    /// What the backend in force can do, for a screen deciding what to offer.
    pub fn capabilities(&self) -> Capabilities {
        match &self.remote {
            Some(remote) => remote.capabilities(),
            None => self.local.capabilities(),
        }
    }

    /// The remote, unless it has already proved unreachable this session.
    ///
    /// Once something has failed, later calls in the same turn go straight to
    /// the local folders. Retrying a dead server on every single operation
    /// would turn one unreachable process into a timeout per memory call.
    fn remote(&self) -> Option<&Arc<dyn Backend>> {
        if self.problem.read().is_some() {
            return None;
        }
        self.remote.as_ref()
    }

    fn degrade(&self, why: String) {
        *self.problem.write() = Some(why);
    }

    /// Every memory that could apply here.
    pub fn list(&self) -> Vec<Value> {
        if let Some(remote) = self.remote() {
            match remote.list() {
                Ok(rows) => return rows,
                Err(why) => self.degrade(format!(
                    "Memories could not be read from {}, so this workspace's own are being used instead. {why}",
                    remote.label()
                )),
            }
        }
        self.local.list()
    }

    /// The memories in force for a turn in this folder.
    pub fn in_scope(&self, rows: &[Value]) -> Vec<Value> {
        self.local.in_scope(rows)
    }

    /// Write one memory, merging it into the one it repeats.
    ///
    /// A server that will not take it does not lose it: the memory is kept in
    /// the workspace instead and the note says where it went. A memory half
    /// written is worse than a memory written somewhere less useful.
    pub fn remember(&self, input: &Value, merge: bool) -> Result<Value, String> {
        if !record::is_storable(input) {
            return Err("A memory needs a title and a body, and may not carry a secret.".into());
        }
        if let Some(remote) = self.remote() {
            match remote.remember(input, merge) {
                Ok(saved) => return Ok(saved),
                Err(why) => self.degrade(format!(
                    "{} could not be written to, so the memory was kept in this workspace instead. {why}",
                    remote.label()
                )),
            }
        }
        self.local.remember(input, merge)
    }

    /// Change one memory, wherever it is.
    pub fn update(&self, id: &str, changes: Value) -> Result<Value, String> {
        if let Some(remote) = self.remote() {
            if id.starts_with(crate::mcp::PREFIX) {
                return remote.update(id, changes);
            }
        }
        self.local.update(id, changes)
    }

    pub fn forget(&self, id: &str) -> Result<(), String> {
        if let Some(remote) = self.remote() {
            if id.starts_with(crate::mcp::PREFIX) {
                return remote.forget(id);
            }
        }
        self.local.forget(id)
    }

    /// Say that these memories were useful.
    pub fn touch(&self, ids: &[String]) {
        let (remote_ids, local_ids): (Vec<String>, Vec<String>) = ids
            .iter()
            .cloned()
            .partition(|id| id.starts_with(crate::mcp::PREFIX));
        if !remote_ids.is_empty() {
            if let Some(remote) = self.remote() {
                remote.touch(&remote_ids);
            }
        }
        if !local_ids.is_empty() {
            self.local.touch(&local_ids);
        }
    }

    /// The project's handover note, if one has been written.
    pub fn note(&self, rows: &[Value]) -> Option<Value> {
        self.local.note(rows)
    }

    pub fn project(&self) -> Option<&Path> {
        self.local.project()
    }
}

/// A record written into a project carries no absolute path.
///
/// It is read back with the folder it was found in, so storing one would only
/// be a stale copy of the truth - and a stale copy of somebody's home directory
/// committed to a repository at that.
fn strip_location(record: &mut Value, scope: &str) {
    if scope != "project" {
        return;
    }
    if let Some(map) = record.as_object_mut() {
        map.remove("folder");
    }
}

/// Stamped here rather than left to the caller, so that everything which writes
/// a memory - the tool, a capture pass, the screen - produces a record the
/// screen can sort.
fn stamp_used(record: &mut Value) {
    let Some(map) = record.as_object_mut() else { return };
    if !map.get("lastUsedAt").is_some_and(Value::is_string) {
        map.insert("lastUsedAt".into(), Value::String(now_iso()));
    }
    let _: &mut Map<String, Value> = map;
}

fn now_iso() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let workspace = Layout::new(dir.path().join("workspace"));
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).expect("a project dir");
        let store = Store::new(workspace, Some(project));
        (dir, store)
    }

    #[test]
    fn a_project_memory_lands_in_the_project_not_the_workspace() {
        let (dir, store) = store();
        let folder = dir.path().join("project");

        store
            .remember(
                &json!({
                    "title": "Deploys go to fly.io",
                    "body": "never render",
                    "scope": "project",
                    "folder": folder.to_string_lossy(),
                }),
                true,
            )
            .expect("the memory is written");

        let inside = folder.join(".inertia").join("memory");
        let written: Vec<_> = std::fs::read_dir(&inside)
            .expect("the project memory folder")
            .flatten()
            .collect();
        assert_eq!(written.len(), 1, "one file, in the project");

        // And nothing in the workspace.
        assert!(std::fs::read_dir(dir.path().join("workspace").join("memory"))
            .map(|entries| entries.flatten().count())
            .unwrap_or(0)
            == 0);
    }

    #[test]
    fn a_global_memory_stays_in_the_workspace() {
        let (dir, store) = store();
        store
            .remember(&json!({ "title": "Call me Tanvir", "body": "not Mr Ahamed" }), true)
            .expect("the memory is written");

        let held = std::fs::read_dir(dir.path().join("workspace").join("memory"))
            .expect("the workspace memory folder")
            .flatten()
            .count();
        assert_eq!(held, 1);
        assert!(!dir.path().join("project").join(".inertia").exists());
    }

    /// A repository cloned somewhere else must not take its memories out of
    /// scope. The path is stamped on read, from the folder the file was found
    /// in, and never stored.
    #[test]
    fn a_project_memory_is_read_back_with_the_folder_it_was_found_in() {
        let (dir, store) = store();
        let folder = dir.path().join("project");
        store
            .remember(
                &json!({
                    "title": "Uses pnpm", "body": "not npm",
                    "scope": "project", "folder": folder.to_string_lossy(),
                }),
                true,
            )
            .expect("written");

        // The file itself names no path.
        let file = std::fs::read_dir(folder.join(".inertia").join("memory"))
            .expect("the folder")
            .flatten()
            .next()
            .expect("one file");
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(file.path()).expect("readable"))
                .expect("json");
        assert!(raw.get("folder").is_none());

        // Read back, it belongs to this project and is in scope for it.
        let rows = store.list();
        assert_eq!(rows.len(), 1);
        assert_eq!(record::scope_of(&rows[0]), "project");
        assert!(record::same_folder(
            &record::text(&rows[0], "folder"),
            &folder.to_string_lossy()
        ));
        assert_eq!(store.in_scope(&rows).len(), 1);
    }

    #[test]
    fn a_project_memory_already_in_the_workspace_goes_on_working() {
        let (dir, store) = store();
        let folder = dir.path().join("project");
        // Written the old way: in the workspace, naming its folder.
        collections::put(
            &Layout::new(dir.path().join("workspace")),
            Collection::Memories,
            json!({
                "id": "old", "title": "Old fact", "body": "still true",
                "scope": "project", "folder": folder.to_string_lossy(),
            }),
        )
        .expect("written");

        let rows = store.list();
        assert_eq!(store.in_scope(&rows).len(), 1);

        // And it is still editable where it lies, rather than being duplicated
        // into the project.
        store.update("old", json!({ "body": "corrected" })).expect("updated");
        assert_eq!(record::text(&store.list()[0], "body"), "corrected");
        assert!(!folder.join(".inertia").join("memory").exists());
    }

    #[test]
    fn the_same_fact_twice_is_one_record() {
        let (_dir, store) = store();
        let first = store
            .remember(&json!({ "title": "Call me Tanvir", "body": "not Mr Ahamed", "tags": ["name"] }), true)
            .expect("written");
        let again = store
            .remember(&json!({ "title": "Call me Tanvir", "body": "Tanvir is fine", "tags": ["person"] }), true)
            .expect("merged");

        assert_eq!(record::text(&first, "id"), record::text(&again, "id"));
        assert_eq!(store.list().len(), 1);
        // The newer wording wins; the tags accumulate.
        assert_eq!(record::text(&again, "body"), "Tanvir is fine");
        assert_eq!(record::tags(&again), ["name", "person"]);
    }

    #[test]
    fn a_memory_carrying_a_credential_is_refused() {
        let (_dir, store) = store();
        let refused = store.remember(
            &json!({ "title": "The key", "body": "ghp_0123456789abcdefghij" }),
            true,
        );
        assert!(refused.is_err());
        assert!(store.list().is_empty());
    }

    #[test]
    fn using_a_memory_counts_it() {
        let (_dir, store) = store();
        let written = store
            .remember(&json!({ "title": "One", "body": "fact" }), true)
            .expect("written");
        let id = record::text(&written, "id");

        store.touch(std::slice::from_ref(&id));
        store.touch(std::slice::from_ref(&id));

        let row = &store.list()[0];
        assert_eq!(row["useCount"], 2);
    }
}

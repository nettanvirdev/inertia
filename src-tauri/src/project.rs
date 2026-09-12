//! Setting a project up to be worked on.
//!
//! `AGENTS.md` and `.inertia/rules/` are read from the repository root down to
//! the working folder at the start of every conversation. They are the most
//! direct way to tell an agent how a project works, and because `AGENTS.md` is
//! the name every other coding agent reads too, a file created here helps
//! Claude Code and Codex as much as it helps this app.
//!
//! Which is exactly why creating one is a decision. Writing into somebody's
//! repository uninvited is the sort of thing that turns up in a diff three days
//! later and is not welcome, so:
//!
//!   - nothing is ever written without the user having chosen a mode that
//!     allows it, and the default mode asks;
//!   - an existing file is never touched, whatever the mode says. A project
//!     that already has an `AGENTS.md` has one somebody wrote;
//!   - a folder is offered this once. Declining is remembered, so a person who
//!     said no is not asked again every time they open the folder.
//!
//! The state lives in the workspace, not in the project. A repository must not
//! gain a file recording that this app once asked about it.

use std::path::{Path, PathBuf};

use inertia_store::layout::Document;
use serde_json::{json, Value};
use tauri::State;

use crate::state::AppState;

const RULES_DIR: &str = ".inertia/rules";

/// Where a project begins.
///
/// Walked upward looking for the marks that mean "this is a checkout", so a
/// conversation pointed at `repo/src/app` reads the `AGENTS.md` at `repo/`.
/// Falls back to the folder itself, which is the right answer for a directory
/// that is not a repository at all.
fn project_root(cwd: &Path) -> PathBuf {
    const MARKS: &[&str] = &[".git", ".hg", ".svn"];
    let mut here = cwd;
    loop {
        if MARKS.iter().any(|mark| here.join(mark).exists()) {
            return here.to_path_buf();
        }
        match here.parent() {
            Some(parent) => here = parent,
            None => return cwd.to_path_buf(),
        }
    }
}

/// The key a folder is remembered under.
///
/// Case-folded on Windows, where `D:\Repo` and `d:\repo` are the same folder
/// and would otherwise be asked about twice.
fn key_for(folder: &Path) -> String {
    let text = folder.to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/').to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

fn read_state(state: &AppState) -> Result<Value, String> {
    let workspace = state.workspace()?;
    Ok(inertia_store::collections::read_document(
        &workspace.layout,
        Document::Projects,
        json!({ "folders": {} }),
    ))
}

fn write_folder(state: &AppState, folder: &Path, entry: Value) -> Result<(), String> {
    let workspace = state.workspace()?;
    let mut document = read_state(state)?;
    let folders = document
        .as_object_mut()
        .and_then(|m| {
            m.entry("folders")
                .or_insert_with(|| json!({}))
                .as_object_mut()
        })
        .ok_or("The projects document is not in the expected shape.")?;
    folders.insert(key_for(folder), entry);

    inertia_store::collections::write_document(&workspace.layout, Document::Projects, &document)
        .map_err(|e| e.to_string())
}

/// What is already in this folder, and whether we have asked about it before.
#[tauri::command]
pub fn project_status(state: State<'_, AppState>, cwd: Option<String>) -> Result<Value, String> {
    let Some(cwd) = cwd.filter(|c| !c.trim().is_empty()) else {
        return Ok(json!({ "available": false }));
    };
    let folder = project_root(Path::new(&cwd));

    let has_agents = folder.join("AGENTS.md").exists();
    let has_claude = folder.join("CLAUDE.md").exists();
    let has_rules = folder.join(RULES_DIR).exists();

    let remembered = read_state(&state)?
        .get("folders")
        .and_then(|f| f.get(key_for(&folder)))
        .cloned()
        .unwrap_or(Value::Null);

    Ok(json!({
        "available": true,
        "folder": folder.to_string_lossy(),
        "hasAgents": has_agents,
        "hasClaude": has_claude,
        "hasRules": has_rules,
        // Nothing to offer when the project already says something about
        // itself, whichever of the two names it used.
        "configured": has_agents || has_claude || has_rules,
        "declined": remembered.get("declined").and_then(Value::as_bool).unwrap_or(false),
        "createdAt": remembered.get("createdAt").cloned().unwrap_or(Value::Null),
    }))
}

/// Writes the starter files.
///
/// Never overwrites. Each half is independent, so a project that already has an
/// `AGENTS.md` but no rules folder can still be given the folder, and the
/// answer says exactly what was created rather than claiming to have done both.
#[tauri::command]
pub fn project_create(
    state: State<'_, AppState>,
    cwd: Option<String>,
    options: Option<Value>,
) -> Result<Value, String> {
    let status = project_status(state.clone(), cwd)?;
    if status["available"] != true {
        return Err("There is no project folder to set up.".into());
    }
    let folder = PathBuf::from(status["folder"].as_str().unwrap_or_default());
    let want_rules = options
        .as_ref()
        .and_then(|o| o.get("rules"))
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut created: Vec<String> = Vec::new();

    if status["hasAgents"] != true && status["hasClaude"] != true {
        let name = folder
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "this project".to_string());
        std::fs::write(folder.join("AGENTS.md"), starter_agents_file(&name))
            .map_err(|e| e.to_string())?;
        created.push("AGENTS.md".into());
    }

    if want_rules && status["hasRules"] != true {
        let dir = folder.join(RULES_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let readme = dir.join("README.md");
        if !readme.exists() {
            std::fs::write(readme, STARTER_RULES_README).map_err(|e| e.to_string())?;
        }
        created.push(RULES_DIR.into());
    }

    write_folder(
        &state,
        &folder,
        json!({
            "createdAt": jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            "declined": false,
        }),
    )?;

    Ok(json!({ "folder": folder.to_string_lossy(), "created": created }))
}

/// Remembers that this folder was offered and turned down.
///
/// Without this, choosing "not now" means being asked again on the next
/// message, which is how a helpful offer becomes something a person learns to
/// dismiss without reading.
#[tauri::command]
pub fn project_decline(state: State<'_, AppState>, cwd: Option<String>) -> Result<Value, String> {
    let status = project_status(state.clone(), cwd)?;
    if status["available"] != true {
        return Ok(json!({ "declined": false }));
    }
    let folder = PathBuf::from(status["folder"].as_str().unwrap_or_default());
    write_folder(&state, &folder, json!({ "declined": true }))?;
    Ok(json!({ "declined": true, "folder": folder.to_string_lossy() }))
}

/// Should the app do something about this folder right now?
///
/// `moment` is `folder` when a project has just been opened and `message` when
/// the person has just sent their first message in it. The answer is one of
/// "create it", "ask about it", or nothing at all - so a caller does not have
/// to know what the modes mean. Its own call rather than something the window
/// works out from `status` plus a preference: the rule belongs in one place,
/// and a second copy of it would eventually disagree.
#[tauri::command]
pub fn project_decide(
    state: State<'_, AppState>,
    cwd: Option<String>,
    options: Option<Value>,
) -> Result<Value, String> {
    let status = project_status(state, cwd)?;
    let field = |key: &str| {
        options
            .as_ref()
            .and_then(|o| o.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };

    if status["available"] != true || status["configured"] == true || status["declined"] == true {
        return Ok(json!({ "action": "none", "state": status }));
    }

    let action = match (field("mode").as_str(), field("moment").as_str()) {
        ("manual", _) => "none",
        // Asking, rather than doing it silently. A file appearing in a
        // repository nobody asked to change is worse than a file that is
        // missing: the missing one is noticed and the surprising one is
        // committed.
        ("ask", _) | ("", _) => "ask",
        ("folder", "folder") | ("message", "message") => "create",
        _ => "none",
    };
    Ok(json!({ "action": action, "state": status }))
}

/// What a new `AGENTS.md` says.
///
/// Deliberately a skeleton with prompts rather than a guess at the project's
/// conventions. A generated file full of confident inventions is worse than an
/// empty one: it is read on every turn by every agent, so a wrong line in it is
/// a wrong line in every conversation, and nobody edits a file that looks
/// finished.
fn starter_agents_file(name: &str) -> String {
    format!(
        "# {name}\n\
\n\
Notes for anyone, human or agent, working in this repository. This file is\n\
read at the start of every conversation, so keep it short and keep it true -\n\
a stale line here is repeated to everyone, forever.\n\
\n\
## What this is\n\
\n\
<!-- One or two sentences. What the project does and who it is for. -->\n\
\n\
## Running it\n\
\n\
<!-- The commands that matter: install, run, test, build. -->\n\
\n\
## Conventions\n\
\n\
<!-- Things that are true across the project and not obvious from one file. -->\n\
\n\
## Watch out for\n\
\n\
<!-- Known traps, things that look wrong but are deliberate. -->\n"
    )
}

/// The one-topic rules folder, with a note saying what it is for.
const STARTER_RULES_README: &str = "# Rules\n\
\n\
One file per topic. Every `.md` file in this folder is read at the start of\n\
every conversation, in name order, along with `AGENTS.md` above it.\n\
\n\
Use this when `AGENTS.md` starts growing sections that only matter\n\
occasionally - `testing.md`, `deploys.md`, `style.md`. Anything long enough\n\
to need its own document is probably a skill instead, which is loaded only\n\
when it is relevant rather than read every time.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkout_is_found_from_a_folder_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let deep = repo.join("src/app");
        std::fs::create_dir_all(&deep).unwrap();

        assert_eq!(project_root(&deep), repo);
    }

    /// A plain directory is its own project, which is the right answer for a
    /// scratch folder that was never a repository.
    #[test]
    fn a_folder_with_no_repository_is_its_own_root() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(project_root(dir.path()), dir.path());
    }

    #[test]
    fn the_starter_file_is_a_skeleton_not_a_guess() {
        let text = starter_agents_file("widgets");
        assert!(text.starts_with("# widgets\n"));
        // Prompts, not claims: every section is an HTML comment for a person to
        // replace.
        assert_eq!(text.matches("<!--").count(), 4);
        assert!(!text.contains("npm install"), "nothing may be invented");
    }

    /// `D:\Repo` and `d:\repo` are the same folder on Windows, and asking about
    /// it twice is exactly the nag declining is meant to prevent.
    #[test]
    fn a_folder_is_remembered_under_one_key() {
        let a = key_for(Path::new("D:/Projects/Thing"));
        let b = key_for(Path::new("D:/Projects/Thing/"));
        assert_eq!(a, b);

        if cfg!(windows) {
            assert_eq!(key_for(Path::new("d:/projects/thing")), a);
        }
    }
}

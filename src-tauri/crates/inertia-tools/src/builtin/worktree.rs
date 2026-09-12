//! Working on a copy of the repository.
//!
//! A git worktree is a second checkout of the same repository in another
//! folder, on its own branch. For an agent it is the difference between "edit
//! the files the person is looking at" and "edit a copy, and hand back a
//! branch": three agents can work on three tasks in three worktrees of one
//! repository without treading on each other or on the person, and a turn
//! that goes wrong leaves the main checkout untouched.
//!
//! Entering changes the working folder for the rest of the turn, and the
//! conversation remembers it, so every later turn starts there too until the
//! agent or the person leaves. The renderer reads `metadata.worktree` off the
//! tool-end event and keeps it on the thread, then sends `cwd` and `worktree`
//! back with every turn - so the shape of that object is a contract, not a
//! convenience. The worktree lives inside the repository under
//! `.inertia/worktrees/<name>`, which is added to the repository's own exclude
//! file so it never shows up as untracked; the branch is `inertia/<name>`.
//! Leaving switches the folder back and, if asked, removes the worktree -
//! never the branch, which is the work.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use serde_json::{json, Value};

/// Where worktrees go, relative to the repository root.
pub const WORKTREES_DIR: &str = ".inertia/worktrees";

/// Where this checkout's worktrees live, as a real path.
///
/// Built one component at a time rather than by joining the constant whole: a
/// `Path` on Windows keeps an embedded forward slash exactly as it was written,
/// so `root.join(".inertia/worktrees")` and the same path arrived at through
/// the filesystem compare unequal while pointing at the same folder - which is
/// a bug the renderer would hit, not just a test.
pub fn worktrees_dir(root: &std::path::Path) -> std::path::PathBuf {
    let mut dir = root.to_path_buf();
    for part in WORKTREES_DIR.split('/') {
        dir.push(part);
    }
    dir
}

/// Branches this makes are namespaced, so they are easy to find and to clean
/// up.
pub const BRANCH_PREFIX: &str = "inertia/";

async fn git(args: &[&str], cwd: &Path) -> std::result::Result<String, String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(args).current_dir(cwd).stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().await.map_err(|e| format!("git could not be run: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A path as the person would write it.
///
/// Canonical where it exists, which folds Windows 8.3 short names (`NETTAN~1`)
/// into the long form git reports, then stripped of the `\\?\` verbatim prefix
/// canonicalize adds on Windows - a path with that prefix is one no shell, and
/// no person, recognises.
fn tidy(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = canonical.to_string_lossy();
    if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
        return PathBuf::from(format!("\\\\{unc}"));
    }
    if let Some(plain) = text.strip_prefix("\\\\?\\") {
        return PathBuf::from(plain);
    }
    canonical
}

/// `git` prints forward slashes even on Windows, and some answers relative to
/// the folder it was asked in.
fn from_git(cwd: &Path, reported: &str) -> PathBuf {
    let raw = Path::new(reported);
    let joined = if raw.is_absolute() || has_drive(reported) {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    };
    tidy(&joined)
}

fn has_drive(path: &str) -> bool {
    let b = path.as_bytes();
    cfg!(windows) && b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()
}

/// The repository a folder belongs to: its main checkout, and whether the
/// folder is itself a worktree of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// The top of the checkout `cwd` is in - the worktree itself, when in one.
    pub top: PathBuf,
    /// The main checkout.
    pub root: PathBuf,
    pub is_worktree: bool,
    pub common_dir: PathBuf,
}

pub async fn repository_of(cwd: &Path) -> Option<Repository> {
    let top = git(&["rev-parse", "--show-toplevel"], cwd).await.ok()?;
    let common = git(&["rev-parse", "--git-common-dir"], cwd).await.ok()?;
    let git_dir = git(&["rev-parse", "--git-dir"], cwd).await.ok()?;
    let common_dir = from_git(cwd, &common);
    let git_dir = from_git(cwd, &git_dir);
    let root = common_dir.parent().map(Path::to_path_buf)?;
    Some(Repository {
        top: from_git(cwd, &top),
        root,
        is_worktree: git_dir != common_dir,
        common_dir,
    })
}

/// A name that is safe as a folder and a branch.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    let trimmed = out.trim_matches(['-', '.']);
    trimmed.chars().take(48).collect()
}

/// Keep `.inertia/` out of `git status` in the main repository.
fn exclude_ours(common_dir: &Path) -> std::io::Result<()> {
    let file = common_dir.join("info").join("exclude");
    let line = "/.inertia/";
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    if current.lines().any(|l| l == line) {
        return Ok(());
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let glue = if !current.is_empty() && !current.ends_with('\n') { "\n" } else { "" };
    std::fs::write(&file, format!("{current}{glue}{line}\n"))
}

async fn branch_exists(root: &Path, branch: &str) -> bool {
    git(
        &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")],
        root,
    )
    .await
    .is_ok()
}

/// A worktree that is there, made or found.
#[derive(Debug, Clone)]
pub struct Made {
    pub name: String,
    pub dir: PathBuf,
    pub branch: String,
    pub repo: Repository,
    pub existing: bool,
}

/// The worktree for `name`, made if it is not there.
///
/// Shared by `worktree_enter`, which moves this conversation into it, and
/// available to a `spawn` with worktree isolation, which starts a helper in
/// it. Fails in the words the model needs; the callers decide what to say
/// about the folder afterwards.
pub async fn ensure_worktree(cwd: &Path, raw_name: &str, base: Option<&str>) -> Result<Made> {
    let name = slug(raw_name);
    if name.is_empty() {
        return Err(Error::InvalidInput(
            "Give the worktree a name made of letters, digits, dots, dashes or underscores.".into(),
        ));
    }
    let Some(repo) = repository_of(cwd).await else {
        return Err(Error::Other(format!(
            "{} is not inside a git repository, so there is nothing to make a worktree of.",
            cwd.display()
        )));
    };
    let dir = worktrees_dir(&repo.root).join(&name);
    let branch = format!("{BRANCH_PREFIX}{name}");

    let existing = dir.is_dir();
    if !existing {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Other(format!("Could not create {}: {e}", parent.display())))?;
        }
        // Best effort: a repository whose `.git/info` cannot be written still
        // gets its worktree, it just shows up as untracked.
        let _ = exclude_ours(&repo.common_dir);
        let dir_text = dir.to_string_lossy().to_string();
        let mut args = vec!["worktree", "add"];
        if branch_exists(&repo.root, &branch).await {
            args.push(&dir_text);
            args.push(&branch);
        } else {
            args.push("-b");
            args.push(&branch);
            args.push(&dir_text);
            args.push(base.unwrap_or("HEAD"));
        }
        git(&args, &repo.root)
            .await
            .map_err(|e| Error::Other(format!("git could not add the worktree: {e}")))?;
    }
    Ok(Made {
        name,
        dir: tidy(&dir),
        branch,
        repo,
        existing,
    })
}

// ── worktree_enter ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct WorktreeEnterTool;

#[async_trait]
impl Tool for WorktreeEnterTool {
    fn id(&self) -> &str {
        "worktree_enter"
    }

    fn description(&self) -> &str {
        "Work on a copy of this repository, on its own branch, instead of the checkout the\n\
         person is looking at.\n\
         \n\
         Creates a git worktree under `.inertia/worktrees/<name>` on branch `inertia/<name>`\n\
         and moves your working folder there for the rest of this conversation. Every file\n\
         tool and command runs in the copy from then on. Use it for a change that should\n\
         land as a branch rather than as edits in place, for work that must not disturb what\n\
         the person has open, or when several agents will change the same repository at once.\n\
         \n\
         Call `worktree_exit` when the work is committed. If a worktree of that name already\n\
         exists it is reused."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "A short name for the work, e.g. 'fix-login'. Becomes the folder and the branch." },
                "base": { "type": "string", "description": "The branch or commit to start from. Defaults to the current HEAD." }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let name = slug(args.get("name").and_then(Value::as_str).unwrap_or(""));
        let name = if name.is_empty() { "worktree".to_string() } else { name };
        PermissionRequest::new("shell", format!("git worktree add {name}")).with_always("git worktree *")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let raw = args.get("name").and_then(Value::as_str).unwrap_or("");
        let name = slug(raw);
        Some(format!("Enter worktree {}", if name.is_empty() { raw } else { &name }))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let raw = args.get("name").and_then(Value::as_str).unwrap_or("");
        let base = args.get("base").and_then(Value::as_str).map(str::trim).filter(|b| !b.is_empty());
        let made = ensure_worktree(&ctx.root, raw, base).await?;

        let dir = made.dir.to_string_lossy().to_string();
        let root = made.repo.root.to_string_lossy().to_string();
        // `cwd` moves the rest of this turn; `worktree` is what the
        // conversation remembers for the turns after it.
        let worktree = json!({
            "path": dir,
            "branch": made.branch,
            "root": root,
            "name": made.name,
        });
        Ok(ToolOutcome {
            title: Some(format!("Working in {}", made.name)),
            output: format!(
                "{} the worktree at {dir} on branch {}.\n\
                 \n\
                 Your working folder is now that copy: relative paths, reads, edits and commands all\n\
                 happen there. The main checkout at {root} is untouched. Commit on {} when\n\
                 the work is done, then call `worktree_exit`.",
                if made.existing { "Reusing" } else { "Created" },
                made.branch,
                made.branch
            ),
            metadata: Some(json!({ "cwd": dir, "worktree": worktree, "created": !made.existing })),
            images: Vec::new(),
        })
    }
}

// ── worktree_exit ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct WorktreeExitTool;

#[async_trait]
impl Tool for WorktreeExitTool {
    fn id(&self) -> &str {
        "worktree_exit"
    }

    fn description(&self) -> &str {
        "Leave the worktree and go back to the main checkout.\n\
         \n\
         The branch is kept - it is the work. Pass `remove: true` to also delete the\n\
         worktree folder, which git refuses while it has uncommitted changes unless\n\
         `force` is set; say what you would lose before forcing anything."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "remove": { "type": "boolean", "description": "Also delete the worktree folder. The branch stays." },
                "force": { "type": "boolean", "description": "Delete even with uncommitted changes. Only with the person's say-so." }
            },
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let remove = args.get("remove").and_then(Value::as_bool).unwrap_or(false);
        PermissionRequest::new(
            "shell",
            if remove { "git worktree remove" } else { "git worktree exit" },
        )
        .with_always("git worktree *")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let remove = args.get("remove").and_then(Value::as_bool).unwrap_or(false);
        Some(if remove { "Leave and remove the worktree" } else { "Leave the worktree" }.to_string())
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let repo = match repository_of(&ctx.root).await {
            Some(repo) if repo.is_worktree => repo,
            _ => {
                return Err(Error::Other(
                    "You are not in a worktree; the working folder is already the main checkout.".into(),
                ))
            }
        };
        let remove = args.get("remove").and_then(Value::as_bool).unwrap_or(false);
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
        let here = repo.top.to_string_lossy().to_string();
        let root = repo.root.to_string_lossy().to_string();

        let mut removed = false;
        if remove {
            let mut flags = vec!["worktree", "remove"];
            if force {
                flags.push("--force");
            }
            flags.push(&here);
            git(&flags, &repo.root).await.map_err(|e| {
                Error::Other(format!(
                    "git would not remove the worktree: {e}. Commit or discard the changes there first, or leave without removing."
                ))
            })?;
            removed = true;
        }

        Ok(ToolOutcome {
            title: Some("Back in the main checkout".into()),
            output: format!(
                "Your working folder is {root} again. {}",
                if removed {
                    format!("The worktree at {here} was removed; its branch is kept.")
                } else {
                    format!("The worktree at {here} is still there.")
                }
            ),
            metadata: Some(json!({ "cwd": root, "worktree": Value::Null, "removed": removed })),
            images: Vec::new(),
        })
    }
}

pub fn worktree_tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(WorktreeEnterTool), Arc::new(WorktreeExitTool)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::tool::PermissionGate;
    use inertia_mock::MockGate;

    fn ctx(root: &Path) -> ToolContext {
        ToolContext {
            root: root.to_path_buf(),
            session: SessionId::from_existing("t1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(MockGate::allow_all()) as Arc<dyn PermissionGate>,
        }
    }

    fn git_sync(args: &[&str], cwd: &Path) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A real repository, because the whole feature is what git does with a
    /// second checkout, and a fake would be testing the fake. `None` when
    /// there is no git on this machine, and the test skips.
    fn repo() -> Option<(tempfile::TempDir, PathBuf)> {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            eprintln!("git is not installed; skipping");
            return None;
        }
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = tidy(dir.path());
        git_sync(&["init", "--quiet"], &root);
        git_sync(&["config", "user.email", "test@example.com"], &root);
        git_sync(&["config", "user.name", "Test"], &root);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        git_sync(&["add", "-A"], &root);
        git_sync(&["commit", "--quiet", "-m", "first"], &root);
        Some((dir, root))
    }

    #[test]
    fn a_name_is_made_safe_for_a_folder_and_a_branch() {
        assert_eq!(slug("Fix the Login!"), "fix-the-login");
        assert_eq!(slug("  --weird--  "), "weird");
        assert_eq!(slug(""), "");
        assert_eq!(slug("///"), "");
    }

    #[tokio::test]
    async fn entering_creates_a_worktree_on_its_own_branch_and_moves_the_folder() {
        let Some((_keep, root)) = repo() else { return };
        let out = WorktreeEnterTool
            .execute(json!({ "name": "Fix login" }), &ctx(&root))
            .await
            .unwrap();

        let dir = worktrees_dir(&root).join("fix-login");
        assert!(dir.join("a.txt").is_file());
        let meta = out.metadata.unwrap();
        assert_eq!(meta["cwd"], json!(dir.to_string_lossy()));
        assert_eq!(meta["created"], true);
        // The exact shape the renderer keeps on the thread and sends back.
        assert_eq!(
            meta["worktree"],
            json!({
                "path": dir.to_string_lossy(),
                "branch": "inertia/fix-login",
                "root": root.to_string_lossy(),
                "name": "fix-login",
            })
        );
        assert_eq!(git_sync(&["rev-parse", "--abbrev-ref", "HEAD"], &dir), "inertia/fix-login");

        // The main checkout is untouched, and does not report the copy as
        // untracked.
        assert_eq!(git_sync(&["status", "--porcelain"], &root), "");
        let exclude = std::fs::read_to_string(root.join(".git").join("info").join("exclude")).unwrap();
        assert!(exclude.contains("/.inertia/"));
    }

    #[tokio::test]
    async fn a_worktree_of_the_same_name_is_reused() {
        let Some((_keep, root)) = repo() else { return };
        WorktreeEnterTool.execute(json!({ "name": "again" }), &ctx(&root)).await.unwrap();
        let second = WorktreeEnterTool
            .execute(json!({ "name": "again" }), &ctx(&root))
            .await
            .unwrap();
        assert_eq!(second.metadata.unwrap()["created"], false);
        assert!(second.output.contains("Reusing"));
    }

    #[tokio::test]
    async fn entering_refuses_outside_a_repository_and_a_name_that_is_not_one() {
        let Some((_keep, root)) = repo() else { return };
        let loose = tempfile::tempdir().unwrap();
        let err = WorktreeEnterTool
            .execute(json!({ "name": "x" }), &ctx(loose.path()))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not inside a git repository"), "{err}");

        let err = WorktreeEnterTool
            .execute(json!({ "name": "///" }), &ctx(&root))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("name made of letters"), "{err}");
    }

    #[tokio::test]
    async fn leaving_goes_back_and_keeps_the_worktree_unless_asked() {
        let Some((_keep, root)) = repo() else { return };
        let entered = WorktreeEnterTool.execute(json!({ "name": "keep" }), &ctx(&root)).await.unwrap();
        let dir = PathBuf::from(entered.metadata.unwrap()["cwd"].as_str().unwrap());

        let left = WorktreeExitTool.execute(json!({}), &ctx(&dir)).await.unwrap();
        let meta = left.metadata.unwrap();
        assert_eq!(meta["cwd"], json!(root.to_string_lossy()));
        assert_eq!(meta["worktree"], Value::Null);
        assert_eq!(meta["removed"], false);
        assert!(dir.exists());

        let left = WorktreeExitTool.execute(json!({ "remove": true }), &ctx(&dir)).await.unwrap();
        assert_eq!(left.metadata.unwrap()["removed"], true);
        assert!(!dir.exists());
        // The work is the branch, and the branch stays.
        let sha = git_sync(&["rev-parse", "--verify", "refs/heads/inertia/keep"], &root);
        assert_eq!(sha.len(), 40);
    }

    #[tokio::test]
    async fn uncommitted_work_is_not_thrown_away_without_force() {
        let Some((_keep, root)) = repo() else { return };
        let entered = WorktreeEnterTool.execute(json!({ "name": "dirty" }), &ctx(&root)).await.unwrap();
        let dir = PathBuf::from(entered.metadata.unwrap()["cwd"].as_str().unwrap());
        std::fs::write(dir.join("a.txt"), "changed\n").unwrap();

        let err = WorktreeExitTool
            .execute(json!({ "remove": true }), &ctx(&dir))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Commit or discard"), "{err}");
        assert!(dir.exists());

        WorktreeExitTool
            .execute(json!({ "remove": true, "force": true }), &ctx(&dir))
            .await
            .unwrap();
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn leaving_the_main_checkout_says_so() {
        let Some((_keep, root)) = repo() else { return };
        let err = WorktreeExitTool.execute(json!({}), &ctx(&root)).await.unwrap_err().to_string();
        assert!(err.contains("already the main checkout"), "{err}");
    }

    #[tokio::test]
    async fn a_folder_knows_which_checkout_it_came_from() {
        let Some((_keep, root)) = repo() else { return };
        let entered = WorktreeEnterTool.execute(json!({ "name": "which" }), &ctx(&root)).await.unwrap();
        let dir = PathBuf::from(entered.metadata.unwrap()["cwd"].as_str().unwrap());
        let inside = repository_of(&dir).await.unwrap();
        assert_eq!(inside.root, root);
        assert!(inside.is_worktree);
        let main = repository_of(&root).await.unwrap();
        assert_eq!(main.root, root);
        assert!(!main.is_worktree);
        let loose = tempfile::tempdir().unwrap();
        assert!(repository_of(loose.path()).await.is_none());
    }
}

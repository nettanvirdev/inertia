//! Where a file tool may reach.
//!
//! Every tool that takes a path resolves it here, so the two boundaries are
//! drawn once rather than once per tool:
//!
//!   - **Off limits.** The workspace's secrets, its permission rules and its
//!     hooks, and any project's own hooks file, are refused whatever the rules
//!     say and whatever the approval dial is set to. An agent that could
//!     rewrite the rules could grant itself anything, and one that could write
//!     a hook could run anything.
//!   - **Outside the working folder.** Reaching anywhere else on the machine
//!     is its own question, `external_directory`, asked once per folder - the
//!     same question `shell` and `present` ask. Without it one "always allow
//!     read" let an agent read `~/.ssh` without anyone seeing it happen. The
//!     workspace's own `files/` folder, where attachments and anything an
//!     agent produced live, counts as inside.
//!
//! Both are decided on the path as the filesystem resolves it, so a link from
//! the working folder to somewhere else is judged by where it goes.

use std::path::{Path, PathBuf};

use inertia_core::tool::{PermissionRequest, ToolContext};
use inertia_core::{Error, Result};
use inertia_store::fsx::{canonical, is_within};
use inertia_store::Layout;

/// A path the model supplied, against the folder it is working in.
///
/// Absolute paths are honoured - the agent legitimately works on projects
/// outside the workspace folder - so this is resolution, not confinement.
/// [`check`] is what decides whether the answer may be touched.
pub fn resolve(root: &Path, supplied: &str) -> PathBuf {
    let path = Path::new(supplied);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// [`resolve`], then [`check`].
pub async fn reach(ctx: &ToolContext, supplied: &str) -> Result<PathBuf> {
    let path = resolve(&ctx.root, supplied);
    check(ctx, &path).await?;
    Ok(path)
}

/// Refuses an off-limits path, and asks about one outside the working folder.
pub async fn check(ctx: &ToolContext, path: &Path) -> Result<()> {
    let fence = Fence::of(ctx);
    let real = canonical(path);
    if fence.bars(&real) {
        return Err(off_limits(path));
    }
    if is_within(&canonical(&ctx.root), &real) || fence.shares(&real) {
        return Ok(());
    }

    // The folder rather than the file, so one answer covers the next file in
    // it. A trailing separator would make the pattern for a drive root read
    // `D:\/*`, which is not a rule anyone would recognise in the settings.
    let dir = if real.is_dir() {
        real.clone()
    } else {
        real.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| real.clone())
    };
    let pattern = format!("{}/*", dir.to_string_lossy().trim_end_matches(['\\', '/']));
    let request = PermissionRequest::new("external_directory", &pattern).with_always(&pattern);
    if !ctx.permissions.ask(&request).await?.is_allowed() {
        return Err(Error::Denied(format!(
            "{} is outside the working folder, and reaching it was not allowed.",
            path.display()
        )));
    }
    Ok(())
}

/// Like [`check`], for a tool that acts on everything under the path - a
/// copy, a move, a delete - which must not carry an off-limits folder along
/// inside the one it was pointed at.
pub async fn check_tree(ctx: &ToolContext, path: &Path) -> Result<()> {
    check(ctx, path).await?;
    if Fence::of(ctx).surrounds(&canonical(path)) {
        return Err(off_limits(path));
    }
    Ok(())
}

fn off_limits(path: &Path) -> Error {
    Error::Denied(format!(
        "{} holds this workspace's secrets or permission rules, or hooks that run as \
         programs around an agent's calls, which no tool may read or change. The person \
         changes those themselves.",
        path.display()
    ))
}

/// The hooks a repository ships, which run as programs on every turn in it.
/// The same path `inertia-hooks` reads them from.
const PROJECT_HOOKS: [&str; 2] = [".inertia", "hooks.json"];

/// Is `real` a project's hooks file, in any folder? Written by an agent into
/// its own working folder, it would run whatever the agent put there on the
/// next turn, with no card in front of it.
fn is_project_hooks(real: &Path) -> bool {
    let tail: Vec<String> = real
        .components()
        .rev()
        .take(2)
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect();
    let same = |a: &str, b: &str| {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    tail.len() == 2 && same(&tail[1], PROJECT_HOOKS[0]) && same(&tail[0], PROJECT_HOOKS[1])
}

/// The off-limits places for one call, resolved once so a walk over a folder
/// can test every entry without touching the disk again.
#[derive(Debug, Clone, Default)]
pub struct Fence {
    off_limits: Vec<PathBuf>,
    shared: Option<PathBuf>,
}

impl Fence {
    pub fn of(ctx: &ToolContext) -> Self {
        let Some(workspace) = ctx.permissions.workspace() else {
            return Self::default();
        };
        let layout = Layout::new(workspace);
        Self {
            off_limits: layout.off_limits(),
            shared: Some(canonical(&layout.root().join("files"))),
        }
    }

    /// Is `real` - a path already resolved by the filesystem - off limits?
    pub fn bars(&self, real: &Path) -> bool {
        is_project_hooks(real) || self.off_limits.iter().any(|place| is_within(place, real))
    }

    /// Does an off-limits place lie somewhere under `real`?
    fn surrounds(&self, real: &Path) -> bool {
        self.off_limits.iter().any(|place| is_within(real, place))
    }

    /// Is `real` in the workspace's `files/` folder, which every
    /// conversation shares?
    fn shares(&self, real: &Path) -> bool {
        self.shared
            .as_ref()
            .is_some_and(|files| is_within(files, real))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::tool::Decision;
    use inertia_mock::{MockGate, Policy};

    use super::*;

    struct Fixture {
        _dirs: (tempfile::TempDir, tempfile::TempDir),
        workspace: PathBuf,
        project: PathBuf,
        outside: PathBuf,
    }

    fn fixture() -> Fixture {
        let workspace = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        Layout::new(workspace.path()).scaffold().unwrap();
        let project = elsewhere.path().join("project");
        let outside = elsewhere.path().join("outside");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        Fixture {
            workspace: workspace.path().to_path_buf(),
            project,
            outside,
            _dirs: (workspace, elsewhere),
        }
    }

    fn ctx(root: &Path, gate: Arc<MockGate>) -> ToolContext {
        ToolContext {
            root: root.to_path_buf(),
            session: SessionId::from_existing("s"),
            call_id: ToolCallId::from_existing("c"),
            permissions: gate,
        }
    }

    #[tokio::test]
    async fn the_workspace_machinery_is_refused_even_when_everything_is_allowed() {
        let f = fixture();
        let gate = Arc::new(MockGate::allow_all().guarding(&f.workspace));
        let ctx = ctx(&f.workspace.join("files/work"), gate.clone());

        for place in [
            "../../secrets/secrets.json",
            "../../settings/permissions.json",
            "../../hooks/hooks.json",
            "../../hooks",
        ] {
            assert!(
                matches!(reach(&ctx, place).await, Err(Error::Denied(_))),
                "{place}"
            );
        }
        assert!(
            gate.asked().is_empty(),
            "nobody may be asked about it either"
        );
        assert!(reach(&ctx, "../../settings/app.json").await.is_ok());
    }

    /// A repository's hooks run on every turn in it, so an agent writing one
    /// into its own working folder would be running code on the next turn.
    #[tokio::test]
    async fn a_project_hooks_file_is_off_limits_in_any_folder() {
        let f = fixture();
        let gate = Arc::new(MockGate::allow_all().guarding(&f.workspace));
        let ctx = ctx(&f.project, gate);
        assert!(matches!(
            reach(&ctx, ".inertia/hooks.json").await,
            Err(Error::Denied(_))
        ));
        assert!(reach(&ctx, ".inertia/rules/style.md").await.is_ok());
        assert!(reach(&ctx, "hooks.json").await.is_ok());
    }

    #[tokio::test]
    async fn a_tree_holding_the_machinery_cannot_be_carried_off_whole() {
        let f = fixture();
        let gate = Arc::new(MockGate::allow_all().guarding(&f.workspace));
        let ctx = ctx(&f.project, gate);
        assert!(matches!(
            check_tree(&ctx, &f.workspace).await,
            Err(Error::Denied(_))
        ));
        assert!(check(&ctx, &f.workspace).await.is_ok());
    }

    #[tokio::test]
    async fn outside_the_working_folder_asks_once_per_folder() {
        let f = fixture();
        let gate =
            Arc::new(MockGate::new(Policy::Scripted(vec![Decision::Deny])).guarding(&f.workspace));
        let ctx = ctx(&f.project, gate.clone());

        assert!(reach(&ctx, "src/main.rs").await.is_ok());
        assert!(
            gate.asked().is_empty(),
            "the working folder is not asked about"
        );

        let file = f.outside.join("id_rsa");
        assert!(matches!(
            reach(&ctx, &file.to_string_lossy()).await,
            Err(Error::Denied(_))
        ));
        let asked = gate.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].key, "external_directory");
        assert!(asked[0].target.ends_with("outside/*"));
        assert_eq!(asked[0].always.as_deref(), Some(asked[0].target.as_str()));
    }

    #[tokio::test]
    async fn the_workspace_files_folder_counts_as_inside() {
        let f = fixture();
        let gate = Arc::new(MockGate::deny_all().guarding(&f.workspace));
        let ctx = ctx(&f.project, gate.clone());
        let attachment = f.workspace.join("files/attachment.png");
        assert!(reach(&ctx, &attachment.to_string_lossy()).await.is_ok());
        assert!(gate.asked().is_empty());
    }

    /// A gate with no workspace behind it - a test, a catalogue - fences
    /// nothing, and still asks about leaving the working folder.
    #[tokio::test]
    async fn without_a_workspace_only_the_working_folder_is_drawn() {
        let f = fixture();
        let gate = Arc::new(MockGate::allow_all());
        let ctx = ctx(&f.project, gate.clone());
        let secrets = f.workspace.join("secrets/secrets.json");
        assert!(reach(&ctx, &secrets.to_string_lossy()).await.is_ok());
        assert_eq!(gate.asked()[0].key, "external_directory");
    }
}

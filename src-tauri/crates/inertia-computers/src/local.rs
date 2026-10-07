//! The machine you are already sitting at.
//!
//! No daemon, no account, no image: a directory under the workspace and the
//! host's own shell. It exists for two reasons. It is the provider that works
//! offline on a laptop with nothing installed, which makes it the one that
//! proves the interface. And it is the one whose results a person can go and
//! look at in their own file manager, which is worth more than it sounds the
//! first time an agent says it wrote a file and you want to believe it.
//!
//! **It is not a sandbox, and it does not pretend to be.** A command run here
//! has the user's permissions and can reach anything the user can. What stands
//! between an agent and the rest of the disk is the permission ruleset, which
//! is a real mechanism and an honest one - but it is consent, not containment.
//! [`LocalProvider::available`] returns that as its reason so the settings
//! screen cannot offer this without saying it.
//!
//! The working directory is the containment that does exist: every path is
//! resolved inside the machine's own folder, and one that escapes it is refused
//! rather than followed. That stops an agent's relative path from wandering; it
//! does not stop an absolute one it was given permission to run.

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use base64::Engine;

use crate::{
    machine_name, ComputerError, Created, DirEntry, ExecRequest, ExecResult, Health, Provider,
    Readiness, Result, Snapshot, Spec, Stats, Status, DEFAULT_TIMEOUT,
};

/// Local machines live under the workspace, so they are backed up, diffable and
/// deletable with everything else.
#[derive(Debug, Clone)]
pub struct LocalProvider {
    root: PathBuf,
}

impl LocalProvider {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            root: workspace_root.into(),
        }
    }

    /// Where one machine's files live. The handle is this path.
    fn home_for(&self, id: &str) -> PathBuf {
        self.root.join("files/machines").join(id)
    }

    fn snapshots_dir(&self, handle: &str) -> PathBuf {
        PathBuf::from(format!("{handle}.snapshots"))
    }

    /// A path inside the machine, or a refusal.
    ///
    /// Resolved lexically rather than with `canonicalize`, because the path may
    /// not exist yet - a write creates it - and `canonicalize` fails on a path
    /// that is not there. That means a symlink already inside the folder can
    /// still point outward; the permission ruleset is what covers that, and
    /// this covers the far more common case of a relative path with `..` in it.
    fn inside(&self, handle: &str, path: &str) -> Result<PathBuf> {
        let home = PathBuf::from(handle);
        let candidate = if path.trim().is_empty() || path == "." {
            home.clone()
        } else {
            let given = Path::new(path);
            if given.is_absolute() {
                given.to_path_buf()
            } else {
                home.join(given)
            }
        };

        let resolved = normalize(&candidate);
        let base = normalize(&home);
        if !resolved.starts_with(&base) {
            return Err(ComputerError::Failed(format!(
                "{path} is outside this machine's folder."
            )));
        }
        Ok(resolved)
    }
}

/// Resolves `.` and `..` without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The shell, and the flag that makes it take a command line.
fn shell() -> (&'static str, Vec<&'static str>) {
    if cfg!(windows) {
        (
            "powershell.exe",
            vec!["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"],
        )
    } else {
        ("/bin/sh", vec!["-lc"])
    }
}

#[async_trait]
impl Provider for LocalProvider {
    fn id(&self) -> &'static str {
        "local"
    }

    fn label(&self) -> &'static str {
        "Local"
    }

    fn blurb(&self) -> &'static str {
        "This machine. No isolation - the agent has your permissions."
    }

    /// Ready, with a warning that cannot be dismissed by not reading it.
    async fn available(&self) -> Readiness {
        if self.root.as_os_str().is_empty() {
            return Readiness::not("A local computer needs a workspace to live in.");
        }
        // Carried on a ready answer on purpose. Nothing can stop this provider
        // from running, which is the point of it, so the answer is a warning
        // rather than a refusal - shown beside it in settings so nobody picks
        // it without reading what it means.
        Readiness::ready().warning(
            "Commands run on this computer with your permissions. Nothing contains them;              the permission rules are what decide whether one runs at all.",
        )
    }

    async fn create(&self, spec: &Spec) -> Result<Created> {
        let home = self.home_for(&spec.id);
        tokio::fs::create_dir_all(&home)
            .await
            .map_err(|e| ComputerError::Failed(e.to_string()))?;

        Ok(Created {
            handle: home.to_string_lossy().to_string(),
            name: machine_name(&spec.id),
            status: Status::Running,
            os: std::env::consts::OS.to_string(),
            image: "host".into(),
            workdir: home.to_string_lossy().to_string(),
        })
    }

    /// There is no daemon to start or stop. A local machine is running exactly
    /// when its folder is there, so the lifecycle calls succeed and change
    /// nothing rather than pretending to a state the folder cannot be in.
    async fn start(&self, _handle: &str) -> Result<()> {
        Ok(())
    }

    async fn stop(&self, _handle: &str) -> Result<()> {
        Ok(())
    }

    async fn pause(&self, _handle: &str) -> Result<()> {
        Ok(())
    }

    async fn resume(&self, _handle: &str) -> Result<()> {
        Ok(())
    }

    async fn remove(&self, handle: &str, _name: Option<&str>) -> Result<()> {
        let home = PathBuf::from(handle);
        // Guarded, because this deletes a directory tree and the handle is a
        // path: a handle that had somehow become empty or `/` would take the
        // disk with it. `..` is refused outright rather than resolved, because
        // `starts_with` compares components and would pass
        // `<machines>/../..` - the whole workspace. And it must be a folder
        // inside the machines folder, not the machines folder itself.
        let machines = self.root.join("files/machines");
        let escapes = home
            .components()
            .any(|part| matches!(part, Component::ParentDir));
        let inside = home
            .strip_prefix(&machines)
            .is_ok_and(|rest| rest.components().next().is_some());
        if escapes || !inside {
            return Err(ComputerError::Failed(
                "That machine is not in this workspace, so it was not removed.".into(),
            ));
        }
        let _ = tokio::fs::remove_dir_all(&home).await;
        let _ = tokio::fs::remove_dir_all(self.snapshots_dir(handle)).await;
        Ok(())
    }

    async fn status(&self, handle: &str) -> Health {
        Health {
            status: if Path::new(handle).is_dir() {
                Status::Running
            } else {
                Status::Missing
            },
            started_at: None,
        }
    }

    /// Nothing is reported.
    ///
    /// The host's own CPU and memory are not this machine's: every other
    /// process on the laptop is in those numbers, and a gauge that moved when
    /// the user opened a browser would be worse than no gauge.
    async fn stats(&self, _handle: &str) -> Stats {
        Stats {
            scope: "host".into(),
            ..Default::default()
        }
    }

    async fn exec(&self, handle: &str, request: &ExecRequest) -> Result<ExecResult> {
        let line = request.command.trim();
        if line.is_empty() {
            return Ok(ExecResult::default());
        }

        let cwd = match request.cwd.as_deref() {
            Some(path) if !path.is_empty() => self.inside(handle, path)?,
            _ => PathBuf::from(handle),
        };

        let (program, flags) = shell();
        let mut command = tokio::process::Command::new(program);
        command
            .args(flags)
            .arg(line)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("TERM", "dumb")
            .env("NO_COLOR", "1");
        for (key, value) in &request.env {
            command.env(key, value);
        }
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let timeout = request.timeout.unwrap_or(DEFAULT_TIMEOUT);
        let started = Instant::now();

        match tokio::time::timeout(timeout, command.output()).await {
            Err(_) => Ok(ExecResult {
                code: -1,
                stderr: format!("Timed out after {timeout:?}."),
                duration_ms: started.elapsed().as_millis() as u64,
                timed_out: true,
                ..Default::default()
            }),
            Ok(Err(e)) => Err(ComputerError::Failed(e.to_string())),
            Ok(Ok(output)) => Ok(ExecResult {
                code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                duration_ms: started.elapsed().as_millis() as u64,
                timed_out: false,
            }),
        }
    }

    async fn list_dir(&self, handle: &str, path: &str) -> Result<Vec<DirEntry>> {
        let dir = self.inside(handle, path)?;
        let mut reader = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| ComputerError::Failed(format!("Cannot read {path}: {e}")))?;

        let mut entries = Vec::new();
        while let Ok(Some(item)) = reader.next_entry().await {
            let meta = item.metadata().await.ok();
            let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
            entries.push(DirEntry {
                name: item.file_name().to_string_lossy().to_string(),
                kind: if is_dir { "dir".into() } else { "file".into() },
                size: if is_dir {
                    None
                } else {
                    meta.as_ref().map(|m| m.len())
                },
                modified_at: meta
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|d| {
                        jiff::Timestamp::from_millisecond(d.as_millis() as i64)
                            .ok()
                            .map(|t| t.to_string())
                    }),
            });
        }

        crate::docker::sort_entries(&mut entries);
        Ok(entries)
    }

    async fn read_file(&self, handle: &str, path: &str) -> Result<String> {
        let file = self.inside(handle, path)?;
        tokio::fs::read_to_string(&file)
            .await
            .map_err(|e| ComputerError::Failed(format!("Cannot read {path}: {e}")))
    }

    async fn read_file_base64(&self, handle: &str, path: &str) -> Result<String> {
        let file = self.inside(handle, path)?;
        let bytes = tokio::fs::read(&file)
            .await
            .map_err(|e| ComputerError::Failed(format!("Cannot read {path}: {e}")))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    async fn write_file(&self, handle: &str, path: &str, content: &str) -> Result<()> {
        let file = self.inside(handle, path)?;
        if let Some(parent) = file.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| ComputerError::Failed(e.to_string()))?;
        }
        tokio::fs::write(&file, content)
            .await
            .map_err(|e| ComputerError::Failed(format!("Cannot write {path}: {e}")))
    }

    /// A copy of the folder, beside it.
    ///
    /// Not a filesystem snapshot: this has to work on any disk, so it is a
    /// recursive copy and it is honest about costing what a copy costs.
    async fn snapshot(&self, handle: &str, name: &str) -> Result<Snapshot> {
        let id = format!("snap-{}", jiff::Timestamp::now().as_millisecond());
        let target = self.snapshots_dir(handle).join(&id);
        tokio::fs::create_dir_all(&target)
            .await
            .map_err(|e| ComputerError::Failed(e.to_string()))?;

        copy_tree(PathBuf::from(handle), target.clone()).await?;
        tokio::fs::write(target.join(".inertia-snapshot"), name)
            .await
            .map_err(|e| ComputerError::Failed(e.to_string()))?;

        Ok(Snapshot {
            id,
            name: name.to_string(),
            created_at: jiff::Timestamp::now().to_string(),
            size: None,
        })
    }

    async fn snapshots(&self, handle: &str) -> Result<Vec<Snapshot>> {
        let dir = self.snapshots_dir(handle);
        let Ok(mut reader) = tokio::fs::read_dir(&dir).await else {
            return Ok(Vec::new());
        };

        let mut out = Vec::new();
        while let Ok(Some(item)) = reader.next_entry().await {
            let id = item.file_name().to_string_lossy().to_string();
            let label = tokio::fs::read_to_string(item.path().join(".inertia-snapshot"))
                .await
                .unwrap_or_else(|_| id.clone());
            let created_at = item
                .metadata()
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|d| jiff::Timestamp::from_millisecond(d.as_millis() as i64).ok())
                .map(|t| t.to_string())
                .unwrap_or_default();
            out.push(Snapshot {
                id,
                name: label,
                created_at,
                size: None,
            });
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    async fn restore(&self, handle: &str, snapshot_id: &str, name: &str) -> Result<Created> {
        let source = self.snapshots_dir(handle).join(snapshot_id);
        if !source.is_dir() {
            return Err(ComputerError::Failed(format!(
                "There is no snapshot called {snapshot_id} any more."
            )));
        }

        // The folder is emptied and refilled rather than replaced, so the
        // handle - which is this path - stays valid and the record does not
        // have to be rewritten.
        let home = PathBuf::from(handle);
        let _ = tokio::fs::remove_dir_all(&home).await;
        tokio::fs::create_dir_all(&home)
            .await
            .map_err(|e| ComputerError::Failed(e.to_string()))?;
        copy_tree(source, home.clone()).await?;
        let _ = tokio::fs::remove_file(home.join(".inertia-snapshot")).await;

        Ok(Created {
            handle: handle.to_string(),
            name: name.to_string(),
            status: Status::Running,
            os: std::env::consts::OS.to_string(),
            image: "host".into(),
            workdir: handle.to_string(),
        })
    }

    /// There is no screen to take. The host's own display belongs to the
    /// person, not to the agent, and capturing it would be surveillance rather
    /// than a feature.
    async fn screenshot(&self, _handle: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

/// Copies a directory tree.
///
/// Iterative rather than recursive-async, which would need boxing on every
/// level for no benefit.
async fn copy_tree(from: PathBuf, to: PathBuf) -> Result<()> {
    let mut pending = vec![(from, to)];
    while let Some((source, target)) = pending.pop() {
        tokio::fs::create_dir_all(&target)
            .await
            .map_err(|e| ComputerError::Failed(e.to_string()))?;

        let Ok(mut reader) = tokio::fs::read_dir(&source).await else {
            continue;
        };
        while let Ok(Some(item)) = reader.next_entry().await {
            let name = item.file_name();
            let Ok(kind) = item.file_type().await else {
                continue;
            };
            if kind.is_dir() {
                // The snapshots folder is a sibling, not a child, so there is
                // no recursion to guard against here.
                pending.push((item.path(), target.join(name)));
            } else {
                let _ = tokio::fs::copy(item.path(), target.join(name)).await;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> (tempfile::TempDir, LocalProvider) {
        let dir = tempfile::tempdir().unwrap();
        let provider = LocalProvider::new(dir.path());
        (dir, provider)
    }

    #[tokio::test]
    async fn a_machine_is_a_folder_under_the_workspace() {
        let (dir, provider) = provider();
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(Path::new(&made.handle).is_dir());
        assert!(made
            .handle
            .starts_with(&dir.path().to_string_lossy().to_string()));
        assert_eq!(made.name, "inertia-box");
    }

    /// The containment that actually exists.
    #[tokio::test]
    async fn a_path_that_escapes_the_folder_is_refused() {
        let (_dir, provider) = provider();
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(provider.inside(&made.handle, "../../etc/passwd").is_err());
        assert!(provider.inside(&made.handle, "notes/../../..").is_err());
        assert!(provider.inside(&made.handle, "notes/deep.txt").is_ok());
        assert!(provider.inside(&made.handle, ".").is_ok());
    }

    #[tokio::test]
    async fn files_round_trip() {
        let (_dir, provider) = provider();
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        provider
            .write_file(&made.handle, "notes/a.txt", "hello")
            .await
            .unwrap();
        assert_eq!(
            provider
                .read_file(&made.handle, "notes/a.txt")
                .await
                .unwrap(),
            "hello"
        );

        let encoded = provider
            .read_file_base64(&made.handle, "notes/a.txt")
            .await
            .unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        assert_eq!(decoded, b"hello");
    }

    #[tokio::test]
    async fn a_listing_puts_directories_first() {
        let (_dir, provider) = provider();
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        provider
            .write_file(&made.handle, "zebra.txt", "z")
            .await
            .unwrap();
        provider
            .write_file(&made.handle, "src/main.rs", "fn main(){}")
            .await
            .unwrap();

        let entries = provider.list_dir(&made.handle, ".").await.unwrap();
        assert_eq!(entries[0].name, "src");
        assert_eq!(entries[0].kind, "dir");
        assert_eq!(entries[1].name, "zebra.txt");
        assert_eq!(entries[1].size, Some(1));
    }

    #[tokio::test]
    async fn a_missing_folder_reads_as_gone_rather_than_broken() {
        let (_dir, provider) = provider();
        let health = provider.status("/nowhere/at/all").await;
        assert_eq!(health.status, Status::Missing);
    }

    /// The handle is a path, so a removal has to prove it is ours first.
    #[tokio::test]
    async fn removing_something_outside_the_workspace_is_refused() {
        let (_dir, provider) = provider();
        let elsewhere = tempfile::tempdir().unwrap();
        let path = elsewhere.path().to_string_lossy().to_string();

        assert!(provider.remove(&path, None).await.is_err());
        assert!(elsewhere.path().is_dir(), "it must still be there");
    }

    /// A handle that starts inside the machines folder and climbs out of it
    /// with `..` still names somewhere else, and so does the machines folder
    /// itself, which holds every machine.
    #[tokio::test]
    async fn removing_by_a_path_that_climbs_out_is_refused() {
        let (dir, provider) = provider();
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let machines = dir.path().join("files/machines");

        let climbing = machines.join("box").join("..").join("..");
        assert!(provider
            .remove(&climbing.to_string_lossy(), None)
            .await
            .is_err());
        assert!(provider
            .remove(&machines.to_string_lossy(), None)
            .await
            .is_err());
        assert!(Path::new(&made.handle).is_dir(), "it must still be there");

        provider.remove(&made.handle, None).await.unwrap();
        assert!(!Path::new(&made.handle).exists());
    }

    #[tokio::test]
    async fn a_snapshot_can_be_restored_over_later_changes() {
        let (_dir, provider) = provider();
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        provider
            .write_file(&made.handle, "a.txt", "first")
            .await
            .unwrap();
        let snap = provider.snapshot(&made.handle, "before").await.unwrap();

        provider
            .write_file(&made.handle, "a.txt", "second")
            .await
            .unwrap();
        provider
            .write_file(&made.handle, "b.txt", "new")
            .await
            .unwrap();

        provider
            .restore(&made.handle, &snap.id, "inertia-box")
            .await
            .unwrap();
        assert_eq!(
            provider.read_file(&made.handle, "a.txt").await.unwrap(),
            "first"
        );
        assert!(
            provider.read_file(&made.handle, "b.txt").await.is_err(),
            "a file made after the snapshot should be gone"
        );
    }

    #[tokio::test]
    async fn snapshots_are_listed_newest_first_with_their_names() {
        let (_dir, provider) = provider();
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        provider.snapshot(&made.handle, "one").await.unwrap();
        let listed = provider.snapshots(&made.handle).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "one");
    }

    #[tokio::test]
    async fn a_command_runs_and_a_failure_is_a_result() {
        let (_dir, provider) = provider();
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let ok = provider
            .exec(
                &made.handle,
                &ExecRequest {
                    command: "echo hi".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(ok.ok());
        assert!(ok.stdout.contains("hi"));

        let failed = provider
            .exec(
                &made.handle,
                &ExecRequest {
                    command: "exit 3".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(failed.code, 3, "a non-zero exit is a result, not an error");
    }

    /// The warning is on the ready answer, so the settings screen cannot offer
    /// this provider without rendering it.
    #[tokio::test]
    async fn being_available_still_says_it_is_not_a_sandbox() {
        let (_dir, provider) = provider();
        let answer = provider.available().await;
        assert!(answer.ready);
        // On `warning`, not `reason`: the screen shows a reason only when a
        // provider cannot run, so a caveat put there would never be read.
        assert!(
            answer.warning.contains("with your permissions"),
            "{}",
            answer.warning
        );
        assert!(answer.reason.is_empty());
    }

    #[test]
    fn dot_segments_resolve_without_touching_the_disk() {
        assert_eq!(normalize(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("/a/./b")), PathBuf::from("/a/b"));
    }
}

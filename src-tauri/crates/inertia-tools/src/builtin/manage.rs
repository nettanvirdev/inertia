//! Moving, copying and organising files, as tools.
//!
//! Everything here was already possible through `shell`, which is exactly why
//! it is worth having: "put these screenshots in the project folder" became a
//! `robocopy` invocation on Windows and a `cp -r` on anything else, the model
//! guessed which shell it had, and the failure arrived as a non-zero exit code
//! with no hint about what went wrong. A tool that names its two paths can
//! check them before it touches anything, say "that would overwrite a folder"
//! in words, and ask the same permission question the file tools already ask.
//!
//! Four tools rather than one with an `action`: a schema carrying every
//! argument any action might need reads to a model as a menu, and it picks the
//! wrong item off a menu.
//!
//! What each one refuses to do:
//!
//!  - Nothing overwrites silently. A destination that exists is an error naming
//!    what is in the way, unless `overwrite` was passed - so the model has to
//!    have decided to replace it, in a field the user can see on the card.
//!  - A destination that is an existing directory means "into it", the way
//!    every file manager and every `mv` behaves. Guessing the other way -
//!    replacing a folder with a file - is not recoverable.
//!  - A directory cannot be copied or moved into itself, which is the loop that
//!    fills a disk before anyone notices.
//!  - Deleting a directory asks under `delete_everything`, the key that always
//!    asks. Deleting one file asks under `edit`, with the rest of the ways a
//!    file changes.
//!
//! Deletion is permanent on every platform. The Electron tool this ports used
//! `fs.rm`, not the recycle bin, and told the model so in its description; a
//! port that quietly started trashing instead would make "deleted" mean two
//! different things depending on which app the person was running.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use serde_json::{json, Value};

use super::fence;

/// A failure the model can act on. The registry turns it into an `ok: false`
/// result carrying exactly this text, which is what Electron's `ToolError` did.
fn refuse(message: impl Into<String>) -> Error {
    Error::Other(message.into())
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A required path argument, resolved against the workspace root.
///
/// Lexically normalised, the way `path.resolve` is: `..` and `.` are folded
/// away without touching the disk. That is what makes the "same file" and
/// "inside itself" checks below mean something - `a/b/..` and `a` have to
/// compare equal for either to fire.
fn resolve(root: &Path, args: &Value, field: &str) -> Result<PathBuf> {
    let raw = text(args, field).map(str::trim).unwrap_or_default();
    if raw.is_empty() {
        return Err(refuse(format!("{field} is required.")));
    }
    let supplied = Path::new(raw);
    let joined = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        root.join(supplied)
    };
    Ok(normalize(&joined))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Popping past the root is a no-op, as it is for every shell.
                if matches!(
                    out.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn stat_of(target: &Path) -> Option<std::fs::Metadata> {
    std::fs::metadata(target).ok()
}

fn basename(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn human(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

/// Where a transfer actually lands.
///
/// `destination` is a full path when it names something that does not exist,
/// and a containing folder when it is one that does. That single rule is what
/// makes "move this into my project" work without the model having to repeat
/// the file name, and it is what people already expect from `mv`.
fn landing(source: &Path, destination: &Path) -> PathBuf {
    match stat_of(destination) {
        Some(stat) if stat.is_dir() => destination.join(basename(source)),
        _ => destination.to_path_buf(),
    }
}

struct Prepared {
    source: PathBuf,
    target: PathBuf,
    from: std::fs::Metadata,
    replacing: bool,
}

/// Everything a copy and a move check before touching anything.
async fn prepare(args: &Value, ctx: &ToolContext, verb: &str) -> Result<Prepared> {
    let source = resolve(&ctx.root, args, "source")?;
    let requested = resolve(&ctx.root, args, "destination")?;
    // The source goes whole, so it may not carry an off-limits folder along
    // inside it.
    fence::check_tree(ctx, &source).await?;

    let Some(from) = stat_of(&source) else {
        return Err(refuse(format!("Nothing to {verb} at {}", source.display())));
    };

    // Checked where it lands rather than where it was pointed: copying a
    // folder called `hooks` into the workspace folder writes the hooks.
    let target = landing(&source, &requested);
    fence::check(ctx, &target).await?;
    if source == target {
        return Err(refuse(format!(
            "The source and the destination are the same file: {}",
            source.display()
        )));
    }
    // `starts_with` is component-wise, so `work` is not a prefix of `workspace`
    // the way a string comparison would claim.
    if from.is_dir() && target.starts_with(&source) {
        return Err(refuse(format!(
            "{} is inside {}, so this would {verb} the folder into itself.",
            target.display(),
            source.display()
        )));
    }

    let onto = stat_of(&target);
    if let Some(existing) = &onto {
        if !flag(args, "overwrite") {
            let what = if existing.is_dir() {
                "a folder".to_string()
            } else {
                human(existing.len())
            };
            return Err(refuse(format!(
                "{} already exists ({what}). Pass overwrite: true to replace it, or choose \
                 another destination.",
                target.display()
            )));
        }
        if existing.is_dir() && !from.is_dir() {
            return Err(refuse(format!(
                "{} is a folder, and a file cannot replace it.",
                target.display()
            )));
        }
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            refuse(format!("Could not create {}: {e}", parent.display()))
        })?;
    }

    Ok(Prepared {
        source,
        target,
        from,
        replacing: onto.is_some(),
    })
}

fn report(verb: &str, done: &Prepared) -> ToolOutcome {
    let directory = done.from.is_dir();
    let what = if directory {
        "folder".to_string()
    } else {
        human(done.from.len())
    };
    ToolOutcome {
        title: Some(basename(&done.target)),
        output: format!(
            "{verb} {} to {} ({what}){}.",
            done.source.display(),
            done.target.display(),
            if done.replacing {
                ", replacing what was there"
            } else {
                ""
            }
        ),
        metadata: Some(json!({
            "source": done.source.display().to_string(),
            "path": done.target.display().to_string(),
            "directory": directory,
            "bytes": if directory { None } else { Some(done.from.len()) },
            "replaced": done.replacing,
        })),
        images: Vec::new(),
    }
}

/// `cp -r` with overwrite: files are replaced, folders are merged into.
fn copy_recursively(source: &Path, target: &Path) -> std::io::Result<()> {
    if !source.is_dir() {
        std::fs::copy(source, target)?;
        return Ok(());
    }
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        copy_recursively(&entry.path(), &target.join(entry.file_name()))?;
    }
    Ok(())
}

/// `rm -rf`: a missing path is already the state that was asked for.
fn remove_recursively(target: &Path) -> std::io::Result<()> {
    let result = match stat_of(target) {
        Some(stat) if stat.is_dir() => std::fs::remove_dir_all(target),
        Some(_) => std::fs::remove_file(target),
        None => return Ok(()),
    };
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Whether a rename failed because the two paths live on different volumes.
///
/// `EXDEV` on Unix, `ERROR_NOT_SAME_DEVICE` on Windows. Nothing else is worth
/// retrying as copy-then-delete: a permission error will fail the copy too.
fn crosses_devices(error: &std::io::Error) -> bool {
    let code = if cfg!(windows) { 17 } else { 18 };
    error.raw_os_error() == Some(code)
}

/// The permission every tool here asks for: a change to files, remembered per
/// exact path, the same way `write` and `edit` are.
fn edit_permission(args: &Value, field: &str) -> PermissionRequest {
    let target = text(args, field).unwrap_or(inertia_core::permission::ANY);
    PermissionRequest::new("edit", target).with_always(target)
}

fn transfer_parameters(verb: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "source": {
                "type": "string",
                "description": format!("The absolute path of the file or folder to {verb}")
            },
            "destination": {
                "type": "string",
                "description": format!("The absolute path to {verb} it to, or an existing folder to {verb} it into")
            },
            "overwrite": {
                "type": "boolean",
                "description": "Replace the destination if something is already there. Defaults to false"
            }
        },
        "required": ["source", "destination"],
        "additionalProperties": false
    })
}

fn transfer_render(args: &Value) -> Option<String> {
    Some(format!(
        "{} to {}",
        text(args, "source").unwrap_or_default(),
        text(args, "destination").unwrap_or_default()
    ))
}

// ── file_copy ───────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FileCopyTool;

#[async_trait]
impl Tool for FileCopyTool {
    fn id(&self) -> &str {
        "file_copy"
    }

    fn description(&self) -> &str {
        "Copy a file or a whole folder to another place on this machine, keeping the original. \
         The destination may be the new path, or an existing folder to copy into. Creates any \
         missing parent folders. Refuses to replace something that already exists unless \
         overwrite is true. Use this rather than a shell command: it works the same on every \
         platform and says what went wrong."
    }

    fn parameters(&self) -> Value {
        transfer_parameters("copy")
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        edit_permission(args, "destination")
    }

    fn render(&self, args: &Value) -> Option<String> {
        transfer_render(args)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let prepared = prepare(&args, ctx, "copy").await?;
        copy_recursively(&prepared.source, &prepared.target).map_err(|e| {
            refuse(format!(
                "Could not copy {} to {}: {e}",
                prepared.source.display(),
                prepared.target.display()
            ))
        })?;
        Ok(report("Copied", &prepared))
    }
}

// ── file_move ───────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FileMoveTool;

#[async_trait]
impl Tool for FileMoveTool {
    fn id(&self) -> &str {
        "file_move"
    }

    fn description(&self) -> &str {
        "Move or rename a file or folder on this machine. The destination may be the new path, \
         or an existing folder to move it into. Creates any missing parent folders. Refuses to \
         replace something that already exists unless overwrite is true."
    }

    fn parameters(&self) -> Value {
        transfer_parameters("move")
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        edit_permission(args, "destination")
    }

    fn render(&self, args: &Value) -> Option<String> {
        transfer_render(args)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let prepared = prepare(&args, ctx, "move").await?;
        let describe = |e: std::io::Error| {
            refuse(format!(
                "Could not move {} to {}: {e}",
                prepared.source.display(),
                prepared.target.display()
            ))
        };

        if prepared.replacing {
            remove_recursively(&prepared.target).map_err(describe)?;
        }
        if let Err(error) = std::fs::rename(&prepared.source, &prepared.target) {
            // Across drives - D: to C:, or out of a container mount - rename
            // cannot work at all, and the copy-then-delete it falls back to is
            // what every file manager does. Anything else is a real failure.
            if !crosses_devices(&error) {
                return Err(describe(error));
            }
            copy_recursively(&prepared.source, &prepared.target).map_err(describe)?;
            remove_recursively(&prepared.source).map_err(describe)?;
        }
        Ok(report("Moved", &prepared))
    }
}

// ── file_folder ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FileFolderTool;

#[async_trait]
impl Tool for FileFolderTool {
    fn id(&self) -> &str {
        "file_folder"
    }

    fn description(&self) -> &str {
        "Create a folder on this machine, including any missing parents. Does nothing if it \
         already exists."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The absolute path of the folder to create" }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        edit_permission(args, "path")
    }

    fn render(&self, args: &Value) -> Option<String> {
        text(args, "path").map(str::to_string)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let target = resolve(&ctx.root, &args, "path")?;
        fence::check(ctx, &target).await?;
        let existing = stat_of(&target);
        if let Some(stat) = &existing {
            if !stat.is_dir() {
                return Err(refuse(format!(
                    "{} is a file, so a folder cannot be made there.",
                    target.display()
                )));
            }
        }
        std::fs::create_dir_all(&target)
            .map_err(|e| refuse(format!("Could not create {}: {e}", target.display())))?;

        let created = existing.is_none();
        Ok(ToolOutcome {
            title: Some(basename(&target)),
            output: if created {
                format!("Created {}.", target.display())
            } else {
                format!("{} already exists.", target.display())
            },
            metadata: Some(json!({ "path": target.display().to_string(), "created": created })),
            images: Vec::new(),
        })
    }
}

// ── file_delete ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FileDeleteTool;

#[async_trait]
impl Tool for FileDeleteTool {
    fn id(&self) -> &str {
        "file_delete"
    }

    fn description(&self) -> &str {
        "Delete a file, or a folder and everything in it. This is permanent - nothing goes to \
         the recycle bin - so deleting a folder always asks first. Pass recursive: true to \
         delete a folder."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The absolute path of the file or folder to delete" },
                "recursive": { "type": "boolean", "description": "Required to delete a folder and everything inside it" }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    // One file is a change to files, so it asks under `edit` with the card
    // the tool definition names. A folder is a sweep, and a sweep asks again
    // under the key that never stops asking - decided in `execute`, where the
    // path has actually been looked at.
    fn permission(&self, args: &Value) -> PermissionRequest {
        edit_permission(args, "path")
    }

    fn render(&self, args: &Value) -> Option<String> {
        text(args, "path").map(str::to_string)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let target = resolve(&ctx.root, &args, "path")?;
        fence::check_tree(ctx, &target).await?;
        let Some(stat) = stat_of(&target) else {
            return Err(refuse(format!("Nothing to delete at {}", target.display())));
        };

        if stat.is_dir() {
            if !flag(&args, "recursive") {
                return Err(refuse(format!(
                    "{} is a folder. Pass recursive: true to delete it and everything in it.",
                    target.display()
                )));
            }
            let inside = std::fs::read_dir(&target)
                .map(|entries| entries.count())
                .unwrap_or(0);
            // No `always`: this is the key that asks every time, and a grant
            // that could be remembered would make it a different key.
            let sweep = PermissionRequest::new("delete_everything", target.display().to_string());
            if !ctx.permissions.ask(&sweep).await?.is_allowed() {
                return Err(Error::Denied(format!(
                    "Delete {} and the {inside} {} in it",
                    target.display(),
                    if inside == 1 { "entry" } else { "entries" }
                )));
            }
        }

        remove_recursively(&target)
            .map_err(|e| refuse(format!("Could not delete {}: {e}", target.display())))?;

        Ok(ToolOutcome {
            title: Some(basename(&target)),
            output: if stat.is_dir() {
                format!("Deleted the folder {}.", target.display())
            } else {
                format!("Deleted {}.", target.display())
            },
            metadata: Some(json!({
                "path": target.display().to_string(),
                "directory": stat.is_dir(),
            })),
            images: Vec::new(),
        })
    }
}

/// The four, in the order the Electron app listed them.
pub fn manage_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(FileCopyTool),
        Arc::new(FileMoveTool),
        Arc::new(FileFolderTool),
        Arc::new(FileDeleteTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::tool::Decision;
    use inertia_mock::{MockGate, Policy};

    fn ctx(root: &Path, gate: Arc<MockGate>) -> ToolContext {
        ToolContext {
            root: root.to_path_buf(),
            session: SessionId::from_existing("s1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: gate,
        }
    }

    fn make(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let file = dir.join(name);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("dirs");
        std::fs::write(&file, contents).expect("file");
        file
    }

    fn display(path: &Path) -> String {
        path.display().to_string()
    }

    fn message(error: Error) -> String {
        error.to_string()
    }

    #[tokio::test]
    async fn copies_a_file_and_leaves_the_original_where_it_was() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = make(dir.path(), "shot.png", "png-bytes");
        let destination = dir.path().join("assets").join("shot.png");

        let out = FileCopyTool
            .execute(
                json!({ "source": display(&source), "destination": display(&destination) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect("the copy ran");

        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "png-bytes");
        assert!(source.exists());
        // The parent folder did not exist a moment ago; a copy that has to be
        // preceded by a mkdir is two calls where the model only wanted one.
        let metadata = out.metadata.expect("metadata");
        assert_eq!(metadata["path"], json!(display(&destination)));
        assert_eq!(metadata["directory"], json!(false));
        assert_eq!(metadata["bytes"], json!(9));
        assert_eq!(metadata["replaced"], json!(false));
        assert_eq!(out.title.as_deref(), Some("shot.png"));
        assert_eq!(
            out.output,
            format!("Copied {} to {} (9 B).", display(&source), display(&destination))
        );
    }

    #[tokio::test]
    async fn a_destination_that_is_an_existing_folder_is_somewhere_to_put_the_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = make(dir.path(), "notes.md", "hello\n");
        let inbox = dir.path().join("inbox");
        std::fs::create_dir(&inbox).unwrap();

        let out = FileMoveTool
            .execute(
                json!({ "source": display(&source), "destination": display(&inbox) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect("the move ran");

        assert!(inbox.join("notes.md").exists());
        assert!(!source.exists());
        assert!(out.output.starts_with("Moved "));
        assert_eq!(out.metadata.unwrap()["path"], json!(display(&inbox.join("notes.md"))));
    }

    #[tokio::test]
    async fn a_move_to_a_new_name_is_a_rename() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = make(dir.path(), "draft.md", "hello\n");
        let renamed = dir.path().join("final.md");

        FileMoveTool
            .execute(
                json!({ "source": display(&source), "destination": display(&renamed) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect("the rename ran");

        assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "hello\n");
        assert!(!source.exists());
    }

    #[tokio::test]
    async fn copies_a_folder_and_everything_under_it() {
        let dir = tempfile::tempdir().expect("a temp dir");
        make(dir.path(), "shots/a/one.txt", "one");
        make(dir.path(), "shots/b/two.txt", "two");
        let backup = dir.path().join("backup");

        let out = FileCopyTool
            .execute(
                json!({ "source": display(&dir.path().join("shots")), "destination": display(&backup) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect("the copy ran");

        assert_eq!(std::fs::read_to_string(backup.join("a/one.txt")).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(backup.join("b/two.txt")).unwrap(), "two");
        let metadata = out.metadata.expect("metadata");
        assert_eq!(metadata["directory"], json!(true));
        assert_eq!(metadata["bytes"], json!(null));
        assert!(out.output.ends_with("(folder)."), "{}", out.output);
    }

    #[tokio::test]
    async fn will_not_overwrite_by_accident_and_says_what_is_in_the_way() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = make(dir.path(), "a.txt", "new");
        let blocked = make(dir.path(), "b.txt", "already here");
        let gate = Arc::new(MockGate::allow_all());

        let refused = FileCopyTool
            .execute(
                json!({ "source": display(&source), "destination": display(&blocked) }),
                &ctx(dir.path(), gate.clone()),
            )
            .await
            .expect_err("the destination exists");
        let text = message(refused);
        assert!(text.contains("already exists (12 B)"), "{text}");
        assert!(text.contains("Pass overwrite: true"), "{text}");
        assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "already here");

        let allowed = FileCopyTool
            .execute(
                json!({ "source": display(&source), "destination": display(&blocked), "overwrite": true }),
                &ctx(dir.path(), gate),
            )
            .await
            .expect("overwrite was asked for");
        assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "new");
        assert_eq!(allowed.metadata.unwrap()["replaced"], json!(true));
        assert!(allowed.output.ends_with(", replacing what was there."), "{}", allowed.output);
    }

    #[tokio::test]
    async fn a_file_cannot_replace_a_folder_even_with_overwrite() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let source = make(dir.path(), "a.txt", "new");
        let folder = dir.path().join("keep");
        // The landing rule puts the file inside, so the collision has to be
        // with a folder that carries the file's own name.
        std::fs::create_dir_all(folder.join("a.txt")).unwrap();

        let err = FileCopyTool
            .execute(
                json!({ "source": display(&source), "destination": display(&folder), "overwrite": true }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect_err("a folder is in the way");
        assert!(message(err).contains("a file cannot replace it"));
    }

    #[tokio::test]
    async fn refuses_to_move_a_folder_inside_itself() {
        let dir = tempfile::tempdir().expect("a temp dir");
        make(dir.path(), "work/one.txt", "hello\n");
        let work = dir.path().join("work");

        let err = FileMoveTool
            .execute(
                json!({ "source": display(&work), "destination": display(&work.join("nested")) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect_err("into itself");
        assert!(message(err).contains("into itself"));
        assert!(work.join("one.txt").exists());
    }

    #[tokio::test]
    async fn a_sibling_whose_name_extends_the_source_is_not_inside_it() {
        let dir = tempfile::tempdir().expect("a temp dir");
        make(dir.path(), "work/one.txt", "hello\n");
        let work = dir.path().join("work");
        let workspace = dir.path().join("workspace");

        FileMoveTool
            .execute(
                json!({ "source": display(&work), "destination": display(&workspace) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect("a string-prefix is not a path-prefix");
        assert!(workspace.join("one.txt").exists());
    }

    #[tokio::test]
    async fn says_so_when_the_source_is_not_there() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let ghost = dir.path().join("ghost.txt");
        let err = FileMoveTool
            .execute(
                json!({ "source": display(&ghost), "destination": display(&dir.path().join("x.txt")) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect_err("nothing there");
        assert_eq!(message(err), format!("Nothing to move at {}", display(&ghost)));
    }

    #[tokio::test]
    async fn a_relative_path_is_resolved_against_the_root() {
        let dir = tempfile::tempdir().expect("a temp dir");
        make(dir.path(), "a.txt", "x");
        let out = FileCopyTool
            .execute(
                json!({ "source": "a.txt", "destination": "sub/../copy.txt" }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect("the copy ran");
        assert!(dir.path().join("copy.txt").exists());
        assert_eq!(out.metadata.unwrap()["path"], json!(display(&dir.path().join("copy.txt"))));
    }

    #[tokio::test]
    async fn makes_a_folder_and_is_content_when_it_is_already_there() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let target = dir.path().join("one").join("two").join("three");
        let gate = Arc::new(MockGate::allow_all());

        let first = FileFolderTool
            .execute(json!({ "path": display(&target) }), &ctx(dir.path(), gate.clone()))
            .await
            .expect("created");
        assert_eq!(first.metadata.unwrap()["created"], json!(true));
        assert_eq!(first.output, format!("Created {}.", display(&target)));
        assert!(target.is_dir());

        let second = FileFolderTool
            .execute(json!({ "path": display(&target) }), &ctx(dir.path(), gate))
            .await
            .expect("already there is fine");
        assert_eq!(second.metadata.unwrap()["created"], json!(false));
        assert_eq!(second.output, format!("{} already exists.", display(&target)));
    }

    #[tokio::test]
    async fn a_folder_cannot_be_made_where_a_file_is() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = make(dir.path(), "taken", "x");
        let err = FileFolderTool
            .execute(
                json!({ "path": display(&file) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect_err("a file is there");
        assert!(message(err).contains("is a file, so a folder cannot be made there"));
    }

    #[tokio::test]
    async fn deletes_one_file_without_asking_a_second_question() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = make(dir.path(), "scratch.txt", "x");
        let gate = Arc::new(MockGate::allow_all());

        let out = FileDeleteTool
            .execute(json!({ "path": display(&file) }), &ctx(dir.path(), gate.clone()))
            .await
            .expect("deleted");

        assert!(!file.exists());
        assert_eq!(out.output, format!("Deleted {}.", display(&file)));
        assert_eq!(out.metadata.unwrap()["directory"], json!(false));
        assert!(gate.asked().iter().all(|q| q.key != "delete_everything"));
    }

    #[tokio::test]
    async fn will_not_delete_a_folder_unless_asked_to_and_then_asks_under_the_sweep_key() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let out = dir.path().join("out");
        make(dir.path(), "out/a", "a");
        make(dir.path(), "out/b", "b");
        let gate = Arc::new(MockGate::allow_all());

        let refused = FileDeleteTool
            .execute(json!({ "path": display(&out) }), &ctx(dir.path(), gate.clone()))
            .await
            .expect_err("a folder without recursive");
        assert_eq!(
            message(refused),
            format!(
                "{} is a folder. Pass recursive: true to delete it and everything in it.",
                display(&out)
            )
        );
        assert!(out.join("a").exists());
        assert!(gate.asked().is_empty(), "refusing must not raise a prompt");

        let done = FileDeleteTool
            .execute(
                json!({ "path": display(&out), "recursive": true }),
                &ctx(dir.path(), gate.clone()),
            )
            .await
            .expect("deleted");
        assert!(!out.exists());
        assert_eq!(done.output, format!("Deleted the folder {}.", display(&out)));
        assert_eq!(done.metadata.unwrap()["directory"], json!(true));

        let sweep = gate
            .asked()
            .into_iter()
            .find(|q| q.key == "delete_everything")
            .expect("the sweep key was asked");
        assert_eq!(sweep.target, display(&out));
        assert_eq!(sweep.always, None, "the sweep key is never remembered");
    }

    #[tokio::test]
    async fn a_refused_sweep_leaves_the_folder_alone() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let out = dir.path().join("out");
        make(dir.path(), "out/a", "a");
        let gate = Arc::new(MockGate::new(Policy::Scripted(vec![Decision::Deny])));

        let err = FileDeleteTool
            .execute(
                json!({ "path": display(&out), "recursive": true }),
                &ctx(dir.path(), gate),
            )
            .await
            .expect_err("the person said no");
        assert!(matches!(err, Error::Denied(_)), "{err}");
        // The count is the point of the card: "and the 1 entry in it" is what
        // tells someone whether they are about to lose something.
        assert!(message(err).contains("the 1 entry in it"));
        assert!(out.join("a").exists());
    }

    #[tokio::test]
    async fn nothing_to_delete_says_so() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let ghost = dir.path().join("ghost");
        let err = FileDeleteTool
            .execute(
                json!({ "path": display(&ghost) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .expect_err("nothing there");
        assert_eq!(message(err), format!("Nothing to delete at {}", display(&ghost)));
    }

    // The permission descriptors are what rules are written against, so their
    // shape is part of the contract with the renderer and the settings pane.
    #[test]
    fn every_tool_asks_under_edit_remembered_per_destination() {
        let copy = FileCopyTool.permission(&json!({ "source": "/a", "destination": "/b" }));
        assert_eq!((copy.key.as_str(), copy.target.as_str()), ("edit", "/b"));
        assert_eq!(copy.always.as_deref(), Some("/b"));

        let folder = FileFolderTool.permission(&json!({ "path": "/c" }));
        assert_eq!((folder.key.as_str(), folder.target.as_str()), ("edit", "/c"));

        let delete = FileDeleteTool.permission(&json!({ "path": "/d" }));
        assert_eq!((delete.key.as_str(), delete.target.as_str()), ("edit", "/d"));
        assert_eq!(delete.always.as_deref(), Some("/d"));
    }

    #[test]
    fn the_ids_and_labels_match_the_electron_tools() {
        let tools = manage_tools();
        let ids: Vec<&str> = tools.iter().map(|t| t.id()).collect();
        assert_eq!(ids, vec!["file_copy", "file_move", "file_folder", "file_delete"]);
        assert_eq!(
            FileMoveTool.render(&json!({ "source": "a", "destination": "b" })).as_deref(),
            Some("a to b")
        );
        assert_eq!(FileDeleteTool.render(&json!({ "path": "x" })).as_deref(), Some("x"));
    }

    #[test]
    fn sizes_read_the_way_electron_printed_them() {
        assert_eq!(human(9), "9 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(3 * 1024 * 1024), "3.0 MB");
    }
}

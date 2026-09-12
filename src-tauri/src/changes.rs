//! What a turn changed, and how to take it back.
//!
//! An agent that edits files leaves the person with two questions: what did it
//! touch, and can I undo that. Until this existed the answers were "read the
//! tool cards" and "git, if you had committed first".
//!
//! The shape is borrowed from Electron's shadow-git snapshot, but the
//! machinery is not: this is a small content-addressed store of its own.
//! A snapshot walks the working folder, writes every file's bytes into
//! `cache/snapshots/<folder>/blobs/` under a key derived from its contents,
//! and writes the list of `key -> path` as a tree under `trees/`. The tree's
//! own id is derived from that listing, so two identical trees have the same
//! id and "nothing changed" is a string comparison. The difference between
//! two trees is exactly the list of files a turn changed, and putting a tree
//! back on disk is a revert.
//!
//! Why not shell out to git, which Electron did: an agent's working folder is
//! very often not a git repository, git is not always installed, and a
//! `git add -A` over a project the size of this one is slower than every
//! non-write step of the turn put together. The blob store makes the second
//! snapshot of a turn cost only the files that changed, because unchanged
//! contents hash to keys that are already written.
//!
//! Two things this is not. It is not a commit history: the project's own git
//! is for that, and this never writes to it. And it is not a backup: the store
//! lives under `cache/`, is safe to delete, and only ever holds what a turn
//! was about to change.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};

/// Folders no snapshot should ever contain.
///
/// The same list `look`'s tree walk skips, plus the ones that only matter when
/// something is going to be *copied*: a `.venv` is a gigabyte of files nobody
/// wants to revert, and the app's own cache under a working folder pointed at
/// the workspace would snapshot the snapshot store and grow every time.
const SKIP: &[&str] = &[
    "node_modules",
    ".git",
    ".hg",
    ".svn",
    "dist",
    "build",
    "out",
    ".next",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".cache",
    ".turbo",
    ".gradle",
    ".idea",
    "vendor",
];

/// Files this big are listed but never stored.
///
/// A snapshot that copies a 400MB model weight file to answer "what did the
/// turn change" costs more than the turn did. They are left out of the tree
/// entirely, which means a change to one is not reported and not reverted -
/// the honest trade, and the alternative is a turn that stalls on `write`.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// A folder with more files than this is not snapshotted at all.
///
/// The walk stops and the turn gets no changes strip, rather than spending a
/// minute of the person's time on a folder somebody pointed at their home
/// directory.
const MAX_FILES: usize = 20_000;

/// Deeper than this and it is a symlink loop or a generated tree, either way
/// not something worth reverting.
const MAX_DEPTH: usize = 24;

/* -- the store ------------------------------------------------------------ */

/// A 64-bit FNV-1a, paired with the length of what was hashed.
///
/// Not a cryptographic hash and not trying to be: these are cache keys, and
/// nothing downstream trusts them as proof of anything. Written by hand
/// rather than pulling a hashing crate into the app crate, which has none -
/// FNV is eight lines and the failure mode of a collision here (two different
/// files of the *same length* colliding in 64 bits) is not one a project of
/// twenty thousand files will meet.
fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{:x}-{hash:016x}", bytes.len())
}

/// Where the store for one working folder lives.
///
/// Keyed by the folder rather than shared, so two projects never share a tree
/// namespace, and lowercased first because Windows hands the same folder back
/// with different capitalisation depending on who asked.
fn store_for(root: &Path, cwd: &Path) -> PathBuf {
    let key = fingerprint(cwd.display().to_string().to_lowercase().as_bytes());
    let mut path = root.to_path_buf();
    // One component at a time: a joined "cache/snapshots" keeps its embedded
    // slash inside the PathBuf on Windows and compares unequal to the same
    // path built properly.
    path.push("cache");
    path.push("snapshots");
    path.push(key);
    path
}

fn blobs(store: &Path) -> PathBuf {
    let mut path = store.to_path_buf();
    path.push("blobs");
    path
}

fn trees(store: &Path) -> PathBuf {
    let mut path = store.to_path_buf();
    path.push("trees");
    path
}

fn prepare(root: &Path, cwd: &Path) -> Option<PathBuf> {
    if !cwd.is_dir() {
        return None;
    }
    let store = store_for(root, cwd);
    std::fs::create_dir_all(blobs(&store)).ok()?;
    std::fs::create_dir_all(trees(&store)).ok()?;
    Some(store)
}

/// Whether snapshots can be taken for this folder at all.
pub fn available(root: &Path, cwd: &Path) -> bool {
    prepare(root, cwd).is_some()
}

/// One tree: the relative path of every file, and the blob holding its bytes.
type Tree = BTreeMap<String, String>;

fn write_tree(store: &Path, tree: &Tree) -> Option<String> {
    let listing: String = tree
        .iter()
        .map(|(path, blob)| format!("{blob}\t{path}\n"))
        .collect();
    let id = fingerprint(listing.as_bytes());
    let mut file = trees(store);
    file.push(&id);
    if !file.exists() {
        std::fs::write(&file, listing.as_bytes()).ok()?;
    }
    Some(id)
}

fn read_tree(store: &Path, id: &str) -> Option<Tree> {
    if !is_id(id) {
        return None;
    }
    let mut file = trees(store);
    file.push(id);
    let listing = std::fs::read_to_string(file).ok()?;
    let mut tree = Tree::new();
    for line in listing.lines() {
        if let Some((blob, path)) = line.split_once('\t') {
            tree.insert(path.to_string(), blob.to_string());
        }
    }
    Some(tree)
}

/// Ids and blob keys are ours, and they end up in a path. A renderer that
/// sends `../../settings.json` as a snapshot id must read nothing.
fn is_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn read_blob(store: &Path, key: &str) -> Option<Vec<u8>> {
    if !is_id(key) {
        return None;
    }
    let mut file = blobs(store);
    file.push(key);
    std::fs::read(file).ok()
}

/* -- taking one ----------------------------------------------------------- */

/// Snapshot the working folder. `None` when a snapshot is not possible or not
/// worth it here - no such folder, or a tree too large to walk.
///
/// Call it lazily, before the first tool that could change a file, so a turn
/// that only reads costs nothing.
pub fn track(root: &Path, cwd: &Path) -> Option<String> {
    let store = prepare(root, cwd)?;
    let mut tree = Tree::new();
    let skip_store = store_for(root, cwd);
    if !walk(cwd, cwd, &skip_store, 0, &mut tree, &store) {
        // Too many files. No tree is written, so `changes` has nothing to
        // compare against and the strip simply does not appear.
        return None;
    }
    write_tree(&store, &tree)
}

/// Returns false when the walk hit the file cap and the snapshot is abandoned.
fn walk(
    cwd: &Path,
    dir: &Path,
    skip_store: &Path,
    depth: usize,
    tree: &mut Tree,
    store: &Path,
) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        // A folder we cannot read is a folder we cannot revert. Not fatal:
        // the rest of the tree is still worth having.
        return true;
    };

    for entry in entries.flatten() {
        if tree.len() > MAX_FILES {
            return false;
        }
        let path = entry.path();
        // A working folder that contains the workspace - somebody pointing an
        // agent at the Inertia folder itself - would otherwise snapshot the
        // snapshot store, and each snapshot would grow the next.
        if path == skip_store {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name().to_string_lossy().to_string();

        if meta.is_symlink() {
            // Following one is how a walk ends up outside the folder it was
            // asked about, or in a loop.
            continue;
        }
        if meta.is_dir() {
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            if !walk(cwd, &path, skip_store, depth + 1, tree, store) {
                return false;
            }
            continue;
        }
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        if name.ends_with(".log") {
            continue;
        }

        let Some(relative) = relative_of(cwd, &path) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let key = fingerprint(&bytes);
        let mut blob = blobs(store);
        blob.push(&key);
        // Content-addressed, so the common case - a file that did not change
        // between two snapshots - writes nothing at all.
        if !blob.exists() {
            let _ = std::fs::write(&blob, &bytes);
        }
        tree.insert(relative, key);
    }
    true
}

/// The path a file is known by inside a tree: relative to the working folder,
/// with forward slashes, which is what the renderer shows and what a diff
/// header carries.
fn relative_of(cwd: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(cwd).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            // Anything else means the path left the folder, and nothing that
            // did belongs in a tree we will later write back out.
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The absolute path of a tree entry, rebuilt component by component.
///
/// Never `cwd.join(relative)`: a joined path with embedded separators is a
/// different `PathBuf` on Windows to the same path built properly, and this
/// one is about to be written to.
fn absolute_of(cwd: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = cwd.to_path_buf();
    for part in relative.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

/* -- what changed --------------------------------------------------------- */

/// The files that differ between two snapshots, with line counts.
///
/// `to` may be omitted to compare against the folder as it is now, which is
/// what "what did this turn change" asks at the end of a turn. The answer is
/// `{ "files": [...], "to": id }`, exactly the shape `ChangesStrip` reads.
pub fn changes(root: &Path, cwd: &Path, from: &str, to: Option<&str>) -> Value {
    let Some(store) = prepare(root, cwd) else {
        return json!({ "files": [], "to": Value::Null });
    };
    let target = match to {
        Some(id) => Some(id.to_string()),
        None => track(root, cwd),
    };
    let (Some(before), Some(target)) = (read_tree(&store, from), target) else {
        return json!({ "files": [], "to": to.map(Value::from).unwrap_or(Value::Null) });
    };
    if target == from {
        return json!({ "files": [], "to": target });
    }
    let Some(after) = read_tree(&store, &target) else {
        return json!({ "files": [], "to": target });
    };

    let mut files = Vec::new();
    for (path, blob) in &before {
        match after.get(path) {
            None => files.push(entry(&store, path, "D", Some(blob), None)),
            Some(now) if now != blob => {
                files.push(entry(&store, path, "M", Some(blob), Some(now)))
            }
            Some(_) => {}
        }
    }
    for (path, blob) in &after {
        if !before.contains_key(path) {
            files.push(entry(&store, path, "A", None, Some(blob)));
        }
    }
    // The store is a BTreeMap, so deletions and modifications are already in
    // path order; the additions were appended after them.
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));

    json!({ "files": files, "to": target })
}

/// One row of the strip. `from` is always null: renames are reported as an
/// add and a delete, because detecting one means comparing every added file
/// against every deleted one and the strip reads correctly without it.
fn entry(store: &Path, path: &str, status: &str, before: Option<&str>, after: Option<&str>) -> Value {
    let old = before.and_then(|key| text_of(store, key));
    let new = after.and_then(|key| text_of(store, key));

    // Either side unreadable as text means binary, and the strip says so
    // rather than claiming a line count it cannot have.
    let counts = match (before, after) {
        (Some(_), Some(_)) => match (&old, &new) {
            (Some(old), Some(new)) => Some(count(&inertia_tools::builtin::patch::unified_diff(
                old, new, path,
            ))),
            _ => None,
        },
        (None, Some(_)) => new.as_ref().map(|text| (lines(text), 0)),
        (Some(_), None) => old.as_ref().map(|text| (0, lines(text))),
        (None, None) => None,
    };

    json!({
        "path": path,
        "status": status,
        "from": Value::Null,
        "additions": counts.map(|(added, _)| added),
        "deletions": counts.map(|(_, removed)| removed),
    })
}

fn text_of(store: &Path, key: &str) -> Option<String> {
    String::from_utf8(read_blob(store, key)?).ok()
}

fn lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// Added and removed lines in a unified diff, the two numbers `git numstat`
/// would have given.
fn count(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => added += 1,
            Some(b'-') => removed += 1,
            _ => {}
        }
    }
    (added, removed)
}

/// The patch for one file, or for everything the turn changed when `file` is
/// omitted. Empty when there is nothing to show, so the caller can treat "no
/// diff" as "no change".
pub fn diff(root: &Path, cwd: &Path, from: &str, to: &str, file: Option<&str>) -> String {
    let Some(store) = prepare(root, cwd) else {
        return String::new();
    };
    let (Some(before), Some(after)) = (read_tree(&store, from), read_tree(&store, to)) else {
        return String::new();
    };

    let mut paths: Vec<&String> = before.keys().chain(after.keys()).collect();
    paths.sort();
    paths.dedup();

    let mut out = String::new();
    for path in paths {
        if file.is_some_and(|wanted| wanted != path.as_str()) {
            continue;
        }
        let old = before.get(path).and_then(|key| text_of(&store, key));
        let new = after.get(path).and_then(|key| text_of(&store, key));
        if old.is_none() && new.is_none() {
            // Both sides binary, or both missing. Nothing textual to show.
            continue;
        }
        let patch = inertia_tools::builtin::patch::unified_diff(
            old.as_deref().unwrap_or_default(),
            new.as_deref().unwrap_or_default(),
            path,
        );
        if !patch.is_empty() {
            out.push_str(&patch);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

/* -- going back ----------------------------------------------------------- */

/// Put the working folder back the way snapshot `to` had it.
///
/// Every file in the snapshot is written out as it was, and files that exist
/// now but did not then - anything the turn created - are removed, because a
/// revert that leaves the new files behind has not reverted. Files the
/// snapshot never saw (skipped folders, anything too large) are not touched.
///
/// `since` is the snapshot the changes were reported against; without one the
/// folder is snapshotted afresh, so "revert" still means "back to `to`" even
/// if the person has been editing since.
pub fn revert(root: &Path, cwd: &Path, to: &str, since: Option<&str>) -> Value {
    let Some(store) = prepare(root, cwd) else {
        return json!({ "reverted": false, "reason": "There is no snapshot to go back to." });
    };
    let Some(wanted) = read_tree(&store, to) else {
        return json!({ "reverted": false, "reason": "There is no snapshot to go back to." });
    };

    let now = since
        .and_then(|id| read_tree(&store, id))
        .or_else(|| track(root, cwd).and_then(|id| read_tree(&store, &id)));

    let mut restored = 0;
    let mut failed: Option<String> = None;
    for (path, key) in &wanted {
        let (Some(target), Some(bytes)) = (absolute_of(cwd, path), read_blob(&store, key)) else {
            failed = Some(format!("Could not read the saved copy of {path}."));
            continue;
        };
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&target, &bytes) {
            Ok(()) => restored += 1,
            Err(error) => failed = Some(format!("Could not put {path} back: {error}")),
        }
    }

    let mut deleted = 0;
    if let Some(now) = now {
        for path in now.keys() {
            if wanted.contains_key(path) {
                continue;
            }
            if let Some(target) = absolute_of(cwd, path) {
                // A file that is already gone is the outcome wanted.
                if std::fs::remove_file(&target).is_ok() {
                    deleted += 1;
                }
            }
        }
    }

    if restored == 0 && deleted == 0 && !wanted.is_empty() {
        return json!({
            "reverted": false,
            "reason": failed.unwrap_or_else(|| "Nothing could be put back.".into()),
        });
    }

    // Re-snapshot so the next comparison starts from what is now on disk.
    let tree = track(root, cwd);
    json!({ "reverted": true, "deleted": deleted, "restored": restored, "tree": tree })
}

/* -- the two transcript events -------------------------------------------- */

/// The `changes` event, or `None` when the turn changed nothing.
///
/// Nothing is emitted for a turn that only read, because an empty strip under
/// a reply is a line that says "no" to a question nobody asked.
pub fn changes_event(cwd: &Path, from: &str, result: &Value) -> Option<Value> {
    let files = result.get("files")?.as_array()?;
    if files.is_empty() {
        return None;
    }
    Some(json!({
        "type": "changes",
        "from": from,
        "to": result.get("to").cloned().unwrap_or(Value::Null),
        "cwd": cwd.display().to_string(),
        "files": files,
    }))
}

/// One lifecycle-hook run, as the transcript event the reply draws a notice
/// from.
///
/// The whole summary goes through, not a chosen few fields: the renderer reads
/// `handler`, `event`, `status`, `subject` and `reason`, the activity log reads
/// the timings, and a summary that arrives half-empty is a notice that says
/// `Hook "hook"` and tells nobody anything. `type` is added rather than
/// renamed from `handlerType` for the same reason the summary keeps them
/// apart - a hook run that called itself "command" was once filed under a tool
/// that does not exist.
pub fn hook_event(summary: &inertia_hooks::RunSummary) -> Value {
    let mut event = serde_json::Map::new();
    event.insert("type".into(), json!("hook"));
    if let Ok(Value::Object(fields)) = serde_json::to_value(summary) {
        event.extend(fields);
    }
    Value::Object(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A working folder with a workspace root beside it, which is how the app
    /// has them: the store must never land inside the folder being watched.
    fn places() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let mut root = dir.path().to_path_buf();
        root.push("workspace");
        let mut cwd = dir.path().to_path_buf();
        cwd.push("project");
        std::fs::create_dir_all(&root).expect("the workspace");
        std::fs::create_dir_all(&cwd).expect("the project");
        (dir, root, cwd)
    }

    fn write(cwd: &Path, relative: &str, text: &str) {
        let path = absolute_of(cwd, relative).expect("a path inside the folder");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the parent folder");
        }
        std::fs::write(path, text).expect("the file");
    }

    fn paths(files: &Value) -> Vec<(String, String)> {
        files
            .get("files")
            .and_then(Value::as_array)
            .expect("a files array")
            .iter()
            .map(|file| {
                (
                    file["path"].as_str().unwrap_or_default().to_string(),
                    file["status"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn a_turn_that_creates_changes_and_deletes_reports_exactly_those_three() {
        let (_dir, root, cwd) = places();
        write(&cwd, "kept.txt", "unchanged\n");
        write(&cwd, "edited.txt", "one\ntwo\nthree\n");
        write(&cwd, "gone.txt", "delete me\n");

        let before = track(&root, &cwd).expect("a snapshot");

        write(&cwd, "made.txt", "brand new\nsecond line\n");
        write(&cwd, "edited.txt", "one\nTWO\nthree\n");
        std::fs::remove_file(absolute_of(&cwd, "gone.txt").unwrap()).expect("the delete");

        let result = changes(&root, &cwd, &before, None);
        assert_eq!(
            paths(&result),
            vec![
                ("edited.txt".to_string(), "M".to_string()),
                ("gone.txt".to_string(), "D".to_string()),
                ("made.txt".to_string(), "A".to_string()),
            ],
            "the strip must list what changed and nothing else"
        );

        let files = result["files"].as_array().unwrap();
        // An added file is all additions, a deleted one all deletions, and an
        // edit is counted line by line the way numstat counts it.
        assert_eq!(files[0]["additions"], 1);
        assert_eq!(files[0]["deletions"], 1);
        assert_eq!(files[1]["additions"], 0);
        assert_eq!(files[1]["deletions"], 1);
        assert_eq!(files[2]["additions"], 2);
        assert_eq!(files[2]["deletions"], 0);
    }

    #[test]
    fn a_turn_that_changed_nothing_says_so() {
        let (_dir, root, cwd) = places();
        write(&cwd, "a.txt", "hello\n");
        let before = track(&root, &cwd).expect("a snapshot");

        let result = changes(&root, &cwd, &before, None);
        assert!(paths(&result).is_empty());
        // Two identical trees are the same tree, so the strip never appears.
        assert_eq!(result["to"], Value::String(before));
    }

    #[test]
    fn nothing_is_emitted_for_a_turn_that_only_read() {
        let (_dir, root, cwd) = places();
        write(&cwd, "a.txt", "hello\n");
        let before = track(&root, &cwd).expect("a snapshot");
        let result = changes(&root, &cwd, &before, None);
        assert!(changes_event(&cwd, &before, &result).is_none());
    }

    #[test]
    fn the_event_carries_what_the_strip_reads() {
        let (_dir, root, cwd) = places();
        write(&cwd, "a.txt", "hello\n");
        let before = track(&root, &cwd).expect("a snapshot");
        write(&cwd, "a.txt", "goodbye\n");
        let result = changes(&root, &cwd, &before, None);

        let event = changes_event(&cwd, &before, &result).expect("an event");
        assert_eq!(event["type"], "changes");
        assert_eq!(event["from"], Value::String(before));
        assert_eq!(event["to"], result["to"]);
        assert_eq!(event["cwd"], cwd.display().to_string());
        assert_eq!(event["files"][0]["path"], "a.txt");
        assert_eq!(event["files"][0]["status"], "M");
    }

    #[test]
    fn the_diff_is_a_patch_the_card_can_parse() {
        let (_dir, root, cwd) = places();
        write(&cwd, "a.txt", "one\ntwo\n");
        let before = track(&root, &cwd).expect("a snapshot");
        write(&cwd, "a.txt", "one\ntwo and a half\n");
        let after = track(&root, &cwd).expect("a second snapshot");

        let patch = diff(&root, &cwd, &before, &after, Some("a.txt"));
        assert!(patch.starts_with("--- a/a.txt"), "got: {patch}");
        assert!(patch.contains("+two and a half"));
        assert!(patch.contains("-two"));

        // A file nobody touched has no patch of its own.
        assert_eq!(diff(&root, &cwd, &before, &after, Some("missing.txt")), "");
    }

    #[test]
    fn reverting_puts_the_folder_back_and_removes_what_the_turn_made() {
        let (_dir, root, cwd) = places();
        write(&cwd, "edited.txt", "original\n");
        write(&cwd, "gone.txt", "keep me\n");
        let before = track(&root, &cwd).expect("a snapshot");

        write(&cwd, "edited.txt", "meddled with\n");
        write(&cwd, "nested/made.txt", "the turn wrote this\n");
        std::fs::remove_file(absolute_of(&cwd, "gone.txt").unwrap()).expect("the delete");
        let after = track(&root, &cwd).expect("a second snapshot");

        let result = revert(&root, &cwd, &before, Some(&after));
        assert_eq!(result["reverted"], true);

        let read = |relative: &str| {
            std::fs::read_to_string(absolute_of(&cwd, relative).unwrap()).ok()
        };
        assert_eq!(read("edited.txt").as_deref(), Some("original\n"));
        assert_eq!(read("gone.txt").as_deref(), Some("keep me\n"));
        assert_eq!(read("nested/made.txt"), None, "a created file must go");

        // And the folder now matches the snapshot it went back to.
        let again = changes(&root, &cwd, &before, None);
        assert!(paths(&again).is_empty(), "got {:?}", paths(&again));
    }

    #[test]
    fn reverting_to_a_snapshot_that_does_not_exist_says_so_rather_than_writing() {
        let (_dir, root, cwd) = places();
        write(&cwd, "a.txt", "hello\n");
        let result = revert(&root, &cwd, "not-a-snapshot", None);
        assert_eq!(result["reverted"], false);
        assert!(result["reason"].as_str().unwrap_or_default().contains("no snapshot"));
        assert_eq!(
            std::fs::read_to_string(absolute_of(&cwd, "a.txt").unwrap()).unwrap(),
            "hello\n"
        );
    }

    #[test]
    fn a_snapshot_id_from_the_window_cannot_walk_out_of_the_store() {
        let (_dir, root, cwd) = places();
        write(&cwd, "a.txt", "hello\n");
        let before = track(&root, &cwd).expect("a snapshot");

        assert!(paths(&changes(&root, &cwd, "../../../settings", None)).is_empty());
        assert_eq!(diff(&root, &cwd, &before, "..\\..\\hooks", None), "");
        assert_eq!(revert(&root, &cwd, "../../..", None)["reverted"], false);
    }

    #[test]
    fn the_noisy_folders_are_never_walked() {
        let (_dir, root, cwd) = places();
        write(&cwd, "src/main.rs", "fn main() {}\n");
        write(&cwd, "node_modules/left-pad/index.js", "module.exports = 1;\n");
        write(&cwd, "target/debug/huge.bin", "binary-ish\n");
        write(&cwd, ".git/HEAD", "ref: refs/heads/master\n");
        let before = track(&root, &cwd).expect("a snapshot");

        // Touching them changes nothing, because they were never in the tree.
        write(&cwd, "node_modules/left-pad/index.js", "module.exports = 2;\n");
        write(&cwd, "target/debug/huge.bin", "different\n");
        write(&cwd, ".git/HEAD", "ref: refs/heads/other\n");
        assert!(paths(&changes(&root, &cwd, &before, None)).is_empty());

        write(&cwd, "src/main.rs", "fn main() { work(); }\n");
        assert_eq!(
            paths(&changes(&root, &cwd, &before, None)),
            vec![("src/main.rs".to_string(), "M".to_string())]
        );
    }

    #[test]
    fn the_store_is_never_snapshotted_even_when_it_sits_inside_the_folder() {
        let (_dir, _root, cwd) = places();
        // The workspace inside the working folder: somebody pointing an agent
        // at the Inertia folder itself.
        let mut root = cwd.clone();
        root.push("workspace");
        std::fs::create_dir_all(&root).expect("the workspace");
        write(&cwd, "a.txt", "hello\n");

        let first = track(&root, &cwd).expect("a snapshot");
        let second = track(&root, &cwd).expect("a second snapshot");
        // Identical trees: the first snapshot's own blobs did not become part
        // of the second, which is what stops each one growing the next.
        assert_eq!(first, second);
    }

    #[test]
    fn a_binary_file_is_reported_without_a_line_count() {
        let (_dir, root, cwd) = places();
        let path = absolute_of(&cwd, "logo.png").unwrap();
        std::fs::write(&path, [0xff, 0xd8, 0x00, 0x01]).expect("the file");
        let before = track(&root, &cwd).expect("a snapshot");
        std::fs::write(&path, [0xff, 0xd8, 0x00, 0x02, 0x03]).expect("the edit");

        let result = changes(&root, &cwd, &before, None);
        let files = result["files"].as_array().unwrap();
        assert_eq!(files[0]["path"], "logo.png");
        assert_eq!(files[0]["status"], "M");
        // Null, which is what the strip draws as "binary".
        assert!(files[0]["additions"].is_null());
        assert!(files[0]["deletions"].is_null());
    }

    #[test]
    fn a_hook_run_becomes_the_event_the_notice_is_written_from() {
        let summary = inertia_hooks::RunSummary {
            at: 1_700_000_000_000,
            event: "PreToolUse".into(),
            subject: Some("shell".into()),
            handler: "no-rm".into(),
            source: "workspace".into(),
            handler_type: "command".into(),
            status: "blocked".into(),
            duration_ms: 12,
            error: None,
            reason: Some("rm is not allowed here".into()),
            stderr: None,
            agent: Some("builder".into()),
            thread_id: Some("t_1".into()),
        };

        let event = hook_event(&summary);
        assert_eq!(event["type"], "hook");
        // Exactly the keys the transcript reducer reaches for.
        assert_eq!(event["handler"], "no-rm");
        assert_eq!(event["event"], "PreToolUse");
        assert_eq!(event["status"], "blocked");
        assert_eq!(event["subject"], "shell");
        assert_eq!(event["reason"], "rm is not allowed here");
        // camelCase, because the window reads it and never sees a Rust name.
        assert_eq!(event["handlerType"], "command");
        assert_eq!(event["durationMs"], 12);
        assert_eq!(event["threadId"], "t_1");
        // `type` is the kind of event, not the kind of handler.
        assert_ne!(event["type"], event["handlerType"]);
    }
}

/* -- the commands the window calls --------------------------------------- */

/// Whether this folder can be snapshotted at all.
///
/// `root` is resolved from the open workspace rather than taken from the
/// window: the renderer names the folder it is working in, and the store it is
/// written to is this app's business.
#[tauri::command]
pub fn snapshot_available(
    state: tauri::State<'_, crate::state::AppState>,
    cwd: String,
) -> Result<serde_json::Value, String> {
    let workspace = state.workspace()?;
    Ok(serde_json::json!({
        "available": available(workspace.layout.root(), Path::new(&cwd)),
    }))
}

/// The unified diff for one file of a turn, or for everything it touched.
#[tauri::command]
pub fn snapshot_diff(
    state: tauri::State<'_, crate::state::AppState>,
    options: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let workspace = state.workspace()?;
    let text = |key: &str| {
        options
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let cwd = text("cwd").unwrap_or_default();
    let from = text("from").ok_or("There is no snapshot to compare against.")?;
    let to = text("to").unwrap_or_default();
    Ok(serde_json::json!(diff(
        workspace.layout.root(),
        Path::new(&cwd),
        &from,
        &to,
        text("file").as_deref(),
    )))
}

/// Put the folder back to how it was before the turn.
#[tauri::command]
pub fn snapshot_revert(
    state: tauri::State<'_, crate::state::AppState>,
    options: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let workspace = state.workspace()?;
    let text = |key: &str| {
        options
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    let cwd = text("cwd").unwrap_or_default();
    let to = text("to").ok_or("There is no snapshot to go back to.")?;
    Ok(revert(
        workspace.layout.root(),
        Path::new(&cwd),
        &to,
        text("since").as_deref(),
    ))
}

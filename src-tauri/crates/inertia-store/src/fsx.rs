//! The only module in the workspace that touches the filesystem for workspace
//! data. Everything else goes through it.
//!
//! Three of the behaviours here were written in response to real data loss and
//! must not be simplified away:
//!
//!   - **fsync before rename.** Rename being atomic protects against a visible
//!     half-written file. It does not protect against the OS reporting the new
//!     name exists while the bytes are still in page cache. After a power
//!     event that left files full of NUL bytes, the sync went in.
//!   - **Retry on Windows contention.** Antivirus, search indexers and backup
//!     agents transiently hold file handles. Without the retry, renames failed
//!     silently and - worse - deletes "succeeded" while the file reappeared on
//!     the next listing. A deleted conversation came back.
//!   - **Corruption is not emptiness.** A file that exists but does not parse
//!     is damaged, not absent. Treating the two the same read a thread as
//!     empty, rendered it empty, and let the next save overwrite the only copy
//!     with that emptiness.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Delay ladder for an operation blocked by another process holding a handle.
///
/// Six attempts, about 2.6 seconds in total. Long enough to outlast a virus
/// scanner opening a file it just saw appear; short enough that a genuinely
/// stuck file reports rather than hanging the app.
const RETRY_DELAYS_MS: &[u64] = &[20, 60, 150, 400, 800, 1200];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A relative path resolved outside the workspace. Never sanitised
    /// silently - a path that escapes is a bug or an attack, and both deserve
    /// to be loud.
    #[error("path escapes the workspace: {0}")]
    Escapes(String),

    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not encode {path}: {source}")]
    Encode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub type Result<T> = std::result::Result<T, StoreError>;

fn io(path: &Path) -> impl Fn(std::io::Error) -> StoreError + '_ {
    move |source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Resolves `relative` against `root`, refusing anything that escapes.
///
/// Deliberately a pure path computation - it does not touch the filesystem and
/// does not follow symlinks, so it behaves identically whether or not the file
/// exists. `..` is resolved lexically and then checked, rather than stripped:
/// stripping turns a caller's mistake into a silent write to the wrong place.
pub fn resolve_inside(root: &Path, relative: impl AsRef<Path>) -> Result<PathBuf> {
    let relative = relative.as_ref();

    // An absolute path is never "inside" by accident.
    if relative.is_absolute() {
        return Err(StoreError::Escapes(relative.display().to_string()));
    }

    let mut resolved = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => resolved.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() || !resolved.starts_with(root) {
                    return Err(StoreError::Escapes(relative.display().to_string()));
                }
            }
            // A drive letter or root inside a "relative" path.
            Component::Prefix(_) | Component::RootDir => {
                return Err(StoreError::Escapes(relative.display().to_string()))
            }
        }
    }

    if !resolved.starts_with(root) {
        return Err(StoreError::Escapes(relative.display().to_string()));
    }

    // The lexical walk above cannot see a link. A symlink or junction inside
    // the workspace passes every component check and then reads or writes
    // wherever it points, so the answer is checked again as the filesystem
    // resolves it.
    if !is_within(&canonical(root), &canonical(&resolved)) {
        return Err(StoreError::Escapes(relative.display().to_string()));
    }

    Ok(resolved)
}

/// The path as the filesystem resolves it: links followed, `..` folded, the
/// real case of every existing component.
///
/// A path that does not exist yet - a file about to be written - resolves
/// through its deepest ancestor that does, with the rest appended as given, so
/// a new file under a link lands where the link really points. Windows'
/// `\\?\` prefix is dropped, so the answer compares with paths written the
/// ordinary way. A path with no existing ancestor at all comes back unchanged.
pub fn canonical(path: &Path) -> PathBuf {
    let parts: Vec<Component> = path.components().collect();
    for keep in (1..=parts.len()).rev() {
        let head: PathBuf = parts[..keep].iter().collect();
        let Ok(real) = std::fs::canonicalize(&head) else {
            continue;
        };
        let mut out = without_verbatim(real);
        // Nothing past `head` exists, so nothing past it can be a link, and
        // folding these lexically is exactly what the filesystem will do.
        for part in &parts[keep..] {
            match part {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        return out;
    }
    path.to_path_buf()
}

/// `\\?\C:\x` as `C:\x`, and `\\?\UNC\host\share` as `\\host\share`.
fn without_verbatim(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// Is `candidate` the same as `base`, or somewhere beneath it?
///
/// Compared as text rather than with `Path::starts_with`, because Windows
/// filesystems are case-insensitive and accept either slash: comparing
/// case-sensitively there would report `D:\OSS` as outside `D:\oss`. Purely
/// lexical - pass both through [`canonical`] first when a link could matter.
pub fn is_within(base: &Path, candidate: &Path) -> bool {
    let sep = if cfg!(windows) { '\\' } else { '/' };
    let fold = |p: &Path| {
        let s = p.to_string_lossy();
        let s = if cfg!(windows) {
            s.replace('/', "\\").to_lowercase()
        } else {
            s.to_string()
        };
        s.trim_end_matches(sep).to_string()
    };
    let a = fold(base);
    let b = fold(candidate);
    a == b || b.starts_with(&format!("{a}{sep}"))
}

/// Whether a failure is the kind another process causes by holding a handle.
fn is_contention(error: &std::io::Error) -> bool {
    if matches!(error.kind(), std::io::ErrorKind::PermissionDenied) {
        return true;
    }
    #[cfg(windows)]
    {
        // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
        if matches!(error.raw_os_error(), Some(5) | Some(32) | Some(33)) {
            return true;
        }
    }
    false
}

/// Retries an operation through transient contention.
fn when_released<T>(mut operation: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let mut last = match operation() {
        Ok(value) => return Ok(value),
        Err(e) => e,
    };

    for delay in RETRY_DELAYS_MS {
        if !is_contention(&last) {
            break;
        }
        std::thread::sleep(Duration::from_millis(*delay));
        match operation() {
            Ok(value) => return Ok(value),
            Err(e) => last = e,
        }
    }

    Err(last)
}

pub fn ensure_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(io(dir))
}

/// Writes bytes atomically: temp file, fsync, rename.
pub fn write_then_rename(file: &Path, contents: &str) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = file.parent() {
        ensure_dir(parent)?;
    }

    // The pid separates two processes; the counter separates two writes inside
    // one. Both halves are needed. Without the counter, two flushes of the same
    // record race on a single temp name: the first rename moves the file the
    // second is about to rename, and the second fails with "cannot find the
    // file specified" naming the *destination*, which reads like a missing
    // folder and is not. That is what was dropping turn records.
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = file.with_extension(format!(
        "{}.{}.{}.tmp",
        file.extension().and_then(|e| e.to_str()).unwrap_or(""),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));

    let write = || -> std::io::Result<()> {
        let mut handle = std::fs::File::create(&temp)?;
        handle.write_all(contents.as_bytes())?;
        handle.flush()?;
        // Best effort. Network shares and virtual drives reject fsync with
        // EINVAL, and the write itself has already succeeded - failing here
        // would refuse to save on exactly the setups that need saving most.
        let _ = handle.sync_all();
        Ok(())
    };

    if let Err(e) = write() {
        let _ = std::fs::remove_file(&temp);
        return Err(StoreError::Io {
            path: temp,
            source: e,
        });
    }

    if let Err(e) = when_released(|| std::fs::rename(&temp, file)) {
        // Leaving the temp behind would accumulate litter that the directory
        // listing then has to filter forever.
        let _ = std::fs::remove_file(&temp);
        return Err(StoreError::Io {
            path: file.to_path_buf(),
            source: e,
        });
    }

    Ok(())
}

/// Writes a value as pretty JSON with a trailing newline.
///
/// The formatting is not cosmetic: the workspace folder is meant to be kept in
/// git, and two-space indent with a trailing newline is what makes a diff
/// readable and a merge possible.
pub fn write_json<T: Serialize>(file: &Path, value: &T) -> Result<()> {
    let mut json = serde_json::to_string_pretty(value).map_err(|source| StoreError::Encode {
        path: file.to_path_buf(),
        source,
    })?;
    json.push('\n');
    write_then_rename(file, &json)
}

pub fn write_text(file: &Path, contents: &str) -> Result<()> {
    write_then_rename(file, contents)
}

/// What happened while reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOutcome {
    /// Read and parsed.
    Loaded,
    /// Nothing there. The ordinary case for a setting never changed.
    Absent,
    /// The file exists but did not parse. The raw bytes have been preserved at
    /// the returned path, and the caller got a default.
    ///
    /// **Callers must surface this.** Returning a default silently is how the
    /// original lost a conversation.
    Damaged { kept_at: PathBuf },
}

/// Reads JSON, falling back to a default and never mistaking damage for
/// absence.
pub fn read_json<T: DeserializeOwned + Default>(file: &Path) -> (T, ReadOutcome) {
    let raw = match std::fs::read_to_string(file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (T::default(), ReadOutcome::Absent)
        }
        // An unreadable file is not an absent one either, but there is nothing
        // to preserve that is not already on disk.
        Err(_) => return (T::default(), ReadOutcome::Absent),
    };

    match serde_json::from_str::<T>(&raw) {
        Ok(value) => (value, ReadOutcome::Loaded),
        Err(_) => {
            let kept_at = keep_damaged(file, &raw);
            (T::default(), ReadOutcome::Damaged { kept_at })
        }
    }
}

/// Copies the unparsable bytes aside, once.
///
/// Only if the sidecar does not already exist: the copy nearest the original
/// failure is the useful one, and overwriting it on every subsequent read
/// would replace the evidence with whatever the app has since written.
fn keep_damaged(file: &Path, raw: &str) -> PathBuf {
    let sidecar = file.with_extension(format!(
        "{}.damaged",
        file.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    if !sidecar.exists() {
        let _ = std::fs::write(&sidecar, raw);
    }
    sidecar
}

pub fn read_text(file: &Path) -> Option<String> {
    std::fs::read_to_string(file).ok()
}

pub fn remove(file: &Path) -> Result<()> {
    match when_released(|| std::fs::remove_file(file)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StoreError::Io {
            path: file.to_path_buf(),
            source: e,
        }),
    }
}

/// Removes a directory and everything under it.
///
/// Used for the records that are folders - a skill, with its scripts and
/// reference files beside the SKILL.md. Deleting only the file we wrote would
/// leave a folder behind that still lists as a skill and has nothing in it.
pub fn remove_dir_all(dir: &Path) -> Result<()> {
    match when_released(|| std::fs::remove_dir_all(dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StoreError::Io {
            path: dir.to_path_buf(),
            source: e,
        }),
    }
}

pub fn exists(file: &Path) -> bool {
    file.exists()
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ListOptions<'a> {
    pub only_dirs: bool,
    /// Extension to keep, without the dot.
    pub ext: Option<&'a str>,
}

/// Lists a directory, sorted, with the app's own bookkeeping filtered out.
///
/// A missing directory is an empty list, not an error: a workspace that has
/// never had an agent has no `agents/` folder, and that is not a problem.
pub fn list_dir(dir: &Path, options: ListOptions<'_>) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut names: Vec<String> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();

            // Dotfiles, and our own temp and damaged sidecars. A `.damaged`
            // file appearing in the UI as a conversation would be alarming and
            // useless.
            if name.starts_with('.') || name.ends_with(".tmp") || name.ends_with(".damaged") {
                return None;
            }

            let is_dir = entry.file_type().ok()?.is_dir();
            if options.only_dirs != is_dir {
                return None;
            }

            if let Some(ext) = options.ext {
                if !is_dir && !name.ends_with(&format!(".{ext}")) {
                    return None;
                }
            }

            Some(name)
        })
        .collect();

    names.sort();
    names
}

/// Turns arbitrary text into a filename-safe slug.
pub fn slugify(value: &str, fallback: &str) -> String {
    let mut slug = String::with_capacity(value.len());
    let mut pending_dash = false;

    for ch in value.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(lower);
        } else {
            // Runs of anything else collapse to a single dash, and a trailing
            // run produces none at all.
            pending_dash = true;
        }
    }

    slug.truncate(64);
    let slug = slug.trim_matches('-').to_string();

    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Settings {
        theme: String,
        zoom: u32,
    }

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // ── path confinement ────────────────────────────────────────────────

    #[test]
    fn ordinary_relative_paths_resolve() {
        let root = Path::new("/ws");
        assert_eq!(
            resolve_inside(root, "settings/app.json").unwrap(),
            Path::new("/ws/settings/app.json")
        );
    }

    /// Never sanitised silently: a path that escapes is a hard error.
    #[test]
    fn escaping_is_refused_not_stripped() {
        let root = Path::new("/ws");
        assert!(resolve_inside(root, "../outside.json").is_err());
        assert!(resolve_inside(root, "settings/../../outside.json").is_err());
        assert!(resolve_inside(root, "a/b/../../../c").is_err());
    }

    #[test]
    fn a_parent_that_stays_inside_is_allowed() {
        let root = Path::new("/ws");
        assert_eq!(
            resolve_inside(root, "settings/../agents/a.json").unwrap(),
            Path::new("/ws/agents/a.json")
        );
    }

    #[test]
    fn absolute_paths_are_refused() {
        let root = Path::new("/ws");
        assert!(resolve_inside(root, "/etc/passwd").is_err());
    }

    #[test]
    fn a_current_dir_component_is_harmless() {
        let root = Path::new("/ws");
        assert_eq!(
            resolve_inside(root, "./settings/./app.json").unwrap(),
            Path::new("/ws/settings/app.json")
        );
    }

    /// Links a directory, or says the platform would not let it - creating a
    /// symlink on Windows needs developer mode, and a test that cannot set up
    /// its link has nothing to say about following one.
    fn link_dir(target: &Path, link: &Path) -> bool {
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(target, link);
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(target, link);
        made.is_ok()
    }

    /// Every component of `files/escape/secret.txt` is a plain name, so the
    /// lexical walk passes it; only following the link shows where it goes.
    #[test]
    fn a_link_out_of_the_workspace_is_refused() {
        let workspace = temp();
        let outside = temp();
        std::fs::create_dir_all(workspace.path().join("files")).unwrap();
        if !link_dir(outside.path(), &workspace.path().join("files/escape")) {
            return;
        }
        assert!(resolve_inside(workspace.path(), "files/escape/secret.txt").is_err());
        assert!(resolve_inside(workspace.path(), "files/escape").is_err());
        assert!(resolve_inside(workspace.path(), "files/plain.txt").is_ok());
    }

    #[test]
    fn a_path_that_does_not_exist_yet_resolves_through_its_existing_ancestor() {
        let dir = temp();
        let real = canonical(dir.path());
        assert_eq!(canonical(&dir.path().join("a/b/../c.txt")), real.join("a").join("c.txt"));
        assert!(!real.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn containment_ignores_case_and_slashes_where_the_filesystem_does() {
        assert!(is_within(Path::new("/ws"), Path::new("/ws")));
        assert!(is_within(Path::new("/ws"), Path::new("/ws/secrets/a.json")));
        assert!(!is_within(Path::new("/ws"), Path::new("/wsx/a.json")));
        if cfg!(windows) {
            assert!(is_within(Path::new(r"D:\OSS"), Path::new("d:/oss/a.txt")));
        }
    }

    // ── writing ─────────────────────────────────────────────────────────

    #[test]
    fn json_round_trips() {
        let dir = temp();
        let file = dir.path().join("settings/app.json");
        let value = Settings {
            theme: "dark".into(),
            zoom: 2,
        };

        write_json(&file, &value).unwrap();
        let (loaded, outcome) = read_json::<Settings>(&file);
        assert_eq!(outcome, ReadOutcome::Loaded);
        assert_eq!(loaded, value);
    }

    /// The folder is meant to live in git, so the byte formatting is part of
    /// the contract.
    #[test]
    fn json_is_pretty_with_a_trailing_newline() {
        let dir = temp();
        let file = dir.path().join("a.json");
        write_json(
            &file,
            &Settings {
                theme: "dark".into(),
                zoom: 2,
            },
        )
        .unwrap();

        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(raw.ends_with("}\n"), "no trailing newline: {raw:?}");
        assert!(raw.contains("\n  \"theme\""), "not 2-space indented: {raw:?}");
    }

    #[test]
    fn writing_creates_missing_parents() {
        let dir = temp();
        let file = dir.path().join("deeply/nested/here/a.json");
        write_json(&file, &Settings::default()).unwrap();
        assert!(file.exists());
    }

    /// No `.tmp` may survive a successful write, or the listing filter has to
    /// hide an ever-growing pile of them.
    #[test]
    fn no_temp_files_are_left_behind() {
        let dir = temp();
        write_json(&dir.path().join("a.json"), &Settings::default()).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");
    }

    #[test]
    fn a_rewrite_replaces_the_previous_contents_entirely() {
        let dir = temp();
        let file = dir.path().join("a.json");

        write_json(
            &file,
            &Settings {
                theme: "a-very-long-theme-name".into(),
                zoom: 9,
            },
        )
        .unwrap();
        write_json(
            &file,
            &Settings {
                theme: "x".into(),
                zoom: 1,
            },
        )
        .unwrap();

        let (loaded, _) = read_json::<Settings>(&file);
        assert_eq!(loaded.theme, "x");
        // Not a partial overwrite leaving trailing bytes of the longer value.
        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(!raw.contains("very-long"), "stale bytes survived: {raw}");
    }

    // ── reading, and the difference between absent and damaged ──────────

    #[test]
    fn an_absent_file_is_not_damage() {
        let dir = temp();
        let (loaded, outcome) = read_json::<Settings>(&dir.path().join("nope.json"));
        assert_eq!(outcome, ReadOutcome::Absent);
        assert_eq!(loaded, Settings::default());
    }

    /// The distinction that cost a conversation: unparsable is damaged, and
    /// the bytes are preserved before anything else happens.
    #[test]
    fn a_corrupt_file_is_preserved_and_reported() {
        let dir = temp();
        let file = dir.path().join("thread.json");
        std::fs::write(&file, "{ this is not json").unwrap();

        let (loaded, outcome) = read_json::<Settings>(&file);
        assert_eq!(loaded, Settings::default());

        let ReadOutcome::Damaged { kept_at } = outcome else {
            panic!("corruption was reported as {outcome:?}");
        };
        assert!(kept_at.exists(), "the original bytes were not preserved");
        assert_eq!(
            std::fs::read_to_string(&kept_at).unwrap(),
            "{ this is not json"
        );
    }

    /// The copy nearest the failure is the useful one; later reads must not
    /// overwrite it.
    #[test]
    fn the_first_damaged_copy_is_the_one_kept() {
        let dir = temp();
        let file = dir.path().join("thread.json");

        std::fs::write(&file, "original damage").unwrap();
        let (_, first) = read_json::<Settings>(&file);
        let ReadOutcome::Damaged { kept_at } = first else {
            panic!("expected damage");
        };

        std::fs::write(&file, "later damage").unwrap();
        read_json::<Settings>(&file);

        assert_eq!(
            std::fs::read_to_string(&kept_at).unwrap(),
            "original damage",
            "the first copy was overwritten"
        );
    }

    // ── listing ─────────────────────────────────────────────────────────

    #[test]
    fn listing_sorts_and_hides_our_bookkeeping() {
        let dir = temp();
        for name in [
            "b.json",
            "a.json",
            "c.json.damaged",
            "d.json.1234.tmp",
            ".hidden",
        ] {
            std::fs::write(dir.path().join(name), "{}").unwrap();
        }

        let names = list_dir(dir.path(), ListOptions::default());
        assert_eq!(names, vec!["a.json", "b.json"]);
    }

    #[test]
    fn listing_can_filter_by_extension_or_directories() {
        let dir = temp();
        std::fs::write(dir.path().join("a.json"), "{}").unwrap();
        std::fs::write(dir.path().join("b.md"), "").unwrap();
        std::fs::create_dir(dir.path().join("skill-one")).unwrap();

        assert_eq!(
            list_dir(
                dir.path(),
                ListOptions {
                    ext: Some("json"),
                    ..Default::default()
                }
            ),
            vec!["a.json"]
        );
        assert_eq!(
            list_dir(
                dir.path(),
                ListOptions {
                    only_dirs: true,
                    ..Default::default()
                }
            ),
            vec!["skill-one"]
        );
    }

    /// A workspace that has never had an agent has no `agents/` folder, and
    /// that is not an error.
    #[test]
    fn listing_a_missing_directory_is_empty() {
        let dir = temp();
        assert!(list_dir(&dir.path().join("never-created"), ListOptions::default()).is_empty());
    }

    // ── removal ─────────────────────────────────────────────────────────

    #[test]
    fn removing_is_idempotent() {
        let dir = temp();
        let file = dir.path().join("a.json");
        std::fs::write(&file, "{}").unwrap();

        remove(&file).unwrap();
        assert!(!file.exists());
        // Removing what is already gone is success, not failure.
        remove(&file).unwrap();
    }

    // ── slugs ───────────────────────────────────────────────────────────

    #[test]
    fn slugs_are_lowercase_and_dash_separated() {
        assert_eq!(slugify("My First Agent", "agent"), "my-first-agent");
        assert_eq!(slugify("Hello,   World!!", "x"), "hello-world");
    }

    #[test]
    fn slugs_have_no_leading_or_trailing_dashes() {
        assert_eq!(slugify("  spaced  ", "x"), "spaced");
        assert_eq!(slugify("!!!bang!!!", "x"), "bang");
    }

    #[test]
    fn an_unsluggable_name_falls_back() {
        assert_eq!(slugify("", "agent"), "agent");
        assert_eq!(slugify("!!!", "memory"), "memory");
        // Non-ASCII with no ASCII alphanumerics left.
        assert_eq!(slugify("日本語", "note"), "note");
    }

    #[test]
    fn slugs_are_bounded_in_length() {
        let slug = slugify(&"a".repeat(200), "x");
        assert!(slug.len() <= 64, "got {} chars", slug.len());
    }

    /// Two writes of one record, at once, from one process.
    ///
    /// The real case is a turn record: a scheduled write and the write that
    /// settles the turn can land together. When both used the same temp name
    /// the loser reported "cannot find the file specified" against the
    /// destination path and the record was lost.
    #[test]
    fn two_writes_of_one_file_at_once_both_land() {
        let dir = temp();
        let file = dir.path().join("turn.json");

        let failures: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|n| {
                    let file = file.clone();
                    scope.spawn(move || write_text(&file, &format!("attempt {n}")))
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().unwrap().err())
                .map(|error| error.to_string())
                .collect()
        });

        assert!(failures.is_empty(), "{failures:?}");
        assert!(std::fs::read_to_string(&file).unwrap().starts_with("attempt "));
    }
}

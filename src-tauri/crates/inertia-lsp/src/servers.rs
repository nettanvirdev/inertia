//! Which language server, if any, is installed for this file.
//!
//! The load-bearing word is "if any". A language server is a thing a developer
//! installs on purpose, and most checkouts an agent opens have none. That is
//! not a degraded mode to be warned about, it is the ordinary case: this file's
//! job is to answer "nothing here" quickly, cache that answer, and let every
//! caller carry on exactly as it did before any of this existed.
//!
//! Nothing is ever installed or downloaded. A missing server is a missing
//! server. Offering to fetch a hundred megabytes of Python tooling because
//! somebody edited a `.py` would be a far worse trade than not having
//! diagnostics.
//!
//! Binaries are looked for in the project's own `node_modules/.bin` before
//! PATH: a repository that pinned a version of the server meant that version,
//! and the one on PATH is whatever was installed globally three years ago.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;
use serde_json::Value;

/// How far up the tree a project root is looked for before giving up.
const MAX_ASCENT: usize = 12;

/// One entry in the built-in table, or one declared by a project.
#[derive(Debug, Clone)]
struct Definition {
    id: String,
    extensions: Vec<String>,
    bin: String,
    args: Vec<String>,
    /// A configured command is a path the project chose, so it is taken as
    /// written rather than looked up: resolving it would silently prefer some
    /// other binary of the same name that happened to be on PATH.
    literal: bool,
    root_files: Vec<String>,
    env: Vec<(String, String)>,
    initialization: Option<Value>,
}

/// The servers, tried in order per extension; the first one that is actually
/// installed wins and the rest are not considered.
///
/// pyright before ruff is deliberate. Both speak LSP for Python and both are
/// commonly present, but ruff reports lint findings and pyright reports type
/// errors, and a type error is the thing a model breaks when it edits code it
/// cannot run. Where both are installed, the one that catches the worse
/// mistake is the one to ask.
///
/// The root files are how a workspace root is found. It matters more than it
/// looks: a TypeScript server rooted at the file's own directory sees a project
/// of one file, decides every import is unresolved, and reports a screenful of
/// errors that are all artefacts of where it was pointed.
/// id, extensions, binary, arguments, root markers.
type Builtin = (
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

const BUILTIN: &[Builtin] = &[
    (
        "typescript",
        &[".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"],
        "typescript-language-server",
        &["--stdio"],
        &["tsconfig.json", "jsconfig.json", "package.json"],
    ),
    (
        "pyright",
        &[".py", ".pyi"],
        "pyright-langserver",
        &["--stdio"],
        &[
            "pyproject.toml",
            "setup.py",
            "setup.cfg",
            "requirements.txt",
            "Pipfile",
        ],
    ),
    (
        "ruff",
        &[".py", ".pyi"],
        "ruff",
        &["server"],
        &["pyproject.toml", "ruff.toml", ".ruff.toml"],
    ),
    ("gopls", &[".go"], "gopls", &[], &["go.work", "go.mod"]),
    (
        "rust-analyzer",
        &[".rs"],
        "rust-analyzer",
        &[],
        &["Cargo.toml"],
    ),
];

fn builtin() -> Vec<Definition> {
    BUILTIN
        .iter()
        .map(|(id, extensions, bin, args, root_files)| Definition {
            id: (*id).to_string(),
            extensions: extensions.iter().map(|e| (*e).to_string()).collect(),
            bin: (*bin).to_string(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            literal: false,
            root_files: root_files.iter().map(|f| (*f).to_string()).collect(),
            env: Vec::new(),
            initialization: None,
        })
        .collect()
}

/// The `languageId` a server is told a document is written in.
///
/// Getting this wrong is quiet rather than loud: the server accepts the
/// document and declines to analyse it, so the file simply never produces
/// diagnostics and nothing anywhere says why.
const LANGUAGE_IDS: &[(&str, &str)] = &[
    (".ts", "typescript"),
    (".mts", "typescript"),
    (".cts", "typescript"),
    (".tsx", "typescriptreact"),
    (".js", "javascript"),
    (".mjs", "javascript"),
    (".cjs", "javascript"),
    (".jsx", "javascriptreact"),
    (".py", "python"),
    (".pyi", "python"),
    (".go", "go"),
    (".rs", "rust"),
];

fn extension_of(file: &Path) -> String {
    match file.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!(".{}", ext.to_ascii_lowercase()),
        None => String::new(),
    }
}

/// The language a server is told this document is written in.
pub fn language_id(file: &Path) -> String {
    let extension = extension_of(file);
    LANGUAGE_IDS
        .iter()
        .find(|(ext, _)| *ext == extension)
        .map(|(_, id)| (*id).to_string())
        .unwrap_or_else(|| "plaintext".to_string())
}

// ── is this switched on ─────────────────────────────────────────────────

const OFF: &[&str] = &["0", "false", "off", "no"];

/// The project's own answer about language servers, read from `.inertia.json`.
///
/// Three shapes, mirroring the formatter's switch so there is one idea to
/// learn: absent means the defaults, `false` means none at all, and an object
/// both disables individual servers and declares extra ones. The last of those
/// is not decoration - it is the only way to point Inertia at a server it has
/// never heard of, and it is how this code is tested end to end.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub enabled: bool,
    extra: Vec<Definition>,
    disabled: HashSet<String>,
}

impl Settings {
    fn on() -> Self {
        Self {
            enabled: true,
            extra: Vec::new(),
            disabled: HashSet::new(),
        }
    }

    fn off() -> Self {
        Self {
            enabled: false,
            extra: Vec::new(),
            disabled: HashSet::new(),
        }
    }
}

pub fn settings(cwd: Option<&Path>) -> Settings {
    let flag = std::env::var("INERTIA_LSP")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !flag.is_empty() && OFF.contains(&flag.as_str()) {
        return Settings::off();
    }
    let Some(cwd) = cwd else {
        return Settings::on();
    };

    let mut path = cwd.to_path_buf();
    path.push(".inertia.json");
    // No file, or one somebody is halfway through editing. Neither is a reason
    // to stop, and neither is worth a message on every edit.
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::on();
    };
    let Ok(document) = serde_json::from_str::<Value>(&text) else {
        return Settings::on();
    };
    let Some(config) = document.get("lsp") else {
        return Settings::on();
    };

    if config == &Value::Bool(false) {
        return Settings::off();
    }
    let Some(entries) = config.as_object() else {
        return Settings::on();
    };

    let mut extra = Vec::new();
    let mut disabled = HashSet::new();
    for (id, entry) in entries {
        let Some(entry) = entry.as_object() else {
            continue;
        };
        if entry.get("disabled") == Some(&Value::Bool(true)) {
            disabled.insert(id.clone());
            continue;
        }
        let command: Vec<String> = entry
            .get("command")
            .and_then(Value::as_array)
            .map(|list| list.iter().map(string_of).collect())
            .unwrap_or_default();
        if command.is_empty() {
            continue;
        }
        extra.push(Definition {
            id: id.clone(),
            extensions: entry
                .get("extensions")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .map(|e| string_of(e).to_ascii_lowercase())
                        .collect()
                })
                .unwrap_or_default(),
            bin: command[0].clone(),
            args: command[1..].to_vec(),
            literal: true,
            root_files: entry
                .get("rootFiles")
                .and_then(Value::as_array)
                .map(|list| list.iter().map(string_of).collect())
                .unwrap_or_default(),
            env: entry
                .get("env")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .map(|(key, value)| (key.clone(), string_of(value)))
                        .collect()
                })
                .unwrap_or_default(),
            initialization: entry.get("initialization").cloned(),
        });
    }
    Settings {
        enabled: true,
        extra,
        disabled,
    }
}

fn string_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// ── finding the root ────────────────────────────────────────────────────

/// The nearest directory above `file` holding one of `names`.
///
/// Names are tried outermost-marker-first per directory but nearest-directory
/// first overall, so a `tsconfig.json` in a package beats a `package.json` at
/// the monorepo root, which is the project boundary the server should see.
pub fn find_root(file: &Path, names: &[String], fallback: Option<&Path>) -> PathBuf {
    let parent = || {
        file.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| file.to_path_buf())
    };
    if names.is_empty() {
        return fallback.map(Path::to_path_buf).unwrap_or_else(parent);
    }

    let mut dir = parent();
    for _ in 0..MAX_ASCENT {
        for name in names {
            // One component at a time: a joined "a/b" keeps its slash inside
            // the PathBuf on Windows and stops comparing equal to itself.
            let mut candidate = dir.clone();
            candidate.push(name);
            if candidate.exists() {
                return dir;
            }
        }
        match dir.parent() {
            Some(up) if up != dir => dir = up.to_path_buf(),
            _ => break,
        }
    }
    fallback.map(Path::to_path_buf).unwrap_or_else(parent)
}

// ── is this binary here at all ──────────────────────────────────────────

/// The extensions Windows will actually execute, in the order it tries them.
#[cfg(windows)]
const EXECUTABLE_EXTENSIONS: &[&str] = &["", ".cmd", ".exe", ".bat", ".com"];
#[cfg(not(windows))]
const EXECUTABLE_EXTENSIONS: &[&str] = &[""];

/// Where `bin` actually lives, looking in `extra` before PATH, or `None`.
///
/// A name with a separator in it is a path the caller chose, and is only
/// checked for existence.
pub fn which(bin: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let named = Path::new(bin);
    if named.components().count() > 1 {
        return exists_as_program(named);
    }

    let from_path = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();

    for dir in extra.iter().cloned().chain(from_path) {
        let mut candidate = dir;
        candidate.push(bin);
        if let Some(found) = exists_as_program(&candidate) {
            return Some(found);
        }
    }
    None
}

fn exists_as_program(candidate: &Path) -> Option<PathBuf> {
    for extension in EXECUTABLE_EXTENSIONS {
        let with = if extension.is_empty() {
            candidate.to_path_buf()
        } else {
            let mut name = candidate.as_os_str().to_os_string();
            name.push(extension);
            PathBuf::from(name)
        };
        if with.is_file() {
            return Some(with);
        }
    }
    None
}

/// Where a project keeps the servers it installed itself.
fn local_bins(cwd: Option<&Path>) -> Vec<PathBuf> {
    match cwd {
        Some(cwd) => {
            let mut dir = cwd.to_path_buf();
            dir.push("node_modules");
            dir.push(".bin");
            vec![dir]
        }
        None => Vec::new(),
    }
}

// ── putting it together ─────────────────────────────────────────────────

/// A server to run, with everything needed to start it.
#[derive(Debug, Clone)]
pub struct ServerSpec {
    pub id: String,
    pub bin: PathBuf,
    pub args: Vec<String>,
    pub root: PathBuf,
    pub env: Vec<(String, String)>,
    pub initialization: Option<Value>,
}

/// `Some(None)` in here is a remembered "nothing is installed for this", which
/// is the answer almost every time.
type Cache = Mutex<HashMap<String, Option<Definition>>>;

/// Resolutions already worked out, keyed by extension and working directory.
///
/// Without this, every edit walks PATH looking for four binaries that are not
/// there. That is a few dozen failed stat calls charged to something whose
/// whole promise is that you cannot tell it is running.
fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The server to use for this file, or `None`.
///
/// `None` is the answer in the overwhelming majority of cases and costs a map
/// lookup after the first call.
pub fn resolve(file: &Path, cwd: Option<&Path>) -> Option<ServerSpec> {
    let extension = extension_of(file);
    if extension.is_empty() {
        return None;
    }

    let config = settings(cwd);
    if !config.enabled {
        return None;
    }

    let key = format!(
        "{extension} {}",
        cwd.map(|c| c.display().to_string()).unwrap_or_default()
    );
    // `Some(None)` is a cache hit meaning "nothing is installed for this", and
    // it is the answer almost every time. Only an outright miss does the work.
    let cached = cache().lock().get(&key).cloned();
    let chosen = match cached {
        Some(hit) => hit,
        None => {
            let mut found = None;
            for server in config.extra.iter().chain(builtin().iter()) {
                if config.disabled.contains(&server.id) {
                    continue;
                }
                if !server.extensions.is_empty() && !server.extensions.contains(&extension) {
                    continue;
                }
                let bin = if server.literal {
                    PathBuf::from(&server.bin)
                } else {
                    match which(&server.bin, &local_bins(cwd)) {
                        Some(path) => path,
                        None => continue,
                    }
                };
                let mut definition = server.clone();
                definition.bin = bin.display().to_string();
                found = Some(definition);
                break;
            }
            cache().lock().insert(key, found.clone());
            found
        }
    };

    let chosen = chosen?;
    Some(ServerSpec {
        id: chosen.id.clone(),
        bin: PathBuf::from(&chosen.bin),
        args: chosen.args.clone(),
        env: chosen.env.clone(),
        initialization: chosen.initialization.clone(),
        root: find_root(&std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf()), &chosen.root_files, cwd),
    })
}

/// Forget every cached lookup. For tests, and for when the project changes.
pub fn reset() {
    cache().lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_type_nobody_claims_has_no_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = dir.path().to_path_buf();
        file.push("notes.txt");
        reset();
        assert!(resolve(&file, Some(dir.path())).is_none());
    }

    #[test]
    fn a_file_with_no_extension_has_no_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = dir.path().to_path_buf();
        file.push("Makefile");
        reset();
        assert!(resolve(&file, Some(dir.path())).is_none());
    }

    /// A project may point Inertia at a server it has never heard of. This is
    /// also how the rest of the suite gets a server to talk to.
    #[test]
    fn a_project_can_declare_its_own_server() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = dir.path().to_path_buf();
        config.push(".inertia.json");
        std::fs::write(
            &config,
            r#"{"lsp":{"demo":{"command":["demo-server","--stdio"],"extensions":[".demo"],"rootFiles":[]}}}"#,
        )
        .unwrap();

        let mut file = dir.path().to_path_buf();
        file.push("a.demo");
        reset();
        let spec = resolve(&file, Some(dir.path())).expect("the declared server");
        assert_eq!(spec.id, "demo");
        // Taken as written: a configured command is not looked up on PATH.
        assert_eq!(spec.bin, PathBuf::from("demo-server"));
        assert_eq!(spec.args, vec!["--stdio".to_string()]);
        assert_eq!(spec.root, dir.path());
        reset();
    }

    #[test]
    fn a_project_can_switch_the_whole_thing_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = dir.path().to_path_buf();
        config.push(".inertia.json");
        std::fs::write(&config, r#"{"lsp":false}"#).unwrap();
        let mut file = dir.path().to_path_buf();
        file.push("a.ts");
        reset();
        assert!(resolve(&file, Some(dir.path())).is_none());
        reset();
    }

    #[test]
    fn a_root_is_the_nearest_directory_holding_a_marker() {
        let dir = tempfile::tempdir().unwrap();
        let mut package = dir.path().to_path_buf();
        package.push("pkg");
        std::fs::create_dir_all(&package).unwrap();
        let mut marker = package.clone();
        marker.push("tsconfig.json");
        std::fs::write(&marker, "{}").unwrap();

        let mut file = package.clone();
        file.push("src");
        std::fs::create_dir_all(&file).unwrap();
        file.push("a.ts");

        let found = find_root(&file, &["tsconfig.json".to_string()], Some(dir.path()));
        assert_eq!(found, package);
    }

    #[test]
    fn a_root_with_no_marker_falls_back_to_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = dir.path().to_path_buf();
        file.push("deep");
        std::fs::create_dir_all(&file).unwrap();
        file.push("a.ts");
        let found = find_root(&file, &["nothing-like-this".to_string()], Some(dir.path()));
        assert_eq!(found, dir.path());
    }

    #[test]
    fn language_ids_follow_the_extension() {
        assert_eq!(language_id(Path::new("a.tsx")), "typescriptreact");
        assert_eq!(language_id(Path::new("a.PY")), "python");
        assert_eq!(language_id(Path::new("a.md")), "plaintext");
    }

    #[test]
    fn a_binary_that_is_not_installed_is_not_found() {
        assert!(which("inertia-no-such-language-server", &[]).is_none());
    }
}

//! Telling the model what it just broke, and answering what it asks.
//!
//! Everything else in this crate exists for this file, and this file exists
//! mostly for one line appended to the result of an edit. Without it a model
//! that introduces a bad import or a type error finds out only if it happens to
//! run a build - usually several turns later, usually after building more code
//! on top of the mistake. With it the mistake comes back on the turn that made
//! it, while the model still has the reason for the change in front of it.
//!
//! The design is a single promise made to every caller: this is invisible
//! unless it has something useful to say. No server installed, no root found, a
//! server that will not start, a handshake that hangs, a crash mid-request -
//! all of them produce an empty string and an edit that reads exactly as it did
//! before any of this existed. Nothing here is ever allowed to fail a write
//! that already landed on disk.
//!
//! Servers are spawned lazily, one per (root, language), and reused. They are
//! shut down on quit, because a language server nobody owns is a gigabyte of
//! RAM with no window, no icon, and no reason for the user to suspect it was
//! ours.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::needless_pass_by_value
    )
)]

pub mod client;
pub mod diagnostic;
pub mod protocol;
pub mod servers;
pub mod testing;
pub mod uri;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::future::{FutureExt, Shared};
use parking_lot::Mutex;
use serde_json::{json, Value};

pub use client::{Client, Launcher, ProcessLauncher, Published, Transport};
pub use uri::{from_uri, normalize, spelling, to_uri};

/// The whole budget for producing diagnostics on an edit: spawning a server if
/// there is not one yet, handshaking, sending the document and waiting for a
/// reply.
///
/// Three seconds is chosen against the alternative, not in a vacuum. A cold
/// gopls or rust-analyzer takes far longer than this to index a repository the
/// first time, and waiting for it would add half a minute to the first edit of
/// every session. Instead the first edit gives up, the spawn carries on in the
/// background, and by the second edit the server is warm and answers instantly.
/// A tool that is occasionally quiet is much better than one that is regularly
/// slow.
pub const BUDGET_MS: u64 = 3_000;

/// How many other files a `write` may report having broken.
pub const MAX_OTHER_FILES: usize = 5;

/// How long a query may take before it is reported as unanswered.
pub const QUERY_BUDGET_MS: u64 = 8_000;

/// How many times a server may die and be started again before its language is
/// written off for this session. A server that crashes on the file being edited
/// would otherwise be respawned on every keystroke of a refactor.
const MAX_RESTARTS: u32 = 2;

/// A connection that may still be handshaking.
///
/// Shared rather than awaited directly, because the caller gives up after its
/// budget and the connection must carry on: that is what makes the second edit
/// of a session fast when the first one timed out.
type Ready = Shared<futures::channel::oneshot::Receiver<Option<Arc<Client>>>>;

/// The methods the `lsp` tool can ask for, and what each answer is called.
///
/// Names the model uses, not the protocol's: "definition" rather than
/// `textDocument/definition`. The mapping is here because it is the only place
/// that needs to know both.
pub const OPERATIONS: &[(&str, &str, bool)] = &[
    ("definition", "textDocument/definition", true),
    ("type_definition", "textDocument/typeDefinition", true),
    ("implementation", "textDocument/implementation", true),
    ("references", "textDocument/references", true),
    ("hover", "textDocument/hover", true),
    ("symbols", "textDocument/documentSymbol", false),
    ("workspace_symbols", "workspace/symbol", false),
];

/// The protocol method and whether it needs a position, for one operation name.
pub fn operation(name: &str) -> Option<(&'static str, bool)> {
    OPERATIONS
        .iter()
        .find(|(id, _, _)| *id == name)
        .map(|(_, method, needs)| (*method, *needs))
}

/// Every operation name the `lsp` tool accepts, in the order they are listed to
/// the model.
pub fn operation_names() -> Vec<&'static str> {
    OPERATIONS.iter().map(|(id, _, _)| *id).collect()
}

/// The kinds a symbol can be, by the protocol's numbering.
const SYMBOL_KINDS: &[&str] = &[
    "symbol",
    "file",
    "module",
    "namespace",
    "package",
    "class",
    "method",
    "property",
    "field",
    "constructor",
    "enum",
    "interface",
    "function",
    "variable",
    "constant",
    "string",
    "number",
    "boolean",
    "array",
    "object",
    "key",
    "null",
    "enum member",
    "struct",
    "event",
    "operator",
    "type parameter",
];

/// One place in one file, one-based the way a person reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationRow {
    pub file: PathBuf,
    pub line: u64,
    pub character: u64,
    pub end_line: u64,
}

/// One declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRow {
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub file: Option<PathBuf>,
    pub line: u64,
}

/// What the server said.
///
/// [`Answer::Unsupported`] is a server that refused or failed the request,
/// which is a different thing from a server that answered "nothing there": the
/// first means ask another way, the second means there is nothing to find.
#[derive(Debug, Clone)]
pub enum Answer {
    Unsupported { error: String },
    Hover(String),
    Symbols(Vec<SymbolRow>),
    Locations(Vec<LocationRow>),
}

/// One question for the language server.
#[derive(Debug, Clone)]
pub struct Question<'a> {
    pub operation: &'a str,
    /// One-based, the way `read` numbers its lines.
    pub line: u64,
    pub character: u64,
    pub symbol: Option<&'a str>,
    pub budget: Duration,
}

impl<'a> Question<'a> {
    pub fn new(operation: &'a str) -> Self {
        Self {
            operation,
            line: 1,
            character: 1,
            symbol: None,
            budget: Duration::from_millis(QUERY_BUDGET_MS),
        }
    }
}

/// The language servers this app is running.
///
/// One of these is shared by the whole app: the `read` tool warms a server up,
/// the `edit` tool asks it what broke, and the `lsp` tool asks it questions -
/// all of them the same process, because three copies would be three
/// rust-analyzers indexing the same repository.
pub struct Lsp {
    launcher: Arc<dyn Launcher>,
    /// In-flight or settled connections, keyed by root and language.
    clients: Mutex<HashMap<String, Ready>>,
    /// Keys written off for this session, and how many times each has died.
    broken: Mutex<HashSet<String>>,
    restarts: Mutex<HashMap<String, u32>>,
}

impl std::fmt::Debug for Lsp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lsp")
            .field("servers", &self.clients.lock().len())
            .field("broken", &self.broken.lock().len())
            .finish()
    }
}

impl Default for Lsp {
    fn default() -> Self {
        Self::new()
    }
}

impl Lsp {
    pub fn new() -> Self {
        Self::with_launcher(Arc::new(ProcessLauncher))
    }

    /// The same thing over a launcher somebody else supplied, which is how the
    /// tests get a server to talk to on a machine with none installed.
    pub fn with_launcher(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            launcher,
            clients: Mutex::new(HashMap::new()),
            broken: Mutex::new(HashSet::new()),
            restarts: Mutex::new(HashMap::new()),
        }
    }

    /// The client for this file, starting one if needed.
    ///
    /// `None` for every ordinary reason there is no server: none installed for
    /// the extension, none configured, one that refused to start. Callers treat
    /// `None` and "no diagnostics" identically, which is what keeps the failure
    /// paths from needing their own handling anywhere else.
    async fn client_for(&self, file: &Path, cwd: Option<&Path>) -> Option<Arc<Client>> {
        let spec = servers::resolve(file, cwd)?;
        let key = format!("{} {}", spec.id, normalize(&spec.root));

        loop {
            if self.broken.lock().contains(&key) {
                return None;
            }

            let ready = {
                let mut clients = self.clients.lock();
                match clients.get(&key) {
                    Some(existing) => existing.clone(),
                    None => {
                        let (tx, rx) = futures::channel::oneshot::channel();
                        let launcher = Arc::clone(&self.launcher);
                        let spec = spec.clone();
                        tokio::spawn(async move {
                            let client = match Client::connect(&spec, launcher.as_ref()).await {
                                Ok(client) => Some(Arc::new(client)),
                                Err(error) => {
                                    tracing::debug!(
                                        server = %spec.id,
                                        "language server could not be started: {error}"
                                    );
                                    None
                                }
                            };
                            let _ = tx.send(client);
                        });
                        let shared = rx.shared();
                        clients.insert(key.clone(), shared.clone());
                        shared
                    }
                }
            };

            match ready.await {
                Ok(Some(client)) if client.alive() => return Some(client),
                // It died since we last used it. Drop it and start again, but
                // only a couple of times - a server that dies reliably is a
                // server this session is better off without.
                Ok(Some(_)) => {
                    self.clients.lock().remove(&key);
                    let count = {
                        let mut restarts = self.restarts.lock();
                        let count = restarts.entry(key.clone()).or_insert(0);
                        *count += 1;
                        *count
                    };
                    if count > MAX_RESTARTS {
                        self.broken.lock().insert(key.clone());
                        tracing::info!(
                            server = %spec.id,
                            "the language server died {count} times and was given up on for this session"
                        );
                        return None;
                    }
                }
                // A server that cannot be started or cannot complete a
                // handshake is not going to behave differently in five seconds,
                // and retrying it on every edit would spawn a process per edit
                // for the rest of the session.
                Ok(None) | Err(_) => {
                    self.broken.lock().insert(key.clone());
                    return None;
                }
            }
        }
    }

    /// Everything the server for this file currently knows, keyed by lookup
    /// key, or `None` when there is no server.
    ///
    /// The file is read from disk rather than taken from the caller,
    /// deliberately. The formatter may have rewritten it after the write, and
    /// diagnostics that describe a version of the file that no longer exists
    /// are worse than none: they point at line numbers the model cannot find.
    pub async fn collect(
        &self,
        file: &Path,
        cwd: Option<&Path>,
        budget: Duration,
    ) -> Option<HashMap<String, Published>> {
        let started = std::time::Instant::now();
        let client = tokio::time::timeout(budget, self.client_for(file, cwd))
            .await
            .ok()
            .flatten()?;

        let text = tokio::fs::read_to_string(file).await.ok()?;
        client.open(file, &text).await;

        let remaining = budget.saturating_sub(started.elapsed());
        if !remaining.is_zero() {
            client.wait_for_diagnostics(file, remaining).await;
        }
        Some(client.all())
    }

    /// The text to append to a tool result, which is almost always `""`.
    ///
    /// `others` is how many *other* files may be reported. It is zero for
    /// `edit`, whose blast radius is the file it changed, and five for `write`,
    /// which replaces a whole file and routinely breaks its importers - that is
    /// the case where the model has genuinely no other way to find out.
    pub async fn report(&self, file: &Path, cwd: Option<&Path>, others: usize) -> String {
        let absolute = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        let Some(all) = self
            .collect(&absolute, cwd, Duration::from_millis(BUDGET_MS))
            .await
        else {
            return String::new();
        };

        let key = normalize(&absolute);
        let mut out = String::new();

        // Named with the path the caller used, not the one the server answered
        // with. tsserver lowercases everything it says on Windows, and a model
        // told its error is in `c:\users\...` will go looking for a second file.
        let here = diagnostic::report(
            &absolute,
            all.get(&key)
                .map(|entry| entry.items.as_slice())
                .unwrap_or(&[]),
        );
        if !here.is_empty() {
            out.push_str("\n\nLSP errors detected in this file, please fix:\n");
            out.push_str(&here);
        }

        if others > 0 {
            let mut blocks: Vec<String> = Vec::new();
            // Sorted, so a report about four broken importers names them in the
            // same order twice running.
            let mut keys: Vec<&String> = all.keys().collect();
            keys.sort();
            for other in keys {
                if other == &key {
                    continue;
                }
                let Some(entry) = all.get(other) else {
                    continue;
                };
                if !diagnostic::interesting(&entry.items) {
                    continue;
                }
                let block = diagnostic::report(&spelling(&entry.file), &entry.items);
                if !block.is_empty() {
                    blocks.push(block);
                }
                if blocks.len() >= others {
                    break;
                }
            }
            if !blocks.is_empty() {
                out.push_str("\n\nLSP errors detected in other files:\n");
                out.push_str(&blocks.join("\n"));
            }
        }

        out
    }

    /// Start indexing this file in the background because the model is likely
    /// to edit it next.
    ///
    /// Deliberately not awaited and deliberately incapable of failing. A read
    /// is one of the cheapest, most frequent things an agent does, and the read
    /// must not get slower or less reliable because a language server exists.
    pub fn warm(self: &Arc<Self>, file: &Path, cwd: Option<&Path>) {
        let lsp = Arc::clone(self);
        let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        let cwd = cwd.map(Path::to_path_buf);
        tokio::spawn(async move {
            let Some(client) = lsp.client_for(&file, cwd.as_deref()).await else {
                return;
            };
            let Ok(text) = tokio::fs::read_to_string(&file).await else {
                return;
            };
            client.open(&file, &text).await;
        });
    }

    /// Ask the language server about a place in a file.
    ///
    /// `None` when there is no server for this file at all - which the caller
    /// reports as "no language server here" rather than as a failure, because
    /// that is what it is on a machine with none installed. A server that does
    /// not implement the method answers null, which is indistinguishable from
    /// "nothing there" in the protocol and is reported honestly as "nothing
    /// found".
    ///
    /// The document is opened first. A server is only obliged to answer about
    /// files it has been told the contents of, and the file on disk is the
    /// version everyone else in this app is talking about.
    pub async fn query(
        &self,
        file: &Path,
        cwd: Option<&Path>,
        question: &Question<'_>,
    ) -> Option<Answer> {
        let (method, _) = operation(question.operation)?;
        let client = tokio::time::timeout(question.budget, self.client_for(file, cwd))
            .await
            .ok()
            .flatten()?;

        if question.operation != "workspace_symbols" {
            let text = tokio::fs::read_to_string(file).await.ok()?;
            client.open(file, &text).await;
        }

        let position = json!({
            "line": question.line.saturating_sub(1),
            "character": question.character.saturating_sub(1),
        });
        let document = json!({ "uri": to_uri(file) });
        let params = match question.operation {
            "workspace_symbols" => json!({ "query": question.symbol.unwrap_or_default() }),
            "symbols" => json!({ "textDocument": document }),
            "references" => json!({
                "textDocument": document,
                "position": position,
                "context": { "includeDeclaration": false },
            }),
            _ => json!({ "textDocument": document, "position": position }),
        };

        let raw = match client.ask(method, params, Some(question.budget)).await {
            Ok(raw) => raw,
            Err(error) => return Some(Answer::Unsupported { error }),
        };

        Some(match question.operation {
            "hover" => Answer::Hover(to_hover(&raw)),
            "symbols" | "workspace_symbols" => {
                Answer::Symbols(to_symbols(&raw, Some(&spelling(file))))
            }
            _ => Answer::Locations(to_locations(&raw)),
        })
    }

    /// Stop every server. Safe to call twice, and safe having started none.
    ///
    /// Bounded at every step, because this runs inside the quit path. A server
    /// that will not answer `shutdown` is killed. The alternative is an app
    /// that appears to hang on Quit, which users resolve with the task manager
    /// and which then leaves behind exactly the orphaned server this is here to
    /// prevent.
    pub async fn shutdown(&self) {
        let running: Vec<Ready> = self.clients.lock().drain().map(|(_, slot)| slot).collect();
        for slot in running {
            // A connection still handshaking is abandoned rather than waited
            // for: its transport is dropped with the task, which kills the
            // child.
            let Ok(Ok(Some(client))) = tokio::time::timeout(Duration::from_millis(100), slot).await
            else {
                continue;
            };
            let _ = tokio::time::timeout(Duration::from_secs(2), client.shutdown()).await;
        }
    }

    /// Forget every server and cached lookup. For tests.
    pub async fn reset(&self) {
        self.shutdown().await;
        self.broken.lock().clear();
        self.restarts.lock().clear();
        servers::reset();
    }
}

/// A protocol Location, LocationLink or the arrays of either, flattened.
pub fn to_locations(result: &Value) -> Vec<LocationRow> {
    let list: Vec<&Value> = match result {
        Value::Array(items) => items.iter().collect(),
        Value::Null => Vec::new(),
        one => vec![one],
    };

    let mut out = Vec::new();
    for entry in list {
        if !entry.is_object() {
            continue;
        }
        let uri = entry
            .get("uri")
            .or_else(|| entry.get("targetUri"))
            .and_then(Value::as_str);
        let range = entry
            .get("range")
            .or_else(|| entry.get("targetSelectionRange"))
            .or_else(|| entry.get("targetRange"));
        let (Some(uri), Some(range)) = (uri, range) else {
            continue;
        };
        // A server may answer with an `untitled:` or `jar:` document, which is
        // not a file anyone can open. Nothing useful can be said about it.
        let Some(file) = from_uri(uri) else { continue };

        let start_line = at(range, "start", "line");
        out.push(LocationRow {
            file: spelling(&file),
            line: start_line + 1,
            character: at(range, "start", "character") + 1,
            end_line: range
                .pointer("/end/line")
                .and_then(Value::as_u64)
                .unwrap_or(start_line)
                + 1,
        });
    }
    out
}

fn at(range: &Value, end: &str, field: &str) -> u64 {
    range
        .pointer(&format!("/{end}/{field}"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// Hover contents, in any of the three shapes the protocol allows.
pub fn to_hover(result: &Value) -> String {
    let Some(contents) = result.get("contents") else {
        return String::new();
    };
    let one = |value: &Value| match value {
        Value::String(text) => text.clone(),
        Value::Object(_) => value
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    let text = match contents {
        Value::Array(items) => items.iter().map(one).collect::<Vec<_>>().join("\n\n"),
        other => one(other),
    };
    text.trim().to_string()
}

/// DocumentSymbol (nested) or SymbolInformation (flat), as one flat list.
pub fn to_symbols(result: &Value, file: Option<&Path>) -> Vec<SymbolRow> {
    let mut out = Vec::new();
    let Some(items) = result.as_array() else {
        return out;
    };
    walk_symbols(items, None, file, &mut out);
    out
}

fn walk_symbols(
    items: &[Value],
    container: Option<&str>,
    file: Option<&Path>,
    out: &mut Vec<SymbolRow>,
) {
    for item in items {
        if !item.is_object() {
            continue;
        }
        let range = item
            .get("selectionRange")
            .or_else(|| item.get("range"))
            .or_else(|| item.pointer("/location/range"));
        let named = item
            .pointer("/location/uri")
            .and_then(Value::as_str)
            .and_then(from_uri);
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        out.push(SymbolRow {
            kind: SYMBOL_KINDS
                .get(item.get("kind").and_then(Value::as_u64).unwrap_or(0) as usize)
                .copied()
                .unwrap_or("symbol")
                .to_string(),
            container: item
                .get("containerName")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .or_else(|| container.map(str::to_string)),
            file: named
                .map(|path| spelling(&path))
                .or_else(|| file.map(Path::to_path_buf)),
            line: range.map(|range| at(range, "start", "line")).unwrap_or(0) + 1,
            name: name.clone(),
        });

        if let Some(children) = item.get("children").and_then(Value::as_array) {
            walk_symbols(children, Some(&name), file, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{fake_launcher, DeadLauncher};

    /// A project pointed at the fake server for `.demo` files, the same way a
    /// user points Inertia at a server it has never heard of.
    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut config = dir.path().to_path_buf();
        config.push(".inertia.json");
        std::fs::write(
            &config,
            r#"{"lsp":{"demo":{"command":["demo-server"],"extensions":[".demo"],"rootFiles":[]}}}"#,
        )
        .unwrap();
        for (name, contents) in files {
            let mut file = dir.path().to_path_buf();
            file.push(name);
            std::fs::write(&file, contents).unwrap();
        }
        servers::reset();
        dir
    }

    fn at(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let mut file = dir.path().to_path_buf();
        file.push(name);
        file
    }

    #[tokio::test]
    async fn a_file_no_server_claims_reports_nothing() {
        let dir = project(&[("notes.txt", "hello\n")]);
        let lsp = Lsp::with_launcher(fake_launcher());
        assert_eq!(
            lsp.report(&at(&dir, "notes.txt"), Some(dir.path()), 0)
                .await,
            ""
        );
        lsp.reset().await;
    }

    #[tokio::test]
    async fn diagnostics_come_back_with_the_edit_that_caused_them() {
        let dir = project(&[("a.demo", "fine\n@@error still broken\n")]);
        let lsp = Lsp::with_launcher(fake_launcher());
        let out = lsp.report(&at(&dir, "a.demo"), Some(dir.path()), 0).await;
        assert!(
            out.contains("LSP errors detected in this file"),
            "got {out:?}"
        );
        assert!(out.contains("ERROR [2:1] still broken"), "got {out:?}");
        lsp.reset().await;
    }

    /// `write` replaces a whole file and routinely breaks its importers, which
    /// is the one case the model has no other way to find out about.
    #[tokio::test]
    async fn a_write_may_report_the_files_it_broke() {
        let dir = project(&[
            ("a.demo", "@@other b.demo importer is broken now\n"),
            ("b.demo", "ok\n"),
        ]);
        let lsp = Lsp::with_launcher(fake_launcher());

        let quiet = lsp.report(&at(&dir, "a.demo"), Some(dir.path()), 0).await;
        assert!(!quiet.contains("other files"), "got {quiet:?}");

        let loud = lsp
            .report(&at(&dir, "a.demo"), Some(dir.path()), MAX_OTHER_FILES)
            .await;
        assert!(
            loud.contains("LSP errors detected in other files"),
            "got {loud:?}"
        );
        assert!(loud.contains("importer is broken now"), "got {loud:?}");
        lsp.reset().await;
    }

    /// The whole promise: a server that will not start is silence, not an
    /// error out of a write that already landed.
    #[tokio::test]
    async fn a_server_that_will_not_start_is_silence() {
        let dir = project(&[("a.demo", "@@error still broken\n")]);
        let lsp = Lsp::with_launcher(Arc::new(DeadLauncher));
        assert_eq!(
            lsp.report(&at(&dir, "a.demo"), Some(dir.path()), 0).await,
            ""
        );
        // ...and it is not tried again for the rest of the session.
        assert_eq!(
            lsp.report(&at(&dir, "a.demo"), Some(dir.path()), 0).await,
            ""
        );
        assert!(lsp.broken.lock().len() == 1);
        lsp.reset().await;
    }

    #[tokio::test]
    async fn a_question_about_a_file_with_no_server_has_no_answer() {
        let dir = project(&[("notes.txt", "hello\n")]);
        let lsp = Lsp::with_launcher(fake_launcher());
        let answer = lsp
            .query(
                &at(&dir, "notes.txt"),
                Some(dir.path()),
                &Question::new("definition"),
            )
            .await;
        assert!(answer.is_none());
        lsp.reset().await;
    }

    #[tokio::test]
    async fn a_definition_comes_back_one_based() {
        let dir = project(&[("a.demo", "@@def thing\ncall thing()\n")]);
        let lsp = Lsp::with_launcher(fake_launcher());
        let mut question = Question::new("definition");
        question.line = 2;
        question.character = 6; // the `t` of `thing`
        let answer = lsp
            .query(&at(&dir, "a.demo"), Some(dir.path()), &question)
            .await
            .expect("a server");
        match answer {
            Answer::Locations(rows) => {
                assert_eq!(rows.len(), 1, "got {rows:?}");
                assert_eq!(rows[0].line, 1);
                assert_eq!(rows[0].character, 1);
            }
            other => panic!("got {other:?}"),
        }
        lsp.reset().await;
    }

    #[tokio::test]
    async fn symbols_carry_their_kind_and_line() {
        let dir = project(&[("a.demo", "@@sym 12 doTheThing\n@@sym 5 Widget\n")]);
        let lsp = Lsp::with_launcher(fake_launcher());
        let answer = lsp
            .query(
                &at(&dir, "a.demo"),
                Some(dir.path()),
                &Question::new("symbols"),
            )
            .await
            .expect("a server");
        match answer {
            Answer::Symbols(rows) => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].kind, "function");
                assert_eq!(rows[0].name, "doTheThing");
                assert_eq!(rows[0].line, 1);
                assert_eq!(rows[1].kind, "class");
            }
            other => panic!("got {other:?}"),
        }
        lsp.reset().await;
    }

    #[tokio::test]
    async fn hover_is_whatever_the_server_says() {
        let dir = project(&[("a.demo", "@@hover const x: number\n")]);
        let lsp = Lsp::with_launcher(fake_launcher());
        let answer = lsp
            .query(
                &at(&dir, "a.demo"),
                Some(dir.path()),
                &Question::new("hover"),
            )
            .await
            .expect("a server");
        match answer {
            Answer::Hover(text) => assert_eq!(text, "const x: number"),
            other => panic!("got {other:?}"),
        }
        lsp.reset().await;
    }

    /// The one test that talks to a real language server, when the machine has
    /// one.
    ///
    /// Skipped rather than failed where it does not: a suite that only passes
    /// on a developer's own laptop is a suite nobody trusts. ruff is the choice
    /// because it starts in milliseconds - gopls and rust-analyzer index a
    /// whole module before they say anything, which is minutes on a cold cache.
    #[tokio::test]
    async fn a_real_language_server_reports_a_real_mistake() {
        let Some(_) = servers::which("ruff", &[]) else {
            eprintln!("skipped: ruff is not installed on this machine");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        // A root marker, so the server is pointed at a project rather than at a
        // directory it will decide has no configuration.
        std::fs::write(at(&dir, "pyproject.toml"), "[project]\nname = \"demo\"\n").unwrap();
        // F401, in ruff's default rule set: imported and never used.
        std::fs::write(at(&dir, "a.py"), "import os\n").unwrap();
        servers::reset();

        let lsp = Lsp::new();
        let all = lsp
            .collect(&at(&dir, "a.py"), Some(dir.path()), Duration::from_secs(20))
            .await;
        lsp.reset().await;

        let Some(all) = all else {
            eprintln!("skipped: ruff did not start");
            return;
        };
        let items = all
            .get(&normalize(&at(&dir, "a.py")))
            .map(|entry| entry.items.clone())
            .unwrap_or_default();
        assert!(
            items
                .iter()
                .any(|item| item["message"].as_str().unwrap_or_default().contains("os")),
            "a real server should have noticed the unused import: {items:?}"
        );
    }

    // ── the shape normalisers, which see whatever a server feels like ────

    #[test]
    fn a_location_link_is_flattened_like_a_location() {
        let link = json!({
            "targetUri": to_uri(Path::new("/tmp/a.ts")),
            "targetSelectionRange": { "start": { "line": 4, "character": 2 }, "end": { "line": 4, "character": 8 } },
        });
        let rows = to_locations(&json!([link]));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].line, 5);
        assert_eq!(rows[0].character, 3);
    }

    #[test]
    fn a_single_location_is_treated_as_a_list_of_one() {
        let one = json!({
            "uri": to_uri(Path::new("/tmp/a.ts")),
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
        });
        assert_eq!(to_locations(&one).len(), 1);
        assert!(to_locations(&Value::Null).is_empty());
    }

    #[test]
    fn a_document_that_is_not_a_file_is_dropped() {
        let rows = to_locations(&json!([{
            "uri": "untitled:Untitled-1",
            "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
        }]));
        assert!(rows.is_empty());
    }

    #[test]
    fn nested_symbols_are_flattened_with_their_container() {
        let nested = json!([{
            "name": "Widget",
            "kind": 5,
            "selectionRange": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 6 } },
            "children": [{
                "name": "render",
                "kind": 6,
                "selectionRange": { "start": { "line": 3, "character": 2 }, "end": { "line": 3, "character": 8 } },
            }],
        }]);
        let rows = to_symbols(&nested, None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].name, "render");
        assert_eq!(rows[1].kind, "method");
        assert_eq!(rows[1].container.as_deref(), Some("Widget"));
        assert_eq!(rows[1].line, 4);
    }

    #[test]
    fn hover_reads_all_three_shapes() {
        assert_eq!(to_hover(&json!({ "contents": "plain" })), "plain");
        assert_eq!(
            to_hover(&json!({ "contents": { "kind": "markdown", "value": " fenced " } })),
            "fenced"
        );
        assert_eq!(
            to_hover(&json!({ "contents": ["one", { "value": "two" }] })),
            "one\n\ntwo"
        );
        assert_eq!(to_hover(&json!({})), "");
    }

    #[test]
    fn every_operation_names_a_protocol_method() {
        for name in operation_names() {
            assert!(operation(name).is_some(), "{name}");
        }
        assert!(operation("nonsense").is_none());
        assert_eq!(
            operation("references").unwrap().0,
            "textDocument/references"
        );
        assert!(
            !operation("symbols").unwrap().1,
            "symbols needs no position"
        );
    }
}

//! One conversation with one language server.
//!
//! A client owns a child process, the JSON-RPC bookkeeping on top of its stdio,
//! and the diagnostics that process has published so far. It knows nothing
//! about which server it is talking to or why - `servers.rs` decided that, and
//! `lib.rs` decides when.
//!
//! Two things shape almost every decision below.
//!
//! The first is that a language server is not a program that answers questions;
//! it is a program that changes its mind. Diagnostics arrive as unsolicited
//! notifications, sometimes twice for one file - a syntactic pass first and a
//! semantic pass a moment later - and the second one is the one worth reading.
//! So `wait_for_diagnostics` waits for a publish, then keeps waiting a beat for
//! a better one, and gives up on a deadline either way.
//!
//! The second is that every failure here is somebody else's success. The caller
//! is on the far side of a write that already landed. A server that will not
//! start, dies mid-sentence, or speaks a protocol we cannot parse must produce
//! exactly one outcome: no diagnostics. Never an error out of a tool that did
//! its job.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, oneshot};

use crate::protocol::{encode, Parser};
use crate::servers::{language_id, ServerSpec};
use crate::uri::{from_uri, normalize, to_uri};

/// How long `initialize` gets before the language is written off for this
/// session. gopls and rust-analyzer index a whole module before they answer, so
/// this is generous - but it is bounded, because a handshake that never
/// completes must not leave every future edit waiting on it.
pub const HANDSHAKE_TIMEOUT_MS: u64 = 15_000;

/// A request that has not been answered by now is not going to be.
pub const REQUEST_TIMEOUT_MS: u64 = 5_000;

/// After the first publish for a file, how long to wait for a better one.
///
/// TypeScript publishes syntax errors immediately and type errors a moment
/// later. Returning on the first publish reports a clean file that is about to
/// be told it has six type errors.
pub const SETTLE_MS: u64 = 200;

/// Grace between asking a server to exit and killing it.
const SHUTDOWN_GRACE_MS: u64 = 1_500;

/// Publishes seen but not yet read. Generous, because a lagging receiver loses
/// a wake-up and a lost wake-up is a missing diagnostic report.
const PUBLISH_BACKLOG: usize = 64;

/// Everything a client needs to talk to a server, with the process it belongs
/// to if there is one.
///
/// Split out from the spawn so a test can drive a client over an in-memory pipe
/// rather than over a language server nobody's machine has installed.
pub struct Transport {
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
    pub child: Option<tokio::process::Child>,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport").finish_non_exhaustive()
    }
}

/// How a server is started.
#[async_trait]
pub trait Launcher: Send + Sync + std::fmt::Debug {
    async fn launch(&self, spec: &ServerSpec) -> Result<Transport, String>;
}

/// The real one: a child process over its own stdio.
#[derive(Debug, Default)]
pub struct ProcessLauncher;

#[async_trait]
impl Launcher for ProcessLauncher {
    async fn launch(&self, spec: &ServerSpec) -> Result<Transport, String> {
        let mut command = build_command(spec);
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = command
            .spawn()
            .map_err(|error| format!("{} could not be started: {error}", spec.id))?;

        let writer = child
            .stdin
            .take()
            .ok_or_else(|| format!("{} has no stdin.", spec.id))?;
        let reader = child
            .stdout
            .take()
            .ok_or_else(|| format!("{} has no stdout.", spec.id))?;
        // Read and drop: a server whose stderr fills its pipe buffer stops
        // writing to stdout too, and then nothing works and nothing says why.
        if let Some(mut stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut sink = [0u8; 4096];
                while let Ok(read) = stderr.read(&mut sink).await {
                    if read == 0 {
                        break;
                    }
                }
            });
        }

        Ok(Transport {
            writer: Box::new(writer),
            reader: Box::new(reader),
            child: Some(child),
        })
    }
}

fn build_command(spec: &ServerSpec) -> tokio::process::Command {
    let bin = spec.bin.to_string_lossy().into_owned();

    #[cfg(windows)]
    let shim = {
        let lower = bin.to_ascii_lowercase();
        lower.ends_with(".cmd") || lower.ends_with(".bat")
    };
    #[cfg(not(windows))]
    let shim = false;

    let mut command = if shim {
        // A `.cmd` or `.bat` shim - which is what npm writes into
        // `node_modules/.bin` on Windows - is not an executable image, so it
        // goes through the command interpreter explicitly. Explicitly rather
        // than through a shell flag: a shell would re-parse the arguments as
        // one string, which is the same hole under a different name.
        let interpreter = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
        let mut command = tokio::process::Command::new(interpreter);
        let mut tail = format!("/d /s /c \"{bin}\"");
        for argument in &spec.args {
            tail.push(' ');
            if argument.contains(' ') {
                tail.push('"');
                tail.push_str(argument);
                tail.push('"');
            } else {
                tail.push_str(argument);
            }
        }
        #[cfg(windows)]
        {
            // `cmd /c` re-parses its own tail, so the quoting above must
            // survive Rust deciding to quote it a second time. tokio's own
            // `raw_arg` is the only way to say that.
            command.raw_arg(tail);
        }
        command
    } else {
        let mut command = tokio::process::Command::new(&spec.bin);
        command.args(&spec.args);
        command
    };

    command.current_dir(&spec.root);
    for (key, value) in &spec.env {
        command.env(key, value);
    }

    #[cfg(windows)]
    {
        // No console window flashing up behind the app on every spawn.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

/// What Inertia tells a server it can do. Deliberately close to nothing.
fn client_capabilities() -> Value {
    json!({
        "workspace": {
            "configuration": true,
            "workspaceFolders": true,
            "didChangeConfiguration": { "dynamicRegistration": false },
            // For `lsp`'s workspace_symbols. Here rather than in a second
            // `workspace` key further down: that one would silently replace
            // this whole object, and the client would stop declaring workspace
            // folders.
            "symbol": { "dynamicRegistration": false }
        },
        "textDocument": {
            "synchronization": { "didOpen": true, "didChange": true, "didSave": false },
            "publishDiagnostics": { "relatedInformation": false, "versionSupport": false },
            // What the `lsp` tool asks for. Declared, because a server that has
            // not been told the client understands these is entitled to refuse
            // them.
            "definition": { "dynamicRegistration": false, "linkSupport": true },
            "typeDefinition": { "dynamicRegistration": false, "linkSupport": true },
            "implementation": { "dynamicRegistration": false, "linkSupport": true },
            "references": { "dynamicRegistration": false },
            "hover": { "dynamicRegistration": false, "contentFormat": ["markdown", "plaintext"] },
            "documentSymbol": { "dynamicRegistration": false, "hierarchicalDocumentSymbolSupport": true }
        },
        "window": { "workDoneProgress": true }
    })
}

/// Everything one server has said about one file.
#[derive(Debug, Clone)]
pub struct Published {
    /// The path as the server spelled it, or as we opened it when we opened it.
    pub file: PathBuf,
    pub items: Vec<Value>,
}

#[derive(Debug, Clone)]
struct Document {
    version: i64,
    file: PathBuf,
}

struct Inner {
    id: String,
    root: PathBuf,
    initialization: Option<Value>,
    writer: tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>,
    diagnostics: Mutex<HashMap<String, Published>>,
    documents: Mutex<HashMap<String, Document>>,
    /// A publish for this key, or `None` when the server died. Both wake
    /// `wait_for_diagnostics`, which is the point: a dead server must not leave
    /// an edit waiting out its whole budget.
    publishes: broadcast::Sender<Option<String>>,
    dead: Mutex<Option<String>>,
    next_id: AtomicI64,
    child: Mutex<Option<tokio::process::Child>>,
}

impl Inner {
    /// Everything waiting on this server is told once, and nothing is left
    /// hanging.
    fn die(&self, reason: impl Into<String>) {
        let reason = reason.into();
        {
            let mut dead = self.dead.lock();
            if dead.is_some() {
                return;
            }
            *dead = Some(reason.clone());
        }
        let waiting: Vec<_> = self.pending.lock().drain().map(|(_, tx)| tx).collect();
        for tx in waiting {
            let _ = tx.send(Err(reason.clone()));
        }
        let _ = self.publishes.send(None);
        if let Some(mut child) = self.child.lock().take() {
            // Already gone is the outcome we wanted anyway.
            let _ = child.start_kill();
        }
    }

    fn dead_reason(&self) -> Option<String> {
        self.dead.lock().clone()
    }

    async fn send(&self, message: &Value) -> Result<(), String> {
        if let Some(reason) = self.dead_reason() {
            return Err(reason);
        }
        let bytes = encode(message);
        let written = {
            let mut writer = self.writer.lock().await;
            match writer.write_all(&bytes).await {
                Ok(()) => writer.flush().await,
                Err(error) => Err(error),
            }
        };
        // A pipe that will not take bytes is a server that is gone, whatever
        // its exit status says later.
        if let Err(error) = written {
            let reason = error.to_string();
            self.die(reason.clone());
            return Err(reason);
        }
        Ok(())
    }

    async fn respond(&self, id: &Value, result: Value) {
        if id.is_null() {
            return;
        }
        // The server is not waiting for this reply any more if it has gone.
        let _ = self
            .send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
            .await;
    }

    async fn handle(&self, message: Value) {
        let has_id = message.get("id").is_some_and(|id| !id.is_null());
        let method = message.get("method").and_then(Value::as_str);

        if has_id && method.is_none() {
            let Some(id) = message.get("id").and_then(Value::as_i64) else {
                return;
            };
            let Some(tx) = self.pending.lock().remove(&id) else {
                return;
            };
            let answer = match message.get("error") {
                Some(error) => Err(error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("LSP request failed")
                    .to_string()),
                None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(answer);
            return;
        }

        if method == Some("textDocument/publishDiagnostics") {
            let uri = message
                .get("params")
                .and_then(|p| p.get("uri"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(spelled) = from_uri(uri) else {
                return;
            };
            let key = normalize(&spelled);
            let items = message
                .get("params")
                .and_then(|p| p.get("diagnostics"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            // The key is flattened for lookup and unreadable. The path is kept
            // beside it as the server spelled it, because a report about a file
            // that was never opened has nothing else to name it by.
            let file = self
                .documents
                .lock()
                .get(&key)
                .map(|document| document.file.clone())
                .unwrap_or(spelled);
            self.diagnostics
                .lock()
                .insert(key.clone(), Published { file, items });
            let _ = self.publishes.send(Some(key));
            return;
        }

        // A request from the server. Answering nothing is not an option:
        // several servers block their own startup on `workspace/configuration`
        // and would sit there forever, which is the handshake timeout firing
        // for a server that started perfectly well.
        if has_id {
            let Some(id) = message.get("id").cloned() else {
                return;
            };
            match method {
                Some("workspace/configuration") => {
                    let count = message
                        .get("params")
                        .and_then(|p| p.get("items"))
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or(0);
                    let answer = self.initialization.clone().unwrap_or(Value::Null);
                    let items: Vec<Value> = (0..count).map(|_| answer.clone()).collect();
                    self.respond(&id, Value::Array(items)).await;
                }
                Some("workspace/workspaceFolders") => {
                    self.respond(&id, json!([folder(&self.root)])).await;
                }
                _ => self.respond(&id, Value::Null).await,
            }
        }
    }
}

fn folder(root: &Path) -> Value {
    json!({
        "name": root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        "uri": to_uri(root),
    })
}

/// A live connection to one language server.
pub struct Client {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("id", &self.inner.id)
            .field("root", &self.inner.root)
            .field("alive", &self.alive())
            .finish()
    }
}

impl Client {
    /// Start a server and complete the handshake.
    ///
    /// Either a client or an error; never a half-built one. Anything that fails
    /// has already killed the child, because the alternative is a language
    /// server with no owner sitting on a gigabyte of RAM for as long as the app
    /// runs.
    pub async fn connect(spec: &ServerSpec, launcher: &dyn Launcher) -> Result<Self, String> {
        let transport = launcher.launch(spec).await?;
        Self::over(transport, spec).await
    }

    /// The same handshake over a transport somebody else built.
    pub async fn over(transport: Transport, spec: &ServerSpec) -> Result<Self, String> {
        let (publishes, _) = broadcast::channel(PUBLISH_BACKLOG);
        let inner = Arc::new(Inner {
            id: spec.id.clone(),
            root: spec.root.clone(),
            initialization: spec.initialization.clone(),
            writer: tokio::sync::Mutex::new(transport.writer),
            pending: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            documents: Mutex::new(HashMap::new()),
            publishes,
            dead: Mutex::new(None),
            next_id: AtomicI64::new(1),
            child: Mutex::new(transport.child),
        });

        let reading = Arc::clone(&inner);
        let mut reader = transport.reader;
        tokio::spawn(async move {
            let mut parser = Parser::new();
            let mut chunk = [0u8; 16 * 1024];
            loop {
                let read = match reader.read(&mut chunk).await {
                    Ok(0) => {
                        reading.die(format!("{} exited.", reading.id));
                        return;
                    }
                    Ok(read) => read,
                    Err(error) => {
                        reading.die(error.to_string());
                        return;
                    }
                };
                let messages = match parser.push(&chunk[..read]) {
                    Ok(messages) => messages,
                    Err(error) => {
                        // The framing is broken, so every byte after this one
                        // is at an unknown offset. There is no recovering a
                        // stream like that.
                        reading.die(error);
                        return;
                    }
                };
                for message in messages {
                    reading.handle(message).await;
                }
            }
        });

        let client = Self { inner };
        let handshake = client
            .request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                    "clientInfo": { "name": "inertia" },
                    "rootUri": to_uri(&spec.root),
                    "rootPath": spec.root.to_string_lossy(),
                    "workspaceFolders": [folder(&spec.root)],
                    "initializationOptions": spec.initialization.clone().unwrap_or(Value::Null),
                    "capabilities": client_capabilities(),
                }),
                Duration::from_millis(HANDSHAKE_TIMEOUT_MS),
            )
            .await;

        if let Err(error) = handshake {
            client.inner.die(error.clone());
            return Err(error);
        }
        client.notify("initialized", json!({})).await;
        if let Some(initialization) = spec.initialization.clone() {
            client
                .notify(
                    "workspace/didChangeConfiguration",
                    json!({ "settings": initialization }),
                )
                .await;
        }
        Ok(client)
    }

    pub fn id(&self) -> &str {
        &self.inner.id
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    pub fn alive(&self) -> bool {
        self.inner.dead.lock().is_none()
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        if let Some(reason) = self.inner.dead_reason() {
            return Err(reason);
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().insert(id, tx);

        if let Err(error) = self
            .inner
            .send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await
        {
            self.inner.pending.lock().remove(&id);
            return Err(error);
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(format!("{} stopped answering.", self.inner.id)),
            Err(_) => {
                self.inner.pending.lock().remove(&id);
                Err(format!(
                    "{} did not answer {method} within {} ms.",
                    self.inner.id,
                    timeout.as_millis()
                ))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) {
        // A notification nobody is listening to is not a failure anyone can act
        // on: the server is dead and every caller is about to find that out.
        let _ = self
            .inner
            .send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await;
    }

    /// One request, for the things the `lsp` tool asks about.
    ///
    /// A narrow passthrough rather than the whole protocol: the caller names a
    /// method and gets the raw result, and everything about turning that into
    /// something a model can read is done above, where the shapes of the four
    /// answers can be normalised in one place.
    pub async fn ask(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value, String> {
        self.request(
            method,
            params,
            timeout.unwrap_or(Duration::from_millis(REQUEST_TIMEOUT_MS)),
        )
        .await
    }

    /// Tell the server what a file now contains.
    ///
    /// The first call for a file is a `didOpen` and every later one is a
    /// `didChange` with the whole text. Sending the whole text is the
    /// unfashionable choice and it is the right one here: incremental sync
    /// means tracking a per-document mirror and computing ranges against it,
    /// and a mirror that drifts by one character makes the server report errors
    /// at positions that do not exist in the file the user is looking at.
    pub async fn open(&self, file: &Path, text: &str) -> i64 {
        let key = normalize(file);
        let absolute = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());

        let existing = self.inner.documents.lock().get(&key).cloned();
        match existing {
            None => {
                self.inner.documents.lock().insert(
                    key,
                    Document {
                        version: 1,
                        file: absolute,
                    },
                );
                self.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": to_uri(file),
                            "languageId": language_id(file),
                            "version": 1,
                            "text": text,
                        }
                    }),
                )
                .await;
                1
            }
            Some(document) => {
                let version = document.version + 1;
                self.inner.documents.lock().insert(
                    key,
                    Document {
                        version,
                        file: document.file,
                    },
                );
                self.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": to_uri(file), "version": version },
                        "contentChanges": [{ "text": text }],
                    }),
                )
                .await;
                version
            }
        }
    }

    /// Wait for this file's diagnostics to arrive and settle.
    ///
    /// Returns on the deadline no matter what, because the caller is an edit
    /// result and an edit result cannot wait. A server that publishes nothing
    /// for a clean file - some do - is indistinguishable from a slow one, and
    /// both end the same way: the deadline passes and whatever is cached, which
    /// is usually nothing, is what gets reported.
    pub async fn wait_for_diagnostics(&self, file: &Path, budget: Duration) -> Vec<Value> {
        let key = normalize(file);
        let mut publishes = self.inner.publishes.subscribe();
        let deadline = tokio::time::Instant::now() + budget;
        let mut settle: Option<tokio::time::Instant> = None;

        while self.inner.dead_reason().is_none() {
            let wake = settle.map_or(deadline, |at| at.min(deadline));
            if wake <= tokio::time::Instant::now() {
                break;
            }
            match tokio::time::timeout_at(wake, publishes.recv()).await {
                // The deadline, or the settle window closing. Either way this
                // is the answer.
                Err(_) => break,
                // The server died. There is nothing more coming.
                Ok(Ok(None)) | Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Ok(Ok(Some(published))) if published == key => {
                    // Restarted on every publish for this file, so a server
                    // that sends three passes is heard out rather than reported
                    // on its first.
                    settle = Some(tokio::time::Instant::now() + Duration::from_millis(SETTLE_MS));
                }
                // Another file, or a receiver that fell behind. Neither says
                // anything about this file.
                Ok(Ok(Some(_))) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            }
        }

        self.inner
            .diagnostics
            .lock()
            .get(&key)
            .map(|entry| entry.items.clone())
            .unwrap_or_default()
    }

    /// Everything this server has said about anything, keyed by lookup key.
    pub fn all(&self) -> HashMap<String, Published> {
        self.inner.diagnostics.lock().clone()
    }

    /// Ask the server to stop, and make sure it does.
    ///
    /// The polite sequence is a `shutdown` request then an `exit` notification,
    /// and a server that ignores either is killed. A leaked language server is
    /// a gigabyte of RAM with no window and no icon; the user cannot find it to
    /// close it and would have no reason to think it was ours.
    pub async fn shutdown(&self) {
        if self.inner.dead_reason().is_some() {
            return;
        }
        let grace = Duration::from_millis(SHUTDOWN_GRACE_MS);
        if self.request("shutdown", Value::Null, grace).await.is_ok() {
            self.notify("exit", Value::Null).await;
        }

        let child = self.inner.child.lock().take();
        if let Some(mut child) = child {
            match tokio::time::timeout(grace, child.wait()).await {
                Ok(_) => {}
                // It was never going to exit on its own; the kill is the real
                // shutdown.
                Err(_) => {
                    let _ = child.start_kill();
                }
            }
        }
        self.inner.die(format!("{} shut down.", self.inner.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{fake_server, spec_for};

    #[tokio::test]
    async fn a_server_that_is_not_installed_says_so_plainly() {
        let dir = tempfile::tempdir().unwrap();
        let spec = ServerSpec {
            id: "demo".to_string(),
            bin: PathBuf::from("inertia-no-such-language-server"),
            args: Vec::new(),
            root: dir.path().to_path_buf(),
            env: Vec::new(),
            initialization: None,
        };
        let error = Client::connect(&spec, &ProcessLauncher)
            .await
            .expect_err("nothing by that name is installed");
        assert!(
            error.starts_with("demo could not be started: "),
            "got {error}"
        );
    }

    #[tokio::test]
    async fn a_handshake_completes_and_a_request_is_answered() {
        let dir = tempfile::tempdir().unwrap();
        let spec = spec_for(dir.path());
        let transport = fake_server();
        let client = Client::over(transport, &spec).await.unwrap();
        assert!(client.alive());

        let answer = client
            .ask(
                "textDocument/definition",
                json!({ "textDocument": { "uri": to_uri(&dir.path().join("a.demo")) } }),
                None,
            )
            .await
            .unwrap();
        assert!(answer.is_array(), "got {answer}");
    }

    #[tokio::test]
    async fn diagnostics_published_for_an_open_file_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = dir.path().to_path_buf();
        file.push("a.demo");
        std::fs::write(&file, "@@error still broken\n").unwrap();

        let spec = spec_for(dir.path());
        let client = Client::over(fake_server(), &spec).await.unwrap();
        client.open(&file, "@@error still broken\n").await;

        let items = client
            .wait_for_diagnostics(&file, Duration::from_millis(2_000))
            .await;
        assert_eq!(items.len(), 1, "got {items:?}");
        assert_eq!(items[0]["message"], "still broken");
    }

    #[tokio::test]
    async fn a_dead_server_does_not_hold_an_edit_up_for_its_whole_budget() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = dir.path().to_path_buf();
        file.push("a.demo");

        let spec = spec_for(dir.path());
        let client = Client::over(fake_server(), &spec).await.unwrap();
        client.inner.die("the server fell over");

        let started = std::time::Instant::now();
        let items = client
            .wait_for_diagnostics(&file, Duration::from_secs(30))
            .await;
        assert!(items.is_empty());
        assert!(started.elapsed() < Duration::from_secs(5), "it waited");
    }
}

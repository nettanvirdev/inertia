//! A live connection to one MCP server.
//!
//! Owns the child process, the framing, and the map of requests waiting for a
//! reply. Three behaviours here are the difference between "works on my
//! machine" and "works":
//!
//!   - **Stderr is captured.** A server that dies before speaking JSON-RPC - a
//!     bad token, a missing runtime - produces a stack trace on stderr and a
//!     non-zero exit, not a protocol error. Without the capture, the user sees
//!     "the server closed" and has nothing to act on.
//!   - **Progress restarts the clock.** A tool call that legitimately takes ten
//!     minutes must not be killed at sixty seconds, and the server says so by
//!     reporting progress.
//!   - **The whole process tree is killed.** These servers are usually launched
//!     through a shim (`npx`, `uvx`), so killing the process we spawned leaves
//!     the real server running and holding its port.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

use crate::protocol::{
    self, Incoming, RpcError, CONNECT_TIMEOUT_MS, MAX_TOOL_PAGES, METHOD_NOT_FOUND,
    PROTOCOL_VERSION, REQUEST_TIMEOUT_MS,
};

/// The last of a server's diagnostic output to keep.
///
/// Enough for a stack trace and the lines around it; bounded so a server that
/// logs every request cannot grow without limit.
const STDERR_LIMIT: usize = 8_000;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("could not start `{command}`: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the server closed the connection{}", .stderr.as_ref().map(|s| format!(":\n{s}")).unwrap_or_default())]
    Closed { stderr: Option<String> },
    #[error("{0}")]
    Rpc(RpcError),
    #[error("the server did not answer within {0}s")]
    Timeout(u64),
    #[error("{0}")]
    Other(String),
}

/// How a server is reached.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A child process speaking newline-delimited JSON on its stdio. The
    /// common case by a wide margin, so it is the default for a record that
    /// does not say.
    #[default]
    Stdio,
    /// A URL taking one POST per message.
    Http,
}

/// One configured server, as stored in the workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerRecord {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub enabled: bool,

    // stdio
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,

    // http
    pub url: String,
    pub headers: HashMap<String, String>,

    /// Per-server override of the request timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    // Last-known state, so the settings pane can show something before a
    // connection is attempted. Process state itself is never persisted.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub tool_count: usize,
}

/// A tool as the server describes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Value,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

/// A connected server.
pub struct Connection {
    record: ServerRecord,
    outgoing: tokio::sync::mpsc::UnboundedSender<String>,
    pending: Pending,
    next_id: AtomicU64,
    /// Bumped whenever the server reports progress for any request, so a
    /// waiting call can tell "silent" from "still working".
    progress: Arc<AtomicU64>,
    stderr: Arc<Mutex<String>>,
    /// Killing the tree rather than the process we spawned: these servers are
    /// usually behind a shim.
    child_id: Option<u32>,
    pub tools: Vec<ToolDescriptor>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("server", &self.record.id)
            .field("tools", &self.tools.len())
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Connects to a server over HTTP and completes the handshake.
    ///
    /// Streamable HTTP: every message is a POST to the one URL, and the reply
    /// is either a JSON document or an SSE stream carrying several. A server
    /// that hands back a session id on initialize gets it on every later
    /// request, which is what lets it keep per-client state.
    ///
    /// The transport swaps underneath and nothing above it changes: `request`
    /// and `notify` still write JSON lines into `outgoing`, and replies still
    /// arrive through `dispatch`. That is the whole reason this fits in one
    /// function rather than being a second client.
    pub async fn connect_http(record: ServerRecord) -> Result<Self, McpError> {
        let url = record.url.trim().to_string();
        if url.is_empty() {
            return Err(McpError::Other(
                "This server is set to HTTP but has no URL.".to_string(),
            ));
        }

        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in &record.headers {
            // A header the user typed can be anything. One that cannot be sent
            // is skipped rather than failing the connection, because the
            // others may be all the server needed.
            if let (Ok(name), Ok(value)) = (
                reqwest::header::HeaderName::try_from(name.as_str()),
                reqwest::header::HeaderValue::from_str(value.trim()),
            ) {
                headers.insert(name, value);
            }
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(
                record.timeout_ms.unwrap_or(REQUEST_TIMEOUT_MS),
            ))
            .default_headers(headers)
            .build()
            .map_err(|e| McpError::Other(format!("could not build an HTTP client: {e}")))?;

        let (outgoing, mut messages) = tokio::sync::mpsc::unbounded_channel::<String>();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let progress = Arc::new(AtomicU64::new(0));
        let stderr = Arc::new(Mutex::new(String::new()));

        {
            let pending = pending.clone();
            let progress = progress.clone();
            let outgoing = outgoing.clone();
            let stderr = stderr.clone();
            let url = url.clone();

            tokio::spawn(async move {
                // Handed back by the server on initialize, and required on
                // every request after it.
                let mut session: Option<String> = None;

                while let Some(line) = messages.recv().await {
                    let request = http
                        .post(&url)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        // Both, because the server chooses which it sends.
                        .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
                        .header("mcp-protocol-version", protocol::PROTOCOL_VERSION);
                    let request = match &session {
                        Some(id) => request.header("mcp-session-id", id.as_str()),
                        None => request,
                    };

                    let response = match request.body(line).send().await {
                        Ok(response) => response,
                        Err(e) => {
                            *stderr.lock() = format!("Could not reach {url}: {e}");
                            break;
                        }
                    };

                    if let Some(id) = response
                        .headers()
                        .get("mcp-session-id")
                        .and_then(|v| v.to_str().ok())
                    {
                        session = Some(id.to_string());
                    }

                    let status = response.status();
                    let content_type = response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let body = response.text().await.unwrap_or_default();

                    if !status.is_success() {
                        // Kept rather than thrown: this is what the settings
                        // pane shows under "Server output", and a bare status
                        // explains nothing.
                        *stderr.lock() = format!(
                            "{} {}",
                            status.as_u16(),
                            body.trim().chars().take(600).collect::<String>()
                        );
                        break;
                    }

                    // 202 with no body is how a notification is acknowledged.
                    if body.trim().is_empty() {
                        continue;
                    }

                    if content_type.contains("text/event-stream") {
                        for message in sse_messages(&body) {
                            if let Ok(value) = serde_json::from_str::<Value>(&message) {
                                dispatch(&value, &pending, &progress, &outgoing);
                            }
                        }
                    } else if let Ok(value) = serde_json::from_str::<Value>(&body) {
                        // A batch answer is an array; a single one is not.
                        match value {
                            Value::Array(items) => {
                                for item in items {
                                    dispatch(&item, &pending, &progress, &outgoing);
                                }
                            }
                            single => dispatch(&single, &pending, &progress, &outgoing),
                        }
                    }
                }

                // Nothing waiting will ever be answered now. Dropped rather
                // than answered with a synthetic error, so the waiting side
                // reports the closure with the diagnostics attached.
                pending.lock().clear();
            });
        }

        let mut connection = Self {
            record,
            outgoing,
            pending,
            next_id: AtomicU64::new(1),
            progress,
            stderr,
            // Nothing was spawned, so there is no tree to kill.
            child_id: None,
            tools: Vec::new(),
        };

        connection.handshake().await?;
        Ok(connection)
    }

    /// Spawns a stdio server and completes the handshake.
    pub async fn connect_stdio(record: ServerRecord) -> Result<Self, McpError> {
        let mut command = spawn_command(&record);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Inherit the parent environment - PATH, HOME, proxy settings -
            // with the record's own on top. A server launched without PATH
            // cannot find the runtime it needs.
            .envs(&record.env);

        let mut child = command.spawn().map_err(|source| McpError::Spawn {
            command: record.command.clone(),
            source,
        })?;

        let child_id = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let progress = Arc::new(AtomicU64::new(0));
        let stderr = Arc::new(Mutex::new(String::new()));
        let (outgoing, mut to_send) = tokio::sync::mpsc::unbounded_channel::<String>();

        // Writer.
        if let Some(mut stdin) = stdin {
            tokio::spawn(async move {
                while let Some(line) = to_send.recv().await {
                    if stdin.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                    if stdin.write_all(b"\n").await.is_err() {
                        break;
                    }
                    let _ = stdin.flush().await;
                }
            });
        }

        // Diagnostics.
        if let Some(pipe) = stderr_pipe {
            let sink = stderr.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut buffer = sink.lock();
                    buffer.push_str(&line);
                    buffer.push('\n');
                    // Keep the tail: the failure is at the end.
                    if buffer.len() > STDERR_LIMIT {
                        let cut = buffer.len() - STDERR_LIMIT;
                        let cut = (cut..buffer.len())
                            .find(|i| buffer.is_char_boundary(*i))
                            .unwrap_or(buffer.len());
                        *buffer = buffer[cut..].to_string();
                    }
                }
            });
        }

        // Reader.
        if let Some(stdout) = stdout {
            let pending = pending.clone();
            let progress = progress.clone();
            let outgoing = outgoing.clone();
            tokio::spawn(async move {
                let mut parser = protocol::FrameParser::new();
                let mut reader = BufReader::new(stdout);
                let mut chunk = String::new();

                loop {
                    chunk.clear();
                    match reader.read_line(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }

                    for message in parser.push(&chunk) {
                        dispatch(&message, &pending, &progress, &outgoing);
                    }
                }

                // The connection is gone; nothing waiting will ever be
                // answered. Dropping the senders rather than sending a
                // synthetic error is deliberate: the waiting side then reports
                // the closure *with the server's stderr attached*, which is
                // the only thing that explains why it died.
                pending.lock().clear();
            });
        }

        let mut connection = Self {
            record,
            outgoing,
            pending,
            next_id: AtomicU64::new(1),
            progress,
            stderr,
            child_id,
            tools: Vec::new(),
        };

        connection.handshake().await?;
        Ok(connection)
    }

    /// `initialize`, then `notifications/initialized`, then the tool list.
    async fn handshake(&mut self) -> Result<(), McpError> {
        let initialize = tokio::time::timeout(
            Duration::from_millis(CONNECT_TIMEOUT_MS),
            self.request(
                "initialize",
                Some(serde_json::json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    // Deliberately empty: this client does not implement
                    // sampling, roots or elicitation, and says so by not
                    // advertising them.
                    "capabilities": {},
                    "clientInfo": { "name": "Inertia", "version": env!("CARGO_PKG_VERSION") },
                })),
            ),
        )
        .await
        .map_err(|_| McpError::Timeout(CONNECT_TIMEOUT_MS / 1_000))??;

        // Fire-and-forget, and sent before anything else is asked: lenient
        // servers tolerate the omission, strict ones hang without it.
        self.notify("notifications/initialized", None);

        let has_tools = initialize
            .get("capabilities")
            .and_then(|c| c.get("tools"))
            .is_some();

        if has_tools {
            self.tools = self.list_tools().await?;
        }

        Ok(())
    }

    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;

        for _ in 0..MAX_TOOL_PAGES {
            let params = cursor
                .as_ref()
                .map(|c| serde_json::json!({ "cursor": c }));
            let page = self.request("tools/list", params).await?;

            if let Some(found) = page.get("tools").and_then(Value::as_array) {
                for descriptor in found {
                    if let Ok(tool) = serde_json::from_value::<ToolDescriptor>(descriptor.clone()) {
                        tools.push(tool);
                    }
                }
            }

            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);

            if cursor.is_none() {
                break;
            }
        }
        // Falling out of the loop with a cursor still set means the server is
        // paging forever. Twenty pages is already far more than any real tool
        // list, so what we have is what it gets.

        Ok(tools)
    }

    /// Calls a tool.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        self.request(
            "tools/call",
            Some(serde_json::json!({ "name": name, "arguments": arguments })),
        )
        .await
    }

    fn notify(&self, method: &str, params: Option<Value>) {
        let message = protocol::Notification::new(method, params);
        if let Ok(line) = serde_json::to_string(&message) {
            let _ = self.outgoing.send(line);
        }
    }

    /// Sends a request and waits for its reply.
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, mut receiver) = oneshot::channel();
        self.pending.lock().insert(id, sender);

        // Asking for progress is what lets a slow call stay alive. The token
        // is the request id, which every server echoes back unchanged.
        let params = Some(protocol::with_progress_token(params, id));
        let message = protocol::Request::new(id, method, params);

        let line = serde_json::to_string(&message)
            .map_err(|e| McpError::Other(format!("could not encode the request: {e}")))?;

        if self.outgoing.send(line).is_err() {
            self.pending.lock().remove(&id);
            return Err(McpError::Closed {
                stderr: self.diagnostics(),
            });
        }

        let interval = Duration::from_millis(self.record.timeout_ms.unwrap_or(REQUEST_TIMEOUT_MS));

        // Wait in intervals rather than once. If the interval elapses but the
        // server has reported progress since the last check, it is working and
        // the clock restarts. A server that has gone genuinely silent still
        // times out.
        loop {
            let before = self.progress.load(Ordering::Relaxed);

            match tokio::time::timeout(interval, &mut receiver).await {
                Ok(Ok(Ok(result))) => return Ok(result),
                Ok(Ok(Err(error))) => return Err(McpError::Rpc(error)),
                // The reader task dropped the sender: the connection died.
                Ok(Err(_)) => {
                    // The stderr reader is a separate task and may still be
                    // draining the pipe. Whatever the server said on its way
                    // out is the only useful part of this error, so it is
                    // worth a moment to let it arrive.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    return Err(McpError::Closed {
                        stderr: self.diagnostics(),
                    });
                }
                Err(_) => {
                    if self.progress.load(Ordering::Relaxed) == before {
                        self.pending.lock().remove(&id);
                        return Err(McpError::Timeout(interval.as_secs()));
                    }
                    // Progress arrived; keep waiting.
                }
            }
        }
    }

    /// Whatever the server said on stderr, if anything.
    pub fn diagnostics(&self) -> Option<String> {
        let captured = self.stderr.lock();
        let trimmed = captured.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    pub fn record(&self) -> &ServerRecord {
        &self.record
    }

    /// Stops the server and everything it started.
    pub fn shutdown(&self) {
        let Some(pid) = self.child_id else {
            return;
        };
        kill_tree(pid);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Builds the spawn command for a stdio server.
///
/// On Windows the common launchers (`npx`, `uvx`) are `.cmd` shims, which
/// cannot be spawned directly. Going through `cmd.exe` is the only way, and it
/// has to be `/d /s /c` with the command line quoted as one argument or paths
/// containing spaces break.
fn spawn_command(record: &ServerRecord) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut line = quote_windows(&record.command);
        for arg in &record.args {
            line.push(' ');
            line.push_str(&quote_windows(arg));
        }

        let mut command = tokio::process::Command::new("cmd.exe");
        command.arg("/d").arg("/s").arg("/c").raw_arg(format!("\"{line}\""));
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = tokio::process::Command::new(&record.command);
        command.args(&record.args);
        // Its own process group, so the whole tree can be signalled at once.
        command.process_group(0);
        command
    }
}

#[cfg(windows)]
fn quote_windows(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    if !value.contains([' ', '\t', '"']) {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('"', "\\\""))
}

/// Kills a process and everything it spawned.
///
/// The server is usually behind a shim, so killing only what we spawned leaves
/// the real server running and holding whatever it had open.
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/pid", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        // Negative pid addresses the process group set at spawn time.
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &format!("-{pid}")])
            .status();
    }
}


/// Routes one decoded message to whoever is waiting for it.
///
/// Shared by both transports. The stdio reader and the HTTP pump differ only in
/// where the bytes came from; what a reply means is the same either way, and a
/// second copy of this is how the two would eventually disagree about a server
/// request nobody answered.
fn dispatch(
    message: &Value,
    pending: &Pending,
    progress: &Arc<AtomicU64>,
    outgoing: &tokio::sync::mpsc::UnboundedSender<String>,
) {
    match protocol::classify(message) {
        Some(Incoming::Response { id, result }) => {
            if let Some(sender) = pending.lock().remove(&id) {
                let _ = sender.send(Ok(result));
            }
        }
        Some(Incoming::Failure { id, error }) => {
            if let Some(sender) = pending.lock().remove(&id) {
                let _ = sender.send(Err(error));
            }
        }
        Some(Incoming::Notification { method, .. }) if method == "notifications/progress" => {
            // Any progress at all means the server is alive and working.
            progress.fetch_add(1, Ordering::Relaxed);
        }
        Some(Incoming::ServerRequest { id, method }) => {
            // Answered rather than ignored: an unanswered request leaves the
            // server waiting forever.
            let reply = serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": METHOD_NOT_FOUND,
                    "message": format!("Inertia does not implement {method}."),
                }
            });
            let _ = outgoing.send(reply.to_string());
        }
        _ => {}
    }
}

/// Pulls the JSON messages out of an SSE body.
///
/// A streamable-HTTP server may answer one POST with a stream of events rather
/// than a single document, so the reply to a request can arrive several frames
/// in, behind progress notifications. Only `data:` carries payload here; event
/// names and comments are framing.
///
/// Multiple `data:` lines in one event are joined with **no separator**, the
/// same deviation from the SSE spec the provider transport makes and for the
/// same reason: the payload is JSON, and a split lands mid-token where a
/// newline would corrupt it.
pub(crate) fn sse_messages(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();

    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            current.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_is_the_default_kind() {
        let record: ServerRecord = serde_json::from_str(r#"{"id":"a"}"#).unwrap();
        assert_eq!(record.kind, Kind::Stdio);
    }

    #[test]
    fn the_transport_field_is_called_type_on_disk() {
        let record: ServerRecord =
            serde_json::from_str(r#"{"id":"a","type":"http","url":"https://x"}"#).unwrap();
        assert_eq!(record.kind, Kind::Http);
        assert_eq!(record.url, "https://x");
    }

    #[test]
    fn a_tool_descriptor_reads_the_camel_case_schema_field() {
        let descriptor: ToolDescriptor = serde_json::from_str(
            r#"{"name":"read","description":"reads","inputSchema":{"type":"object"}}"#,
        )
        .unwrap();
        assert_eq!(descriptor.name, "read");
        assert_eq!(descriptor.input_schema["type"], "object");
    }

    /// A server that describes a tool with no schema is common, and must not
    /// fail to parse.
    #[test]
    fn a_tool_without_a_schema_still_parses() {
        let descriptor: ToolDescriptor = serde_json::from_str(r#"{"name":"ping"}"#).unwrap();
        assert_eq!(descriptor.name, "ping");
        assert!(descriptor.description.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn windows_quoting_handles_spaces_and_quotes() {
        assert_eq!(quote_windows("npx"), "npx");
        assert_eq!(
            quote_windows(r"C:\Program Files\node\npx.cmd"),
            r#""C:\Program Files\node\npx.cmd""#
        );
        assert_eq!(quote_windows(""), "\"\"");
    }
}

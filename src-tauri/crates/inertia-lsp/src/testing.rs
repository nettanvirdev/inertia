//! A language server that is not one.
//!
//! Nothing in this repository installs typescript-language-server, pyright,
//! gopls or rust-analyzer, and a test suite that only passes on a machine with
//! one of those installed is a test suite that does not run. So the tests drive
//! this instead: a real JSON-RPC peer, speaking the real framing, over a real
//! asynchronous pipe. Everything between the client's `initialize` and the
//! bytes on the wire is the code that ships.
//!
//! What a document means is written into the document, which keeps a test for
//! the tool a matter of writing a file:
//!
//!   - `@@error <text>` and `@@warn`, `@@info`, `@@hint` - a diagnostic on that
//!     line, at that severity.
//!   - `@@other <file> <text>` - an error published against a neighbouring file.
//!   - `@@def <name>` / `@@use <name>` - where `<name>` is defined, and a use.
//!   - `@@hover <text>` - what hovering that line says.
//!   - `@@sym <kind> <name>` - a declaration, at the protocol's kind number.
//!
//! This module is compiled into the crate rather than hidden behind `cfg(test)`
//! because the `lsp` tool lives in another crate and its tests need a server
//! too.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::client::{Launcher, Transport};
use crate::protocol::{encode, Parser};
use crate::servers::ServerSpec;
use crate::uri::{from_uri, to_uri};

/// The spec a test hands to the client: a server called `demo`, rooted here.
pub fn spec_for(root: &Path) -> ServerSpec {
    ServerSpec {
        id: "demo".to_string(),
        bin: PathBuf::from("demo-server"),
        args: Vec::new(),
        root: root.to_path_buf(),
        env: Vec::new(),
        initialization: None,
    }
}

/// A transport with a fake server already running on the far end of it.
pub fn fake_server() -> Transport {
    let (near, far) = tokio::io::duplex(256 * 1024);
    let (client_reader, client_writer) = tokio::io::split(near);
    tokio::spawn(serve(far));
    Transport {
        writer: Box::new(client_writer),
        reader: Box::new(client_reader),
        child: None,
    }
}

/// A launcher that hands out fake servers, whatever it is asked for.
#[derive(Debug, Default)]
pub struct FakeLauncher;

#[async_trait]
impl Launcher for FakeLauncher {
    async fn launch(&self, _spec: &ServerSpec) -> Result<Transport, String> {
        Ok(fake_server())
    }
}

/// A launcher that never manages to start anything, for the failure path.
#[derive(Debug, Default)]
pub struct DeadLauncher;

#[async_trait]
impl Launcher for DeadLauncher {
    async fn launch(&self, spec: &ServerSpec) -> Result<Transport, String> {
        Err(format!(
            "{} could not be started: no such file or directory",
            spec.id
        ))
    }
}

/// A launcher every test can share when it wants the fake server.
pub fn fake_launcher() -> Arc<dyn Launcher> {
    Arc::new(FakeLauncher)
}

// ── the server itself ───────────────────────────────────────────────────

async fn serve(stream: tokio::io::DuplexStream) {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut parser = Parser::new();
    let mut documents: BTreeMap<String, String> = BTreeMap::new();
    let mut chunk = [0u8; 16 * 1024];

    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        let Ok(messages) = parser.push(&chunk[..read]) else {
            return;
        };
        for message in messages {
            let mut out: Vec<Value> = Vec::new();
            let exiting = respond(&message, &mut documents, &mut out);
            for reply in out {
                if writer.write_all(&encode(&reply)).await.is_err() {
                    return;
                }
            }
            if writer.flush().await.is_err() || exiting {
                return;
            }
        }
    }
}

/// Handles one message, appending whatever it answers with. Returns whether
/// the server was told to exit.
fn respond(
    message: &Value,
    documents: &mut BTreeMap<String, String>,
    out: &mut Vec<Value>,
) -> bool {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let id = message.get("id").cloned();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    let reply = |out: &mut Vec<Value>, result: Value| {
        if let Some(id) = id.clone() {
            out.push(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        }
    };

    match method {
        "initialize" => reply(
            out,
            json!({ "capabilities": { "textDocumentSync": 1, "hoverProvider": true } }),
        ),
        "initialized" | "workspace/didChangeConfiguration" => {}
        "textDocument/didOpen" => {
            let uri = uri_of(&params);
            let text = params
                .pointer("/textDocument/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            documents.insert(uri.clone(), text.clone());
            publish(&uri, &text, out);
        }
        "textDocument/didChange" => {
            let uri = uri_of(&params);
            let text = params
                .pointer("/contentChanges/0/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            documents.insert(uri.clone(), text.clone());
            publish(&uri, &text, out);
        }
        "textDocument/definition"
        | "textDocument/typeDefinition"
        | "textDocument/implementation"
        | "textDocument/references" => {
            let uri = uri_of(&params);
            let text = documents.get(&uri).cloned().unwrap_or_default();
            let lines: Vec<&str> = text.split('\n').collect();
            let at = params
                .pointer("/position/line")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let character = params
                .pointer("/position/character")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;

            // The name is the word the position sits at the start of.
            let name: String = lines
                .get(at)
                .map(|line| line.chars().skip(character).collect::<String>())
                .unwrap_or_default()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();

            let want = if method == "textDocument/references" {
                "use"
            } else {
                "def"
            };
            let mut found = Vec::new();
            if !name.is_empty() {
                for (index, line) in lines.iter().enumerate() {
                    if !marks(line, want, &name) {
                        continue;
                    }
                    found.push(json!({
                        "uri": uri,
                        "range": {
                            "start": { "line": index, "character": 0 },
                            "end": { "line": index, "character": line.chars().count() },
                        }
                    }));
                }
            }
            reply(out, Value::Array(found));
        }
        "textDocument/hover" => {
            let uri = uri_of(&params);
            let at = params
                .pointer("/position/line")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let text = documents.get(&uri).cloned().unwrap_or_default();
            let line = text.split('\n').nth(at).unwrap_or("").to_string();
            let answer = match after(&line, "@@hover ") {
                Some(value) => json!({ "contents": { "kind": "markdown", "value": value } }),
                None => Value::Null,
            };
            reply(out, answer);
        }
        "textDocument/documentSymbol" => {
            let uri = uri_of(&params);
            let text = documents.get(&uri).cloned().unwrap_or_default();
            let mut symbols = Vec::new();
            for (index, line) in text.split('\n').enumerate() {
                let Some((kind, name)) = symbol(line) else {
                    continue;
                };
                symbols.push(json!({
                    "name": name,
                    "kind": kind,
                    "range": { "start": { "line": index, "character": 0 }, "end": { "line": index, "character": 1 } },
                    "selectionRange": { "start": { "line": index, "character": 0 }, "end": { "line": index, "character": 1 } },
                }));
            }
            reply(out, Value::Array(symbols));
        }
        "workspace/symbol" => {
            let wanted = params
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let mut symbols = Vec::new();
            for (uri, text) in documents.iter() {
                for (index, line) in text.split('\n').enumerate() {
                    let Some((kind, name)) = symbol(line) else {
                        continue;
                    };
                    if !name.contains(&wanted) {
                        continue;
                    }
                    symbols.push(json!({
                        "name": name,
                        "kind": kind,
                        "location": {
                            "uri": uri,
                            "range": { "start": { "line": index, "character": 0 }, "end": { "line": index, "character": 1 } },
                        }
                    }));
                }
            }
            reply(out, Value::Array(symbols));
        }
        "shutdown" => reply(out, Value::Null),
        "exit" => return true,
        _ => reply(out, Value::Null),
    }
    false
}

fn uri_of(params: &Value) -> String {
    params
        .pointer("/textDocument/uri")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The text after `marker` on this line, trimmed, if the line carries it.
fn after(line: &str, marker: &str) -> Option<String> {
    let at = line.find(marker)?;
    Some(line[at + marker.len()..].trim().to_string())
}

/// Whether this line is `@@def name` or `@@use name` for exactly `name`.
fn marks(line: &str, want: &str, name: &str) -> bool {
    let marker = format!("@@{want} ");
    match after(line, &marker) {
        Some(rest) => rest
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .next()
            .is_some_and(|word| word == name),
        None => false,
    }
}

/// `@@sym <kind> <name>` as its two parts.
fn symbol(line: &str) -> Option<(u64, String)> {
    let rest = after(line, "@@sym ")?;
    let mut words = rest.split_whitespace();
    let kind = words.next()?.parse::<u64>().ok()?;
    Some((kind, words.next()?.to_string()))
}

const SEVERITIES: &[(&str, u64)] = &[("error", 1), ("warn", 2), ("info", 3), ("hint", 4)];

fn publish(uri: &str, text: &str, out: &mut Vec<Value>) {
    let mut items = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        for (marker, severity) in SEVERITIES {
            let Some(message) = after(line, &format!("@@{marker} ")) else {
                continue;
            };
            items.push(json!({
                "severity": severity,
                "range": {
                    "start": { "line": index, "character": 0 },
                    "end": { "line": index, "character": line.chars().count() },
                },
                "message": message,
                "source": "fake",
            }));
            break;
        }
    }
    out.push(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": items },
    }));

    // `@@other <file> <text>`: an error in a neighbour, which is how a write
    // that breaks its importers is tested.
    let directory = from_uri(uri)
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    for line in text.split('\n') {
        let Some(rest) = after(line, "@@other ") else {
            continue;
        };
        let mut words = rest.splitn(2, char::is_whitespace);
        let Some(name) = words.next() else { continue };
        let message = words.next().unwrap_or("").trim().to_string();
        let mut neighbour = directory.clone();
        neighbour.push(name);
        out.push(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": to_uri(&neighbour),
                "diagnostics": [{
                    "severity": 1,
                    "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } },
                    "message": message,
                    "source": "fake",
                }],
            },
        }));
    }
}

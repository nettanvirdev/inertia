//! Memories kept by an MCP memory server.
//!
//! This is what makes memory shared rather than private to this app: point
//! Inertia and another coding agent at the same server and they know the same
//! things. Something decided in Claude Code is known here, and the other way
//! round. No copy, no sync, one store.
//!
//! The awkward part is that every memory server names its tools differently -
//! `add_memories`, `add_memory`, `create_entities` - and returns a different
//! shape. So a server record may carry a mapping, and where it does not, the
//! names below are tried against what the server actually advertises. Guessing
//! from the advertised list is worth doing: the alternative is a person having
//! to fill in four dropdowns before anything works, having already told the app
//! which server to use.
//!
//! Everything a server answers with is treated as data written by someone else,
//! because it is: another agent, on another day, put those rows there. Shapes
//! are read defensively and anything unrecognised is skipped rather than guessed
//! at - a row that cannot be understood yields no memory, never an error that
//! would cost somebody their turn.
//!
//! The transport is a trait (`Server`) rather than a connection, for two
//! reasons. It keeps the whole of the awkward part - name guessing, header
//! encoding, defensive reading - testable without a live process. And it is
//! where the one genuinely ugly thing lives: `Store` is synchronous and MCP is
//! not, so exactly one implementation (`LiveServer`) does the waiting, and
//! nothing above it has to know.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::record;
use crate::store::{Backend, Capabilities};

/// What a memory id from a server is prefixed with.
///
/// Namespacing is not cosmetic. Remote and local rows feed one keyed render and
/// one lookup by id, and a remote row that shadowed a local one would be edited
/// or deleted in the wrong place entirely.
pub const PREFIX: &str = "mcp:";

/// The names the servers people actually run use, in the order to try them.
///
/// OpenMemory, mem0 and supermemory between them cover most of what anybody has
/// connected, and a server that uses none of these can be mapped by hand on its
/// own record.
pub const LIST_NAMES: &[&str] = &[
    "list_memories",
    "get_memories",
    "listMemories",
    "search_nodes",
    "read_graph",
];
pub const RECALL_NAMES: &[&str] = &[
    "search_memory",
    "search_memories",
    "searchMemories",
    "retrieve_memory",
];
pub const REMEMBER_NAMES: &[&str] = &[
    "add_memories",
    "add_memory",
    "create_memory",
    "store_memory",
    "create_entities",
];
pub const UPDATE_NAMES: &[&str] = &["update_memory", "edit_memory"];
pub const FORGET_NAMES: &[&str] = &[
    "delete_memory",
    "delete_memories",
    "forget_memory",
    "delete_entities",
];

/// Which tool on this server does which job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Jobs {
    pub list: Option<String>,
    pub recall: Option<String>,
    pub remember: Option<String>,
    pub update: Option<String>,
    pub forget: Option<String>,
}

/// Work out the mapping. The record's own mapping wins, where it names a tool
/// the server actually has.
///
/// A name that is configured but absent falls back to the guess rather than
/// being used: that is a server somebody reconfigured, or a name typed wrongly,
/// and calling a tool that is not there fails every memory read with a message
/// about an unknown tool.
pub fn mapping(record: Option<&Value>, advertised: &[String]) -> Jobs {
    let available: HashSet<&str> = advertised.iter().map(String::as_str).collect();
    let configured = record.and_then(|row| row.get("memory"));

    let pick = |job: &str, candidates: &[&str]| -> Option<String> {
        let named = configured
            .and_then(|map| map.get(job))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty());
        if let Some(name) = named {
            if available.contains(name) {
                return Some(name.to_string());
            }
        }
        candidates
            .iter()
            .find(|name| available.contains(*name))
            .map(|name| (*name).to_string())
    };

    Jobs {
        list: pick("list", LIST_NAMES),
        recall: pick("recall", RECALL_NAMES),
        remember: pick("remember", REMEMBER_NAMES),
        update: pick("update", UPDATE_NAMES),
        forget: pick("forget", FORGET_NAMES),
    }
}

/* -- carrying what a memory server has no field for ----------------------- */

/// A memory server stores text, and we have more than text to say.
///
/// Which project a memory belongs to, and whether it is the rolling handover
/// note rather than an ordinary fact, have nowhere to live on a server whose
/// whole model is a string. Losing them costs two real things: the handover note
/// stops being replaceable and piles up a new copy every session, and a
/// project's notes stop being that project's.
///
/// So they travel in the text, on one line at the top, and are taken back off on
/// the way in. It is not elegant and it is honest: a server that stores what it
/// was given returns what it was given, and any other client reading the same
/// store sees one tidy line rather than something broken.
const HEADER_OPEN: &str = "[inertia:";

pub fn encode(record: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    let kind = record::text(record, "kind");
    if !kind.is_empty() && kind != record::DEFAULT_KIND {
        parts.push(format!("kind={kind}"));
    }
    let folder = record::text(record, "folder");
    if !folder.is_empty() {
        parts.push(format!("project={folder}"));
    }

    let title = record::text(record, "title");
    let body = record::text(record, "body");
    let text = if title.is_empty() {
        body
    } else {
        format!("{title}\n\n{body}")
    };

    if parts.is_empty() {
        text
    } else {
        format!("{HEADER_OPEN}{}]\n{text}", parts.join("; "))
    }
}

/// What came back off the front of a stored string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub body: String,
    pub kind: String,
    pub folder: Option<String>,
}

pub fn decode(text: &str) -> Decoded {
    let plain = |body: &str| Decoded {
        body: body.to_string(),
        kind: record::DEFAULT_KIND.to_string(),
        folder: None,
    };

    let Some(rest) = text.strip_prefix(HEADER_OPEN) else {
        return plain(text);
    };
    // Up to the first `]`, the way it was written. A header somebody mangled by
    // hand with no closing bracket is not a header, and the whole string is the
    // body rather than nothing at all.
    let Some(end) = rest.find(']') else {
        return plain(text);
    };

    let mut kind = record::DEFAULT_KIND.to_string();
    let mut folder = None;
    for pair in rest[..end].split(';') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "kind" => kind = value.to_string(),
            "project" => folder = Some(value.to_string()),
            _ => {}
        }
    }

    let body = rest[end + 1..].strip_prefix('\n').unwrap_or(&rest[end + 1..]);
    Decoded { body: body.to_string(), kind, folder }
}

/* -- reading whatever the server sent ------------------------------------- */

/// Rows out of whatever the server answered with.
///
/// Servers answer with an array, or an object holding one under any of a handful
/// of keys. Reading all of them is a few lines; not reading them is a backend
/// that silently shows nothing.
pub fn rows_of(data: &Value) -> Vec<Value> {
    if let Some(rows) = data.as_array() {
        return rows.clone();
    }
    for key in ["memories", "results", "items", "documents", "entities", "data"] {
        if let Some(rows) = data.get(key).and_then(Value::as_array) {
            return rows.clone();
        }
    }
    Vec::new()
}

fn first_of(row: &Value, fields: &[&str]) -> String {
    for field in fields {
        let found = match row.get(*field) {
            Some(Value::String(text)) => text.trim().to_string(),
            // An id a server chose to send as a number is still an id.
            Some(Value::Number(number)) => number.to_string(),
            _ => String::new(),
        };
        if !found.is_empty() {
            return found;
        }
    }
    String::new()
}

/// One remote row as a memory this app can render.
///
/// Returns nothing for a row it cannot understand. That is the whole defensive
/// posture here: these rows were written by another agent on another day, and a
/// backend that invents a memory out of a shape it did not recognise is worse
/// than one that shows fewer.
pub fn to_memory(row: &Value, server_id: &str) -> Option<Value> {
    let id = first_of(row, &["id", "memory_id", "uuid", "key"]);
    if id.is_empty() {
        return None;
    }

    let stored = first_of(row, &["memory", "content", "text", "body"]);
    if stored.is_empty() {
        return None;
    }

    let decoded = decode(&stored);
    let body = decoded.body.trim().to_string();
    if body.is_empty() {
        return None;
    }

    // A title written as the first line, the way `encode` puts it there. Split
    // on the blank line rather than the first newline so a multi-line body stays
    // whole; a row some other client wrote has no blank line and becomes one
    // memory whose title is its opening.
    let split = body.find("\n\n").filter(|at| *at > 0);
    let written = first_of(row, &["title", "name"]);

    let opening = match split {
        Some(at) => body[..at].to_string(),
        None => body.lines().next().unwrap_or_default().to_string(),
    };
    let title: String = if written.is_empty() { opening } else { written.clone() }
        .chars()
        .take(120)
        .collect();

    let text = match split {
        Some(at) if written.is_empty() => body[at + 2..].to_string(),
        _ => body.clone(),
    };

    let tags: Vec<String> = row
        .get("tags")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|tag| match tag {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    let stamp = |names: &[&str]| -> Value {
        for name in names {
            match row.get(*name) {
                Some(Value::Null) | None => continue,
                Some(found) => return found.clone(),
            }
        }
        Value::Null
    };

    Some(json!({
        "id": format!("{PREFIX}{server_id}:{id}"),
        "remoteId": id,
        "origin": "remote",
        "server": server_id,
        "title": title,
        "body": text,
        "kind": decoded.kind,
        "tags": tags,
        // Restored from the header when we wrote it. A row another client wrote
        // has none, and is global - which is the honest reading of a fact that
        // came from somewhere with no idea of projects.
        "scope": if decoded.folder.is_some() { "project" } else { "global" },
        "folder": decoded.folder,
        "source": "imported",
        "createdAt": stamp(&["created_at", "createdAt"]),
        "updatedAt": stamp(&["updated_at", "updatedAt"]),
        "useCount": 0,
        "confidence": 1.0,
    }))
}

/// Strip the namespace back off before talking to the server.
pub fn remote_id_of(id: &str) -> String {
    match id.strip_prefix(PREFIX) {
        // `mcp:<server>:<id>`, and the server's own id may itself hold colons.
        Some(rest) => rest.split_once(':').map_or(rest, |(_, id)| id).to_string(),
        None => id.to_string(),
    }
}

/* -- the transport -------------------------------------------------------- */

/// One connected memory server, as this module needs it.
///
/// Synchronous on purpose. `Store` is synchronous and so is every caller of it,
/// and the alternative - making the whole memory path async so a setting most
/// people never change can do network IO - would change the shape of code that
/// has nothing to do with memory servers. The waiting happens in `LiveServer`
/// and nowhere else.
pub trait Server: std::fmt::Debug + Send + Sync {
    /// The tool names this server advertises right now. Empty when it is not
    /// connected, which is different from connected and offering none.
    fn advertised(&self) -> Vec<String>;

    /// Call one, giving back whatever it answered with, as data.
    fn call(&self, tool: &str, args: Value) -> Result<Value, String>;
}

/// Memories on somebody else's server.
#[derive(Debug)]
pub struct Remote {
    server: Arc<dyn Server>,
    id: String,
    label: String,
    jobs: Jobs,
}

impl Remote {
    /// Build a backend for one server, or refuse in a sentence worth reading.
    ///
    /// Refusing is normal and not a failure of the app: a server comes up on a
    /// supervisor tick and may simply not be up yet. The caller falls back to
    /// the workspace's own memories and says why, rather than a conversation
    /// failing because a memory backend was slow to start.
    ///
    /// `backend` is the preference the settings screen writes, which is
    /// `mcp:<server id>`. `record` is that server's own `plugins.mcp` document,
    /// where an explicit tool mapping may live.
    pub fn open(
        server: Arc<dyn Server>,
        backend: &str,
        record: Option<&Value>,
    ) -> Result<Self, String> {
        let id = backend.strip_prefix(PREFIX).unwrap_or(backend).trim().to_string();
        if id.is_empty() {
            return Err("No memory server was chosen.".into());
        }

        let label = record
            .map(|row| record::text(row, "name"))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| id.clone());

        let advertised = server.advertised();
        if advertised.is_empty() {
            return Err(format!(
                "{label} is not connected yet, or offers no tools. Check it under Settings, Integrations."
            ));
        }

        let jobs = mapping(record, &advertised);
        if jobs.list.is_none() && jobs.recall.is_none() {
            return Err(format!(
                "{label} does not offer a tool for reading memories that this app recognises. \
                 Name one on the server's own settings."
            ));
        }

        Ok(Self { server, id, label, jobs })
    }

    /// The tool that reads everything.
    ///
    /// A server with no listing tool is read through its search tool with an
    /// empty query, which most treat as "everything". That is a guess, and the
    /// better one: ranking happens here anyway, so the alternative is a chosen
    /// backend that can never show a single memory.
    fn reading(&self) -> Option<&String> {
        self.jobs.list.as_ref().or(self.jobs.recall.as_ref())
    }

    fn rows(&self, data: &Value) -> Vec<Value> {
        rows_of(data)
            .iter()
            .filter_map(|row| to_memory(row, &self.id))
            .collect()
    }
}

impl Backend for Remote {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            list: self.reading().is_some(),
            remember: self.jobs.remember.is_some(),
            update: self.jobs.update.is_some(),
            forget: self.jobs.forget.is_some(),
        }
    }

    fn list(&self) -> Result<Vec<Value>, String> {
        let Some(tool) = self.reading() else {
            return Ok(Vec::new());
        };
        let data = self
            .server
            .call(tool, json!({ "limit": 200, "page": 1, "query": "" }))?;
        Ok(self.rows(&data))
    }

    fn remember(&self, input: &Value, merge: bool) -> Result<Value, String> {
        let Some(tool) = self.jobs.remember.as_ref() else {
            return Err(format!("{} cannot be written to.", self.label));
        };

        // The same refusal the local store makes, and more necessary here: a
        // shared server makes a repeat likelier rather than less, because
        // another agent may have written the same fact yesterday. When the
        // server cannot update, the duplicate is skipped rather than written - a
        // second copy is worse than no write, because the store is the thing
        // that has to stay usable.
        if merge {
            let held = self.list()?;
            if let Some(twin) = record::find_duplicate(&held, input) {
                let id = record::text(twin, "id");
                return match self.jobs.update {
                    Some(_) => self.update(&id, record::merged(twin, input)),
                    None => Ok(twin.clone()),
                };
            }
        }

        // One piece of text, title first, with anything the server has no field
        // for carried on a header line. A server that models memories as plain
        // strings - which most do - has nowhere else to put any of it.
        let text = encode(input);
        let data = self.server.call(
            tool,
            json!({ "text": text, "messages": [{ "role": "user", "content": text }] }),
        )?;

        // A server that answers with something unreadable still stored the
        // memory. Reporting a failure would invite the agent to write it again.
        Ok(self
            .rows(&data)
            .into_iter()
            .next()
            .unwrap_or_else(|| input.clone()))
    }

    fn update(&self, id: &str, changes: Value) -> Result<Value, String> {
        let Some(tool) = self.jobs.update.as_ref() else {
            return Err(format!("{} cannot change a stored memory.", self.label));
        };
        let remote = remote_id_of(id);
        let text = encode(&changes);
        self.server.call(
            tool,
            json!({ "memory_id": remote, "id": remote, "text": text }),
        )?;
        let mut updated = changes;
        if let Some(map) = updated.as_object_mut() {
            map.insert("id".into(), Value::String(id.to_string()));
        }
        Ok(updated)
    }

    fn forget(&self, id: &str) -> Result<(), String> {
        let Some(tool) = self.jobs.forget.as_ref() else {
            return Err(format!("{} cannot delete a single memory.", self.label));
        };
        let remote = remote_id_of(id);
        self.server
            .call(tool, json!({ "memory_id": remote, "id": remote }))?;
        Ok(())
    }

    /// A memory server has nowhere to keep a use count, and inventing a field on
    /// somebody else's store to hold our bookkeeping would be rude and lossy.
    fn touch(&self, _ids: &[String]) {}
}

/* -- the live connection -------------------------------------------------- */

pub use live::LiveServer;

mod live {
    use std::future::Future;
    use std::path::PathBuf;
    use std::sync::Arc;

    use async_trait::async_trait;
    use inertia_core::tool::{
        Decision, PermissionGate, PermissionRequest, Tool, ToolContext, ToolProvider,
    };
    use inertia_core::{Action, Result as CoreResult, SessionId, ToolCallId};
    use inertia_mcp::provider::{tool_id, McpProvider};
    use serde_json::Value;
    use tokio::runtime::{Handle, RuntimeFlavor};

    use super::Server;

    /// The gate these calls pass through.
    ///
    /// Deliberately open, and it is not a hole. Nothing here is model-initiated:
    /// the person chose this server as their memory backend on the settings
    /// screen, and asking them to approve each read of their own memories would
    /// be asking them to approve the same decision over and over. The tools the
    /// model itself can call still go through the real gate in the registry.
    #[derive(Debug)]
    struct Chosen;

    #[async_trait]
    impl PermissionGate for Chosen {
        async fn ask(&self, _request: &PermissionRequest) -> CoreResult<Decision> {
            Ok(Decision::Allow)
        }
        async fn verdict(&self, _key: &str, _target: &str) -> Action {
            Action::Allow
        }
    }

    /// Wait for one future from synchronous code.
    ///
    /// `Store` is synchronous and MCP is not, and this is the one place that
    /// bridges the two. On the app's multi-threaded runtime `block_in_place`
    /// hands the worker's other tasks to a sibling thread, so blocking here
    /// stalls nothing. Off a runtime entirely - a test, a CLI - the handle can
    /// simply be driven. Inside a current-thread runtime neither is safe, so the
    /// work is pushed to the stored handle's threads and the answer waited for.
    fn wait<F>(handle: &Handle, future: F) -> Result<F::Output, String>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        match Handle::try_current() {
            Ok(current) if current.runtime_flavor() == RuntimeFlavor::MultiThread => {
                Ok(tokio::task::block_in_place(|| current.block_on(future)))
            }
            Ok(_) => {
                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                handle.spawn(async move {
                    let _ = tx.send(future.await);
                });
                rx.recv()
                    .map_err(|_| "The memory server call did not finish.".to_string())
            }
            Err(_) => Ok(handle.block_on(future)),
        }
    }

    /// A connected MCP server, reached the same way the agent reaches one.
    ///
    /// Going through the tool rather than the connection is deliberate: the
    /// connection is not reachable from outside `inertia-mcp`, and the tool path
    /// already flattens whatever content blocks the server sent into one string.
    /// Memory servers answer with JSON in a text block, so that string is parsed
    /// back; anything that is not JSON yields no rows, which is the same
    /// defensive answer an unrecognised shape gets.
    #[derive(Debug)]
    pub struct LiveServer {
        provider: Arc<McpProvider>,
        id: String,
        root: PathBuf,
        handle: Handle,
    }

    impl LiveServer {
        pub fn new(
            provider: Arc<McpProvider>,
            server_id: impl Into<String>,
            root: PathBuf,
            handle: Handle,
        ) -> Self {
            Self { provider, id: server_id.into(), root, handle }
        }

        /// The namespaced id the provider gives this server's tools.
        fn tool_id_for(&self, tool: &str) -> String {
            let name = self
                .provider
                .connected()
                .into_iter()
                .find(|record| record.id == self.id)
                .map(|record| {
                    if record.name.is_empty() {
                        record.id
                    } else {
                        record.name
                    }
                })
                .unwrap_or_else(|| self.id.clone());
            tool_id(&name, tool)
        }
    }

    impl Server for LiveServer {
        fn advertised(&self) -> Vec<String> {
            self.provider
                .tools_for(&self.id)
                .into_iter()
                .map(|descriptor| descriptor.name)
                .collect()
        }

        fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
            let wanted = self.tool_id_for(tool);
            let provider = self.provider.clone();
            let root = self.root.clone();

            let found: Option<Arc<dyn Tool>> = wait(&self.handle, async move {
                provider
                    .tools(&root)
                    .await
                    .ok()?
                    .into_iter()
                    .find(|candidate| candidate.id() == wanted)
            })?;
            let Some(found) = found else {
                return Err(format!("{tool} is not offered by the server right now."));
            };

            let ctx = ToolContext {
                root: self.root.clone(),
                session: SessionId::from_existing("memory"),
                call_id: ToolCallId::from("memory"),
                permissions: Arc::new(Chosen),
            };
            let outcome = wait(&self.handle, async move { found.execute(args, &ctx).await })?
                .map_err(|e| e.to_string())?;

            Ok(serde_json::from_str(outcome.output.trim()).unwrap_or(Value::Null))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use inertia_store::Layout;
    use parking_lot::Mutex;

    /// A server that answers from a script, so everything above the transport
    /// can be exercised without a process.
    #[derive(Debug, Default)]
    struct Fake {
        advertised: Vec<String>,
        answers: Mutex<Vec<(String, Value)>>,
        calls: Mutex<Vec<(String, Value)>>,
        broken: Option<String>,
    }

    impl Fake {
        fn with(names: &[&str]) -> Self {
            Self {
                advertised: names.iter().map(|n| (*n).to_string()).collect(),
                ..Self::default()
            }
        }

        fn answering(self, tool: &str, data: Value) -> Self {
            self.answers.lock().push((tool.to_string(), data));
            self
        }

        fn unreachable(names: &[&str], why: &str) -> Self {
            Self { broken: Some(why.to_string()), ..Self::with(names) }
        }
    }

    impl Server for Fake {
        fn advertised(&self) -> Vec<String> {
            self.advertised.clone()
        }

        fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
            self.calls.lock().push((tool.to_string(), args));
            if let Some(why) = &self.broken {
                return Err(why.clone());
            }
            let held = self.answers.lock();
            Ok(held
                .iter()
                .find(|(name, _)| name == tool)
                .map(|(_, data)| data.clone())
                .unwrap_or(Value::Null))
        }
    }

    fn advertised(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    /* -- which tool does which job ---------------------------------------- */

    #[test]
    fn it_recognises_openmemory() {
        let jobs = mapping(
            None,
            &advertised(&[
                "add_memories",
                "search_memory",
                "list_memories",
                "delete_all_memories",
            ]),
        );
        assert_eq!(jobs.remember.as_deref(), Some("add_memories"));
        assert_eq!(jobs.recall.as_deref(), Some("search_memory"));
        assert_eq!(jobs.list.as_deref(), Some("list_memories"));
    }

    #[test]
    fn it_recognises_mem0() {
        let jobs = mapping(
            None,
            &advertised(&[
                "add_memory",
                "search_memories",
                "get_memories",
                "update_memory",
                "delete_memory",
            ]),
        );
        assert_eq!(jobs.remember.as_deref(), Some("add_memory"));
        assert_eq!(jobs.recall.as_deref(), Some("search_memories"));
        assert_eq!(jobs.list.as_deref(), Some("get_memories"));
        assert_eq!(jobs.update.as_deref(), Some("update_memory"));
        assert_eq!(jobs.forget.as_deref(), Some("delete_memory"));
    }

    #[test]
    fn it_recognises_supermemory() {
        let jobs = mapping(None, &advertised(&["search_memory", "add_memory", "listMemories"]));
        assert_eq!(jobs.list.as_deref(), Some("listMemories"));
        assert_eq!(jobs.recall.as_deref(), Some("search_memory"));
    }

    #[test]
    fn it_recognises_a_knowledge_graph_server() {
        let jobs = mapping(
            None,
            &advertised(&["create_entities", "delete_entities", "search_nodes", "read_graph"]),
        );
        assert_eq!(jobs.list.as_deref(), Some("search_nodes"));
        assert_eq!(jobs.remember.as_deref(), Some("create_entities"));
        assert_eq!(jobs.forget.as_deref(), Some("delete_entities"));
    }

    #[test]
    fn an_explicit_mapping_on_the_record_wins() {
        let jobs = mapping(
            Some(&json!({ "memory": { "recall": "my_custom_search" } })),
            &advertised(&["search_memory", "my_custom_search"]),
        );
        assert_eq!(jobs.recall.as_deref(), Some("my_custom_search"));
    }

    /// A server that was reconfigured, or a name typed wrongly. Falling back to
    /// the guess beats calling a tool that is not there.
    #[test]
    fn a_mapping_naming_a_tool_the_server_lacks_is_ignored() {
        let jobs = mapping(
            Some(&json!({ "memory": { "recall": "gone" } })),
            &advertised(&["search_memory"]),
        );
        assert_eq!(jobs.recall.as_deref(), Some("search_memory"));
    }

    #[test]
    fn a_job_the_server_cannot_do_is_nothing_rather_than_a_guess() {
        let jobs = mapping(None, &advertised(&["search_memory"]));
        assert_eq!(jobs.forget, None);
        assert_eq!(jobs.remember, None);
    }

    /* -- reading whatever shape the server sent --------------------------- */

    #[test]
    fn it_takes_the_shapes_servers_actually_wrap_rows_in() {
        assert_eq!(rows_of(&json!([{ "id": "1" }])).len(), 1);
        for key in ["memories", "results", "items", "documents", "entities", "data"] {
            assert_eq!(rows_of(&json!({ key: [{ "id": "1" }] })).len(), 1, "{key}");
        }
    }

    #[test]
    fn an_unrecognised_shape_yields_no_memories_rather_than_an_error() {
        assert!(rows_of(&Value::Null).is_empty());
        assert!(rows_of(&json!({ "unexpected": true })).is_empty());
        assert!(rows_of(&json!("text")).is_empty());
    }

    /// The same, through the backend rather than the helper: a server answering
    /// with something nobody recognises leaves the store empty and the turn
    /// intact.
    #[test]
    fn a_server_answering_nonsense_leaves_the_store_empty() {
        let server = Arc::new(
            Fake::with(&["list_memories"]).answering("list_memories", json!({ "ok": true })),
        );
        let remote = Remote::open(server, "mcp:s1", None).expect("opened");
        assert_eq!(remote.list().expect("answered"), Vec::<Value>::new());
    }

    #[test]
    fn it_reads_the_several_names_servers_give_the_same_two_fields() {
        for row in [
            json!({ "id": "1", "memory": "Uses pnpm." }),
            json!({ "memory_id": "2", "content": "Uses pnpm." }),
            json!({ "uuid": "3", "text": "Uses pnpm." }),
        ] {
            assert_eq!(record::text(&to_memory(&row, "s").expect("a memory"), "body"), "Uses pnpm.");
        }
    }

    #[test]
    fn it_skips_a_row_with_nothing_usable_in_it() {
        assert!(to_memory(&json!({ "memory": "No id." }), "s").is_none());
        assert!(to_memory(&json!({ "id": "1" }), "s").is_none());
        assert!(to_memory(&json!({ "id": "1", "memory": "   " }), "s").is_none());
        assert!(to_memory(&Value::Null, "s").is_none());
    }

    /// Both lists feed one keyed render and one lookup by id. A collision would
    /// edit or delete the wrong record entirely.
    #[test]
    fn a_remote_id_cannot_shadow_a_local_one() {
        let made = to_memory(&json!({ "id": "abc", "memory": "..." }), "server-1").expect("a memory");
        assert_eq!(record::text(&made, "id"), "mcp:server-1:abc");
        assert_eq!(record::text(&made, "remoteId"), "abc");
        assert_eq!(record::text(&made, "origin"), "remote");

        assert_eq!(remote_id_of("mcp:server-1:abc"), "abc");
        assert_eq!(remote_id_of("mcp:server-1:abc:def"), "abc:def");
        assert_eq!(remote_id_of("plain"), "plain");
    }

    /* -- the header ------------------------------------------------------- */

    /// Without this the handover note stops being replaceable and a new copy
    /// piles up every session, which is the one memory that must never
    /// accumulate.
    #[test]
    fn the_header_round_trips_kind_and_project_through_a_text_only_store() {
        let text = encode(&json!({
            "title": "Where we left off in api",
            "body": "Routes done. Tests next.",
            "kind": "handover",
            "folder": "D:/work/api",
        }));
        let back = to_memory(&json!({ "id": "1", "memory": text }), "s1").expect("a memory");

        assert_eq!(record::text(&back, "kind"), "handover");
        assert_eq!(record::text(&back, "folder"), "D:/work/api");
        assert_eq!(record::text(&back, "scope"), "project");
        assert_eq!(record::text(&back, "title"), "Where we left off in api");
        assert_eq!(record::text(&back, "body"), "Routes done. Tests next.");
    }

    /// A shared store should not fill up with our bookkeeping when there is
    /// nothing to bookkeep.
    #[test]
    fn an_ordinary_global_fact_carries_no_header_at_all() {
        let text = encode(&json!({ "title": "Uses pnpm", "body": "Not npm.", "kind": "fact" }));
        assert_eq!(text, "Uses pnpm\n\nNot npm.");
    }

    #[test]
    fn a_row_another_agent_wrote_reads_as_an_ordinary_global_memory() {
        let back = to_memory(
            &json!({ "id": "1", "memory": "Something Claude Code saved yesterday" }),
            "s1",
        )
        .expect("a memory");
        assert_eq!(record::text(&back, "kind"), "fact");
        assert_eq!(record::text(&back, "scope"), "global");
        assert_eq!(record::text(&back, "title"), "Something Claude Code saved yesterday");
    }

    #[test]
    fn a_multi_line_body_stays_whole() {
        let back =
            to_memory(&json!({ "id": "1", "memory": "A title\n\nLine one.\nLine two." }), "s1")
                .expect("a memory");
        assert_eq!(record::text(&back, "title"), "A title");
        assert_eq!(record::text(&back, "body"), "Line one.\nLine two.");
    }

    #[test]
    fn a_header_somebody_mangled_by_hand_is_survivable() {
        assert_eq!(decode("[inertia:nonsense]\nBody.").body, "Body.");
        assert_eq!(decode("[inertia:]\nBody.").kind, record::DEFAULT_KIND);
        assert_eq!(decode("no header here").body, "no header here");
        assert_eq!(decode("").body, "");
        // No closing bracket is not a header, so none of it is lost.
        assert_eq!(decode("[inertia:kind=handover\nBody.").body, "[inertia:kind=handover\nBody.");
    }

    /* -- the backend, end to end ------------------------------------------ */

    #[test]
    fn a_server_with_no_reading_tool_is_refused_in_a_sentence() {
        let server = Arc::new(Fake::with(&["add_memory"]));
        let why = Remote::open(server, "mcp:s1", Some(&json!({ "name": "Notes" })))
            .expect_err("refused");
        assert!(why.starts_with("Notes does not offer a tool for reading memories"));
    }

    #[test]
    fn a_server_that_is_not_connected_yet_is_refused_in_a_sentence() {
        let server = Arc::new(Fake::with(&[]));
        let why = Remote::open(server, "mcp:s1", None).expect_err("refused");
        assert!(why.contains("not connected yet"));
    }

    #[test]
    fn a_stored_memory_comes_back_off_the_server() {
        let text = encode(&json!({ "title": "Uses pnpm", "body": "Not npm." }));
        let server = Arc::new(
            Fake::with(&["list_memories", "add_memory", "delete_memory"])
                .answering("list_memories", json!({ "memories": [{ "id": "7", "memory": text }] })),
        );
        let remote = Remote::open(server, "mcp:s1", None).expect("opened");

        let rows = remote.list().expect("answered");
        assert_eq!(rows.len(), 1);
        assert_eq!(record::text(&rows[0], "title"), "Uses pnpm");

        let can = remote.capabilities();
        assert!(can.list && can.remember && can.forget);
        // Nothing on this server edits, so the screen must not offer it.
        assert!(!can.update);
        assert!(remote.update("mcp:s1:7", json!({})).is_err());
    }

    /// A shared server makes a repeat likelier rather than less: another agent
    /// may have written the same fact yesterday.
    #[test]
    fn the_same_fact_twice_is_not_written_twice() {
        let text = encode(&json!({ "title": "Call me Tanvir", "body": "not Mr Ahamed" }));
        let server = Arc::new(
            Fake::with(&["list_memories", "add_memory"])
                .answering("list_memories", json!([{ "id": "7", "memory": text }])),
        );
        let remote = Remote::open(server.clone(), "mcp:s1", None).expect("opened");

        let kept = remote
            .remember(&json!({ "title": "Call me Tanvir", "body": "Tanvir is fine" }), true)
            .expect("answered");
        assert_eq!(record::text(&kept, "id"), "mcp:s1:7");
        // The server was read, and never written to.
        assert!(server.calls.lock().iter().all(|(tool, _)| tool == "list_memories"));
    }

    #[test]
    fn writing_sends_the_text_the_way_a_string_store_wants_it() {
        let server = Arc::new(
            Fake::with(&["list_memories", "add_memory"]).answering("list_memories", json!([])),
        );
        let remote = Remote::open(server.clone(), "mcp:s1", None).expect("opened");
        remote
            .remember(
                &json!({ "title": "Deploys", "body": "fly.io", "kind": "handover", "folder": "D:/api" }),
                true,
            )
            .expect("answered");

        let calls = server.calls.lock();
        let (_, args) = calls.iter().find(|(tool, _)| tool == "add_memory").expect("written");
        let text = args["text"].as_str().expect("text");
        assert!(text.starts_with("[inertia:kind=handover; project=D:/api]\n"));
        // And the shape servers that model a conversation want, in the same call.
        assert_eq!(args["messages"][0]["content"], json!(text));
    }

    /* -- the fallback ----------------------------------------------------- */

    fn local_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let store = Store::new(Layout::new(dir.path().join("workspace")), None);
        (dir, store)
    }

    /// The whole promise of this seam: a server that will not answer costs the
    /// turn nothing.
    #[test]
    fn an_unreachable_server_falls_back_to_local_memories_and_says_why() {
        let (_dir, local) = local_store();
        local
            .remember(&json!({ "title": "Call me Tanvir", "body": "not Mr Ahamed" }), true)
            .expect("written locally");

        let server = Arc::new(Fake::unreachable(&["list_memories"], "The process exited."));
        let remote = Remote::open(server, "mcp:s1", Some(&json!({ "name": "Notes" })))
            .expect("opened");
        let store = local.with_remote(Arc::new(remote));

        let rows = store.list();
        assert_eq!(rows.len(), 1, "the workspace's own memories are used instead");
        assert_eq!(record::text(&rows[0], "title"), "Call me Tanvir");

        let why = store.problem().expect("a note saying why");
        assert!(why.contains("Notes"), "{why}");
        assert!(why.contains("The process exited."), "{why}");
    }

    /// A memory the server refuses is kept rather than lost.
    #[test]
    fn a_write_the_server_refuses_lands_in_the_workspace() {
        let (_dir, local) = local_store();
        let server = Arc::new(Fake::unreachable(&["list_memories", "add_memory"], "Timed out."));
        let store = local
            .with_remote(Arc::new(Remote::open(server, "mcp:s1", None).expect("opened")));

        let saved = store
            .remember(&json!({ "title": "Deploys go to fly.io", "body": "never render" }), true)
            .expect("kept somewhere");
        assert!(!record::text(&saved, "id").starts_with(PREFIX));
        assert_eq!(store.list().len(), 1);
        assert!(store.problem().expect("a note").contains("Timed out."));
    }

    /// Building the remote failing is the common case - a server that has not
    /// started yet - and must read as a note, not as a broken workspace.
    #[test]
    fn a_backend_that_cannot_be_built_leaves_a_working_local_store() {
        let (_dir, local) = local_store();
        let why = Remote::open(Arc::new(Fake::with(&[])), "mcp:s1", None).expect_err("refused");
        let store = local.fell_back(why);

        store
            .remember(&json!({ "title": "One", "body": "fact" }), true)
            .expect("written");
        assert_eq!(store.list().len(), 1);
        assert!(store.problem().expect("a note").contains("not connected yet"));
    }

    /// Once a server has failed, the rest of the turn does not wait on it again.
    #[test]
    fn a_dead_server_is_not_retried_for_every_call() {
        let (_dir, local) = local_store();
        let server = Arc::new(Fake::unreachable(&["list_memories"], "Refused."));
        let store = local.with_remote(Arc::new(
            Remote::open(server.clone(), "mcp:s1", None).expect("opened"),
        ));

        store.list();
        store.list();
        store.list();
        assert_eq!(server.calls.lock().len(), 1);
    }
}

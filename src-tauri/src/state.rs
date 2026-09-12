//! What the running app holds, and where the concrete implementations are
//! chosen.
//!
//! This is the one place that knows a `Provider` is an Anthropic client rather
//! than a mock, and that a `Store` is a folder on disk rather than a map. Every
//! crate below it was written against the traits, which is what lets
//! `inertia-devkit` assemble the same objects with fakes and get the same
//! behaviour.

use std::collections::HashMap;
use std::sync::Arc;

use inertia_core::tool::{PermissionGate, ToolRegistry};
use inertia_core::Provider;
use inertia_composio::provider::{ComposioProvider, ConnectionRecord};
use inertia_mcp::client::ServerRecord;
use inertia_mcp::provider::McpProvider;
use inertia_openapi::provider::{ImportRecord, OpenApiProvider};
use inertia_provider::{AnthropicProvider, OpenAiProvider, ProviderConfig};
use inertia_store::settings::Protocol;
use inertia_store::{Collection, Conversations, Layout, Settings};
use inertia_tools::{builtin_tools, Lists, ReadState, Registry};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::permission::UiPermissionGate;

/// A workspace that is currently open.
pub struct Workspace {
    pub layout: Layout,
    pub settings: Arc<Settings>,
    pub conversations: Conversations,
    /// Which files each conversation has read. Lives here rather than per-turn
    /// so a file read in one turn is still known in the next.
    pub reads: Arc<ReadState>,
    /// Each conversation's task list, for `todowrite`. Held beside the reads
    /// for the same reason: it belongs to a conversation rather than to a turn,
    /// and a list that reset every turn would be rewritten from scratch on
    /// every step of the job it is tracking.
    pub lists: Arc<Lists>,
    /// What `shell` left running in the background, shared with the tools that
    /// list, read, type into and stop it. Process state rather than per-turn:
    /// a dev server started in one turn has to still be there in the next, and
    /// a table rebuilt every turn would leave orphans nothing could name.
    pub background: Arc<inertia_tools::builtin::Background>,
    /// The language servers. One set for the whole workspace: three copies
    /// would be three rust-analyzers indexing the same repository.
    pub lsp: Arc<inertia_lsp::Lsp>,
    /// Live MCP connections. Process state, shared across turns - connecting a
    /// server per turn would mean spawning a process per message.
    pub mcp: Arc<McpProvider>,
    /// Imported OpenAPI documents.
    pub openapi: Arc<OpenApiProvider>,
    /// Connected Composio apps.
    pub composio: Arc<ComposioProvider>,
    /// How a write from inside a turn tells the window.
    ///
    /// The screens do not poll and there is no folder watcher: a pane redraws
    /// because something announced the change. A tool that saves an agent has
    /// to make the same announcement the Agents screen makes when it saves one,
    /// or the record is on disk and the list still shows what was there before.
    ///
    /// Silent until a window claims it, so a workspace opened by a routine with
    /// no window running writes normally and announces to nobody.
    pub emit: crate::inertia_tools::Emitter,
}

impl Workspace {
    pub fn open(root: std::path::PathBuf) -> Result<Self, String> {
        let layout = Layout::new(root);
        layout.scaffold().map_err(|e| e.to_string())?;

        Ok(Self {
            settings: Arc::new(Settings::new(layout.clone())),
            conversations: Conversations::new(layout.clone()),
            reads: Arc::new(ReadState::new()),
            lists: Arc::new(Lists::new()),
            background: Arc::new(inertia_tools::builtin::Background::default()),
            lsp: Arc::new(inertia_lsp::Lsp::new()),
            mcp: Arc::new(McpProvider::new()),
            openapi: Arc::new(OpenApiProvider::new()),
            composio: Arc::new(ComposioProvider::new()),
            emit: crate::inertia_tools::silent(),
            layout,
        })
    }

    /// The MCP servers configured in this workspace, connected or not.
    pub fn mcp_records(&self) -> Vec<ServerRecord> {
        read_collection(&self.layout, Collection::Mcp)
    }

    pub fn save_mcp_record(&self, record: &ServerRecord) -> Result<(), String> {
        inertia_store::fsx::write_json(
            &self.layout.record(Collection::Mcp, &record.id),
            record,
        )
        .map_err(|e| e.to_string())
    }

    pub fn delete_mcp_record(&self, id: &str) -> Result<(), String> {
        inertia_store::fsx::remove(&self.layout.record(Collection::Mcp, id))
            .map_err(|e| e.to_string())
    }

    pub fn openapi_records(&self) -> Vec<ImportRecord> {
        read_collection(&self.layout, Collection::OpenApi)
    }

    /// Saves an import and the document it was built from.
    ///
    /// The spec is kept beside the record so the import survives the URL it
    /// came from going away, and so the exact document producing the current
    /// tools can be inspected rather than guessed at.
    pub fn save_openapi(
        &self,
        record: &ImportRecord,
        document: &serde_json::Value,
    ) -> Result<(), String> {
        inertia_store::fsx::write_json(
            &self.layout.record(Collection::OpenApi, &record.id),
            record,
        )
        .map_err(|e| e.to_string())?;

        inertia_store::fsx::write_json(&self.openapi_spec_path(&record.id), document)
            .map_err(|e| e.to_string())
    }

    pub fn read_openapi_spec(&self, id: &str) -> Option<serde_json::Value> {
        match inertia_store::fsx::read_json::<serde_json::Value>(&self.openapi_spec_path(id)) {
            (document, inertia_store::fsx::ReadOutcome::Loaded) => Some(document),
            _ => None,
        }
    }

    fn openapi_spec_path(&self, id: &str) -> std::path::PathBuf {
        self.layout
            .root()
            .join("plugins/openapi/specs")
            .join(format!("{id}.json"))
    }

    pub fn delete_openapi(&self, id: &str) -> Result<(), String> {
        inertia_store::fsx::remove(&self.layout.record(Collection::OpenApi, id))
            .map_err(|e| e.to_string())?;
        inertia_store::fsx::remove(&self.openapi_spec_path(id)).map_err(|e| e.to_string())
    }

    pub fn composio_records(&self) -> Vec<ConnectionRecord> {
        read_collection(&self.layout, Collection::Composio)
    }

    pub fn save_composio(&self, record: &ConnectionRecord) -> Result<(), String> {
        inertia_store::fsx::write_json(
            &self.layout.record(Collection::Composio, &record.id),
            record,
        )
        .map_err(|e| e.to_string())
    }

    pub fn delete_composio(&self, id: &str) -> Result<(), String> {
        inertia_store::fsx::remove(&self.layout.record(Collection::Composio, id))
            .map_err(|e| e.to_string())
    }
}

/// Reads every record in a collection, skipping any that will not parse.
///
/// One unreadable file must not hide the rest - and the damaged original has
/// already been preserved by the reader, so nothing is lost by moving on.
fn read_collection<T: serde::de::DeserializeOwned + Default>(
    layout: &Layout,
    collection: Collection,
) -> Vec<T> {
    let dir = layout.collection_dir(collection);
    inertia_store::fsx::list_dir(
        &dir,
        inertia_store::fsx::ListOptions {
            ext: Some("json"),
            ..Default::default()
        },
    )
    .into_iter()
    .filter_map(|name| {
        let (record, outcome) = inertia_store::fsx::read_json::<T>(&dir.join(&name));
        match outcome {
            inertia_store::fsx::ReadOutcome::Loaded => Some(record),
            inertia_store::fsx::ReadOutcome::Damaged { kept_at } => {
                tracing::warn!(
                    file = %name,
                    kept_at = %kept_at.display(),
                    "a record could not be read; the original was preserved"
                );
                None
            }
            inertia_store::fsx::ReadOutcome::Absent => None,
        }
    })
    .collect()
}

/// A turn in flight, so it can be stopped.
pub struct Running {
    /// Dropping this cancels the turn: the provider request and any tool
    /// future go with it.
    pub cancel: tokio::sync::oneshot::Sender<()>,
    /// Pending approval cards belonging to this turn, refused when it stops so
    /// nothing is left blocked on a card the user can no longer see.
    pub gate: Arc<UiPermissionGate>,
    /// The conversation this turn belongs to, so what it was holding open can
    /// be released when it stops.
    pub session: String,
    /// Where a message typed while this turn was still running goes.
    ///
    /// Held beside the cancel handle because it answers the same question from
    /// the other side: the window has a turn id and something the person said,
    /// and either stops the turn or speaks into it.
    pub steer: inertia_agent::Steer,
}

#[derive(Default)]
pub struct AppState {
    workspace: Mutex<Option<Arc<Workspace>>>,
    running: Mutex<HashMap<String, Running>>,
    /// The project folder the person is working in.
    ///
    /// Held here because two things need it and neither is given it: the Memory
    /// screen, which asks for a collection by name and has nowhere to put a
    /// folder, and a capture pass, which runs minutes after the turn that set
    /// it. A turn sets it, and opening a conversation on a folder sets it.
    project: Mutex<Option<std::path::PathBuf>>,
    /// The conversations still waiting to be mined for what they were worth.
    pub capture: Arc<inertia_memory::Capture>,
    /// Where each conversation's last turn ran.
    ///
    /// Process state, not workspace state: it answers "has the folder moved
    /// since the last thing I said", which is only a question within one run of
    /// the app.
    cwds: Mutex<HashMap<String, std::path::PathBuf>>,
    /// When each failing MCP server may next be tried. Process state: what is
    /// actually up, as opposed to what the record on disk remembers.
    pub mcp_backoff: Arc<crate::supervisor::Backoff>,
    /// Finished subagent runs, so a `task` follow-up continues one rather than
    /// paying for a fresh read of everything it already read.
    pub tasks: Arc<crate::task_tool::Sessions>,
    /// Who is in each group conversation, and whose turn it is.
    ///
    /// Process-wide rather than per-turn, because the room is what the next
    /// turn reads to find out it exists: an agent invited during one turn has
    /// to still be in the roster when the following one is seated.
    pub rooms: Arc<crate::group::Rooms>,
    /// Every helper run the team tools have started, across every conversation.
    ///
    /// App-wide rather than per-turn because that is the whole feature: `spawn`
    /// returns while the run is still going, and the turn that reads the answer
    /// back with `collect` is a later one.
    pub runs: Arc<crate::crew::Runs>,
    /// Questions waiting on the person, across every turn.
    ///
    /// Process-wide for the same reason the permission cards are: the command
    /// that answers one comes from the window holding an id, and it has no
    /// reason to know which turn raised it.
    pub questions: Arc<crate::question::Questions>,
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open_workspace(&self, root: std::path::PathBuf) -> Result<(), String> {
        let workspace = Workspace::open(root)?;
        *self.workspace.lock() = Some(Arc::new(workspace));
        // The remembered screenshots belong to the machines of the workspace
        // that is being left. Kept, they would have an agent told that the
        // screen "has not changed" against a picture of somebody else's.
        crate::computer_tools::forget_frames();
        Ok(())
    }

    /// The same, with a window to announce writes to.
    ///
    /// Two ways in because there are two: a person opening a folder, and a
    /// routine firing in a process whose window may never have opened. The
    /// second must not be prevented from running by the absence of the first.
    pub fn open_workspace_for(
        &self,
        app: &AppHandle,
        root: std::path::PathBuf,
    ) -> Result<(), String> {
        let mut workspace = Workspace::open(root)?;
        let handle = app.clone();
        workspace.emit = std::sync::Arc::new(move |payload| {
            let _ = handle.emit("workspace:changed", payload);
        });
        *self.workspace.lock() = Some(Arc::new(workspace));
        crate::computer_tools::forget_frames();
        Ok(())
    }

    /// The open workspace, or an error the frontend can show.
    ///
    /// Every command that touches workspace data goes through this, so
    /// "no workspace yet" is reported once, in one wording, rather than as a
    /// different failure per command.
    pub fn workspace(&self) -> Result<Arc<Workspace>, String> {
        self.workspace
            .lock()
            .clone()
            .ok_or_else(|| "No workspace is open.".to_string())
    }

    /// Records where this turn runs, and answers where the last one did.
    ///
    /// `None` when nothing has moved, which is the common case and the only
    /// one the prompt should stay quiet about.
    pub fn remember_cwd(&self, thread: &str, cwd: &std::path::Path) -> Option<String> {
        let mut cwds = self.cwds.lock();
        let previous = cwds.insert(thread.to_string(), cwd.to_path_buf());
        previous
            .filter(|before| before != cwd)
            .map(|before| before.display().to_string())
    }

    /// Forgets the open workspace, without touching the folder.
    pub fn close_workspace(&self) {
        *self.workspace.lock() = None;
        *self.project.lock() = None;
        self.capture.reset();
    }

    /// Which project is in front of the person.
    pub fn project(&self) -> Option<std::path::PathBuf> {
        self.project.lock().clone()
    }

    pub fn set_project(&self, folder: Option<std::path::PathBuf>) {
        *self.project.lock() = folder.filter(|path| path.is_dir());
    }

    /// Memories, across the workspace and the project that is open.
    pub fn memory(&self) -> Result<inertia_memory::Store, String> {
        let workspace = self.workspace()?;
        let backend = memory_backend(&workspace);
        Ok(memory_store(&workspace, self.project(), &backend))
    }

    /// What the settings screen says about memory.
    pub fn memory_settings(&self) -> Result<inertia_memory::Settings, String> {
        let workspace = self.workspace()?;
        let identity = inertia_store::collections::read_document(
            &workspace.layout,
            inertia_store::layout::Document::Identity,
            serde_json::json!({}),
        );
        // `settings/identity.json` holds `{ user: { preferences: { ... } } }`,
        // which is where the settings screen writes every memory switch.
        Ok(inertia_memory::Settings::from_preferences(
            identity
                .pointer("/user/preferences")
                .unwrap_or(&serde_json::Value::Null),
        ))
    }

    pub fn workspace_root(&self) -> Option<String> {
        self.workspace
            .lock()
            .as_ref()
            .map(|w| w.layout.root().display().to_string())
    }

    pub fn register(&self, turn_id: String, running: Running) {
        self.running.lock().insert(turn_id, running);
    }

    /// Stops a turn, refusing anything it had waiting for approval.
    pub fn stop(&self, turn_id: &str) -> bool {
        let Some(running) = self.running.lock().remove(turn_id) else {
            return false;
        };
        running.gate.abandon_all();
        // And anything the turn was holding open a question for. A card whose
        // turn has stopped can never be answered, and left on screen it is a
        // conversation that looks like it is still waiting on the person.
        self.questions.abandon_session(&running.session);
        // Dropping the sender is what the turn task is watching for.
        drop(running.cancel);
        true
    }

    /// Hand a message to a turn that is still going.
    ///
    /// `false` means there was no such turn, or it had already ended, which is
    /// not a failure: the window sends it as an ordinary message instead.
    pub fn steer(&self, turn_id: &str, text: &str) -> bool {
        let steer = self.running.lock().get(turn_id).map(|r| r.steer.clone());
        steer.is_some_and(|steer| steer.send(text))
    }

    pub fn finished(&self, turn_id: &str) {
        self.running.lock().remove(turn_id);
    }

    /// Stops everything, and says how many that was.
    /// Everything a stopped turn was holding open, released.
    ///
    /// The cards go too: an approval or a question whose turn has gone can
    /// never be answered, and one left on screen is a conversation that looks
    /// like it is still waiting on the person.
    pub fn stop_all(&self) -> usize {
        self.questions.abandon_all();
        let running: Vec<String> = self.running.lock().keys().cloned().collect();
        running.iter().filter(|id| self.stop(id)).count()
    }

    /// Every approval card currently on screen, across every running turn.
    pub fn pending_permissions(&self) -> Vec<crate::permission::Ask> {
        self.running
            .lock()
            .values()
            .flat_map(|running| running.gate.pending())
            .collect()
    }

    /// Which turns are still running.
    ///
    /// The window asks this when a turn has gone quiet, because it cannot tell
    /// a wedged turn from a slow one by watching. An answer that does not name
    /// the turn is what lets it stop waiting.
    pub fn running_ids(&self) -> Vec<String> {
        self.running.lock().keys().cloned().collect()
    }

    /// Routes an approval answer to whichever turn is waiting on it.
    ///
    /// Broadcast rather than addressed: the frontend knows the request id but
    /// has no reason to know which turn raised it, and only one turn can be
    /// holding any given id.
    pub fn answer_permission(&self, id: &str, answer: crate::permission::Answer) {
        for running in self.running.lock().values() {
            running.gate.answer(id, answer);
        }
    }
}

/// Builds the provider a model reference names.
///
/// Returns a descriptive error rather than `None`: "no provider" and "the
/// provider is disabled" and "that model reference is malformed" need
/// different fixes, and the user is the one who has to make them.
pub fn provider_for(
    settings: &Settings,
    reference: &str,
) -> Result<(Arc<dyn Provider>, String), String> {
    if reference.trim().is_empty() {
        return Err("No model is selected. Choose one in Settings → Providers.".into());
    }

    let models = settings.models();
    let (provider_id, model) = inertia_store::Models::split_reference(reference).ok_or_else(
        || format!("`{reference}` is not a valid model reference; it should look like `anthropic/claude-sonnet-4`."),
    )?;

    let record = models.provider(provider_id).ok_or_else(|| {
        format!("No provider called `{provider_id}` is configured. Add one in Settings → Providers.")
    })?;

    if !record.enabled {
        return Err(format!("The `{provider_id}` provider is turned off."));
    }

    let mut config = ProviderConfig::new(
        record.id.clone(),
        record.base_url.clone(),
        settings.api_key_for(record),
    );
    for (name, value) in &record.headers {
        config = config.with_header(name, value);
    }

    let provider: Arc<dyn Provider> = match record.protocol() {
        Protocol::Anthropic => Arc::new(AnthropicProvider::new(config)),
        Protocol::OpenAi => Arc::new(OpenAiProvider::new(config)),
    };

    Ok((provider, model.to_string()))
}

/// Assembles the tool registry for one turn.
///
/// `project` is the folder the turn works in, which the memory tools need and
/// no other builtin does: a memory saved with `scope: "project"` has to land in
/// the right project's own folder, and one saved against the wrong one is
/// filed where it will never be recalled.
pub fn registry_for(
    workspace: &Arc<Workspace>,
    gate: Arc<dyn PermissionGate>,
    project: Option<std::path::PathBuf>,
) -> Arc<Registry> {
    registry_with(workspace, gate, project, Vec::new(), &[])
}

/// The same, plus tools that exist only for this turn, minus the ones this
/// turn's mode does not hold.
///
/// The group tools are the first case: they hold the conversation and the seat
/// they were built for, and a turn in a conversation that is not a room has no
/// business seeing them at all. Adding them here rather than behind a runtime
/// check inside each tool is what keeps "this is not a group conversation" out
/// of the model's list of things it could try.
///
/// `withheld` is the second, and it is the half that makes the composer's mode
/// pill mean anything. Plan mode's own prompt says "the tools that change
/// anything are withheld for this turn": a build that says that and then hands
/// the model `write` and `shell` anyway is not a weaker Plan mode, it is a
/// build that lies to the model about what it can do. Withheld rather than
/// refused at call time, because a tool a model can see is a tool it will try.
pub fn registry_with(
    workspace: &Arc<Workspace>,
    gate: Arc<dyn PermissionGate>,
    project: Option<std::path::PathBuf>,
    extra: Vec<Arc<dyn inertia_core::tool::Tool>>,
    withheld: &[&str],
) -> Arc<Registry> {
    // The store the settings screen asked for, which may be a memory server
    // shared with another coding agent rather than this workspace's folders.
    let memory = memory_store(workspace, project, &memory_backend(workspace));
    let keep = |tools: Vec<Arc<dyn inertia_core::tool::Tool>>| {
        tools
            .into_iter()
            .filter(|tool| !withheld.contains(&tool.id()))
            .collect::<Vec<_>>()
    };
    let registry = Registry::new(gate)
            .with_tools(keep(builtin_tools(
                workspace.reads.clone(),
                workspace.lists.clone(),
                workspace.background.clone(),
                workspace.lsp.clone(),
            )))
            .with_tools(keep(extra))
            // Inertia's own setup, as tools. Everything the app shows is a
            // file in the workspace folder, so an agent could already write one
            // with `write` - badly, and without the app noticing. These
            // validate first and announce after. Without them an agent asked
            // about its own team ran `ls` over the workspace and reported that
            // there were no agents in it.
            .with_tools(keep(crate::inertia_tools::inertia_tools(
                workspace.layout.clone(),
                workspace.emit.clone(),
                // The live one: connecting an app, saving an MCP server and
                // importing an API all reach real services, and a tool that
                // quietly did nothing would be worse than one that refuses.
                Arc::new(crate::inertia_tools::LiveApps::new(workspace.clone())),
            )))
            // The other half of the bargain the injected block makes: the
            // prompt carries titles, and these read one in full, write a new
            // one, or throw one away.
            .with_tools(inertia_memory::tools::all(memory))
            // Every external source joins the same flat list as the
            // builtins, sorted together by id, so the model cannot tell them
            // apart - and a broken one is dropped rather than taking the turn
            // down with it.
        .with_provider(workspace.mcp.clone())
        .with_provider(workspace.openapi.clone())
        .with_provider(workspace.composio.clone());
    Arc::new(registry)
}

/// The tool facts for a live registry, for the on-demand loader.
///
/// A thin adapter rather than a method on `Registry`: grouping is this app's
/// policy, and the registry should not have to know about it.
#[derive(Debug)]
pub struct RegistryFacts {
    registry: Arc<Registry>,
    root: std::path::PathBuf,
}

impl RegistryFacts {
    pub fn new(registry: Arc<Registry>, root: std::path::PathBuf) -> Arc<Self> {
        Arc::new(Self { registry, root })
    }
}

#[async_trait::async_trait]
impl crate::tool_access::Facts for RegistryFacts {
    async fn facts(&self) -> Vec<crate::tool_access::ToolFact> {
        self.registry
            .live(&self.root)
            .await
            .iter()
            .map(|tool| crate::tool_access::ToolFact::of(tool.as_ref()))
            .collect()
    }
}

/// The registry the model is actually handed, given how the person set
/// "Tool access".
///
/// On "all" this is the registry itself. On "load tools when they are needed"
/// it is wrapped, so the request carries the families a turn uses constantly
/// and one gate tool that names the rest. The setting is read from the same
/// place the renderer writes it, and until now nothing read it at all: the
/// control in Settings did nothing whatever it was set to.
pub fn as_configured(
    workspace: &Arc<Workspace>,
    registry: Arc<Registry>,
    root: &std::path::Path,
) -> Arc<dyn ToolRegistry> {
    let identity = inertia_store::collections::read_document(
        &workspace.layout,
        inertia_store::layout::Document::Identity,
        serde_json::json!({}),
    );
    if !crate::tool_access::wanted(&identity) {
        return registry;
    }
    let facts = RegistryFacts::new(registry.clone(), root.to_path_buf());
    crate::tool_access::Deferred::wrap(registry, facts)
}

/// Which memory backend the settings screen chose, for this workspace.
///
/// Read from where the renderer writes it. The screen has offered "keep them
/// on this machine" or "share them with a memory server" since the first
/// build, and nothing read the answer: whichever was chosen, memories went to
/// the workspace folder.
fn memory_backend(workspace: &Workspace) -> String {
    let identity = inertia_store::collections::read_document(
        &workspace.layout,
        inertia_store::layout::Document::Identity,
        serde_json::json!({}),
    );
    inertia_memory::Settings::from_preferences(
        identity
            .pointer("/user/preferences")
            .unwrap_or(&serde_json::Value::Null),
    )
    .backend
}

/// The memory store, with that backend behind it.
///
/// A remote that cannot be built falls back to the workspace's own folders and
/// records why, rather than failing: the alternative is a typo in a server name
/// meaning no agent in this workspace can hold a conversation.
pub fn memory_store(
    workspace: &Workspace,
    project: Option<std::path::PathBuf>,
    backend: &str,
) -> inertia_memory::Store {
    let store = inertia_memory::Store::new(workspace.layout.clone(), project);
    if backend.is_empty() || backend == inertia_memory::LOCAL_BACKEND {
        return store;
    }
    // Off a runtime there is nothing to drive the connection, so the local
    // folders are the honest answer rather than a panic.
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return store.fell_back(
            "Memories are being kept in this workspace: the memory server could not be              reached from here.",
        );
    };

    let id = backend
        .strip_prefix(inertia_memory::mcp::PREFIX)
        .unwrap_or(backend)
        .to_string();
    let record =
        inertia_store::collections::get(&workspace.layout, inertia_store::Collection::Mcp, &id)
            .ok()
            .flatten();
    let server = Arc::new(inertia_memory::mcp::LiveServer::new(
        workspace.mcp.clone(),
        id,
        workspace.layout.root().to_path_buf(),
        handle,
    ));
    match inertia_memory::mcp::Remote::open(server, backend, record.as_ref()) {
        Ok(remote) => store.with_remote(Arc::new(remote)),
        Err(why) => store.fell_back(why),
    }
}

/// The permission gate for one turn.
pub fn gate_for(
    app: &AppHandle,
    workspace: &Workspace,
    session: String,
    agent: Option<String>,
) -> Arc<UiPermissionGate> {
    Arc::new(UiPermissionGate::new(
        app.clone(),
        workspace.settings.clone(),
        session,
        agent,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_agent::prompt::Mode;
    use inertia_core::tool::Tool;

    /// A registry built the way a turn builds one, so what these assert is the
    /// list the model is actually handed.
    async fn ids(mode: Mode, extra: Vec<Arc<dyn Tool>>) -> Vec<String> {
        let dir = tempfile::tempdir().expect("a temp dir");
        let workspace = Arc::new(Workspace::open(dir.path().to_path_buf()).expect("a workspace"));
        let gate: Arc<dyn PermissionGate> = Arc::new(inertia_mock::MockGate::allow_all());
        let registry = registry_with(&workspace, gate, None, extra, &mode.withheld());
        registry
            .specs()
            .await
            .into_iter()
            .map(|spec| spec.name)
            .collect()
    }

    #[tokio::test]
    async fn an_autonomous_turn_is_handed_the_tools_that_do_the_work() {
        let tools = ids(Mode::Autonomous, Vec::new()).await;
        for tool in ["read", "write", "edit", "glob", "grep", "shell", "ls", "todowrite"] {
            assert!(tools.contains(&tool.to_string()), "Autonomous is missing {tool}");
        }
        // And not the ones that only mean something somewhere else.
        for tool in ["invite", "handover", "part", "present_plan"] {
            assert!(!tools.contains(&tool.to_string()), "Autonomous was handed {tool}");
        }
    }

    #[tokio::test]
    async fn a_plan_turn_cannot_change_anything_and_can_hand_over_a_plan() {
        let tools = ids(Mode::Plan, Vec::new()).await;
        // Its own prompt says these are withheld. They have to actually be.
        for tool in ["write", "edit", "shell"] {
            assert!(!tools.contains(&tool.to_string()), "Plan still holds {tool}");
        }
        // Being unable to write is not being unable to investigate.
        for tool in ["read", "glob", "grep", "ls"] {
            assert!(tools.contains(&tool.to_string()), "Plan lost {tool}");
        }
        assert!(tools.contains(&"present_plan".to_string()));
    }

    #[tokio::test]
    async fn a_chat_turn_cannot_reach_a_helper_to_do_what_it_may_not_do_itself() {
        let tools = ids(Mode::Chat, Vec::new()).await;
        // The hole this closes: the turn that may not write a file asks a
        // subagent - which holds full tools of its own - to write it.
        assert!(!tools.contains(&"task".to_string()));
        assert!(!tools.contains(&"shell".to_string()));
        assert!(!tools.contains(&"present_plan".to_string()));
    }

    #[tokio::test]
    async fn a_tool_built_for_this_turn_joins_the_same_list_and_obeys_the_same_mode() {
        let extra: Vec<Arc<dyn Tool>> =
            vec![Arc::new(inertia_tools::builtin::look::PresentPlanTool)];

        // Plan holds it...
        assert!(ids(Mode::Plan, extra.clone()).await.contains(&"present_plan".to_string()));
        // ...and nothing else does, even when the turn offers it.
        assert!(!ids(Mode::Autonomous, extra).await.contains(&"present_plan".to_string()));
    }
}

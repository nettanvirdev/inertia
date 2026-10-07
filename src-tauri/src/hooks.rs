//! The hooks command surface: what the settings pane reads, and the two things
//! it can do.
//!
//! Read-mostly on purpose. The hooks themselves are a file the user edits in
//! their own editor - a hook is a program, and a form for writing programs is
//! worse than a text editor - so the pane's job is to show what is loaded, say
//! what is wrong with it, and show what ran. The two writes are the example
//! file, for somebody starting from nothing, and the master switch.

use inertia_hooks::{Recorder, RunSummary};
use inertia_store::layout::Document;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

/// The channel a hook run is announced on.
const HOOK_RUN: &str = "hooks:run";

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The pane asks for the hooks in force for one working folder.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListOptions {
    pub cwd: Option<String>,
}

/// Everything loaded for the workspace, with its warnings and where it came from.
#[tauri::command]
pub fn hooks_list(
    state: State<'_, AppState>,
    options: Option<ListOptions>,
) -> Result<Value, String> {
    let workspace = state.workspace().ok();
    let layout = workspace.as_ref().map(|w| &w.layout);
    let cwd = options
        .unwrap_or_default()
        .cwd
        .filter(|c| !c.trim().is_empty())
        .map(std::path::PathBuf::from);

    let loaded = inertia_hooks::load(layout, cwd.as_deref(), None);

    // Enabled and disabled in one list, because the pane draws them together:
    // somebody wondering why their hook did not fire needs to see that it is
    // there and switched off, not to find nothing at all.
    let mut handlers = loaded.handlers.clone();
    handlers.extend(loaded.disabled.iter().cloned());

    Ok(json!({
        "handlers": handlers.iter().map(|handler| json!({
            "id": handler.id,
            "event": handler.event,
            "matcher": handler.matcher,
            "type": match handler.kind {
                inertia_hooks::Kind::Prompt => "prompt",
                inertia_hooks::Kind::Command => "command",
            },
            "name": handler.name,
            "command": handler.command,
            // Enough to recognise which hook this is, not the whole essay: the
            // pane shows it on one line.
            "prompt": handler.prompt.as_ref().map(|p| p.chars().take(300).collect::<String>()),
            "timeoutMs": handler.timeout_ms,
            "enabled": handler.enabled,
            "source": handler.source,
        })).collect::<Vec<_>>(),
        "warnings": loaded.warnings,
        "sources": loaded.sources,
        "events": inertia_hooks::EVENTS,
        "path": layout.map(|layout| layout.document(Document::Hooks).display().to_string()),
    }))
}

#[tauri::command]
pub fn hooks_recent(recorder: State<'_, Arc<Recorder>>) -> Vec<RunSummary> {
    recorder.recent()
}

/// Write the example file. Refuses to overwrite one that exists.
#[tauri::command]
pub fn hooks_write_example(state: State<'_, AppState>) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let existing =
        inertia_store::collections::read_document(&workspace.layout, Document::Hooks, Value::Null);
    if !existing.is_null() {
        // Never silently: the file is the user's own program, and replacing it
        // with an example would throw away work that exists nowhere else.
        return Err("hooks/hooks.json already exists. Edit it rather than replacing it.".into());
    }

    inertia_store::collections::write_document(
        &workspace.layout,
        Document::Hooks,
        &inertia_hooks::example(),
    )
    .map_err(err)?;

    Ok(json!({ "path": workspace.layout.document(Document::Hooks).display().to_string() }))
}

/// The master switch: `enabled` at the top of the workspace file.
#[tauri::command]
pub fn hooks_set_enabled(state: State<'_, AppState>, enabled: bool) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let mut doc =
        inertia_store::collections::read_document(&workspace.layout, Document::Hooks, json!({}));
    if !doc.is_object() {
        doc = json!({});
    }
    doc["enabled"] = Value::Bool(enabled);

    inertia_store::collections::write_document(&workspace.layout, Document::Hooks, &doc)
        .map_err(err)?;
    Ok(json!({ "enabled": enabled }))
}

/// Open the file in whatever the user edits JSON with.
#[tauri::command]
pub fn hooks_reveal(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    use tauri_plugin_opener::OpenerExt;

    let workspace = state.workspace()?;
    let file = workspace.layout.document(Document::Hooks);
    let opener = app.opener();

    if opener
        .open_path(file.display().to_string(), None::<&str>)
        .is_err()
    {
        // No file yet: open the folder instead, which is where it would go.
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
            opener
                .open_path(folder.display().to_string(), None::<&str>)
                .map_err(err)?;
        }
    }

    Ok(json!({ "path": file.display().to_string() }))
}

/// Forward every hook run to the window.
///
/// Subscribed once at startup rather than per pane, because the buffer is
/// process-wide and a run that happened while the settings screen was closed
/// is exactly the one somebody opens it to find.
pub fn forward_runs(app: &AppHandle, recorder: &Arc<Recorder>) {
    let handle = app.clone();
    recorder.on_run(Arc::new(move |summary: &RunSummary| {
        let _ = handle.emit(HOOK_RUN, summary);
    }));
}

/* -- firing them during a turn -------------------------------------------- */

/// The turn's hooks, bound to the loop's lifecycle seam.
///
/// Loaded once per turn rather than per call: a person editing a hook wants
/// the next *turn* to use it, and re-reading three files before every tool
/// call would put the filesystem in the hot path of every step.
pub struct TurnHooks {
    handlers: Vec<inertia_hooks::Handler>,
    context: inertia_hooks::Context,
    recorder: Arc<Recorder>,
    /// How a prompt handler puts its question to a model.
    ///
    /// `None` where no model could be resolved, which a prompt handler reports
    /// as such rather than silently approving.
    ask: Option<Arc<dyn inertia_hooks::Ask>>,
    /// Where a hook's run goes so the transcript can show it. `None` on the
    /// paths with no window to show it in.
    announce: Option<Announce>,
}

/// How a hook run reaches the transcript.
pub type Announce = Arc<dyn Fn(Value) + Send + Sync>;

/// Hand-written: neither the model nor the announcer has a `Debug`, and what
/// is worth printing about a turn's hooks is how many there are.
impl std::fmt::Debug for TurnHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnHooks")
            .field("handlers", &self.handlers.len())
            .field("can_ask_a_model", &self.ask.is_some())
            .finish_non_exhaustive()
    }
}

/// One model, no tools, for a prompt handler.
///
/// The re-entrancy question this used to defer: a prompt hook asks a question
/// of its own, and the answer is that it is not a turn - no tools, no hooks of
/// its own, no history - so it cannot reach back round into the loop that
/// fired it.
struct AskModel {
    provider: Arc<dyn inertia_core::Provider>,
    model: String,
}

impl std::fmt::Debug for AskModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AskModel")
            .field("model", &self.model)
            .finish()
    }
}

#[async_trait::async_trait]
impl inertia_hooks::Ask for AskModel {
    async fn ask(&self, system: &str, prompt: &str) -> Result<String, String> {
        use futures::StreamExt;
        use inertia_core::provider::{ChatRequest, StreamEvent};

        let mut stream = self.provider.stream_chat(ChatRequest {
            model: self.model.clone(),
            system: system.to_string(),
            history: vec![inertia_core::message::Entry::user(prompt.to_string())],
            tools: Vec::new(),
            // The same question twice should get the same verdict; a hook that
            // blocks a command on Tuesday and allows it on Wednesday is a hook
            // nobody keeps.
            temperature: Some(0.0),
            ..Default::default()
        });

        let mut said = String::new();
        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta { text } => said.push_str(&text),
                StreamEvent::Error { message, .. } => return Err(message),
                _ => {}
            }
        }
        Ok(said)
    }
}

impl TurnHooks {
    /// Everything in force for one turn, or `None` when there is nothing to
    /// run - which is the common case, and worth not attaching at all so the
    /// loop does not await a listener that has nothing to say.
    pub fn load(
        layout: &inertia_store::Layout,
        cwd: &std::path::Path,
        recorder: Arc<Recorder>,
        context: inertia_hooks::Context,
    ) -> Option<Self> {
        let loaded = inertia_hooks::load(Some(layout), Some(cwd), None);
        if loaded.handlers.is_empty() {
            return None;
        }
        Some(Self {
            handlers: loaded.handlers,
            context,
            recorder,
            ask: None,
            announce: None,
        })
    }

    /// Give prompt handlers a model to ask.
    ///
    /// Resolved from the same settings the turn resolved its own model from,
    /// and quietly left alone when nothing resolves: a prompt handler then
    /// reports that it had no model, which is visible in the Hooks pane.
    pub fn with_model(mut self, settings: &Arc<inertia_store::Settings>, reference: &str) -> Self {
        if let Ok((provider, model)) = crate::state::provider_for(settings, reference) {
            self.ask = Some(Arc::new(AskModel { provider, model }));
        }
        self
    }

    /// Where each run goes so the transcript can show it.
    pub fn announcing(mut self, announce: Announce) -> Self {
        self.announce = Some(announce);
        self
    }

    async fn fire(
        &self,
        event: &str,
        subject: Option<&str>,
        input: Value,
    ) -> inertia_core::Reaction {
        let (outcome, runs) = inertia_hooks::fire(
            event,
            subject,
            input,
            &self.handlers,
            &self.context,
            self.ask.as_deref(),
            &self.recorder,
        )
        .await;

        // Into the transcript as well as into the pane. A hook that blocked a
        // command is the reason the turn did something unexpected, and a person
        // reading the conversation back should not have to go and find it in
        // another screen.
        if let Some(announce) = &self.announce {
            for run in &runs {
                announce(crate::changes::hook_event(run));
            }
        }

        inertia_core::Reaction {
            block: outcome.block,
            reason: outcome.reason,
            context: outcome.context,
            message: outcome.message,
            updated_input: outcome.updated_input,
        }
    }
}

/// Tool arguments reach a hook as JSON, not as the string the provider sent.
///
/// A hook that has to `JSON.parse` a field before it can look at a command is
/// a hook everybody gets wrong once, and the format is Claude Code's, where
/// `tool_input` is an object.
fn arguments_of(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

#[async_trait::async_trait]
impl inertia_core::Lifecycle for TurnHooks {
    async fn pre_tool(&self, call: &inertia_core::ToolCall) -> inertia_core::Reaction {
        self.fire(
            "PreToolUse",
            Some(&call.name),
            json!({ "tool_name": call.name, "tool_input": arguments_of(&call.arguments) }),
        )
        .await
    }

    async fn post_tool(&self, result: &inertia_core::ToolResult) -> inertia_core::Reaction {
        self.fire(
            "PostToolUse",
            Some(&result.tool),
            json!({
                "tool_name": result.tool,
                "tool_response": { "ok": result.ok, "output": result.output },
            }),
        )
        .await
    }

    async fn stopping(&self) -> inertia_core::Reaction {
        self.fire("Stop", None, json!({})).await
    }
}

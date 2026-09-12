//! Memory, wired into the running app.
//!
//! The deciding - what is worth keeping, what reaches the prompt, within what
//! budget - is all in `inertia-memory`, which knows nothing about Tauri and
//! nothing about any particular model. This is the three things it cannot do
//! from there: reach a provider, know which folder the person is working in, and
//! be called at the right moments of a turn.

use std::sync::Arc;

use futures::StreamExt;
use inertia_core::message::Entry;
use inertia_core::provider::{ChatRequest, StreamEvent};
use inertia_memory::capture::{Armed, Complete};
use serde_json::{json, Value};
use tauri::State;

use crate::state::AppState;

/// How much of a reply a capture pass may produce.
///
/// Five memories and a two-hundred-word note, with room to be wrong about it.
/// Unbounded would let one confused pass spend a turn's worth of tokens on a
/// conversation nobody is watching.
const MAX_REPLY_TOKENS: u32 = 2048;

/// One model call, resolved when it is made rather than when it is armed.
///
/// A conversation can sit quiet for minutes before its pass runs, and holding a
/// live provider - and therefore a key - in a map for that long is a cost with
/// nothing on the other side of it. This holds a model reference and builds the
/// provider at the moment it is asked.
#[derive(Debug)]
pub struct ModelPass {
    settings: Arc<inertia_store::Settings>,
    model_ref: String,
}

impl ModelPass {
    pub fn new(settings: Arc<inertia_store::Settings>, model_ref: String) -> Self {
        Self { settings, model_ref }
    }
}

#[async_trait::async_trait]
impl Complete for ModelPass {
    async fn complete(&self, system: &str, text: &str) -> Result<String, String> {
        let (provider, model) = crate::state::provider_for(&self.settings, &self.model_ref)?;

        let request = ChatRequest {
            model,
            system: system.to_string(),
            history: vec![Entry::user(text)],
            max_tokens: Some(MAX_REPLY_TOKENS),
            ..Default::default()
        };

        let mut stream = provider.stream_chat(request);
        let mut out = String::new();
        while let Some(event) = stream.next().await {
            match event {
                StreamEvent::Delta { text } => out.push_str(&text),
                StreamEvent::Error { message, .. } => return Err(message),
                StreamEvent::Done { .. } => break,
                // Reasoning is not the answer, and neither is anything else a
                // provider volunteers on the way.
                _ => {}
            }
        }
        Ok(out)
    }
}

/// Arm a conversation's capture pass, now that a turn has finished.
///
/// Best-effort throughout: memory is an improvement to the next conversation,
/// never a precondition for this one, so everything here that could fail is
/// allowed to and says nothing.
pub fn arm(state: &AppState, thread_id: &str, history: &[Entry], model_ref: &str) {
    let Ok(settings) = state.memory_settings() else { return };
    if !settings.enabled || settings.capture == "off" {
        return;
    }
    let (Ok(store), Ok(workspace)) = (state.memory(), state.workspace()) else {
        return;
    };

    let entry = Armed {
        store,
        transcript: transcript(history),
        count: history.len(),
        agent_id: None,
        model: Arc::new(ModelPass::new(
            Arc::clone(&workspace.settings),
            model_ref.to_string(),
        )),
        settings,
    };
    Arc::clone(&state.capture).arm(thread_id, entry);
}

/// The conversation, as text for a model that was not there.
///
/// Roles are named because the pass is asked to notice corrections the person
/// made, and a correction is only a correction when you can see who said it.
/// Tool output is left out entirely: it is the bulkiest part of a transcript and
/// the least likely to hold anything durable.
fn transcript(history: &[Entry]) -> String {
    history
        .iter()
        .filter_map(|entry| {
            let text = entry.text();
            if text.trim().is_empty() {
                return None;
            }
            let who = match entry {
                Entry::User(_) => "User",
                Entry::Assistant(_) => "Assistant",
                // Tool output is the bulkiest part of a transcript and the
                // least likely to hold anything durable.
                Entry::Tool(_) => return None,
            };
            Some(format!("{who}: {text}"))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/* -- commands ------------------------------------------------------------- */

/// What the settings screen says about memory, and where records are kept.
///
/// The paths are in the answer because "where did my memory go" is the first
/// question the split store raises, and a screen that can name both folders
/// answers it without anyone reading the source.
#[tauri::command]
pub fn memory_status(state: State<'_, AppState>) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let settings = state.memory_settings()?;
    let project = state.project();
    // Built rather than described, because the honest answer to "where are my
    // memories kept" is where they are actually going: a memory server that
    // will not answer means this workspace's own folders, and the screen has
    // to say so rather than keep showing the server it was pointed at.
    let store = state.memory()?;

    Ok(json!({
        "backend": settings.backend,
        "problem": store.problem(),
        "capabilities": store.capabilities(),
        "enabled": settings.enabled,
        "capture": settings.capture,
        "review": settings.review,
        "budget": settings.budget,
        "projectMemory": settings.project_memory,
        "workspacePath": workspace.layout.collection_dir(
            inertia_store::layout::Collection::Memories).display().to_string(),
        "projectPath": project.as_ref().map(|folder| {
            inertia_memory::store::project_layout(folder)
                .collection_dir(inertia_store::layout::Collection::Memories)
                .display()
                .to_string()
        }),
        "project": project.map(|folder| folder.display().to_string()),
    }))
}

/// Which folder the person is working in.
///
/// Called when a conversation's folder changes. Memory is scoped by it, so a
/// screen showing the wrong project's memories is a screen showing memories that
/// will not be used.
#[tauri::command]
pub fn memory_set_project(state: State<'_, AppState>, cwd: Option<String>) -> Value {
    state.set_project(cwd.map(std::path::PathBuf::from));
    json!({ "project": state.project().map(|p| p.display().to_string()) })
}

/// Read what is already known, for a query.
#[tauri::command]
pub fn memory_recall(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<Value>, String> {
    Ok(inertia_memory::recall(
        &state.memory()?,
        &query,
        limit.unwrap_or(8).clamp(1, 50),
    ))
}

/// Write one memory by hand.
#[tauri::command]
pub fn memory_save(state: State<'_, AppState>, record: Value) -> Result<Value, String> {
    // `merge: false`, as for the screen: a person who typed this means to have
    // it, and folding it into something that reads similarly would look like
    // the save failing.
    state.memory()?.remember(&record, false)
}

#[tauri::command]
pub fn memory_forget(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    state.memory()?.forget(&id)?;
    Ok(json!({ "id": id, "removed": true }))
}

/// Run the pass for one conversation now, rather than waiting for it to go
/// quiet. What the Memory screen's "Look for something worth keeping" does.
#[tauri::command]
pub async fn memory_capture_now(
    state: State<'_, AppState>,
    thread_id: String,
) -> Result<Value, String> {
    let capture = Arc::clone(&state.capture);
    let outcome = capture.run(&thread_id).await;
    Ok(match outcome {
        Some(done) => json!({ "ran": true, "stored": done.stored, "note": done.note }),
        None => json!({ "ran": false, "stored": [], "note": false }),
    })
}

/* -- the prompt ----------------------------------------------------------- */

/// The memories this turn should carry, as a block for the system prompt.
pub fn block_for(state: &AppState, query: &str) -> String {
    let (Ok(store), Ok(settings)) = (state.memory(), state.memory_settings()) else {
        return String::new();
    };
    let memories = inertia_memory::for_turn(&store, &settings, None, query);
    inertia_memory::to_prompt_block(&memories)
}

/// What the last thing said was, which is what the memories are ranked against.
pub fn last_said(history: &[Entry]) -> String {
    history
        .iter()
        .rev()
        .find(|entry| matches!(entry, Entry::User(_)))
        .map(|entry| entry.text().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_transcript_names_who_said_what_and_drops_the_rest() {
        let history = vec![
            Entry::user("use pnpm, not npm"),
            Entry::assistant("noted"),
            Entry::user("   "),
        ];
        let text = transcript(&history);
        assert_eq!(text, "User: use pnpm, not npm\n\nAssistant: noted");
    }

    #[test]
    fn the_query_is_the_last_thing_the_person_said() {
        let history = vec![
            Entry::user("first"),
            Entry::assistant("reply"),
            Entry::user("where do deploys go"),
        ];
        assert_eq!(last_said(&history), "where do deploys go");
        assert_eq!(last_said(&[]), "");
    }
}

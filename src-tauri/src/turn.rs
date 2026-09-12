//! Running a turn, in the vocabulary the renderer speaks.
//!
//! `inertia-agent` has its own event type, shaped around what the loop actually
//! does. The renderer has a different one, shaped around what a transcript
//! needs to draw - a tool card that appears when a call starts rather than when
//! it finishes, a thought that ends when prose begins. Both are right for their
//! side, and this module is the translation between them.
//!
//! It lives in the app crate deliberately. Teaching `inertia-agent` about
//! `tool-start` would tie the loop to one window's drawing model and break the
//! thing that makes it testable headless.
//!
//! Every event goes out on one channel tagged with the turn's id, so a window
//! with two threads generating at once can tell them apart without a
//! subscription per turn.

use std::sync::Arc;

use futures::StreamExt;
use inertia_agent::{Agent, AgentConfig, AgentEvent, StopReason, Turn as AgentTurn};
use inertia_core::id::SessionId;
use inertia_core::message::Entry;
use inertia_core::tool::ToolRegistry;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::coalesce::Coalescer;
use crate::state::{gate_for, provider_for, AppState, Running};

/// The one channel every turn reports on.
pub const AGENT_EVENT: &str = "agent:event";

/// What the window asks for when it starts a turn.
///
/// Most of it is carried straight through. The fields this build does not act
/// on yet - the group-conversation ones - are accepted rather than refused, so
/// a renderer that sends them is not broken by a backend that has not caught up
/// and does not have to learn which build it is talking to.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunRequest {
    pub thread_id: String,
    pub agent_id: Option<String>,
    pub model_ref: Option<String>,
    /// The transcript as the window folded it, already windowed and with
    /// attachments resolved. Taken as given rather than rebuilt from disk: the
    /// window knows about compaction boundaries and steers, and a second answer
    /// assembled here would eventually disagree with the one on screen.
    pub history: Vec<Entry>,
    pub message_id: Option<String>,
    pub conversation_mode: Option<String>,
    pub approval: Option<String>,
    pub cwd: Option<String>,
    /// The worktree this conversation entered, as the window remembers it:
    /// `{ root, branch, path }`.
    ///
    /// A `Value` rather than a struct for the reason every record in this app
    /// is one, and here it was load-bearing: typed as `Option<String>` this
    /// field did not merely read back empty, it failed the whole request -
    /// serde takes a missing field as the default and a wrong-typed one as an
    /// error, so the first turn in a conversation that had entered a worktree
    /// would have been refused outright with a deserialisation message.
    pub worktree: Option<Value>,
    pub primary_agent_id: Option<String>,
    pub mentioned: Vec<String>,
    pub continuation: bool,
    pub roster: Vec<String>,
    pub speaker: Option<String>,
    /// Write the conversation down from here, because nobody else will.
    ///
    /// A window keeps the transcript it is drawing and saves it through the
    /// `messages` collection as the reply arrives, so a turn it started must
    /// not be written a second time from this side. A turn started by the
    /// routine scheduler has no such author: left unset, its conversation would
    /// be empty, there would be nothing to open under Routines, and the run's
    /// own outcome - read off the last reply on disk - would report that the
    /// turn left nothing behind.
    pub persist: bool,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Sends one event to the window.
///
/// `id` and `at` are added here rather than at each call site: the renderer
/// routes on the first and closes a thought with the second, and one forgotten
/// stamp would show as a thinking block that never ends.
fn emit(app: &AppHandle, id: &str, thread_id: &str, mut event: Value) {
    if let Value::Object(map) = &mut event {
        map.insert("id".into(), json!(id));
        map.insert("threadId".into(), json!(thread_id));
        map.insert("at".into(), json!(now_ms()));
    }
    // Kept as well as sent. A window that reloads mid-turn, or opens a
    // conversation from last week, has nothing to draw from unless the turn
    // was written down - and this is the one place every event passes through,
    // so recording here is the only version that cannot miss one.
    crate::records::Recorder::global().event(id, &event);
    let _ = app.emit(AGENT_EVENT, event);
}

/// One loop event, as zero or more events the transcript understands.
///
/// `Start` produces nothing: the window synthesises its own `started` from the
/// id this command returns, and a second one would be a duplicate the reducer
/// has no case for.
fn translate(event: &AgentEvent, window: Option<u32>) -> Vec<Value> {
    match event {
        AgentEvent::Start { .. } => vec![],

        AgentEvent::Step { step } => vec![json!({ "type": "step", "step": step })],

        AgentEvent::Delta { text } => vec![json!({ "type": "delta", "text": text })],

        AgentEvent::Reasoning { text } => vec![json!({ "type": "reasoning", "text": text })],

        // Something the person typed while this turn was still running. Echoed
        // into the transcript at the point it actually reached the model, which
        // is the only honest place for it: shown where they typed it, it would
        // claim the model had read it before it had.
        AgentEvent::Steered { text } => vec![json!({ "type": "steer", "text": text })],

        AgentEvent::ToolStarted { call, title } => vec![json!({
            "type": "tool-start",
            "callId": call.id.as_str(),
            "name": call.name,
            "title": title,
            // Parsed rather than passed as the raw string: the card renders
            // named fields, and `parsed_arguments` already falls back to an
            // empty object for the arguments a model sent as a bare `""`.
            "args": call.parsed_arguments(),
        })],

        AgentEvent::ToolFinished { result } => vec![json!({
            "type": "tool-end",
            "callId": result.call_id.as_str(),
            "name": result.tool,
            "title": result.title,
            "ok": result.ok,
            "output": result.output,
            "metadata": tool_metadata(result),
            "durationMs": result.duration_ms,
        })],

        // Not an error: something was worked around and the turn continued. The
        // reducer merges consecutive notices of the same kind, which is what
        // turns "retrying in 1s / 2s / 4s" into one line with a changing number.
        AgentEvent::Notice { message } => {
            vec![json!({ "type": "warning", "kind": "info", "message": message })]
        }

        AgentEvent::Done {
            stopped,
            usage,
            history: _,
        } => {
            let mut out = Vec::new();

            if let Some(usage) = usage {
                // `context` beside the counts, and it is not decoration. The
                // window reads this to draw the gauge in the chat header and to
                // decide when a conversation has to be summarised - and it was
                // never sent, so the gauge never appeared and automatic
                // compaction never once fired. A long conversation simply ran
                // until the provider refused it.
                out.push(json!({
                    "type": "usage",
                    "usage": tokens(usage),
                    "context": context_of(usage, window),
                }));
            }

            // A failure is reported as an error *and* a done. The error is what
            // puts the sentence on screen and settles the running cards; the
            // done is what releases the composer. Sending only the error would
            // leave the thread believing it was still working.
            if let StopReason::Error { message } = stopped {
                out.push(json!({ "type": "error", "message": message }));
            }

            out.push(json!({
                "type": "done",
                "stopped": stop_word(stopped),
                "usage": usage.as_ref().map(tokens),
            }));
            out
        }
    }
}

/// Token counts in the window's names.
///
/// `input` is every input token including the cached ones, because that is what
/// the transcript's own `totalTokens` adds to `output`, and splitting them here
/// would undercount every cached turn. The cache figures travel beside it under
/// the names the pricing table uses, so a conversation's cost can still be
/// worked out properly.
fn tokens(usage: &inertia_core::provider::Usage) -> Value {
    json!({
        "input": usage.total_input(),
        "output": usage.output_tokens,
        "cachedInput": usage.cache_read_tokens,
        "cacheWrite": usage.cache_write_tokens,
    })
}

/// How full the window is, for the gauge and for the summariser.
///
/// `used` is the whole request as the provider counted it - every input token
/// including the cached ones, plus what it wrote back, because all of it is in
/// the transcript the next turn sends. `None` when nobody knows how big the
/// window is: a gauge against a guessed denominator is worse than no gauge, and
/// summarising on one would throw away a conversation for no reason.
fn context_of(usage: &inertia_core::provider::Usage, window: Option<u32>) -> Value {
    match window {
        Some(window) if window > 0 => json!({
            "used": usage.total_input() + usage.output_tokens,
            "window": window,
        }),
        _ => Value::Null,
    }
}

/// The word the renderer branches on. It only distinguishes cancellation from
/// everything else, but the rest are sent because the activity log keeps them.
fn stop_word(reason: &StopReason) -> &'static str {
    match reason {
        StopReason::Complete => "complete",
        StopReason::MaxSteps => "maxSteps",
        StopReason::Error { .. } => "error",
        StopReason::Cancelled => "cancelled",
        StopReason::Refused => "refused",
    }
}

/// What the card shows beyond its output.
///
/// `error: "denied"` is the one the window acts on: a refused call is drawn as
/// blocked rather than failed, and the activity feed files it under permission
/// rather than under failure.
fn tool_metadata(result: &inertia_core::tool::ToolResult) -> Value {
    match result.metadata.clone() {
        Some(value @ Value::Object(_)) => value,
        // An absent map is an empty one rather than `null`: the reducer spreads
        // it into the card's existing metadata, and spreading null throws.
        _ => json!({}),
    }
}

/// Write down anything that went wrong, as it happens.
///
/// Cancellation is deliberately not a failure: somebody pressed Stop, and a log
/// that fills up with their own decisions is a log nobody reads.
fn note_failure(
    layout: &inertia_store::Layout,
    facts: &Value,
    event: &AgentEvent,
    asked: &mut std::collections::HashMap<String, Value>,
) {
    let mut entry = match event {
        AgentEvent::ToolFinished { result } if !result.ok => {
            let args = asked.remove(result.call_id.as_str());
            json!({
                // A refused call and a call that failed are different facts
                // about different problems, and the panel files them apart.
                "kind": if result.was_denied() { "permission" } else { "tool" },
                "tool": result.tool,
                "error": result.output,
                "durationMs": result.duration_ms,
                "args": args,
            })
        }
        AgentEvent::ToolFinished { result } => {
            asked.remove(result.call_id.as_str());
            return;
        }
        AgentEvent::Done {
            stopped: StopReason::Error { message },
            ..
        } => json!({ "kind": "provider", "error": message }),
        AgentEvent::Done {
            stopped: StopReason::MaxSteps,
            ..
        } => json!({
            "kind": "turn-limit",
            "error": "The turn reached its step limit before it finished.",
        }),
        _ => return,
    };

    // The turn's own facts, under the entry's, so a tool that reported its own
    // model or folder keeps it.
    if let (Value::Object(entry), Value::Object(facts)) = (&mut entry, facts) {
        for (key, value) in facts {
            entry.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    crate::failures::record(layout, entry);
}

/// A group turn's seat: who is speaking, and everything that follows from it.
#[derive(Debug)]
struct Seated {
    conversation: String,
    speaker: Option<String>,
    /// The seated agent's own model, which is not the one the window guessed.
    model: Option<String>,
    permissions: crate::group::Permissions,
    /// Who is in the room, as the prompt names them.
    room: inertia_agent::prompt::Room,
    /// How the shared transcript looks from this seat.
    seat: inertia_agent::perspective::Seat,
    /// Every agent on the team, as `(id, name)`, for reading the `@`s out of
    /// what this turn writes. Every agent rather than the roster: naming
    /// somebody who is not here yet is how they get brought in.
    names: Vec<(String, String)>,
}

/// Seat the room for a group turn, and describe it for the prompt.
///
/// Called at the top of every group turn rather than once, because there is no
/// setup moment this process can be sure it has seen - the window reloads, the
/// app restarts, a routine sends into a thread nobody has opened.
///
/// The speaker is decided HERE rather than taken from the request: the room's
/// rule for whose turn it is has to be one rule, and the window is the wrong
/// place to keep it - a second window, a reload mid-chain or a routine would
/// each have their own idea.
fn seat_room(state: &AppState, workspace: &crate::state::Workspace, request: &RunRequest) -> Option<Seated> {
    let conversation = request.thread_id.trim().to_string();
    if conversation.is_empty() {
        return None;
    }

    let settings = inertia_store::collections::read_document(
        &workspace.layout,
        inertia_store::layout::Document::Group,
        json!({}),
    );
    let permissions = crate::group::Permissions::from_settings(&settings);
    let primary = request
        .primary_agent_id
        .as_deref()
        .or(request.agent_id.as_deref())
        .map(str::trim)
        .filter(|id| !id.is_empty());

    state
        .rooms
        .open(&conversation, primary, permissions, &request.roster);

    // Three ways a turn starts, and they seat the room differently.
    //
    // `speaker` is Retry: one named agent goes again, in the place of a turn
    // that failed, leaving the queue and the hop count alone. A continuation is
    // the next link in a chain the last turn started, and the room already
    // knows who that is. Anything else is the person speaking, which is the
    // only one that resets the floor.
    if let Some(again) = request.speaker.as_deref().filter(|id| !id.trim().is_empty()) {
        state.rooms.resume(&conversation, again);
    } else if !request.continuation {
        state.rooms.asked(&conversation, &request.mentioned);
    }

    let (active, roster, permissions) = state.rooms.seated(&conversation)?;
    let speaker = active.or_else(|| primary.map(str::to_string));

    let agents = inertia_store::collections::list(&workspace.layout, inertia_store::Collection::Agents);
    let text = |record: &Value, key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let named: Vec<(String, String, Value)> = agents
        .into_iter()
        .filter_map(|record| {
            let id = text(&record, "id")?;
            let name = text(&record, "name").unwrap_or_else(|| id.clone());
            Some((id, name, record))
        })
        .collect();

    // What to call the person in the transcript. Their own name where they gave
    // one, and always marked as the person: it is the one voice in the room
    // whose word is final, and a label is how a model tells it from a colleague.
    let identity = inertia_store::collections::read_document(
        &workspace.layout,
        inertia_store::layout::Document::Identity,
        json!({}),
    );
    let person = format!(
        "{} (the person)",
        identity
            .get("user")
            .and_then(|user| user.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or("The person")
    );

    let seats = roster
        .iter()
        .filter_map(|id| {
            let (_, name, record) = named.iter().find(|(known, _, _)| known == id)?;
            Some(inertia_agent::prompt::Seat {
                id: id.clone(),
                name: name.clone(),
                role: text(record, "role"),
                description: text(record, "description"),
            })
        })
        .collect();

    Some(Seated {
        model: speaker.as_ref().and_then(|id| {
            named
                .iter()
                .find(|(known, _, _)| known == id)
                .and_then(|(_, _, record)| text(record, "model"))
        }),
        room: inertia_agent::prompt::Room {
            roster: seats,
            // Everyone on the team, for the "elsewhere" line. The filtering
            // against the roster happens in the prompt, which is the only place
            // that knows which of the two lists it is writing.
            //
            // No `description`: it is free text out of a record, and every
            // record's prose in every colleague's system prompt is an injection
            // surface. A name and a role is enough to ask for somebody.
            team: named
                .iter()
                .map(|(id, name, record)| inertia_agent::prompt::Seat {
                    id: id.clone(),
                    name: name.clone(),
                    role: text(record, "role"),
                    description: None,
                })
                .collect(),
            me: speaker.clone(),
            permissions: inertia_agent::prompt::RoomPermissions {
                can_invite: permissions.can_invite,
                can_handover: permissions.can_handover,
                can_leave: permissions.can_leave,
                max_agents: permissions.max_agents,
                max_hops: permissions.max_hops,
            },
            custom: settings
                .get("prompt")
                .and_then(Value::as_str)
                .map(str::to_string),
            person: Some(person.clone()),
        },
        seat: inertia_agent::perspective::Seat {
            me: speaker.clone(),
            names: named
                .iter()
                .map(|(id, name, _)| (id.clone(), name.clone()))
                .collect(),
            person,
        },
        names: named
            .into_iter()
            .map(|(id, name, _)| (id, name))
            .collect(),
        permissions,
        speaker,
        conversation,
    })
}

/// Where this turn's tools resolve relative paths, and where `shell` runs.
///
/// Four answers, in order, and the order is the whole of it:
///
///   1. The folder the conversation entered - a worktree. It outranks
///      everything because the person put the conversation there.
///   2. **The agent's own folder.** This was missing, and the effect was that
///      every agent worked in the workspace's scratch folder whatever their
///      record said: asked to look at its own team, an agent went rummaging
///      through `files/` and reported that the workspace had no agents in it.
///   3. The workspace's chosen working folder, for agents that named none.
///   4. The workspace's own scratch folder, which always exists.
///
/// Each candidate is *checked*, not trusted. An agent's folder is a path
/// somebody typed months ago and the disk has had opinions since: a project
/// deleted, a drive unplugged, a folder renamed. Every tool in the turn
/// inherits this, and a folder that is not there does not fail politely - on
/// Windows a missing working directory is reported as a failure to start the
/// shell at all, so a deleted project comes back to the model as "there is no
/// shell on this machine" and it stops trying to run anything.
fn working_directory(
    workspace: &crate::state::Workspace,
    entered: Option<&str>,
    agent_id: Option<&str>,
) -> std::path::PathBuf {
    let usable = |path: Option<String>| {
        path.map(std::path::PathBuf::from)
            .filter(|path| path.is_dir())
    };

    let of_agent = agent_id.and_then(|id| {
        inertia_store::collections::get(&workspace.layout, inertia_store::Collection::Agents, id)
            .ok()
            .flatten()
            .and_then(|record| {
                record
                    .get("cwd")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|cwd| !cwd.is_empty())
                    .map(str::to_string)
            })
    });

    let of_workspace = inertia_store::collections::read_document(
        &workspace.layout,
        inertia_store::layout::Document::App,
        json!({}),
    )
    .get("workingDirectory")
    .and_then(|value| value.as_str())
    .map(str::to_string);

    usable(entered.map(str::to_string))
        .or_else(|| usable(of_agent))
        .or_else(|| usable(of_workspace))
        .unwrap_or_else(|| workspace.layout.work_dir())
}

/// Starts a turn and returns its id immediately.
///
/// Returning the id rather than awaiting the turn is what lets the window
/// stream, draw tool cards and offer Stop - an awaited command would hold the
/// bridge until the whole turn finished.
#[tauri::command]
pub async fn agent_run(
    app: AppHandle,
    state: State<'_, AppState>,
    request: RunRequest,
) -> Result<Value, String> {
    let workspace = state.workspace()?;

    if request.history.is_empty() {
        return Err("There is nothing to send.".into());
    }

    // In a group it is the room that says who is speaking, and the window that
    // finds out. Everywhere else the request names the agent, as it always has.
    let seated = match request.conversation_mode.as_deref() {
        Some("group") => seat_room(&state, &workspace, &request),
        _ => None,
    };
    let speaker = seated
        .as_ref()
        .and_then(|seat| seat.speaker.clone())
        .or_else(|| request.agent_id.clone());

    /*
     * Which model this seat speaks with.
     *
     * Outside a room there is one answer and the window already worked it out:
     * `modelRef` on the request IS the agent's own model, resolved against the
     * configured providers before the turn was started.
     *
     * In a room that would be quietly wrong. The window can only resolve the
     * model of the conversation's PRIMARY agent - it does not know who the
     * floor will seat, because the floor is decided over here - so every agent
     * in the room would speak with the primary's model. A room of six
     * specialists, each deliberately given a different model, would run six
     * turns on one of them. So the seated agent's own model comes first, and
     * the request's is the fallback rather than the answer.
     *
     * The candidates are tried in order and the first whose provider still
     * exists wins, which keeps an agent pointed at a provider that has since
     * been deleted from ending the turn instead of falling back to something
     * that works.
     */
    let candidates: Vec<String> = [
        seated.as_ref().and_then(|seat| seat.model.clone()),
        request.model_ref.clone(),
        Some(workspace.settings.models().default_model),
    ]
    .into_iter()
    .flatten()
    .filter(|model| !model.trim().is_empty())
    .collect();

    let (reference, provider, model_id) = candidates
        .iter()
        .find_map(|candidate| {
            provider_for(&workspace.settings, candidate)
                .ok()
                .map(|(provider, id)| (candidate.clone(), provider, id))
        })
        // Nothing resolved: report the first candidate's own failure rather
        // than a sentence about a list, because that is the model the person
        // actually chose and its provider is the one they have to fix.
        .ok_or_else(|| match candidates.first() {
            Some(first) => provider_for(&workspace.settings, first)
                .err()
                .unwrap_or_else(|| "No model is configured.".to_string()),
            None => "No model is configured.".to_string(),
        })?;

    // The folder the turn works in.
    let root = working_directory(&workspace, request.cwd.as_deref(), speaker.as_deref());

    // Where the last turn in this conversation ran, so the prompt can say so
    // when it has moved. Read before this turn's folder is recorded, or every
    // turn would report itself as the change.
    let previous_cwd = state.remember_cwd(&request.thread_id, &root);

    // The folder the person is working in, for everything that is scoped by it
    // and is not handed one: the Memory screen, and a capture pass that runs
    // minutes after this turn has finished.
    state.set_project(Some(root.clone()));

    // Minted before the tools, not after: `task` reports a subagent's progress
    // against this turn's id, and a tool cannot be handed an id that does not
    // exist yet.
    let id = format!("turn_{}", uuid::Uuid::now_v7().simple());

    // Opened here, where everything it names is finally known: the seated
    // agent, the model that resolved, and the folder the turn will run in.
    crate::records::Recorder::global().start(
        &id,
        crate::records::Meta {
            thread_id: Some(request.thread_id.clone()),
            message_id: request.message_id.clone(),
            agent_id: speaker.clone(),
            agent_name: speaker
                .as_deref()
                .and_then(|id| crate::agents::resolve(&workspace.layout, id).ok())
                .map(|agent| agent.name),
            provider: None,
            model: Some(model_id.clone()),
            model_ref: Some(reference.clone()),
            cwd: Some(root.display().to_string()),
            layout: Some(workspace.layout.clone()),
        },
    );

    // The seat's own id, not the conversation's: per-agent permission rules
    // belong to whoever is actually about to run a tool.
    let gate = gate_for(&app, &workspace, request.thread_id.clone(), speaker.clone());

    // Every agent on the team as `(id, name)`, for reading the `@`s out of what
    // this turn writes. A seated turn already carries these; a one-to-one turn
    // needs them too now that it can end up in a room.
    let names_for_floor: Vec<(String, String)> = match &seated {
        Some(seat) => seat.names.clone(),
        None => inertia_store::collections::list(&workspace.layout, inertia_store::Collection::Agents)
            .into_iter()
            .filter_map(|record| {
                let id = record.get("id").and_then(Value::as_str)?.trim().to_string();
                if id.is_empty() {
                    return None;
                }
                let name = record
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&id)
                    .to_string();
                Some((id, name))
            })
            .collect(),
    };

    // The room tools. They hold the conversation and the seat they were built
    // for, so `part` can never remove the wrong agent.
    //
    // A turn already in a room gets all three. A turn in an ordinary
    // one-to-one chat gets `invite` and `handover` but not `part`, because
    // those two are how a conversation BECOMES a room - an agent that can see
    // the work has moved to somebody else's area saying so, which is the case
    // the feature is for and the one it could not reach while the tools only
    // existed inside a group. The room is opened when one of them is called.
    let mut extra = match &seated {
        Some(seat) => crate::group_tools::tools_with(
            crate::group_tools::Seat {
                rooms: state.rooms.clone(),
                layout: workspace.layout.clone(),
                conversation: seat.conversation.clone(),
                agent_id: seat.speaker.clone(),
            },
            &seat.permissions,
            true,
        ),
        None => {
            let settings = inertia_store::collections::read_document(
                &workspace.layout,
                inertia_store::layout::Document::Group,
                json!({}),
            );
            crate::group_tools::tools_with(
                crate::group_tools::Seat {
                    rooms: state.rooms.clone(),
                    layout: workspace.layout.clone(),
                    conversation: request.thread_id.clone(),
                    agent_id: speaker.clone(),
                },
                &crate::group::Permissions::from_settings(&settings),
                false,
            )
        }
    };

    // `task`. Built here rather than in the tools crate because running a
    // subagent needs a provider, a registry, a gate and an assembled prompt -
    // all of which live on this side. Chat and Plan do not get it: `task` is in
    // MUTATING, so a turn that may not write a file cannot ask a helper to.
    let delegate = Arc::new(crate::task_tool::AppDelegate {
        app: app.clone(),
        workspace: workspace.clone(),
        project: state.project(),
        cwd: root.clone(),
        turn_id: id.clone(),
        thread_id: request.thread_id.clone(),
        approval: request.approval.clone(),
    });
    extra.push(Arc::new(crate::task_tool::TaskTool::new(
        delegate.clone(),
        state.tasks.clone(),
        request.thread_id.clone(),
        reference.clone(),
    )) as Arc<dyn inertia_core::tool::Tool>);

    // The team tools: delegating without waiting, and everything that follows
    // from it. A family rather than one tool with a `wait` flag, because
    // starting work and needing its answer are two moments and the gap between
    // them is the point. They share the app-wide table of runs, which is what
    // lets this turn read back what a turn ten minutes ago started.
    extra.extend(crate::crew::crew_tools(
        delegate,
        state.runs.clone(),
        crate::crew::Caller {
            conversation: request.thread_id.clone(),
            // The conversation's own turn, not a run: nobody spawned this.
            run_id: None,
            depth: 0,
            agent: speaker
                .as_deref()
                .and_then(|id| crate::agents::resolve(&workspace.layout, id).ok()),
            model: reference.clone(),
            // Nothing digests this turn's steps yet, so `share_context` is
            // accepted and says it had nothing to share rather than pretending.
            recent_context: None,
        },
    ));

    // Loading a skill. The prompt advertises the names and one line each; this
    // is what fetches the body, and without it every skill in the workspace was
    // something the model could read about and never use.
    extra.push(crate::skills::skill_tool(workspace.layout.clone()));

    // The browser pane beside this conversation. App-side rather than in the
    // tools crate because the pane is a child webview of this window, and the
    // tools drive the tab the person is actually looking at.
    extra.extend(crate::preview::browser_tools(app.clone()));

    // Reading the terminal the person is typing in. Read-only on purpose - the
    // agent has `shell` for running its own commands, and two writers on one
    // stdin interleave - so Chat and Plan turns hold it too.
    extra.push(crate::terminal::terminal_tool());

    // The computer family, for an agent whose record names a machine. Empty
    // for everyone else, and for an agent pointing at a machine that has since
    // been removed - that agent carries on with `shell` rather than being
    // offered tools that can only fail.
    extra.extend(crate::computers::tools_for_agent(&state, speaker.as_deref()));

    // Asking the person a question mid-turn, and reading what has gone wrong
    // before. Neither changes anything, so both are offered in every mode.
    //
    // The emitter is `app.emit` rather than this module's `emit`: that one
    // stamps the turn's id over the event's own, and the window settles a
    // question card by matching the question's id - so a question announced
    // through it could never leave the screen.
    let asking = app.clone();
    extra.push(Arc::new(crate::question::QuestionTool::new(
        state.questions.clone(),
        Arc::new(move |event| asking.emit(AGENT_EVENT, event).is_ok()),
    )) as Arc<dyn inertia_core::tool::Tool>);
    extra.push(Arc::new(crate::failures::FailuresTool::new(workspace.layout.clone()))
        as Arc<dyn inertia_core::tool::Tool>);

    // Scheduling a run for later, as the agent that asked. Built per turn
    // because `ToolContext` carries the conversation but not the seat, and a
    // routine written under the wrong agent runs with the wrong powers.
    extra.push(crate::routines::later_tool(
        workspace.layout.clone(),
        speaker.clone(),
        speaker
            .as_deref()
            .and_then(|id| crate::agents::resolve(&workspace.layout, id).ok())
            .map(|agent| agent.name),
        request.approval.clone(),
        workspace.emit.clone(),
    ));

    // What this turn's mode does not hold. The composer's mode pill decides it,
    // and until now it decided nothing: a Plan turn was handed `write`, `edit`
    // and `shell` while being told in its own prompt that they had been taken
    // away.
    let mode = request
        .conversation_mode
        .as_deref()
        .map(inertia_agent::prompt::Mode::parse)
        .unwrap_or_default();
    let withheld = mode.withheld();

    // After the project is known, not before: the memory tools file a
    // project-scoped memory into that folder, and one built against the
    // previous turn's folder would write it where it can never be recalled.
    let registry = crate::state::registry_with(
        &workspace,
        gate.clone(),
        state.project(),
        extra,
        &withheld,
    );

    // What this turn can do, asked of the registry rather than assembled a
    // second time here: the mode withholds some, the permission rules deny
    // others, and the prompt must describe this list.
    //
    // Read before the deferring wrapper goes on, because the two answer
    // different questions. The wrapper says what is in this request; the
    // prompt says what the turn is able to do, and a browser that is one
    // `load_tools` call away is a browser the turn has. Describing only the
    // loaded half would quietly take the browsing and delegating sections of
    // the prompt away from anyone who chose to load tools on demand.
    let held: Vec<String> = registry
        .specs()
        .await
        .into_iter()
        .map(|spec| spec.name)
        .collect();

    // Wrapped when the person asked for tools to be loaded on demand, which
    // is a setting that existed in the window and was implemented nowhere.
    let registry = crate::state::as_configured(&workspace, registry, &root);

    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    // The handle the window steers this turn through, taken before the loop
    // starts so a message typed in the first second is not dropped.
    let steer = inertia_agent::Steer::new();
    state.register(
        id.clone(),
        Running {
            cancel,
            gate: gate.clone(),
            session: request.thread_id.clone(),
            steer: steer.clone(),
        },
    );

    // How hard this agent was told to think. Read from its record, because that
    // is where the agent editor writes it and nothing between the two had ever
    // picked it up.
    let (thinking_budget, reasoning_effort) = speaker
        .as_deref()
        .and_then(|id| {
            inertia_store::collections::get(
                &workspace.layout,
                inertia_store::Collection::Agents,
                id,
            )
            .ok()
            .flatten()
        })
        .map(|record| crate::agents::thinking_of(&record))
        .unwrap_or((None, None));

    let mut agent = Agent::new(
        provider,
        registry,
        gate,
        AgentConfig {
            model: model_id.clone(),
            thinking_budget,
            reasoning_effort,
            ..Default::default()
        },
    )
    // The queue the window already holds. Handed in rather than taken out
    // afterwards, so a message typed between registering the turn and the loop
    // starting is waiting for it rather than lost.
    .with_steering(steer);

    // The person's own lifecycle hooks.
    //
    // This was once attached on the other turn path and nowhere else, which
    // meant hooks ran for a routine's turn and never once for a turn somebody
    // started from the composer - the path that is every turn they actually
    // watch. A `PreToolUse` rule written to block `rm -rf` blocked nothing.
    // There is one path now, so there is one answer.
    //
    // Only when the workspace actually has hooks: attaching a listener with
    // nothing to run would put three file reads and an await into every tool
    // call of every turn, for nothing.
    let recorder = app
        .state::<std::sync::Arc<inertia_hooks::Recorder>>()
        .inner()
        .clone();
    if let Some(hooks) = crate::hooks::TurnHooks::load(
        &workspace.layout,
        &root,
        recorder,
        inertia_hooks::Context {
            session_id: Some(request.thread_id.clone()),
            thread_id: Some(request.thread_id.clone()),
            turn_id: Some(id.clone()),
            // The seat, not the conversation: a per-agent hook has to see the
            // agent that is actually running.
            agent: speaker.clone(),
            cwd: Some(root.clone()),
            root: Some(workspace.layout.root().to_path_buf()),
            model: Some(model_id.clone()),
            ..Default::default()
        },
    ) {
        // A prompt handler asks a model rather than running a command, so it
        // needs one. The same model the turn itself resolved, with no tools and
        // no history - it is a question, not a turn.
        let announcing = app.clone();
        let hook_id = id.clone();
        let hook_thread = request.thread_id.clone();
        let hooks = hooks
            .with_model(&workspace.settings, &reference)
            .announcing(std::sync::Arc::new(move |event| {
                emit(&announcing, &hook_id, &hook_thread, event);
            }));
        agent = agent.with_lifecycle(std::sync::Arc::new(hooks));
    }

    // Ranked against what was just said, so the memories that answer the
    // question are the ones that make the budget - not the ones used most.
    let memories = crate::memory::block_for(&state, &crate::memory::last_said(&request.history));

    // The folder this turn works in, kept for the snapshot the changes strip
    // is built from.
    let task_root = root.clone();
    let task_workspace_root = workspace.layout.root().to_path_buf();
    // Where failures are written down, and the facts every entry is stamped
    // with. The log is what makes "this has failed the same way four times" a
    // thing the agent can find out instead of a thing the person remembers.
    let task_layout = workspace.layout.clone();
    let task_facts = json!({
        "agent": speaker,
        "threadId": request.thread_id,
        "sessionId": request.thread_id,
        "turnId": id,
        "model": model_id,
        "cwd": root.display().to_string(),
    });

    let turn = AgentTurn {
        session: SessionId::from_existing(request.thread_id.clone()),
        // Assembled per turn: the workspace, the date and the tool list all
        // change between turns.
        system: crate::prompt::build(
            &workspace,
            &root,
            &memories,
            &crate::prompt::Turn {
                // The agent the ROOM seated, not the one the window guessed.
                // In a chain the window is a turn behind by definition: it
                // starts the next link from what the last one said was next,
                // and everything that makes a turn that agent's - its persona,
                // its permissions, its model - is settled over here.
                agent_id: speaker.clone(),
                mode: request.conversation_mode.clone(),
                approval: request.approval.clone(),
                model: reference.clone(),
                // A turn is somebody's subagent when it was spawned by another
                // agent, which this shell cannot do yet - so it is never one.
                // Stated rather than left to the default so that the day
                // spawning arrives, this is the line that has to change.
                is_subagent: false,
                worktree: request.worktree.clone(),
                previous_cwd,
                // What the registry actually answered with, so the prompt
                // describes the tools this turn has rather than the ones the
                // app can build.
                tools: held.clone(),
                room: seated.as_ref().map(|seat| seat.room.clone()),
            },
        ),
        // In a group, the shared transcript as seen from this seat: my replies
        // are mine, everyone else's - the person's and the other agents' - are
        // what was said to me, each with a name on it. Shown any other way, a
        // model starts writing the other agents' lines itself.
        history: match &seated {
            Some(seat) => inertia_agent::perspective::perspective(request.history, &seat.seat),
            None => request.history,
        },
        root,
    };

    let task_app = app.clone();
    let task_id = id.clone();
    let task_thread = request.thread_id.clone();
    let task_model = reference.clone();
    // How big this model's context is, carried into the event translator. A
    // window nobody has published stays `None` and the gauge stays hidden,
    // which is the honest answer rather than a bar measured against a guess.
    let task_window = inertia_provider::models::context_window(&model_id);
    // What the capture pass will read. Taken now rather than at the end: the
    // turn owns the history by then, and the pass wants what was sent anyway.
    let task_history = turn.history.clone();

    // Before a single token: the window drew a reply bubble against whichever
    // agent it guessed, and in a group that guess is routinely wrong. Sent
    // rather than returned because the reply starts streaming immediately and
    // the bubble has to carry the right name from its first character - along
    // with the right model, which is the same correction.
    if let Some(seat) = &seated {
        emit(
            &app,
            &id,
            &request.thread_id,
            json!({
                "type": "group",
                "speaker": seat.speaker,
                "modelRef": reference,
                "room": state.rooms.snapshot(&seat.conversation),
            }),
        );
    }

    // The transcript, when there is nobody to write it. See `reply.rs`.
    let mut transcript = request.persist.then(|| {
        crate::reply::Assembling::new(
            &reference,
            speaker.clone(),
            &crate::memory::last_said(&task_history),
            request
                .message_id
                .clone()
                .unwrap_or_else(|| format!("msg_{}", uuid::Uuid::now_v7().simple())),
        )
    });
    let task_conversations = workspace.conversations.clone();

    let task_rooms = state.rooms.clone();
    // The agent's display name, for a banner. Resolved here, while the layout
    // is still in hand.
    let task_agent_name = speaker
        .as_deref()
        .and_then(|id| crate::agents::resolve(&workspace.layout, id).ok())
        .map(|agent| agent.name);
    // Carried whether or not this turn started in a room: a one-to-one chat
    // that called `handover` has one by the time the turn ends, and the floor
    // has to be settled and sent to the window or the agent it was handed to
    // never speaks.
    let task_seat = match seated {
        Some(seat) => (seat.conversation, seat.speaker, seat.names),
        None => (request.thread_id.clone(), speaker.clone(), names_for_floor),
    };

    tauri::async_runtime::spawn(async move {
        let mut events = agent.run(turn);

        // `select!` rather than a token threaded through the loop: dropping the
        // stream is what cancels the provider request, and this is where the
        // drop happens.
        let stop = async move {
            let _ = cancelled.await;
        };
        tokio::pin!(stop);

        // Text is batched before it crosses the bridge. One event per token is
        // one JSON serialisation, one script evaluation in the webview and one
        // React state update per token, and on a fast provider that is the
        // difference between a reply that scrolls and a reply that stutters.
        // See `coalesce` for what is held and what goes straight through.
        let mut buffer = Coalescer::default();

        // What this turn wrote, for the one thing the room reads out of it: the
        // colleagues it named with `@`. Accumulated from the raw deltas rather
        // than from the coalesced ones so a handle split across two tokens is
        // still one handle.
        let mut said = String::new();
        // The floor moves once. A turn that failed reports an `error` and a
        // `done`, and settling on both would spend two of the room's hops on
        // one go and skip whoever was due to speak.
        let mut settled = false;
        // Whether this turn ended by passing the floor on. A room answering one
        // question is several turns, and all but the last of them end like
        // this - so this is what keeps a banner for each of them off the
        // taskbar. See `notify::Choices::chained`.
        let mut chained = false;

        let mut ended = false;
        /*
         * What the folder looked like before this turn touched it.
         *
         * Taken lazily, the first time a tool that writes is about to run
         * rather than at the top of every turn: most turns write nothing, and
         * walking a large repository to find that out would be a cost paid on
         * every message for the benefit of a few.
         *
         * The loop yields `ToolStarted` before the tool runs and does not
         * advance until this consumer polls again, which is what makes this a
         * correct pre-write seam rather than a race.
         */
        const SNAPSHOT_BEFORE: &[&str] = &["write", "edit", "patch", "shell"];
        let mut snapshot: Option<String> = None;
        let mut snapshot_tried = false;
        // What the turn failed with, if it did, for the banner.
        let mut failure: Option<String> = None;
        // The arguments of every call still in flight, by call id.
        let mut asked: std::collections::HashMap<String, Value> =
            std::collections::HashMap::new();
        loop {
            // Read before the `select!` rather than inside it: the arms are
            // evaluated together, and a deadline read after the buffer has
            // already taken this iteration's token would be a tick late.
            let due = buffer.due();

            tokio::select! {
                biased;

                _ = &mut stop => break,

                // Armed only while something is held back. `pending` is what
                // makes an idle turn wait on the provider alone rather than
                // waking sixteen times a second to find an empty buffer.
                _ = async {
                    match due {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    for translated in buffer.flush() {
                        emit(&task_app, &task_id, &task_thread, translated);
                    }
                }

                next = events.next() => {
                    let Some(event) = next else { break };
                    let finished = matches!(event, AgentEvent::Done { .. });
                    if let AgentEvent::Delta { text } = &event {
                        said.push_str(text);
                    }
                    // What the model asked for, kept until the result comes
                    // back: a failure without its arguments is a line in a log
                    // nobody can act on.
                    if let AgentEvent::ToolStarted { call, .. } = &event {
                        asked.insert(call.id.as_str().to_string(), call.parsed_arguments());
                    }
                    // Fed the raw event rather than the translated one: the
                    // stored shape is the window's, and it is derived from the
                    // same events the window's own reducer sees.
                    if let Some(transcript) = transcript.as_mut() {
                        transcript.observe(&event);
                    }
                    note_failure(&task_layout, &task_facts, &event, &mut asked);

                    if let AgentEvent::ToolStarted { call, .. } = &event {
                        if !snapshot_tried && SNAPSHOT_BEFORE.contains(&call.name.as_str()) {
                            snapshot_tried = true;
                            let root = task_workspace_root.clone();
                            let cwd = task_root.clone();
                            // Off the event loop: this walks a folder, and a
                            // stream that stops being polled is a reply that
                            // stops arriving.
                            snapshot = tokio::task::spawn_blocking(move || {
                                crate::changes::track(&root, &cwd)
                            })
                            .await
                            .ok()
                            .flatten();
                        }
                    }
                    for translated in translate(&event, task_window) {
                        for out in buffer.push(translated) {
                            /*
                             * Whose turn it is now - said BEFORE the turn is
                             * over.
                             *
                             * The window stops listening to a turn the moment
                             * it hears `done`, so anything sent after that is
                             * sent to nobody. In the other shell this lived
                             * after the loop for a while, and the room settled
                             * correctly, said so, and the window never heard
                             * it: an invited agent sat in the roster and said
                             * nothing until the person typed again.
                             *
                             * On an error as well as a `done`, on purpose: a
                             * turn that failed still used up its go, and
                             * leaving the floor with an agent that just errored
                             * would have the window start it again.
                             */
                            let terminal = matches!(
                                out.get("type").and_then(Value::as_str),
                                Some("done" | "error")
                            );
                            // Kept for the banner: a turn that failed should
                            // say so on the taskbar rather than claim it
                            // finished.
                            if out.get("type").and_then(Value::as_str) == Some("error") {
                                failure = out
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                            }
                            if terminal && !settled {
                                // A room, whether this turn started in one or
                                // opened one by handing over.
                                let (conversation, speaker, names) = &task_seat;
                                if task_rooms.snapshot(conversation).is_some() {
                                    settled = true;
                                    // Only from a turn that finished: a failed
                                    // one wrote nothing worth reading names out
                                    // of, and half a sentence can name somebody
                                    // it was about to say it was not asking.
                                    let mentioned = if out["type"] == "done" {
                                        inertia_agent::floor::mentioned_agents(
                                            &said,
                                            names.iter().map(|(id, name)| (id.as_str(), name.as_str())),
                                        )
                                    } else {
                                        Vec::new()
                                    };
                                    task_rooms.spoke(
                                        conversation,
                                        speaker.as_deref().unwrap_or_default(),
                                        &mentioned,
                                    );
                                    let room = task_rooms.snapshot(conversation);
                                    // A floor that now seats somebody is a
                                    // conversation still going. `active` is
                                    // `None` when it has gone back to the
                                    // person, which is the moment worth a
                                    // notice.
                                    chained = room
                                        .as_ref()
                                        .and_then(|room| room.get("active"))
                                        .and_then(Value::as_str)
                                        .is_some();
                                    emit(
                                        &task_app,
                                        &task_id,
                                        &task_thread,
                                        json!({ "type": "group", "room": room }),
                                    );
                                }
                            }
                            emit(&task_app, &task_id, &task_thread, out);
                        }
                    }
                    if finished {
                        ended = true;
                        break;
                    }
                }
            }
        }

        // Whatever was still held when the loop left. A cancelled turn is not
        // coming back for its tail, and half a sentence dropped on the floor
        // reads as a bug in the model rather than in the shell.
        for translated in buffer.flush() {
            emit(&task_app, &task_id, &task_thread, translated);
        }

        // A turn that was stopped, or whose stream simply ended, still owes the
        // window a `done`. Without it the composer offers Stop forever and the
        // thread cannot be told from one that is working.
        if !ended {
            // A stopped turn used up its go. The floor is settled with nothing
            // named, which hands it back to the person - the right place for a
            // conversation somebody has just interrupted, and the one place the
            // chain cannot restart itself from.
            if !settled {
                let (conversation, speaker, _) = &task_seat;
                if task_rooms.snapshot(conversation).is_some() {
                    task_rooms.spoke(conversation, speaker.as_deref().unwrap_or_default(), &[]);
                    emit(
                        &task_app,
                        &task_id,
                        &task_thread,
                        json!({ "type": "group", "room": task_rooms.snapshot(conversation) }),
                    );
                }
            }
            emit(
                &task_app,
                &task_id,
                &task_thread,
                json!({ "type": "done", "stopped": "cancelled", "usage": Value::Null }),
            );
        }

        // However it ended. A record left saying "running" is the same lie as
        // a message stuck on "streaming": nothing is coming to finish it.
        // What the turn changed on disk, and the snapshot to go back to. Only
        // when something actually wrote: a turn that read four files has no
        // strip to draw.
        if let Some(from) = snapshot {
            let root = task_workspace_root.clone();
            let cwd = task_root.clone();
            let from_id = from.clone();
            if let Ok(result) = tokio::task::spawn_blocking(move || {
                crate::changes::changes(&root, &cwd, &from_id, None)
            })
            .await
            {
                if let Some(event) = crate::changes::changes_event(&task_root, &from, &result) {
                    emit(&task_app, &task_id, &task_thread, event);
                }
            }
        }

        // Before the record is closed, so a routine's outcome - which reads
        // the last reply on disk - has something to read by the time anything
        // asks. A turn that was stopped still leaves what it managed to say.
        if let Some(mut transcript) = transcript {
            if !ended {
                transcript.interrupted();
            }
            transcript.save(&task_conversations, &task_thread);
        }

        crate::records::Recorder::global()
            .finish(&task_id, if ended { "done" } else { "interrupted" });

        // Somebody who walked away from a four-minute turn has no other way to
        // learn that it is over. Silent while the window is focused, and silent
        // for a turn the person stopped themselves.
        let ending = match (&failure, ended) {
            (Some(error), _) => crate::notify::Ending::Failed {
                error: error.clone(),
            },
            (None, true) => crate::notify::Ending::Finished,
            (None, false) => crate::notify::Ending::Interrupted,
        };
        crate::notify::turn_finished(
            &task_app,
            task_agent_name.as_deref(),
            &ending,
            &said,
            &task_thread,
            &task_id,
            chained,
        );

        if let Some(state) = task_app.try_state::<AppState>() {
            state.finished(&task_id);

            // A conversation does not end, it goes quiet. This arms a timer
            // that will mine it for what it was worth if nothing else is said;
            // the next turn stands the timer down again.
            crate::memory::arm(&state, &task_thread, &task_history, &task_model);
        }
    });

    Ok(json!({ "id": id, "threadId": request.thread_id }))
}

#[tauri::command]
pub fn agent_cancel(state: State<'_, AppState>, id: String) -> Value {
    json!({ "cancelled": state.stop(&id) })
}

#[tauri::command]
pub fn agent_cancel_all(state: State<'_, AppState>) -> Value {
    json!({ "cancelled": state.stop_all() })
}

/// What survived a reload.
///
/// Two questions share this answer. A window asks it when a turn has gone
/// quiet, because it cannot tell a wedged turn from a slow one by watching,
/// and an empty answer is what lets it stop waiting. A window that has just
/// opened asks it to find the turns it walked away from - and for that, an id
/// on its own is useless: the window needs the thread and the message the turn
/// is writing into before it can reattach to it, and the events so far before
/// it can show what happened while nobody was watching.
///
/// The running set is the spine rather than the record log, because the record
/// says "running" about turns this process is no longer running - the ones a
/// previous process left behind. A running turn with no record still answers
/// with its id, which is all the silence check ever wanted.
#[tauri::command]
pub fn agent_active(state: State<'_, AppState>) -> Vec<Value> {
    let mut records: std::collections::HashMap<String, Value> =
        crate::records::Recorder::global()
            .active()
            .into_iter()
            .filter_map(|record| {
                let id = record.get("id")?.as_str()?.to_string();
                Some((id, record))
            })
            .collect();
    state
        .running_ids()
        .into_iter()
        .map(|id| records.remove(&id).unwrap_or_else(|| json!({ "id": id })))
        .collect()
}

/// Hand a message to a turn already in flight.
///
/// `false` means the turn ended before the message got there, and the only
/// right response to that is to send it as an ordinary message - which is what
/// the window does with this answer. Whitespace is refused at the same seam, so
/// a stray newline cannot wake a turn with an empty message to interpret.
#[tauri::command]
pub fn agent_steer(state: State<'_, AppState>, id: String, text: String) -> Value {
    json!({ "steered": state.steer(&id, &text) })
}

/// Every tool an agent could call right now, from every source.
///
/// Listed through a gate that allows everything, because this is the catalogue
/// the settings screen draws - what exists, not what this agent may use. The
/// per-agent filtering happens on the way into a turn.
///
/// The per-turn tools are listed too, built against a conversation that does
/// not exist. They are real tools with real permission keys, and a catalogue
/// that left them out was a settings screen offering no way to write a rule
/// about delegating, asking a question, or driving the browser.
#[tauri::command]
pub async fn tools_list(app: AppHandle, state: State<'_, AppState>) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let gate: Arc<dyn inertia_core::tool::PermissionGate> = Arc::new(CatalogueGate);

    // A stand-in conversation: nothing here is run, only described.
    let catalogue = "tools-catalogue";
    let delegate = Arc::new(crate::task_tool::AppDelegate {
        app: app.clone(),
        workspace: workspace.clone(),
        project: state.project(),
        cwd: workspace.layout.work_dir(),
        turn_id: catalogue.to_string(),
        thread_id: catalogue.to_string(),
        approval: None,
    });
    let mut extra: Vec<Arc<dyn inertia_core::tool::Tool>> = crate::group_tools::tools_for(
        crate::group_tools::Seat {
            rooms: state.rooms.clone(),
            layout: workspace.layout.clone(),
            conversation: catalogue.to_string(),
            agent_id: None,
        },
        &crate::group::Permissions::default(),
    );
    extra.push(Arc::new(crate::task_tool::TaskTool::new(
        delegate.clone(),
        state.tasks.clone(),
        catalogue.to_string(),
        String::new(),
    )) as Arc<dyn inertia_core::tool::Tool>);
    extra.extend(crate::crew::crew_tools(
        delegate,
        state.runs.clone(),
        crate::crew::Caller {
            conversation: catalogue.to_string(),
            run_id: None,
            depth: 0,
            agent: None,
            model: String::new(),
            recent_context: None,
        },
    ));
    extra.push(crate::skills::skill_tool(workspace.layout.clone()));
    extra.push(crate::terminal::terminal_tool());
    extra.extend(crate::preview::browser_tools(app.clone()));
    extra.push(Arc::new(crate::failures::FailuresTool::new(
        workspace.layout.clone(),
    )) as Arc<dyn inertia_core::tool::Tool>);
    let emitting = app.clone();
    extra.push(Arc::new(crate::question::QuestionTool::new(
        state.questions.clone(),
        Arc::new(move |event| emitting.emit(AGENT_EVENT, event).is_ok()),
    )) as Arc<dyn inertia_core::tool::Tool>);
    extra.push(crate::routines::later_tool(
        workspace.layout.clone(),
        None,
        None,
        None,
        workspace.emit.clone(),
    ));

    let registry = crate::state::registry_with(&workspace, gate, state.project(), extra, &[]);

    let tools: Vec<Value> = registry
        .specs()
        .await
        .into_iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "parameters": spec.parameters,
            })
        })
        .collect();

    // The sources that failed travel with the list rather than being logged.
    // A tool list quietly missing an MCP server's twelve tools looks like a
    // server with no tools, and the settings screen is the only place that can
    // say otherwise.
    let problems: Vec<Value> = registry
        .problems()
        .await
        .into_iter()
        .map(|problem| json!({ "source": problem.source, "message": problem.message }))
        .collect();

    Ok(json!({ "tools": tools, "problems": problems }))
}

/* -- the prompts that suspend a tool ------------------------------------- */

/// The answer to one approval card.
///
/// The window sends "once", "always" or "reject" - its own words, not the
/// gate's. Anything unrecognised is a refusal: a typo must not become consent.
#[tauri::command]
pub fn permission_reply(
    state: State<'_, AppState>,
    id: String,
    answer: String,
    _message: Option<String>,
) -> Value {
    let decision = match answer.as_str() {
        "once" | "allow" => crate::permission::Answer::Allow,
        "always" => crate::permission::Answer::AllowAlways,
        _ => crate::permission::Answer::Deny,
    };
    state.answer_permission(&id, decision);
    json!({ "answered": true })
}

/// What is already waiting when a window opens mid-turn.
///
/// The user switched threads, or reloaded. Without this the card is gone and
/// the tool stays blocked on an answer nobody can give.
#[tauri::command]
pub fn permission_waiting(state: State<'_, AppState>, session_id: Option<String>) -> Vec<Value> {
    state
        .pending_permissions()
        .into_iter()
        .filter(|ask| match &session_id {
            // A subagent runs as `parent/task-3`, and there is only one person
            // to ask, so its cards belong to the parent conversation.
            Some(wanted) => root_session(&ask.session_id) == *wanted,
            None => true,
        })
        .map(|ask| serde_json::to_value(ask).unwrap_or(Value::Null))
        .collect()
}

fn root_session(session: &str) -> &str {
    session.split('/').next().unwrap_or(session)
}

/// Allows everything, for listing rather than running.
///
/// Not a shortcut past the permission engine: nothing calls a tool through this
/// gate. It exists so `specs()` can report the full catalogue, which is what a
/// settings screen showing "what is installed" needs.
#[derive(Debug)]
struct CatalogueGate;

#[async_trait::async_trait]
impl inertia_core::tool::PermissionGate for CatalogueGate {
    async fn ask(
        &self,
        _request: &inertia_core::tool::PermissionRequest,
    ) -> inertia_core::Result<inertia_core::tool::Decision> {
        Ok(inertia_core::tool::Decision::Allow)
    }

    async fn verdict(&self, _key: &str, _target: &str) -> inertia_core::Action {
        inertia_core::Action::Allow
    }
}

#[cfg(test)]
mod tests {
    /// `translate` with no window, which is what most of these are about.
    fn translate_for_test(event: &super::AgentEvent) -> Vec<super::Value> {
        super::translate(event, None)
    }

    use super::*;
    use inertia_core::id::ToolCallId;
    use inertia_core::message::ToolCall;
    use inertia_core::provider::Usage;
    use inertia_core::tool::ToolResult;

    fn types(events: &[Value]) -> Vec<&str> {
        events.iter().filter_map(|e| e["type"].as_str()).collect()
    }

    /// The screenshot this reproduces: a Group conversation whose agent reads
    /// "call `handover`" in its own prompt and then reports that no such tool
    /// is in its list. Built the way `agent_run` builds it - a room opened on
    /// the conversation, the group tools made for the seat, the mode's withheld
    /// list applied - and asserted against the specs the model is handed.
    #[tokio::test]
    async fn a_group_turn_hands_the_seated_agent_the_three_room_tools() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let workspace = Arc::new(
            crate::state::Workspace::open(dir.path().to_path_buf()).expect("a workspace"),
        );

        for record in [
            json!({ "id": "agent-inertia-dev", "name": "Inertia Dev" }),
            json!({ "id": "agent-physicist", "name": "Physicist" }),
        ] {
            inertia_store::collections::put(
                &workspace.layout,
                inertia_store::Collection::Agents,
                record,
            )
            .expect("the agent was written");
        }

        // The permissions as the real workspace has them.
        let permissions = crate::group::Permissions::from_settings(&json!({
            "permissions": {
                "canInvite": true, "canHandover": true, "canLeave": true,
                "maxAgents": 10, "maxHops": 20
            }
        }));
        let rooms = std::sync::Arc::new(crate::group::Rooms::default());
        rooms.open("thread-1", Some("agent-inertia-dev"), permissions.clone(), &[]);
        let (speaker, _roster, permissions) =
            rooms.seated("thread-1").expect("the room was seated");
        assert_eq!(speaker.as_deref(), Some("agent-inertia-dev"));

        let extra = crate::group_tools::tools_for(
            crate::group_tools::Seat {
                rooms,
                layout: workspace.layout.clone(),
                conversation: "thread-1".to_string(),
                agent_id: speaker,
            },
            &permissions,
        );
        let mode = inertia_agent::prompt::Mode::parse("group");
        let gate: Arc<dyn inertia_core::tool::PermissionGate> =
            Arc::new(inertia_mock::MockGate::allow_all());
        let registry =
            crate::state::registry_with(&workspace, gate, None, extra, &mode.withheld());
        let names: Vec<String> = registry
            .specs()
            .await
            .into_iter()
            .map(|spec| spec.name)
            .collect();

        for tool in ["invite", "handover", "part", "shell", "write"] {
            assert!(
                names.contains(&tool.to_string()),
                "a group turn was not handed {tool}: {names:?}"
            );
        }
        assert!(!names.contains(&"present_plan".to_string()));
    }

    #[test]
    fn the_start_event_is_not_forwarded() {
        // The window synthesises its own from the id the command returns.
        let out = translate_for_test(&AgentEvent::Start {
            model: "m".into(),
        });
        assert!(out.is_empty());
    }

    #[test]
    fn prose_and_thinking_stay_apart() {
        assert_eq!(
            types(&translate_for_test(&AgentEvent::Delta { text: "hi".into() })),
            ["delta"]
        );
        assert_eq!(
            types(&translate_for_test(&AgentEvent::Reasoning { text: "hm".into() })),
            ["reasoning"]
        );
    }

    #[test]
    fn a_tool_call_carries_the_id_its_result_will_use() {
        let call = ToolCall {
            id: ToolCallId::from_existing("c1".to_string()),
            name: "read".into(),
            arguments: r#"{"path":"a.txt"}"#.into(),
        };
        let out = translate_for_test(&AgentEvent::ToolStarted {
            call,
            title: Some("Read a.txt".into()),
        });
        assert_eq!(out[0]["type"], "tool-start");
        assert_eq!(out[0]["callId"], "c1");
        // Parsed, because the card renders named fields.
        assert_eq!(out[0]["args"]["path"], "a.txt");
    }

    /// Arguments a model sent as a bare `""` must not take the card down.
    #[test]
    fn unparsable_arguments_become_an_empty_object() {
        let call = ToolCall {
            id: ToolCallId::from_existing("c1".to_string()),
            name: "read".into(),
            arguments: String::new(),
        };
        let out = translate_for_test(&AgentEvent::ToolStarted { call, title: None });
        assert_eq!(out[0]["args"], json!({}));
    }

    #[test]
    fn a_refused_call_is_marked_so_the_window_can_draw_it_as_blocked() {
        let mut result = ToolResult::failed(
            ToolCallId::from_existing("c1".to_string()),
            "read",
            "That call was refused.",
        );
        result.title = Some("Read".into());
        result.metadata = Some(json!({ "error": inertia_core::tool::ERROR_DENIED }));
        let out = translate_for_test(&AgentEvent::ToolFinished { result });
        assert_eq!(out[0]["type"], "tool-end");
        assert_eq!(out[0]["ok"], false);
        assert_eq!(out[0]["metadata"]["error"], "denied");
    }

    /// The window reads `context` off the usage event to draw the gauge in the
    /// chat header and to decide when a conversation has to be summarised.
    /// Nothing sent it for the life of this shell, so neither ever happened:
    /// a long conversation ran until the provider refused it.
    #[test]
    fn the_usage_event_says_how_full_the_window_is() {
        let usage = inertia_core::provider::Usage {
            input_tokens: 900,
            output_tokens: 100,
            cache_read_tokens: 100,
            ..Default::default()
        };
        let out = super::translate(
            &AgentEvent::Done {
                stopped: StopReason::Complete,
                history: vec![],
                usage: Some(usage),
            },
            Some(200_000),
        );

        assert_eq!(out[0]["type"], "usage");
        assert_eq!(out[0]["context"]["window"], 200_000);
        assert_eq!(
            out[0]["context"]["used"], 1100,
            "every input token including the cached ones, plus what it wrote"
        );
    }

    /// A gauge measured against a guessed denominator is worse than no gauge,
    /// and summarising on one would throw a conversation away for no reason.
    #[test]
    fn a_model_with_no_published_window_reports_none() {
        let out = super::translate(
            &AgentEvent::Done {
                stopped: StopReason::Complete,
                history: vec![],
                usage: Some(inertia_core::provider::Usage::default()),
            },
            None,
        );
        assert!(out[0]["context"].is_null());
    }

    /// The error puts the sentence on screen and settles the running cards; the
    /// done releases the composer. One without the other leaves the thread
    /// believing it is still working.
    #[test]
    fn a_failed_turn_reports_both_an_error_and_a_done() {
        let out = translate_for_test(&AgentEvent::Done {
            stopped: StopReason::Error {
                message: "the provider refused".into(),
            },
            history: vec![],
            usage: None,
        });
        assert_eq!(types(&out), ["error", "done"]);
        assert_eq!(out[0]["message"], "the provider refused");
        assert_eq!(out[1]["stopped"], "error");
    }

    #[test]
    fn an_ordinary_ending_is_just_a_done() {
        let out = translate_for_test(&AgentEvent::Done {
            stopped: StopReason::Complete,
            history: vec![],
            usage: None,
        });
        assert_eq!(types(&out), ["done"]);
        assert_eq!(out[0]["stopped"], "complete");
    }

    /// The header's context gauge reads this, and it must arrive before the
    /// `done` that closes the turn.
    #[test]
    fn usage_is_reported_before_the_turn_closes() {
        let out = translate_for_test(&AgentEvent::Done {
            stopped: StopReason::Complete,
            history: vec![],
            usage: Some(Usage {
                input_tokens: 100,
                output_tokens: 20,
                ..Default::default()
            }),
        });
        assert_eq!(types(&out), ["usage", "done"]);
        assert_eq!(out[0]["usage"]["input"], 100);
        assert_eq!(out[1]["usage"]["output"], 20);
    }

    /// Cancellation is the one ending the reducer branches on by name.
    #[test]
    fn cancellation_is_named_so_cards_settle_with_the_right_sentence() {
        let out = translate_for_test(&AgentEvent::Done {
            stopped: StopReason::Cancelled,
            history: vec![],
            usage: None,
        });
        assert_eq!(out[0]["stopped"], "cancelled");
    }

    #[test]
    fn a_notice_is_a_warning_rather_than_a_failure() {
        let out = translate_for_test(&AgentEvent::Notice {
            message: "retrying in 2s".into(),
        });
        assert_eq!(out[0]["type"], "warning");
        assert_eq!(out[0]["kind"], "info");
    }

    /// The renderer sends fields this build does not act on. Accepting them is
    /// what keeps a newer window working against an older backend.
    #[test]
    fn a_request_with_group_fields_still_parses() {
        let request: RunRequest = serde_json::from_value(json!({
            "threadId": "t1",
            "agentId": "a1",
            "modelRef": "anthropic/claude-sonnet-4",
            "history": [{ "role": "user", "content": "hi" }],
            "conversationMode": "group",
            "primaryAgentId": "a1",
            "mentioned": ["a2"],
            "continuation": true,
            "roster": ["a1", "a2"],
            "speaker": "a2",
            "somethingFromTheFuture": 1,
        }))
        .unwrap();
        assert_eq!(request.thread_id, "t1");
        assert_eq!(request.roster, ["a1", "a2"]);
        assert_eq!(request.history.len(), 1);
    }

    /// The window's own `toHistory` emits exactly this shape.
    #[test]
    fn a_folded_transcript_deserialises_into_entries() {
        let history: Vec<Entry> = serde_json::from_value(json!([
            { "role": "user", "content": "read a.txt" },
            {
                "role": "assistant",
                "content": "Looking.",
                "toolCalls": [{ "id": "c1", "name": "read", "arguments": "{}" }],
                "agentId": "a1",
                "name": "Dev",
            },
            { "role": "tool", "toolCallId": "c1", "content": "hello" },
        ]))
        .unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[2].text(), "hello");
    }
}

/* -- what an agent is allowed, and what has been granted ----------------- */

/// The effective rules for one agent: the workspace's, with its own on top.
///
/// The merge happens in the store rather than here, so this and the gate that
/// actually enforces the rules cannot disagree about precedence.
#[tauri::command]
pub fn agent_rules(state: State<'_, AppState>, agent_id: Option<String>) -> Result<Vec<Value>, String> {
    let workspace = state.workspace()?;
    Ok(workspace
        .settings
        .permissions()
        .for_agent(agent_id.as_deref())
        .into_iter()
        .map(|rule| json!({ "tool": rule.tool, "pattern": rule.pattern, "action": rule.action }))
        .collect())
}

/// Every "always allow" the user has granted.
///
/// Read back out of the rules rather than kept in memory, because that is where
/// an "always" goes: the gate writes one as a workspace rule so it survives a
/// restart. A list assembled from process state would forget them all on
/// relaunch and show an empty screen to someone who had granted a dozen.
#[tauri::command]
pub fn permission_grants(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let workspace = state.workspace()?;
    let permissions = workspace.settings.permissions();

    let mut grants: Vec<Value> = Vec::new();
    let mut push = |scope: &str, rules: &[inertia_core::permission::Rule]| {
        for rule in rules {
            if rule.action == inertia_core::Action::Allow {
                grants.push(json!({
                    "sessionId": scope,
                    "tool": rule.tool,
                    "pattern": rule.pattern,
                }));
            }
        }
    };

    push("workspace", &permissions.workspace);
    for (agent, rules) in &permissions.agents {
        push(agent, rules);
    }
    Ok(grants)
}

/// Takes an "always allow" back.
///
/// `session_id` names which list it lives in: the workspace's, or one agent's.
/// Removing from the wrong list would leave the grant in force and report that
/// it had gone, which is the worst possible outcome for a permission screen.
#[tauri::command]
pub fn permission_revoke(
    state: State<'_, AppState>,
    session_id: Option<String>,
    tool: String,
    pattern: Option<String>,
) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let mut permissions = workspace.settings.permissions();
    let pattern = pattern.unwrap_or_else(|| inertia_core::permission::ANY.to_string());

    let matches = |rule: &inertia_core::permission::Rule| {
        rule.tool == tool && rule.pattern == pattern && rule.action == inertia_core::Action::Allow
    };

    let scope = session_id.unwrap_or_else(|| "workspace".to_string());
    let removed = if scope == "workspace" {
        let before = permissions.workspace.len();
        permissions.workspace.retain(|r| !matches(r));
        before - permissions.workspace.len()
    } else if let Some(rules) = permissions.agents.get_mut(&scope) {
        let before = rules.len();
        rules.retain(|r| !matches(r));
        before - rules.len()
    } else {
        0
    };

    workspace
        .settings
        .save_permissions(&permissions)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "revoked": removed }))
}

/// Opens a file or folder the agent mentioned, in whatever the OS uses for it.
#[tauri::command]
pub fn agent_open_path(app: AppHandle, target: String) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let path = std::path::PathBuf::from(&target);
    if !path.exists() {
        return Err(format!("There is nothing at {target} any more."));
    }
    app.opener()
        .open_path(target.clone(), None::<&str>)
        .map_err(|e| e.to_string())?;
    Ok(target)
}

/// Drops cached tool lists, so the next turn rebuilds them.
///
/// Called after a write that went around a bridge handler - editing an MCP
/// record through the workspace surface, say. Without it an agent keeps calling
/// the old configuration until the app restarts.
#[tauri::command]
pub fn tools_invalidate(state: State<'_, AppState>) -> Result<Value, String> {
    let workspace = state.workspace()?;
    workspace.composio.drop_cached_tools();
    Ok(json!({ "invalidated": true }))
}

/// Forgets whatever was cached for one conversation.
///
/// The set of files it has read, which is what stops the agent re-reading a
/// file it already has, and its room - who was in the conversation and whose
/// turn it was. A conversation that has been deleted must not keep either alive
/// for the id's next owner: a new thread that inherited a roster would open
/// with four agents nobody invited.
#[tauri::command]
pub fn agent_forget(state: State<'_, AppState>, session_id: String) -> Result<Value, String> {
    let workspace = state.workspace()?;
    workspace.reads.forget(&session_id);
    workspace.lists.forget(&session_id);
    state.tasks.forget(&session_id);
    state.rooms.forget(&session_id);
    // Anything this conversation still has running, stopped rather than left
    // to finish work nobody will ever read.
    state.runs.cancel_conversation(&session_id, "The conversation was deleted.");
    // And any question it was holding the turn open for: the card is gone with
    // the conversation, so nothing can ever answer it.
    state.questions.abandon_session(&session_id);
    // And the helper runs it started. A deleted conversation's runs have
    // nobody left to collect them, and leaving them in the table means the
    // next thread to inherit the id opens with somebody else's work in it.
    state.runs.forget(&session_id);
    // And the turn records it accumulated. The workspace copy is the durable
    // one; these are the in-memory heads, and keeping them means the next
    // thread to be handed this id opens holding a deleted conversation's turns.
    crate::records::Recorder::global().forget(&session_id);
    Ok(json!({ "forgotten": true }))
}

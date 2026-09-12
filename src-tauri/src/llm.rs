//! Talking to a provider directly, outside a turn.
//!
//! Three questions the settings screen asks and the agent loop never does: what
//! providers are configured, what models does this endpoint offer, and does
//! this endpoint work at all. They take a provider *record* rather than an id,
//! because the dialog asks them about a draft the user is still typing - one
//! that has not been saved and may never be.
//!
//! The key is the one thing not taken from the record. A record carries the
//! *name* of a secret, never the secret, so it is resolved here against the
//! workspace. That is also why these run in the backend at all: a browser
//! cannot reach a provider directly, and the key would have to live in the
//! page.
//!
//! And the fourth thing, which is not a question: one completion, no tools, no
//! turn. That is what `/compact` and the thread-titler are made of - a model
//! call the agent loop has no part in - and it is why this module streams as
//! well as answers.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use futures::StreamExt;
use inertia_core::message::{Entry, ToolCall, UserEntry};
use inertia_core::provider::{ChatRequest, StreamEvent, Usage};
use inertia_core::Provider;
use inertia_provider::{AnthropicProvider, OpenAiProvider, ProviderConfig};
use inertia_store::settings::{Protocol, ProviderRecord};
use inertia_store::Settings;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use crate::coalesce::Coalescer;
use crate::state::AppState;

/// Builds a live provider from a record the window handed over.
///
/// `base` is forgotten first: this is the Test button, and the whole point of
/// pressing it is that something changed. A remembered base from before the
/// edit would answer for a URL the user has just stopped using.
fn provider_from(
    state: &AppState,
    record: &ProviderRecord,
    relearn: bool,
) -> Result<Arc<dyn Provider>, String> {
    if record.base_url.trim().is_empty() {
        return Err("This provider has no base URL.".into());
    }
    if inertia_provider::base::candidates(&record.base_url).is_empty() {
        return Err(format!(
            "`{}` is not a URL this app can use. It should look like https://api.example.com/v1.",
            record.base_url
        ));
    }
    if relearn {
        inertia_provider::base::forget(Some(&record.base_url));
    }

    let workspace = state.workspace()?;
    let mut config = ProviderConfig::new(
        record.id.clone(),
        record.base_url.clone(),
        workspace.settings.api_key_for(record),
    );
    for (name, value) in &record.headers {
        config = config.with_header(name, value);
    }

    Ok(match record.protocol() {
        Protocol::Anthropic => Arc::new(AnthropicProvider::new(config)),
        Protocol::OpenAi => Arc::new(OpenAiProvider::new(config)),
    })
}

fn protocol_name(record: &ProviderRecord) -> &'static str {
    match record.protocol() {
        Protocol::Anthropic => "anthropic",
        Protocol::OpenAi => "openai",
    }
}

/// Every configured provider, and which model is the default.
#[tauri::command]
pub fn llm_providers(state: State<'_, AppState>) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let models = workspace.settings.models();
    Ok(json!({
        "providers": models.providers,
        "defaultModel": models.default_model,
    }))
}

/// `GET /models`, as rows the picker can render.
///
/// An endpoint with no model list is a normal outcome, not a failure: plenty of
/// OpenAI-compatible servers do not implement the route, and the user can still
/// type a model id by hand. So this answers with an empty list rather than an
/// error, and only a genuinely unreachable endpoint throws.
#[tauri::command]
pub async fn llm_models(
    state: State<'_, AppState>,
    provider: ProviderRecord,
) -> Result<Vec<Value>, String> {
    let client = provider_from(&state, &provider, false)?;
    let models = client.list_models().await.map_err(|e| e.to_string())?;
    Ok(models
        .into_iter()
        .map(|model| json!({ "id": model.id, "label": model.label }))
        .collect())
}

/// One cheap round trip: does it resolve, speak the protocol, accept the key.
///
/// The model list is the probe. It is the only route both protocols agree on
/// that costs nothing and has no side effects - a chat completion would spend
/// the user's money to answer a question about plumbing.
#[tauri::command]
pub async fn llm_test(
    state: State<'_, AppState>,
    provider: ProviderRecord,
) -> Result<Value, String> {
    let client = provider_from(&state, &provider, true)?;

    let started = std::time::Instant::now();
    let models = client.list_models().await.map_err(|e| e.to_string())?;
    let latency = started.elapsed().as_millis() as u64;

    // Which base actually answered. When it is not the first candidate the user
    // pasted something that needed interpreting, and saying so is the
    // difference between "it works" and "it works, and here is why it did not
    // a moment ago".
    let settled = inertia_provider::base::base_of(&provider.base_url);
    let candidates = inertia_provider::base::candidates(&provider.base_url);
    let interpreted = candidates.first().is_some_and(|first| first != &settled)
        || settled.trim_end_matches('/') != provider.base_url.trim().trim_end_matches('/');

    let note = if models.is_empty() {
        format!("Reached {settled}, but it lists no models. Type a model id by hand.")
    } else if interpreted {
        format!("Reached {settled}.")
    } else {
        String::new()
    };

    Ok(json!({
        "models": models.len(),
        "latencyMs": latency,
        "note": note,
        "protocol": protocol_name(&provider),
        // The protocol was inferred from the URL rather than chosen, so the
        // dialog can offer to make it explicit.
        "detected": provider.protocol.is_none(),
    }))
}

/* -- one completion, which is not a turn --------------------------------- */

/// The one channel every stream reports on.
///
/// Tagged with the stream's id rather than given a channel per stream: a window
/// with a reply streaming and a title being generated at the same time has two
/// of these in flight, and one subscription that routes on an id cannot miss
/// the events of a stream it had not subscribed to yet.
pub const LLM_EVENT: &str = "llm:event";

/// Where a one-shot call gets its model from.
///
/// A seam rather than a direct call to [`crate::state::provider_for`], because
/// everything interesting about these two commands - what request was
/// assembled, what the stream does, what the summariser was asked - is only
/// testable if the provider can be a fake. The app's implementation is three
/// lines below it.
pub trait Models: Send + Sync {
    /// The provider a `provider/model` reference names, and the model id to ask
    /// it for.
    fn resolve(&self, reference: &str) -> Result<(Arc<dyn Provider>, String), String>;

    /// The workspace's default model, for a caller that named none.
    fn default_model(&self) -> String;
}

/// The models as the open workspace has them configured.
struct Configured {
    settings: Arc<Settings>,
}

impl Models for Configured {
    fn resolve(&self, reference: &str) -> Result<(Arc<dyn Provider>, String), String> {
        crate::state::provider_for(&self.settings, reference)
    }

    fn default_model(&self) -> String {
        self.settings.models().default_model
    }
}

/// Where a stream's events go.
///
/// The same seam for the same reason: an `AppHandle` cannot be built in a test
/// (and building app state in one takes the whole test binary down on Windows),
/// so the thing that emits is a trait and the tests collect into a `Vec`.
pub trait Events: Send + Sync + 'static {
    fn emit(&self, event: Value);
}

/// The window, when there is one.
struct Window(AppHandle);

impl Events for Window {
    fn emit(&self, event: Value) {
        // A window that closed mid-stream is an ordinary event, not an error.
        let _ = self.0.emit(LLM_EVENT, event);
    }
}

/// The streams in flight, so a cancel can find the one it means.
///
/// A stream is the one long-lived thing on this bridge. The window starts one
/// and is handed an id; the reply arrives as events carrying that id; the
/// window stops it by id. Modelling it as a request and a response would mean
/// either buffering the whole reply - no streaming - or holding the bridge open
/// for minutes with no way to interrupt it.
#[derive(Debug, Default)]
pub struct Streams {
    /// Dropping a sender is what the stream's task is watching for.
    running: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>,
    next: Mutex<u64>,
}

impl Streams {
    /// The next id. Sequential and process-local, exactly as the other shell's
    /// were: it is a handle, and nothing outside this process ever sees it.
    fn mint(&self) -> String {
        let mut next = self.next.lock();
        *next += 1;
        format!("stream-{next}")
    }

    fn register(&self, id: String, cancel: tokio::sync::oneshot::Sender<()>) {
        self.running.lock().insert(id, cancel);
    }

    fn finished(&self, id: &str) {
        self.running.lock().remove(id);
    }

    /// Stops one stream. `false` means it had already ended, which is not a
    /// failure: the window can press stop on a reply that finished while the
    /// click was crossing the bridge.
    pub fn cancel(&self, id: &str) -> bool {
        self.running.lock().remove(id).is_some()
    }

    /// Stops everything, and says how many that was. Used when the window goes
    /// away mid-stream.
    pub fn cancel_all(&self) -> usize {
        let mut running = self.running.lock();
        let count = running.len();
        running.clear();
        count
    }
}

/// The app's streams.
///
/// Process state rather than something on `AppState`, because that is all it
/// is: a table of what is running right now, owned by the only module that
/// starts anything in it.
fn streams() -> Arc<Streams> {
    static STREAMS: LazyLock<Arc<Streams>> = LazyLock::new(|| Arc::new(Streams::default()));
    STREAMS.clone()
}

/// What the window asks for when it starts a one-shot completion.
///
/// The window builds these messages itself - it has never had a transcript on
/// this path - and builds them in the OpenAI chat-completions shape, which is
/// the shape the other shell's bridge took. They are turned back into the app's
/// own entries below rather than passed through, because each provider now
/// builds its own wire format and only one of the two would recognise these.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatCall {
    pub provider_id: String,
    pub model: String,
    pub messages: Vec<WireMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f32>,
}

/// One message as the window wrote it. Field names are the wire's, not the
/// app's: `tool_call_id`, not `toolCallId`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WireMessage {
    pub role: String,
    pub content: Option<Value>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Vec<WireToolCall>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WireToolCall {
    pub id: String,
    pub function: WireFunction,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WireFunction {
    pub name: String,
    pub arguments: String,
}

/// The window's messages, as the app's own system prompt and history.
fn from_request_messages(messages: Vec<WireMessage>) -> (String, Vec<Entry>) {
    let mut system = String::new();
    let mut history = Vec::new();

    for message in messages {
        let text = || {
            message
                .content
                .as_ref()
                .map(|value| match value {
                    Value::String(said) => said.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .unwrap_or_default()
        };

        match message.role.as_str() {
            // Several system messages are legal here and mean one thing
            // downstream.
            "system" => {
                let said = text();
                system = if system.is_empty() {
                    said
                } else {
                    format!("{system}\n\n{said}")
                };
            }

            "tool" => {
                history.push(Entry::Tool(inertia_core::message::ToolEntry {
                    tool_call_id: inertia_core::id::ToolCallId::from_existing(
                        message.tool_call_id.clone().unwrap_or_default(),
                    ),
                    content: text(),
                    // Absent rather than `true`: this path has no idea whether
                    // the call succeeded, and claiming it did would relabel a
                    // failure the window is showing as an error.
                    ok: None,
                    pruned: false,
                }));
            }

            "assistant" | "agent" => {
                history.push(Entry::Assistant(inertia_core::message::AssistantEntry {
                    content: Some(text()),
                    tool_calls: message
                        .tool_calls
                        .into_iter()
                        .map(|call| ToolCall {
                            id: inertia_core::id::ToolCallId::from_existing(call.id),
                            name: call.function.name,
                            arguments: call.function.arguments,
                        })
                        .collect(),
                    ..Default::default()
                }));
            }

            // A message carrying pictures arrives as content parts. Kept as
            // parts rather than flattened to their text, because a picture the
            // window bothered to attach is the thing it is asking about.
            _ => match message.content {
                Some(Value::Array(_)) => {
                    let parts = message
                        .content
                        .clone()
                        .and_then(|value| {
                            serde_json::from_value::<Vec<inertia_core::message::Part>>(value).ok()
                        })
                        .unwrap_or_default();
                    history.push(Entry::User(UserEntry {
                        parts: Some(parts),
                        ..Default::default()
                    }));
                }
                _ => history.push(Entry::user(text())),
            },
        }
    }

    (system, history)
}

/// The request that will actually be sent.
///
/// No tools, on purpose and by construction: this is the call that is not a
/// turn, and a summariser or a title generator that started reading files would
/// be a bill nobody asked for.
fn assemble(model: String, call: ChatCall) -> ChatRequest {
    let (system, history) = from_request_messages(call.messages);
    ChatRequest {
        model,
        system,
        history,
        tools: Vec::new(),
        temperature: call.temperature,
        max_tokens: call.max_tokens,
        top_p: call.top_p,
        ..Default::default()
    }
}

/// Token counts in the window's names.
///
/// The same names a turn reports, because the same message bubble reads them:
/// `input` is every input token including the cached ones, with the cache
/// figures beside it so a cost can still be worked out.
fn tokens(usage: &Usage) -> Value {
    json!({
        "input": usage.total_input(),
        "output": usage.output_tokens,
        "cachedInput": usage.cache_read_tokens,
        "cacheWrite": usage.cache_write_tokens,
    })
}

/// One provider event, as the event the window listens for.
///
/// Mostly the event itself - the tags already match what `runChat` branches on.
/// The one rewrite is usage, which the window reads as `input`/`output` and
/// every provider reports in its own names.
fn translate(event: StreamEvent) -> Value {
    if let StreamEvent::Done { finish, usage } = &event {
        return json!({
            "type": "done",
            "finish": finish,
            "usage": usage.as_ref().map(tokens),
        });
    }
    serde_json::to_value(&event).unwrap_or_else(|_| json!({ "type": "error", "message": "That reply could not be read." }))
}

/// Every event carries the id of the stream it belongs to.
fn tagged(id: &str, mut event: Value) -> Value {
    if let Value::Object(map) = &mut event {
        map.insert("id".into(), json!(id));
    }
    event
}

/// Runs the stream to its end, or until it is cancelled.
async fn pump(
    provider: Arc<dyn Provider>,
    request: ChatRequest,
    id: String,
    events: Arc<dyn Events>,
    streams: Arc<Streams>,
    cancelled: tokio::sync::oneshot::Receiver<()>,
) {
    let mut stream = provider.stream_chat(request);

    // Text is batched before it crosses the bridge, exactly as a turn's is. One
    // event per token is one JSON serialisation and one script evaluation in
    // the webview per token, which is the difference between a reply that
    // scrolls and one that stutters.
    let mut buffer = Coalescer::default();

    // `select!` rather than a flag read inside the loop: dropping the stream is
    // what cancels the provider request, and this is where the drop happens.
    let stop = async move {
        let _ = cancelled.await;
    };
    tokio::pin!(stop);

    let mut ended = false;
    loop {
        // Read before the `select!`: the arms are evaluated together, and a
        // deadline read after the buffer has taken this iteration's token would
        // be a tick late.
        let due = buffer.due();

        tokio::select! {
            biased;

            _ = &mut stop => break,

            // Armed only while something is held back. `pending` is what makes
            // an idle stream wait on the provider alone rather than waking
            // sixteen times a second to find an empty buffer.
            _ = async {
                match due {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                for out in buffer.flush() {
                    events.emit(tagged(&id, out));
                }
            }

            next = stream.next() => {
                let Some(event) = next else { break };
                let terminal = event.is_terminal();
                for out in buffer.push(translate(event)) {
                    events.emit(tagged(&id, out));
                }
                if terminal {
                    ended = true;
                    break;
                }
            }
        }
    }

    // Whatever was still held when the loop left. A cancelled stream is not
    // coming back for its tail, and half a sentence dropped on the floor reads
    // as a bug in the model rather than in the shell.
    for out in buffer.flush() {
        events.emit(tagged(&id, out));
    }

    // A stream that was stopped, or whose events simply ran out, still owes the
    // window a terminal event. `runChat` unsubscribes on `done` or `error` and
    // on nothing else, so without this the caller's callbacks are never
    // released - a title generator would wait forever and the composer would
    // offer Stop on a reply that had finished.
    if !ended {
        events.emit(tagged(
            &id,
            json!({ "type": "done", "finish": "cancelled", "usage": Value::Null }),
        ));
    }

    streams.finished(&id);
}

/// Starts a stream: what the window is told, and the work to spawn.
///
/// Split in two so a test can await the work rather than race a spawned task,
/// and so the id is registered before anything is emitted - the window can
/// press stop before the handler has answered, and a cancel for an id that is
/// not in the table yet would be lost.
fn begin(
    models: &dyn Models,
    events: Arc<dyn Events>,
    streams: Arc<Streams>,
    call: ChatCall,
) -> Result<(Value, BoxFuture<'static, ()>), String> {
    if call.model.trim().is_empty() {
        return Err("No model was chosen.".into());
    }
    if call.provider_id.trim().is_empty() {
        return Err("No provider was named.".into());
    }

    // A model id can itself contain a slash on a gateway (`openai/gpt-4o`), and
    // a reference splits on the first one only - so this round trips.
    let reference = format!("{}/{}", call.provider_id.trim(), call.model.trim());
    let (provider, model) = models.resolve(&reference)?;

    let provider_id = call.provider_id.trim().to_string();
    let model_ref = call.model.trim().to_string();
    let request = assemble(model, call);

    let id = streams.mint();
    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    streams.register(id.clone(), cancel);

    let started = json!({ "id": id, "provider": provider_id, "model": model_ref });
    let work = pump(provider, request, id, events, streams.clone(), cancelled);

    Ok((started, Box::pin(work)))
}

/// One completion, no tools, no turn.
///
/// Answers with the stream's id immediately; the reply arrives as `llm:event`
/// messages tagged with it. The window decides what a message is - this only
/// knows how to turn a request into text.
#[tauri::command]
pub async fn llm_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ChatCall,
) -> Result<Value, String> {
    let workspace = state.workspace()?;
    let models = Configured {
        settings: workspace.settings.clone(),
    };
    let events: Arc<dyn Events> = Arc::new(Window(app.clone()));

    let (started, work) = begin(&models, events, streams(), request)?;
    // Deliberately not awaited: this command's job is to hand back an id, and
    // the stream outlives the call by design.
    tauri::async_runtime::spawn(work);
    Ok(started)
}

#[tauri::command]
pub fn llm_cancel(id: String) -> Value {
    json!({ "cancelled": streams().cancel(&id) })
}

#[tauri::command]
pub fn llm_cancel_all() -> Value {
    json!({ "cancelled": streams().cancel_all() })
}

/* -- summarising a conversation, because somebody asked ------------------ */

/// What the summariser is asked for.
///
/// A handoff, in the shape Codex and opencode both settled on: not "what
/// happened" but "what the next person needs". The headings are fixed so the
/// note is skimmable and so nothing important can be left out for want of a
/// place to put it - a summary with no "open" section quietly becomes a summary
/// in which nothing is open.
pub const SUMMARY_INSTRUCTION: &str = concat!(
    "You are writing the handoff note for the earlier part of a working session,\n",
    "so the work can continue in a smaller context. Write for the agent picking\n",
    "it up, who has the recent messages but not these. Dense, factual, no\n",
    "preamble, no narrative. Use exactly these headings, and write \"none\" under\n",
    "a heading rather than leaving it out:\n",
    "\n",
    "## Goal\n",
    "What the user asked for, in their terms, and the outcome that would count as done.\n",
    "\n",
    "## Constraints and preferences\n",
    "Rules the user gave, conventions of the project, things they said not to do,\n",
    "how they want to be talked to.\n",
    "\n",
    "## Done\n",
    "What has been completed and verified, with the files and paths touched.\n",
    "\n",
    "## Decisions\n",
    "Choices already made and why, so they are not reopened.\n",
    "\n",
    "## Open\n",
    "What remains, what was tried and failed, the next concrete step.\n",
    "\n",
    "## Worth keeping\n",
    "Facts that were expensive to find: commands that work, errors seen and their\n",
    "causes, values, paths, names. Not tool output; what the output taught.",
);

/// How much of any single old message goes into the digest we ask about.
const MAX_DIGEST_CHARS_PER_ENTRY: usize = 1500;

/// What the window asks for when somebody types `/compact`.
///
/// `history` is the transcript as the window folded it, and it stays
/// `serde_json::Value`: this reads four fields off each entry and writes none
/// of them back, and a typed mirror would drop whatever the window has learned
/// to send since.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SummarizeCall {
    pub model_ref: Option<String>,
    pub history: Vec<Value>,
    pub focus: Option<String>,
}

/// The dropped part of the conversation, written out for something to read.
fn digest(entries: &[Value]) -> String {
    let mut lines = Vec::new();

    for entry in entries {
        let role = match entry.get("role").and_then(Value::as_str) {
            Some("tool") => "tool result",
            // `agent` is what older transcripts wrote, and it never meant
            // anything different.
            Some("agent") => "assistant",
            Some(other) => other,
            None => "user",
        };

        let said = match entry.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => {
                if entry.get("parts").is_some_and(Value::is_array) {
                    "[an image]".to_string()
                } else {
                    String::new()
                }
            }
            Some(other) => other.to_string(),
        };

        // By characters, not bytes: a cut inside a multi-byte character would
        // panic, and every transcript in this app has some.
        let body: String = said.chars().take(MAX_DIGEST_CHARS_PER_ENTRY).collect();

        let asked: Vec<&str> = entry
            .get("toolCalls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|call| call.get("name").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();

        lines.push(if asked.is_empty() {
            format!("{role}: {body}")
        } else {
            format!("{role}: {body}\n[called: {}]", asked.join(", "))
        });
    }

    lines.join("\n\n")
}

/// The instruction, with whatever the person typed after the command.
///
/// Read as a request about emphasis and nothing else: it is appended to the
/// instruction rather than replacing it, so "keep the SQL" adds a priority and
/// cannot turn the summariser into a general-purpose prompt.
fn instruction(focus: &str) -> String {
    if focus.is_empty() {
        return SUMMARY_INSTRUCTION.to_string();
    }
    format!(
        "{SUMMARY_INSTRUCTION}\n\nThe person asked that the note pay particular attention to this:\n{focus}"
    )
}

/// Summarise a conversation, on purpose, because somebody asked.
///
/// This is the whole of compaction in this shell, and it is asked for twice:
/// by `/compact` in the composer, and by the window when the context gauge
/// crosses its threshold. The agent loop itself does not compact - an earlier
/// comment here said it did, describing a design that was never ported, and
/// that sentence is why nobody noticed the window's automatic trigger had
/// nothing feeding it.
///
/// One call, no tools, the same model the conversation is using. Awaited rather
/// than streamed, because the caller wants a note and not a performance - and
/// nothing here reads or writes the transcript, which keeps the split the
/// window's own compaction is built on.
async fn summarise(models: &dyn Models, call: SummarizeCall) -> Result<Value, String> {
    if call.history.is_empty() {
        return Err("There is nothing to summarise yet.".into());
    }

    let reference = call
        .model_ref
        .as_deref()
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| models.default_model());
    let (provider, model) = models.resolve(&reference)?;

    let focus = call.focus.as_deref().unwrap_or_default().trim().to_string();

    let request = ChatRequest {
        model,
        system: instruction(&focus),
        history: vec![Entry::user(digest(&call.history))],
        // Zero: the same conversation summarised twice should not produce two
        // different accounts of what was decided.
        temperature: Some(0.0),
        ..Default::default()
    };

    let mut stream = provider.stream_chat(request);
    let mut summary = String::new();
    let mut usage = None;

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Delta { text } => summary.push_str(&text),
            StreamEvent::Done { usage: reported, .. } => usage = reported,
            // The failure is what the person needs to read, not a shorter
            // sentence about summarising in general.
            StreamEvent::Error { message, .. } => return Err(message),
            _ => {}
        }
    }

    let summary = summary.trim();
    if summary.is_empty() {
        return Err("The model returned an empty summary.".into());
    }

    Ok(json!({ "summary": summary, "usage": usage.as_ref().map(tokens) }))
}

#[tauri::command]
pub async fn agent_summarize(
    state: State<'_, AppState>,
    request: SummarizeCall,
) -> Result<Value, String> {
    let models = Configured {
        settings: state.workspace()?.settings.clone(),
    };
    summarise(&models, request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(base: &str) -> ProviderRecord {
        ProviderRecord {
            id: "test".into(),
            name: "Test".into(),
            enabled: true,
            base_url: base.into(),
            api_key_secret: None,
            protocol: None,
            headers: Default::default(),
            models: Vec::new(),
        }
    }

    #[test]
    fn the_protocol_is_inferred_from_the_url_when_not_stated() {
        assert_eq!(protocol_name(&record("https://api.anthropic.com/v1")), "anthropic");
        assert_eq!(protocol_name(&record("https://api.groq.com/openai/v1")), "openai");
    }

    /// A stated protocol wins over the URL, and the dialog is told it was not a
    /// guess.
    #[test]
    fn a_stated_protocol_is_not_a_detection() {
        let mut stated = record("https://api.minimax.io/anthropic");
        stated.protocol = Some(Protocol::Anthropic);
        assert!(!stated.protocol.is_none());
        assert!(record("https://api.minimax.io/anthropic").protocol.is_none());
    }

    /* -- the call that is not a turn ------------------------------------ */

    use inertia_core::provider::FinishReason;
    use inertia_mock::MockProvider;

    /// The settings, as a script. `resolve` splits the reference the same way
    /// the real one does, so a test still proves the reference was built
    /// correctly out of `providerId` and `model`.
    #[derive(Debug)]
    struct Fake {
        provider: Arc<MockProvider>,
        default_model: String,
        asked: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(provider: MockProvider) -> Self {
            Self {
                provider: Arc::new(provider),
                default_model: "mock/mock-model".into(),
                asked: Mutex::new(Vec::new()),
            }
        }
    }

    impl Models for Fake {
        fn resolve(&self, reference: &str) -> Result<(Arc<dyn Provider>, String), String> {
            self.asked.lock().push(reference.to_string());
            let (_, model) = inertia_store::Models::split_reference(reference)
                .ok_or_else(|| format!("`{reference}` is not a valid model reference."))?;
            Ok((self.provider.clone(), model.to_string()))
        }

        fn default_model(&self) -> String {
            self.default_model.clone()
        }
    }

    /// The window, as a list.
    #[derive(Debug, Default)]
    struct Collected(Mutex<Vec<Value>>);

    impl Events for Collected {
        fn emit(&self, event: Value) {
            self.0.lock().push(event);
        }
    }

    impl Collected {
        fn seen(&self) -> Vec<Value> {
            self.0.lock().clone()
        }

        fn types(&self) -> Vec<String> {
            self.seen()
                .iter()
                .filter_map(|event| event["type"].as_str().map(str::to_string))
                .collect()
        }
    }

    fn call(messages: Vec<Value>) -> ChatCall {
        serde_json::from_value(json!({
            "providerId": "mock",
            "model": "mock-model",
            "messages": messages,
            "temperature": 0.2,
            "maxTokens": 256,
        }))
        .expect("the window's own request shape")
    }

    /// The whole request, as the provider is handed it: the system messages
    /// merged, the history in the app's own shape, and no tools - this is the
    /// call that must never start reading files.
    #[tokio::test]
    async fn the_windows_messages_become_one_request_with_no_tools() {
        let models = Fake::new(MockProvider::new().replying("ok"));
        let events = Arc::new(Collected::default());
        let streams = Arc::new(Streams::default());

        let (started, work) = begin(
            &models,
            events.clone(),
            streams.clone(),
            call(vec![
                json!({ "role": "system", "content": "be brief" }),
                json!({ "role": "system", "content": "and kind" }),
                json!({ "role": "user", "content": "name this chat" }),
                json!({ "role": "assistant", "content": "Sure." }),
            ]),
        )
        .expect("the stream started");
        work.await;

        // The window is told which stream to listen for, on which model.
        assert_eq!(started["id"], "stream-1");
        assert_eq!(started["provider"], "mock");
        assert_eq!(started["model"], "mock-model");
        // Built out of `providerId` and `model`, and split back apart the same
        // way a saved reference is.
        assert_eq!(models.asked.lock().as_slice(), ["mock/mock-model"]);

        let request = models.provider.last_request().expect("a request was sent");
        assert_eq!(request.model, "mock-model");
        assert_eq!(request.system, "be brief\n\nand kind");
        assert_eq!(request.history.len(), 2);
        assert_eq!(request.history[0].text(), "name this chat");
        assert_eq!(request.history[1].text(), "Sure.");
        assert_eq!(request.temperature, Some(0.2));
        assert_eq!(request.max_tokens, Some(256));
        assert!(request.tools.is_empty(), "a one-shot call was offered tools");
    }

    /// A tool round trip has to survive the round trip: the window rebuilds
    /// these from a transcript it already has, and a call whose answer went
    /// missing is a request most providers refuse outright.
    #[tokio::test]
    async fn a_tool_call_and_its_answer_both_survive_translation() {
        let models = Fake::new(MockProvider::new().replying("ok"));
        let streams = Arc::new(Streams::default());

        let (_, work) = begin(
            &models,
            Arc::new(Collected::default()),
            streams,
            call(vec![
                json!({ "role": "user", "content": "read a.txt" }),
                json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "c1",
                        "type": "function",
                        "function": { "name": "read", "arguments": "{\"path\":\"a.txt\"}" },
                    }],
                }),
                json!({ "role": "tool", "tool_call_id": "c1", "content": "hello" }),
            ]),
        )
        .expect("the stream started");
        work.await;

        let request = models.provider.last_request().expect("a request was sent");
        let Entry::Assistant(assistant) = &request.history[1] else {
            panic!("the assistant turn was lost: {:?}", request.history);
        };
        assert_eq!(assistant.tool_calls[0].name, "read");
        assert_eq!(assistant.tool_calls[0].parsed_arguments()["path"], "a.txt");

        let Entry::Tool(answer) = &request.history[2] else {
            panic!("the tool result was lost: {:?}", request.history);
        };
        assert_eq!(answer.tool_call_id.as_str(), "c1");
        assert_eq!(answer.content, "hello");
    }

    /// What `runChat` actually listens for: events tagged with the id it was
    /// handed, prose as `delta`, and a `done` carrying usage in the names the
    /// message bubble reads.
    #[tokio::test]
    async fn the_reply_reaches_the_window_tagged_with_the_streams_id() {
        let models = Fake::new(MockProvider::new().turn(vec![
            StreamEvent::Start { model: "mock-model".into() },
            StreamEvent::Reasoning { text: "hm".into() },
            StreamEvent::Delta { text: "A short ".into() },
            StreamEvent::Delta { text: "answer".into() },
            StreamEvent::Done {
                finish: Some(FinishReason::Stop),
                usage: Some(Usage {
                    input_tokens: 10,
                    output_tokens: 4,
                    cache_read_tokens: 6,
                    ..Default::default()
                }),
            },
        ]));
        let events = Arc::new(Collected::default());
        let streams = Arc::new(Streams::default());

        let (started, work) = begin(
            &models,
            events.clone(),
            streams.clone(),
            call(vec![json!({ "role": "user", "content": "hi" })]),
        )
        .expect("the stream started");
        work.await;

        let seen = events.seen();
        for event in &seen {
            assert_eq!(event["id"], started["id"], "an event lost its stream id");
        }

        // Deltas are coalesced, which is why this is one event rather than two.
        let delta = seen
            .iter()
            .find(|event| event["type"] == "delta")
            .expect("no prose reached the window");
        assert_eq!(delta["text"], "A short answer");

        let reasoning = seen
            .iter()
            .find(|event| event["type"] == "reasoning")
            .expect("no reasoning reached the window");
        assert_eq!(reasoning["text"], "hm");

        let done = seen.last().expect("the stream said nothing");
        assert_eq!(done["type"], "done");
        assert_eq!(done["finish"], "stop");
        // The window adds these two together for the context gauge, so cached
        // input has to count as input.
        assert_eq!(done["usage"]["input"], 16);
        assert_eq!(done["usage"]["output"], 4);
        assert_eq!(done["usage"]["cachedInput"], 6);

        // Nothing is left in the table once a stream has ended.
        assert_eq!(streams.cancel_all(), 0);
    }

    /// A failed stream reports the failure and stops. `runChat` unsubscribes on
    /// `error`, so nothing may follow it.
    #[tokio::test]
    async fn a_failure_arrives_as_an_error_the_caller_can_read() {
        let models = Fake::new(MockProvider::new().failing("the provider refused", Some(500)));
        let events = Arc::new(Collected::default());

        let (_, work) = begin(
            &models,
            events.clone(),
            Arc::new(Streams::default()),
            call(vec![json!({ "role": "user", "content": "hi" })]),
        )
        .expect("the stream started");
        work.await;

        let last = events.seen().last().cloned().expect("nothing was emitted");
        assert_eq!(last["type"], "error");
        assert_eq!(last["message"], "the provider refused");
    }

    /// The case this is here for: the window presses stop, and the caller's
    /// callbacks still have to be released. A cancelled stream that said
    /// nothing terminal would leave the title generator waiting forever and the
    /// composer offering Stop on a reply that had ended.
    #[tokio::test]
    async fn a_cancelled_stream_still_answers_with_a_done() {
        let models = Fake::new(MockProvider::new().replying("never read"));
        let events = Arc::new(Collected::default());
        let streams = Arc::new(Streams::default());

        let (started, work) = begin(
            &models,
            events.clone(),
            streams.clone(),
            call(vec![json!({ "role": "user", "content": "hi" })]),
        )
        .expect("the stream started");

        // Stopped before a single event was pumped, which is the race the
        // window loses constantly: the id exists the moment the handler answers
        // and the person can click by then.
        let id = started["id"].as_str().expect("an id").to_string();
        assert!(streams.cancel(&id), "a live stream could not be found");
        work.await;

        assert_eq!(events.types(), ["done"]);
        let done = events.seen().pop().expect("nothing was emitted");
        assert_eq!(done["finish"], "cancelled");
        assert_eq!(done["id"], started["id"]);
    }

    /// Stopping something that has already ended is an ordinary answer, not a
    /// failure: the click and the last token cross on the bridge.
    #[test]
    fn cancelling_a_stream_that_has_ended_says_so_rather_than_failing() {
        let streams = Streams::default();
        assert!(!streams.cancel("stream-404"));
        assert_eq!(streams.cancel_all(), 0);
    }

    #[tokio::test]
    async fn stopping_everything_says_how_many_that_was() {
        let models = Fake::new(MockProvider::new().replying("a").replying("b"));
        let streams = Arc::new(Streams::default());

        let (_, one) = begin(
            &models,
            Arc::new(Collected::default()),
            streams.clone(),
            call(vec![json!({ "role": "user", "content": "hi" })]),
        )
        .expect("the first stream started");
        let (_, two) = begin(
            &models,
            Arc::new(Collected::default()),
            streams.clone(),
            call(vec![json!({ "role": "user", "content": "hi" })]),
        )
        .expect("the second stream started");

        assert_eq!(streams.cancel_all(), 2);
        // Both still finish tidily, and neither is counted twice.
        one.await;
        two.await;
        assert_eq!(streams.cancel_all(), 0);
    }

    /// A model reference names one model, and a caller that names none gets the
    /// workspace's default rather than an error.
    #[test]
    fn a_model_is_required_before_a_provider_is_even_looked_up() {
        let models = Fake::new(MockProvider::new());
        let mut request = call(vec![json!({ "role": "user", "content": "hi" })]);
        request.model = "  ".into();

        let refused = begin(
            &models,
            Arc::new(Collected::default()),
            Arc::new(Streams::default()),
            request,
        )
        .err()
        .expect("a request with no model was accepted");
        assert_eq!(refused, "No model was chosen.");
        assert!(models.asked.lock().is_empty(), "a provider was resolved anyway");
    }

    /* -- summarising ---------------------------------------------------- */

    fn transcript() -> Vec<Value> {
        vec![
            json!({ "role": "user", "content": "port the llm bridge" }),
            json!({
                "role": "agent",
                "content": "Reading the Electron handler.",
                "toolCalls": [{ "id": "c1", "name": "read", "arguments": "{}" }],
            }),
            json!({ "role": "tool", "toolCallId": "c1", "content": "the file" }),
            json!({ "role": "user", "parts": [{ "type": "image_url", "image_url": { "url": "data:," } }] }),
        ]
    }

    /// The prompt, the digest and the shape the window destructures. All four
    /// in one test because they are one contract: `/compact` reads `summary`
    /// off the answer and puts it in the transcript.
    #[tokio::test]
    async fn the_summariser_is_asked_for_a_handoff_note_about_the_conversation() {
        let models = Fake::new(MockProvider::new().replying("  ## Goal\nPort it.  "));

        let answer = summarise(
            &models,
            serde_json::from_value(json!({
                "modelRef": "mock/mock-model",
                "history": transcript(),
            }))
            .expect("the window's own request shape"),
        )
        .await
        .expect("a summary");

        assert_eq!(answer["summary"], "## Goal\nPort it.");
        assert_eq!(answer["usage"]["input"], 10);

        let request = models.provider.last_request().expect("a request was sent");
        // Every heading, because a note missing one quietly becomes a note in
        // which that section is empty.
        for heading in [
            "## Goal",
            "## Constraints and preferences",
            "## Done",
            "## Decisions",
            "## Open",
            "## Worth keeping",
        ] {
            assert!(request.system.contains(heading), "the instruction lost {heading}");
        }
        assert!(request.tools.is_empty(), "the summariser was offered tools");
        assert_eq!(request.temperature, Some(0.0));

        // One message, and it is the conversation written out.
        assert_eq!(request.history.len(), 1);
        let asked = request.history[0].text();
        assert!(asked.contains("user: port the llm bridge"));
        assert!(asked.contains("assistant: Reading the Electron handler.\n[called: read]"));
        assert!(asked.contains("tool result: the file"));
        assert!(asked.contains("[an image]"));
    }

    /// What the person typed after the command is a priority, not a new prompt.
    #[tokio::test]
    async fn a_focus_is_added_to_the_instruction_rather_than_replacing_it() {
        let models = Fake::new(MockProvider::new().replying("note"));

        summarise(
            &models,
            serde_json::from_value(json!({
                "modelRef": "mock/mock-model",
                "history": transcript(),
                "focus": "  keep the SQL  ",
            }))
            .expect("the window's own request shape"),
        )
        .await
        .expect("a summary");

        let system = models.provider.last_request().expect("a request").system;
        assert!(system.starts_with(SUMMARY_INSTRUCTION), "the instruction was replaced");
        assert!(system.ends_with("particular attention to this:\nkeep the SQL"));
    }

    /// A caller that named no model gets the workspace's default, which is what
    /// a conversation whose agent has no model of its own sends.
    #[tokio::test]
    async fn a_summary_with_no_model_named_falls_back_to_the_default() {
        let models = Fake::new(MockProvider::new().replying("note"));

        summarise(
            &models,
            serde_json::from_value(json!({ "history": transcript() }))
                .expect("the window's own request shape"),
        )
        .await
        .expect("a summary");

        assert_eq!(models.asked.lock().as_slice(), ["mock/mock-model"]);
    }

    #[tokio::test]
    async fn there_is_nothing_to_summarise_before_a_conversation_has_started() {
        let models = Fake::new(MockProvider::new());
        let refused = summarise(&models, SummarizeCall::default())
            .await
            .expect_err("an empty transcript was summarised");
        assert_eq!(refused, "There is nothing to summarise yet.");
        assert!(models.asked.lock().is_empty(), "a provider was resolved anyway");
    }

    /// An empty note must not be written into the transcript as though it were
    /// a summary: the conversation would be folded behind nothing.
    #[tokio::test]
    async fn an_empty_answer_is_a_failure_rather_than_an_empty_note() {
        let models = Fake::new(MockProvider::new().replying("   "));
        let refused = summarise(
            &models,
            serde_json::from_value(json!({ "history": transcript() })).expect("a request"),
        )
        .await
        .expect_err("an empty summary was accepted");
        assert_eq!(refused, "The model returned an empty summary.");
    }

    /// The provider's own sentence, not a shorter one about summarising.
    #[tokio::test]
    async fn a_refused_summary_reports_what_the_provider_said() {
        let models = Fake::new(MockProvider::new().failing("context length exceeded", Some(400)));
        let refused = summarise(
            &models,
            serde_json::from_value(json!({ "history": transcript() })).expect("a request"),
        )
        .await
        .expect_err("a failed summary was accepted");
        assert_eq!(refused, "context length exceeded");
    }

    /// Long messages are cut by character rather than byte: every transcript in
    /// this app has multi-byte characters in it, and a byte slice through one
    /// would take the whole command down.
    #[test]
    fn a_long_message_is_shortened_without_splitting_a_character() {
        let long = "é".repeat(MAX_DIGEST_CHARS_PER_ENTRY + 500);
        let written = digest(&[json!({ "role": "user", "content": long })]);
        assert_eq!(written.chars().count(), "user: ".len() + MAX_DIGEST_CHARS_PER_ENTRY);
    }
}

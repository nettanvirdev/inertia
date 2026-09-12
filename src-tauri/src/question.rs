//! Asking the person a question mid-turn.
//!
//! Without this tool a model that hits a genuine fork has two bad options: stop
//! and end its turn with a question, losing everything it had loaded, or guess
//! and do half the work wrong. This is the third option - pause, ask, carry on
//! in the same turn with the answer in hand.
//!
//! It is the same mechanism as the permission card in `permission.rs`, and
//! deliberately so: one event channel tagged with `channel`, a card in the
//! transcript, a suspended call behind it, and a `settled` event when it ends.
//! A window that subscribed to answers but forgot to subscribe to questions
//! would hang a tool call forever with no sign of why, so there is one
//! subscription and both kinds ride it.
//!
//! Three properties are worth more than the mechanics:
//!
//!   - **Nobody to ask is not a hang.** If the event cannot be delivered - no
//!     window, a routine running unattended - the call comes straight back
//!     telling the model to make the sensible choice and say what it assumed.
//!   - **A cancelled turn releases its questions.** The waiting future removes
//!     its own entry when it is dropped, and `abandon_all` releases anything
//!     still held when a turn is stopped from elsewhere. A question nobody can
//!     answer must never be a call nobody can finish.
//!   - **Dismissing is an answer the model can act on**, not an error that ends
//!     the turn: the person read the question and chose not to settle it, which
//!     the model should hear about and work around.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::State;
use tokio::sync::oneshot;

use crate::state::AppState;

/// Options past this stop being a choice and start being a list to read.
const MAX_OPTIONS: usize = 4;

/// How an event reaches the window.
///
/// A closure rather than an `AppHandle` for the reason every seam in this crate
/// takes one: a test captures the event and answers it from another task, and
/// building an `AppHandle` in a test links Tauri's window chrome into the test
/// binary and takes the whole harness down before a single test runs.
///
/// It answers whether the event was delivered, which is load-bearing rather
/// than cosmetic: an undeliverable question is the unattended case, and the
/// model has to be told to decide for itself instead of waiting on a person who
/// will never see it.
pub type Emit = Arc<dyn Fn(Value) -> bool + Send + Sync>;

/// What a waiting question was settled with.
enum Reply {
    /// The chosen label, or what the person typed.
    Answer(String),
    /// Read and put aside. The sentence is what the model is told.
    Dismissed(String),
    /// The turn was stopped. Nobody is coming back for the answer.
    Stopped,
}

/// One question on screen, and the call suspended behind it.
struct Pending {
    /// Kept as well as the sender so a window that opens mid-turn can be told
    /// what is already waiting, and so `settled` can be sent on whichever path
    /// ends it.
    question: Value,
    /// The conversation that is blocked. A subagent runs as `parent/task-3`,
    /// and the card belongs on the parent's screen.
    session: String,
    /// The window that was told about this one. Held per question rather than
    /// per table because the table outlives any one turn, and the turn is what
    /// owns a way to reach the window.
    emit: Emit,
    reply: oneshot::Sender<Reply>,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("question", &self.question)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// Every question waiting on the person, across every turn.
///
/// Process-wide rather than per-turn, because the thing answering is a command
/// from the window: it knows the question's id and has no reason to know which
/// turn raised it.
#[derive(Default)]
pub struct Questions {
    waiting: Mutex<HashMap<String, Pending>>,
}

impl std::fmt::Debug for Questions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Questions")
            .field("waiting", &self.waiting.lock().len())
            .finish()
    }
}

impl Questions {
    /// Takes a card off the screen.
    ///
    /// Sent for every ending, including the ones the clicking window already
    /// removed optimistically: a turn cancelled from another window, or a
    /// question released because its turn died, are endings that window never
    /// sees.
    fn settled(pending: &Pending, id: &str) {
        (pending.emit)(json!({ "channel": "question", "type": "settled", "id": id }));
    }

    fn finish(&self, id: &str, reply: Reply) -> bool {
        let Some(pending) = self.waiting.lock().remove(id) else {
            return false;
        };
        Self::settled(&pending, id);
        // A closed receiver means the turn went away between the lookup and
        // here, which is ordinary and not worth reporting.
        pending.reply.send(reply).is_ok()
    }

    /// The person answered. `value` is the chosen label or what they typed.
    ///
    /// An id nobody is waiting on is ignored rather than reported: the turn may
    /// have been cancelled while the card was on screen.
    pub fn answer(&self, id: &str, value: &str) -> bool {
        self.finish(id, Reply::Answer(value.to_string()))
    }

    /// The person read it and chose not to settle it.
    pub fn dismiss(&self, id: &str) -> bool {
        self.finish(id, Reply::Dismissed(DISMISSED.to_string()))
    }

    /// Releases everything still outstanding, because the turns holding it are
    /// being stopped. Without this a stopped turn could leave a tool blocked on
    /// a card the person can no longer see.
    pub fn abandon_all(&self) {
        let abandoned: Vec<(String, Pending)> = self.waiting.lock().drain().collect();
        for (id, pending) in abandoned {
            Self::settled(&pending, &id);
            let _ = pending.reply.send(Reply::Stopped);
        }
    }

    /// The same, for one conversation: what a single turn being stopped owns.
    ///
    /// Matched on the root session, because a subagent running as
    /// `parent/task-3` is part of the parent's turn and dies with it.
    pub fn abandon_session(&self, session: &str) {
        let root = root_session(session);
        let mine: Vec<String> = self
            .waiting
            .lock()
            .iter()
            .filter(|(_, pending)| root_session(&pending.session) == root)
            .map(|(id, _)| id.clone())
            .collect();
        for id in mine {
            let Some(pending) = self.waiting.lock().remove(&id) else {
                continue;
            };
            Self::settled(&pending, &id);
            let _ = pending.reply.send(Reply::Stopped);
        }
    }

    /// What this conversation is currently blocked on, for a window that opened
    /// mid-turn and has no card on screen for a call that is still waiting.
    pub fn pending(&self, session: Option<&str>) -> Vec<Value> {
        self.waiting
            .lock()
            .values()
            .filter(|pending| match session {
                Some(wanted) => root_session(&pending.session) == root_session(wanted),
                None => true,
            })
            .map(|pending| pending.question.clone())
            .collect()
    }
}

/// The conversation a session id belongs to. `parent/task-3` is parent's.
fn root_session(session: &str) -> &str {
    session.split('/').next().unwrap_or(session)
}

/// What the model is told when the person waves the question away. Its own
/// sentence rather than a bare "dismissed", because the model has to decide
/// what to do next and "they are not going to answer" is the useful half.
const DISMISSED: &str = "The user dismissed the question without answering. Make the most sensible choice yourself and say what you assumed.";

/// What the model is told when there is no window to ask.
const UNATTENDED: &str =
    "There is nobody to ask right now. Make the most sensible choice and say what you assumed.";

/// Removes a question when the call waiting on it goes away.
///
/// The turn being cancelled drops the tool's future, and without this the entry
/// would sit in the table forever with a card on screen that answers nothing.
struct Guard {
    questions: Arc<Questions>,
    id: String,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let Some(pending) = self.questions.waiting.lock().remove(&self.id) else {
            // Already settled by an answer or an abandon, which sent its own
            // `settled`. Sending a second one is harmless but the table is the
            // honest record of whether this was ours to end.
            return;
        };
        Questions::settled(&pending, &self.id);
    }
}

/// The tool. Built per turn, because the way back to the window is.
pub struct QuestionTool {
    questions: Arc<Questions>,
    emit: Emit,
}

impl std::fmt::Debug for QuestionTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QuestionTool")
            .field("questions", &self.questions)
            .finish_non_exhaustive()
    }
}

impl QuestionTool {
    pub fn new(questions: Arc<Questions>, emit: Emit) -> Self {
        Self { questions, emit }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// The options, as the card renders them: a label, and a line saying what
/// happens if it is picked. Anything without a label is dropped rather than
/// drawn as an empty radio row nobody can choose.
fn options_of(args: &Value) -> Vec<Value> {
    args.get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let label = option.get("label").and_then(Value::as_str)?.trim();
                    if label.is_empty() {
                        return None;
                    }
                    let description = option
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    Some(json!({ "label": label, "description": description }))
                })
                .take(MAX_OPTIONS)
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl Tool for QuestionTool {
    fn id(&self) -> &str {
        "question"
    }

    fn description(&self) -> &str {
        "Ask the user a question and wait for their answer, without ending your turn.\n\
         \n\
         Use it only when the answer changes what you would build and you cannot work it\n\
         out yourself. Two designs that are both defensible, a destructive step that is\n\
         reasonable but not obviously wanted, a missing detail that is not in any file.\n\
         \n\
         Do NOT use it for anything you could answer by reading the code, for permission to\n\
         use a tool (tools ask for themselves), or to check in on work the user already\n\
         asked for. When a sensible default exists, take it and say what you assumed.\n\
         \n\
         Give real options where you can. Choosing from three is faster than writing a\n\
         sentence, and the user can always type something else."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question, in one clear sentence"
                },
                "options": {
                    "type": "array",
                    "description": "Two to four options. Omit to ask an open question.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string", "description": "The short label for this choice" },
                            "description": { "type": "string", "description": "What happens if they pick it" }
                        },
                        "required": ["label"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["question"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        // The question itself is the target, so a rule a person writes reads
        // like the thing it is about. `always` is `*` because there is nothing
        // in one question worth remembering about the next.
        PermissionRequest::new(
            "question",
            args.get("question")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
        .with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        args.get("question")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let text = args
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| {
                Error::InvalidInput("Missing required argument `question`.".to_string())
            })?
            .to_string();

        let id = format!("q-{}", uuid::Uuid::new_v4().simple());
        let question = json!({
            "id": id,
            "sessionId": ctx.session.as_str(),
            "question": text,
            "options": options_of(&args),
            "at": now_ms(),
        });

        let (sender, receiver) = oneshot::channel();
        self.questions.waiting.lock().insert(
            id.clone(),
            Pending {
                question: question.clone(),
                session: ctx.session.as_str().to_string(),
                emit: self.emit.clone(),
                reply: sender,
            },
        );

        // Armed before the event goes out, not after: an answer can come back
        // on another task the instant the window renders the card, and a guard
        // installed afterwards would be racing it.
        let guard = Guard {
            questions: self.questions.clone(),
            id: id.clone(),
        };

        // Not through `turn::emit`. That stamps the turn's own id over the
        // event's, and the window matches a `settled` against the question's
        // id - so a question announced that way could never be taken off the
        // screen.
        let delivered = (self.emit)(json!({
            "channel": "question",
            "type": "asked",
            "id": id,
            "question": question,
        }));
        if !delivered {
            drop(guard);
            return Ok(ToolOutcome {
                title: Some(text),
                output: UNATTENDED.to_string(),
                metadata: Some(json!({ "unattended": true })),
                images: Vec::new(),
            });
        }

        let reply = receiver.await;
        drop(guard);

        match reply {
            Ok(Reply::Answer(answer)) => Ok(ToolOutcome {
                title: Some(text.clone()),
                output: format!("The user answered: {answer}"),
                metadata: Some(json!({ "question": text, "answer": answer })),
                images: Vec::new(),
            }),
            // A refusal the model should read and work around, not an error
            // that ends the turn: it still has the work loaded and can carry on
            // with a stated assumption.
            Ok(Reply::Dismissed(why)) => Ok(ToolOutcome {
                title: Some(text.clone()),
                output: why,
                metadata: Some(json!({ "question": text, "dismissed": true })),
                images: Vec::new(),
            }),
            // The turn was stopped, or the sender went away with it. There is
            // nobody left to read a result, which is the one case that is an
            // error rather than an outcome.
            Ok(Reply::Stopped) | Err(_) => Err(Error::Cancelled),
        }
    }
}

/* -- the commands the window answers with -------------------------------- */

/// The person answered a question. `value` is the chosen label or their text.
///
/// Broadcast against one process-wide table rather than addressed to a turn:
/// the window knows the question's id and has no reason to know which turn
/// raised it, and only one call can be holding any given id.
#[tauri::command]
pub fn agent_answer(state: State<'_, AppState>, id: String, value: String) -> Value {
    json!({ "answered": state.questions.answer(&id, &value) })
}

/// The person put the question aside. The call comes back with a sentence the
/// model can act on rather than being left waiting.
#[tauri::command]
pub fn agent_dismiss(state: State<'_, AppState>, id: String) -> Value {
    json!({ "dismissed": state.questions.dismiss(&id) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::permission::Action;
    use inertia_core::tool::{Decision, PermissionGate};

    /// Allows everything. The gate is consulted by the registry rather than by
    /// the tool, so these tests only need one that exists.
    #[derive(Debug)]
    struct Allow;

    #[async_trait]
    impl PermissionGate for Allow {
        async fn ask(&self, _request: &PermissionRequest) -> Result<Decision> {
            Ok(Decision::Allow)
        }

        async fn verdict(&self, _key: &str, _target: &str) -> Action {
            Action::Allow
        }
    }

    fn ctx(session: &str) -> ToolContext {
        ToolContext {
            root: std::path::PathBuf::from("."),
            session: SessionId::from_existing(session),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(Allow),
        }
    }

    /// A window that records what it was told, so a test can answer the
    /// question it just saw.
    fn window() -> (Emit, Arc<Mutex<Vec<Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let emit: Emit = Arc::new(move |event| {
            sink.lock().push(event);
            true
        });
        (emit, seen)
    }

    /// The id of the first `asked` event, waiting for it to arrive.
    async fn asked(seen: &Arc<Mutex<Vec<Value>>>) -> Value {
        for _ in 0..200 {
            if let Some(event) = seen
                .lock()
                .iter()
                .find(|event| event["type"] == "asked")
                .cloned()
            {
                return event;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("the question was never announced");
    }

    #[tokio::test]
    async fn an_answer_comes_back_to_the_call_that_asked() {
        let (emit, seen) = window();
        let questions = Arc::new(Questions::default());
        let tool = QuestionTool::new(questions.clone(), emit);

        let watcher = tokio::spawn({
            let questions = questions.clone();
            let seen = seen.clone();
            async move {
                let event = asked(&seen).await;
                let id = event["id"].as_str().unwrap_or_default().to_string();
                questions.answer(&id, "The second one");
            }
        });

        let out = tool
            .execute(
                json!({
                    "question": "Which shape should the cache take?",
                    "options": [
                        { "label": "One file", "description": "simpler" },
                        { "label": "One per day", "description": "prunes" }
                    ]
                }),
                &ctx("t1"),
            )
            .await
            .expect("the tool ran");

        watcher.await.expect("the window task finished");

        assert_eq!(out.output, "The user answered: The second one");
        assert_eq!(
            out.metadata.as_ref().and_then(|m| m.get("answer")),
            Some(&json!("The second one"))
        );
        assert_eq!(
            out.title.as_deref(),
            Some("Which shape should the cache take?")
        );

        // Exactly what the card renders, on the channel the window is already
        // subscribed to.
        let event = seen
            .lock()
            .iter()
            .find(|event| event["type"] == "asked")
            .cloned()
            .expect("an asked event");
        assert_eq!(event["channel"], json!("question"));
        assert_eq!(event["question"]["sessionId"], json!("t1"));
        assert_eq!(
            event["question"]["question"],
            json!("Which shape should the cache take?")
        );
        assert_eq!(event["question"]["options"][1]["label"], json!("One per day"));
        assert_eq!(event["question"]["id"], event["id"]);

        // And the card comes off the screen when it ends.
        assert!(seen.lock().iter().any(|event| event["type"] == "settled"));
        assert!(questions.pending(None).is_empty());
    }

    #[tokio::test]
    async fn a_dismissed_question_is_an_answer_the_model_can_act_on() {
        let (emit, seen) = window();
        let questions = Arc::new(Questions::default());
        let tool = QuestionTool::new(questions.clone(), emit);

        let watcher = tokio::spawn({
            let questions = questions.clone();
            let seen = seen.clone();
            async move {
                let event = asked(&seen).await;
                questions.dismiss(event["id"].as_str().unwrap_or_default());
            }
        });

        // Not an `Err`: the turn carries on, having been told to decide for
        // itself. A dismissal that ended the turn would lose the work.
        let out = tool
            .execute(json!({ "question": "Delete the old exports?" }), &ctx("t1"))
            .await
            .expect("a dismissal is still a result");

        watcher.await.expect("the window task finished");

        assert_eq!(out.output, DISMISSED);
        assert_eq!(
            out.metadata.and_then(|m| m.get("dismissed").cloned()),
            Some(json!(true))
        );
        assert!(questions.pending(None).is_empty());
    }

    #[tokio::test]
    async fn a_stopped_turn_releases_the_question_rather_than_hanging() {
        let (emit, seen) = window();
        let questions = Arc::new(Questions::default());
        let tool = QuestionTool::new(questions.clone(), emit);

        let watcher = tokio::spawn({
            let questions = questions.clone();
            let seen = seen.clone();
            async move {
                asked(&seen).await;
                questions.abandon_session("t1");
            }
        });

        let outcome = tool
            .execute(json!({ "question": "Which one?" }), &ctx("t1/task-3"))
            .await;

        watcher.await.expect("the window task finished");

        // Cancellation is the one failure that is an error: there is nobody
        // left to read a result.
        assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
        assert!(questions.pending(None).is_empty());
    }

    #[tokio::test]
    async fn with_nobody_to_ask_the_model_is_told_to_decide_for_itself() {
        let emit: Emit = Arc::new(|_| false);
        let questions = Arc::new(Questions::default());
        let tool = QuestionTool::new(questions.clone(), emit);

        let out = tool
            .execute(json!({ "question": "Which one?" }), &ctx("t1"))
            .await
            .expect("the tool ran");

        assert_eq!(out.output, UNATTENDED);
        // Nothing is left waiting on a person who will never see it.
        assert!(questions.pending(None).is_empty());
    }

    #[tokio::test]
    async fn a_question_dropped_with_its_turn_leaves_nothing_in_the_table() {
        let (emit, seen) = window();
        let questions = Arc::new(Questions::default());
        let tool = QuestionTool::new(questions.clone(), emit);

        // What a cancelled turn does: the stream is dropped, and the tool
        // future goes with it. The scope is the drop - a pinned future lives
        // until the end of the block it was pinned in.
        let ctx = ctx("t1");
        {
            let call = tool.execute(json!({ "question": "Which one?" }), &ctx);
            tokio::pin!(call);
            tokio::select! {
                _ = &mut call => panic!("the call should still be waiting"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
            }
            assert_eq!(questions.pending(None).len(), 1);
        }

        assert!(questions.pending(None).is_empty());
        assert!(seen.lock().iter().any(|event| event["type"] == "settled"));
    }

    #[tokio::test]
    async fn a_subagents_question_belongs_to_the_conversation_that_spawned_it() {
        let (emit, seen) = window();
        let questions = Arc::new(Questions::default());
        let tool = QuestionTool::new(questions.clone(), emit);

        let ctx = ctx("t1/task-3");
        {
            let call = tool.execute(json!({ "question": "Which one?" }), &ctx);
            tokio::pin!(call);
            tokio::select! {
                _ = &mut call => panic!("the call should still be waiting"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {}
            }

            // The person has one screen, and it is the parent's.
            assert_eq!(questions.pending(Some("t1")).len(), 1);
            assert!(questions.pending(Some("t2")).is_empty());
        }
        let _ = seen;
    }
}

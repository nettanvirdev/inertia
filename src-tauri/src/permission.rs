//! The permission gate that asks a person.
//!
//! The rule engine decides allow, deny, or ask. Only the third case reaches
//! the user, and this is what turns it into a card on screen and a suspended
//! tool call waiting on their answer.
//!
//! Two properties matter more than the mechanics:
//!
//!   - **A `deny` never becomes an allow.** The rules are consulted first, and
//!     a denial returns without the user ever being asked, so there is no
//!     dialog whose "Allow" button could contradict a rule.
//!   - **No answer is not consent.** If the window is gone or the conversation
//!     is abandoned, the pending request resolves to a refusal, never a
//!     default yes.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::permission::{evaluate, Action, Rule};
use inertia_core::tool::{Decision, PermissionGate, PermissionRequest};
use inertia_store::Settings;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

/// What the frontend is told about a pending request.
///
/// It rides the turn's own event channel rather than one of its own: a window
/// that subscribed to answers but forgot to subscribe to questions would hang a
/// tool call forever with no sign of why. One subscription, both kinds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    /// Echoed back with the answer.
    pub id: String,
    /// Which conversation is blocked on this. The prompt list routes on it, so
    /// a card raised by one thread's turn does not appear under another.
    pub session_id: String,
    pub key: String,
    pub target: String,
    /// The sentence at the top of the card. Absent falls back to a generic one
    /// the card already carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Present when "always allow" is meaningful for this call. When absent
    /// the UI should not offer the option, because there is nothing sensible
    /// to remember.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub always: Option<String>,
}

/// The answer coming back from the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Answer {
    Allow,
    AllowAlways,
    Deny,
}

impl From<Answer> for Decision {
    fn from(answer: Answer) -> Self {
        match answer {
            Answer::Allow => Decision::Allow,
            Answer::AllowAlways => Decision::AllowAlways,
            Answer::Deny => Decision::Deny,
        }
    }
}

pub struct UiPermissionGate {
    app: AppHandle,
    settings: Arc<Settings>,
    /// The conversation these requests belong to, so a card raised by one
    /// thread's turn does not appear under another.
    session: String,
    /// The agent this conversation is running as, whose rules apply on top of
    /// the workspace's.
    agent: Option<String>,
    waiting: Arc<Mutex<HashMap<String, Pending>>>,
}

/// One request on screen, and the call suspended behind it.
///
/// The `Ask` is kept as well as the sender so a window that opens mid-turn -
/// the user switched threads, or reloaded - can be told what is already
/// waiting. Without it the card would be gone and the tool blocked on an answer
/// nobody can give.
struct Pending {
    ask: Ask,
    reply: oneshot::Sender<Decision>,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending").field("ask", &self.ask).finish()
    }
}

impl std::fmt::Debug for UiPermissionGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiPermissionGate")
            .field("agent", &self.agent)
            .field("waiting", &self.waiting.lock().len())
            .finish()
    }
}

impl UiPermissionGate {
    pub fn new(
        app: AppHandle,
        settings: Arc<Settings>,
        session: String,
        agent: Option<String>,
    ) -> Self {
        Self {
            app,
            settings,
            session,
            agent,
            waiting: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Hands an answer to whichever call is waiting on it.
    ///
    /// An id nobody is waiting on is ignored rather than reported: the turn may
    /// have been cancelled while the card was on screen, and that is ordinary.
    pub fn answer(&self, id: &str, answer: Answer) {
        if let Some(pending) = self.waiting.lock().remove(id) {
            let _ = pending.reply.send(answer.into());
            self.settled(id);
        }
    }

    /// Refuses everything still outstanding.
    ///
    /// Called when a turn is cancelled, so a tool is never left blocked on a
    /// card the user can no longer see.
    pub fn abandon_all(&self) {
        let abandoned: Vec<(String, Pending)> = self.waiting.lock().drain().collect();
        for (id, pending) in abandoned {
            let _ = pending.reply.send(Decision::Deny);
            self.settled(&id);
        }
    }

    /// Takes a card off the screen.
    ///
    /// Sent for every ending, including the ones the window already knows about
    /// from its own optimistic removal: a turn cancelled from another window,
    /// or a rule that answered while the card was open, are both endings the
    /// clicking window never sees.
    fn settled(&self, id: &str) {
        let _ = self.app.emit(
            crate::turn::AGENT_EVENT,
            serde_json::json!({ "channel": "permission", "type": "settled", "id": id }),
        );
    }

    /// What this conversation is currently blocked on.
    pub fn pending(&self) -> Vec<Ask> {
        self.waiting
            .lock()
            .values()
            .map(|pending| pending.ask.clone())
            .collect()
    }

    fn rules(&self) -> Vec<Rule> {
        self.settings.permissions().for_agent(self.agent.as_deref())
    }

    /// Remembers an "always allow" as a rule in the workspace settings.
    fn remember(&self, request: &PermissionRequest) {
        let Some(pattern) = &request.always else {
            return;
        };
        let mut permissions = self.settings.permissions();
        let rule = Rule::new(&request.key, Action::Allow, pattern);

        // Replace an existing rule for the same key and pattern rather than
        // appending a duplicate the user would then have to delete twice.
        if let Some(existing) = permissions
            .workspace
            .iter_mut()
            .find(|r| r.tool == rule.tool && r.pattern == rule.pattern)
        {
            *existing = rule;
        } else {
            permissions.workspace.push(rule);
        }

        if let Err(e) = self.settings.save_permissions(&permissions) {
            // Worth telling the user about, but not worth refusing the call
            // they just approved: the decision stands for this turn either way.
            tracing::warn!(error = %e, "could not persist an always-allow rule");
        }
    }
}

#[async_trait]
impl PermissionGate for UiPermissionGate {
    async fn ask(&self, request: &PermissionRequest) -> inertia_core::Result<Decision> {
        // The rules decide first, so a denial never reaches a dialog whose
        // Allow button could contradict it.
        match evaluate(&self.rules(), &request.key, &request.target).action {
            Action::Allow => return Ok(Decision::Allow),
            Action::Deny => return Ok(Decision::Deny),
            Action::Ask => {}
        }

        let id = uuid::Uuid::new_v4().simple().to_string();
        let (sender, receiver) = oneshot::channel();

        let ask = Ask {
            id: id.clone(),
            session_id: self.session.clone(),
            key: request.key.clone(),
            target: request.target.clone(),
            // The card falls back to a label derived from the key, which is
            // better than a title invented here from the same information.
            title: None,
            always: request.always.clone(),
        };

        self.waiting.lock().insert(
            id.clone(),
            Pending {
                ask: ask.clone(),
                reply: sender,
            },
        );

        let announced = self.app.emit(
            crate::turn::AGENT_EVENT,
            serde_json::json!({
                "channel": "permission",
                "type": "asked",
                "id": id,
                "question": ask,
            }),
        );
        if announced.is_err() {
            // Nobody can answer, so nobody approved.
            self.waiting.lock().remove(&id);
            return Ok(Decision::Deny);
        }

        // A turn stopped on a card nobody is looking at is the one notice that
        // will never resolve itself: the turn waits for a click that cannot
        // happen until somebody comes back to the window.
        crate::notify::permission_waiting(
            &self.app,
            self.agent.as_deref(),
            &request.key,
            &request.target,
        );

        // A dropped sender - the window closed, the turn was abandoned -
        // resolves to a refusal. Silence is not consent.
        let decision = receiver.await.unwrap_or(Decision::Deny);

        if decision == Decision::AllowAlways {
            self.remember(request);
        }

        Ok(decision)
    }

    async fn verdict(&self, key: &str, target: &str) -> Action {
        evaluate(&self.rules(), key, target).action
    }
}

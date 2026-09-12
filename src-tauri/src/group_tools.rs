//! Being in a conversation with other agents, rather than delegating to them.
//!
//! Delegation sends work away: a helper gets a brief, runs in a session of its
//! own, and comes back with an answer nobody watched it produce. These three do
//! the opposite. They change who is in THIS conversation and who is speaking
//! into it, and that is all they do - no session is started here, no transcript
//! is copied, nothing is summarised.
//!
//!   `invite`    bring an agent into the conversation; it answers next
//!   `handover`  give the conversation to someone else and stop being the one
//!               whose name is on it
//!   `part`      step out, having said what there was to say
//!
//! The agents end up aware of each other for free, because they are reading one
//! transcript rather than being told about each other's. That is also what
//! keeps it affordable: the room costs one agent plus the turns the others
//! actually take, and nobody pays for a digest of a conversation they are
//! already in.
//!
//! None of these exist outside a group conversation - they are added to the
//! registry only for a turn that has a room - and none of them can override
//! what the person asked for. Whose turn it is next is settled when the turn
//! ends, in `group`, so a turn is free to invite somebody and then go on
//! talking.
//!
//! Each tool is built per turn and holds the conversation it belongs to and the
//! agent that is speaking. That is deliberate: `ToolContext` carries the
//! session but not the seat, and a tool that had to guess which agent called it
//! would be one `part` away from removing the wrong one.

use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use inertia_store::Layout;
use serde_json::{json, Value};

use crate::group::{Moved, Rooms};

/// What every group tool needs to know about the turn it was built for.
#[derive(Debug, Clone)]
pub struct Seat {
    pub rooms: Arc<Rooms>,
    pub layout: Layout,
    /// The conversation. The room is keyed by this.
    pub conversation: String,
    /// The agent speaking. `None` only in a conversation with no agent set,
    /// which cannot be a group - so the tools refuse rather than guess.
    pub agent_id: Option<String>,
}

impl Seat {
    /// Every agent on the team, as stored.
    fn agents(&self) -> Vec<Value> {
        inertia_store::collections::list(&self.layout, inertia_store::Collection::Agents)
    }

    /// Find the agent the model named, by whatever it called them.
    ///
    /// Models write a colleague's name the way a person would - "Iris",
    /// "@iris", sometimes the id it saw in a roster - so all three resolve. A
    /// near miss is answered with the list rather than with "not found",
    /// because the fix is always to pick one of these and the model should not
    /// have to spend a turn asking.
    fn colleague(&self, wanted: &str) -> std::result::Result<(String, String), String> {
        // `.json` comes off because a model that has listed the agents folder
        // is holding filenames, and `agent-local-x2.json` is one `read` and one
        // wasted call away from the id it already has in its hand.
        let needle = wanted
            .trim()
            .trim_start_matches('@')
            .trim_end_matches(".json")
            .to_lowercase();
        let agents = self.agents();

        if !needle.is_empty() {
            let field = |record: &Value, key: &str| {
                record
                    .get(key)
                    .and_then(Value::as_str)
                    .map(|v| v.trim().trim_start_matches('@').to_lowercase())
            };
            let found = agents
                .iter()
                .find(|record| field(record, "id").as_deref() == Some(&needle))
                .or_else(|| {
                    agents
                        .iter()
                        .find(|record| field(record, "name").as_deref() == Some(&needle))
                })
                .or_else(|| {
                    agents
                        .iter()
                        .find(|record| field(record, "handle").as_deref() == Some(&needle))
                })
                // The handle as the composer spells it: "Nova Reyes" is
                // `@nova-reyes`, and that is what a model reading the roster in
                // its own prompt will write back.
                .or_else(|| {
                    agents.iter().find(|record| {
                        record
                            .get("name")
                            .and_then(Value::as_str)
                            .map(inertia_agent::floor::slug_of)
                            .as_deref()
                            == Some(&needle)
                    })
                });

            if let Some(record) = found {
                let id = record.get("id").and_then(Value::as_str).unwrap_or_default();
                let name = record
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(id);
                return Ok((id.to_string(), name.to_string()));
            }
        }

        let names: Vec<&str> = agents
            .iter()
            .filter_map(|record| {
                record
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| record.get("id").and_then(Value::as_str))
            })
            .collect();

        Err(if names.is_empty() {
            "There are no other agents configured, so there is nobody to bring in.".to_string()
        } else {
            format!(
                "There is no agent called {wanted}. The agents on this team are: {}.",
                names.join(", ")
            )
        })
    }
}

/// A required string argument, or the sentence saying it is missing.
fn required(args: &Value, key: &str) -> std::result::Result<String, Error> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Other(format!("`{key}` is required.")))
}

/// An optional string argument.
fn optional(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// A refusal the model should read and work around, not an error that ends the
/// turn. An agent told the room is full can carry on alone; one whose turn was
/// killed cannot.
fn refused(reason: String) -> ToolOutcome {
    ToolOutcome {
        title: Some("Not done".into()),
        output: reason,
        metadata: Some(json!({ "group": true, "refused": true })),
        images: Vec::new(),
    }
}

#[derive(Debug)]
pub struct InviteTool(Seat);

#[async_trait]
impl Tool for InviteTool {
    fn id(&self) -> &str {
        "invite"
    }

    fn description(&self) -> &str {
        "Bring another agent into this conversation. They answer next.\n\
         \n\
         Writing their `@handle` in your message does the same thing without a tool \
         call; use this when you want to say why they are being brought in.\n\
         \n\
         This is not delegation. The agent joins the conversation you are already in, \
         reads everything that has been said, and speaks into it - the person sees its \
         reply in the same thread, under its own name. Nothing is copied or forwarded, \
         so say why you are bringing it in and it will have the context from the \
         transcript itself.\n\
         \n\
         Use it when the work has reached something somebody else is better at, or when \
         a decision would benefit from an argument.\n\
         \n\
         You keep speaking after this. The agent you invite goes next, and the \
         conversation carries on from there."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "The agent to bring in, by name." },
                "why": { "type": "string", "description": "One line on what you want from them. The person sees this." }
            },
            "required": ["agent", "why"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        PermissionRequest::new("task", optional(args, "agent").unwrap_or_default()).with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        optional(args, "agent").map(|agent| format!("Invite {agent}"))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let wanted = required(&args, "agent")?;
        let why = optional(&args, "why");
        let (id, name) = match self.0.colleague(&wanted) {
            Ok(found) => found,
            Err(reason) => return Ok(refused(reason)),
        };

        match self.0.rooms.invite(
            &self.0.conversation,
            &id,
            self.0.agent_id.as_deref(),
            why.as_deref(),
        ) {
            Moved::No(reason) => Ok(refused(reason)),
            Moved::Yes { already } => Ok(ToolOutcome {
                title: Some(if already {
                    format!("Asked {name}")
                } else {
                    format!("Invited {name}")
                }),
                output: format!(
                    "{name} is {}in the conversation and will answer next. They can read \
                     everything said here, so you do not need to repeat it. Finish what you \
                     were saying - your turn is not over.",
                    if already { "already " } else { "" }
                ),
                metadata: Some(json!({
                    "group": true,
                    "joined": id,
                    "agent": name,
                    "already": already,
                })),
                images: Vec::new(),
            }),
        }
    }
}

#[derive(Debug)]
pub struct HandoverTool(Seat);

#[async_trait]
impl Tool for HandoverTool {
    fn id(&self) -> &str {
        "handover"
    }

    fn description(&self) -> &str {
        "Give this conversation to another agent.\n\
         \n\
         Stronger than `invite`: the conversation becomes theirs. The person sees the \
         name and face at the top of the thread change, and what they say next goes to \
         that agent rather than to you. You stay in the room and can be brought back in \
         or asked something directly, but you are no longer the one it belongs to.\n\
         \n\
         Use it when the work has moved somewhere else for good - the research is done \
         and the building starts, the bug is found and the fix is somebody else's area. \
         Use `invite` when you want their opinion and expect to carry on afterwards.\n\
         \n\
         Say what you are handing over in `note`. It is the last thing you say as the \
         agent in charge, and it is what they and the person will read."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent": { "type": "string", "description": "The agent to hand the conversation to, by name." },
                "note": { "type": "string", "description": "What you are handing over, and where it stands." }
            },
            "required": ["agent", "note"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        PermissionRequest::new("task", optional(args, "agent").unwrap_or_default()).with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        optional(args, "agent").map(|agent| format!("Hand over to {agent}"))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let wanted = required(&args, "agent")?;
        let note = optional(&args, "note");
        let (id, name) = match self.0.colleague(&wanted) {
            Ok(found) => found,
            Err(reason) => return Ok(refused(reason)),
        };

        match self.0.rooms.handover(
            &self.0.conversation,
            &id,
            self.0.agent_id.as_deref(),
            note.as_deref(),
        ) {
            Moved::No(reason) => Ok(refused(reason)),
            Moved::Yes { .. } => Ok(ToolOutcome {
                title: Some(format!("Handed over to {name}")),
                output: format!(
                    "This conversation is {name}'s now, and they speak next. Say anything you \
                     still have to say and then stop - you are no longer the agent it belongs to."
                ),
                metadata: Some(json!({ "group": true, "handover": id, "agent": name })),
                images: Vec::new(),
            }),
        }
    }
}

#[derive(Debug)]
pub struct PartTool(Seat);

#[async_trait]
impl Tool for PartTool {
    fn id(&self) -> &str {
        "part"
    }

    fn description(&self) -> &str {
        "Leave this conversation, having done what you were brought in for.\n\
         \n\
         The conversation goes back to the agent that brought you in, or to the one it \
         belongs to. What you said stays in the transcript and everyone can still read \
         it; you simply stop being asked.\n\
         \n\
         Leave when your part is genuinely finished. Do not leave because a turn ended - \
         the person may well have a follow-up, and being in the room costs nothing when \
         you are not speaking. The agent the conversation belongs to cannot leave it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string", "description": "What you did, in a line. The agent taking over reads this." },
                "to": { "type": "string", "description": "Who should carry on, by name. Omit to hand back to whoever brought you in." }
            },
            "required": ["summary"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        PermissionRequest::new("task", "leave").with_always("*")
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some("Leave the conversation".into())
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let summary = required(&args, "summary")?;
        let Some(me) = self.0.agent_id.as_deref() else {
            return Ok(refused(
                "This conversation is not seated to an agent, so there is nothing to leave.".into(),
            ));
        };

        let next = match optional(&args, "to") {
            None => None,
            Some(wanted) => match self.0.colleague(&wanted) {
                Ok(found) => Some(found),
                Err(reason) => return Ok(refused(reason)),
            },
        };

        match self.0.rooms.leave(
            &self.0.conversation,
            me,
            next.as_ref().map(|(id, _)| id.as_str()),
            Some(&summary),
        ) {
            Moved::No(reason) => Ok(refused(reason)),
            Moved::Yes { .. } => Ok(ToolOutcome {
                title: Some("Left the conversation".into()),
                output: format!(
                    "You are out of this conversation. {} Finish this turn without saying \
                     anything further.",
                    match &next {
                        Some((_, name)) => format!("{name} carries on."),
                        None => "It goes back to the agent it belongs to.".to_string(),
                    }
                ),
                metadata: Some(json!({
                    "group": true,
                    "left": me,
                    "to": next.as_ref().map(|(id, _)| id.clone()),
                })),
                images: Vec::new(),
            }),
        }
    }
}

/// The three tools, for a turn that has a room.
///
/// What the room forbids is left out of the list rather than refused when
/// called. A tool a model can see is a tool it will try, and three refusals in
/// a row is a turn spent finding out what the settings screen already knew.
pub fn tools_for(seat: Seat, permissions: &crate::group::Permissions) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    if permissions.can_invite {
        tools.push(Arc::new(InviteTool(seat.clone())));
    }
    if permissions.can_handover {
        tools.push(Arc::new(HandoverTool(seat.clone())));
    }
    if permissions.can_leave {
        tools.push(Arc::new(PartTool(seat)));
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::permission::Action;
    use inertia_core::tool::{Decision, PermissionGate};

    /// The gate is not what these tests are about: the group tools ask for
    /// permission the way every other tool does, and the registry is what runs
    /// that pipeline.
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

    /// A workspace with two agents in it.
    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        for (id, name, role) in [
            ("a", "Inertia Dev", "Engineer"),
            ("b", "Climate Scientist", "Researcher"),
        ] {
            inertia_store::collections::put(
                &layout,
                inertia_store::Collection::Agents,
                json!({ "id": id, "name": name, "role": role }),
            )
            .expect("the agent was written");
        }
        (dir, layout)
    }

    fn seat(layout: &Layout, rooms: Arc<Rooms>) -> Seat {
        Seat {
            rooms,
            layout: layout.clone(),
            conversation: "t1".into(),
            agent_id: Some("a".into()),
        }
    }

    fn ctx() -> ToolContext {
        ToolContext {
            root: std::path::PathBuf::from("."),
            session: SessionId::from_existing("t1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(Allow),
        }
    }

    #[tokio::test]
    async fn inviting_by_display_name_brings_the_right_agent_in() {
        let (_dir, layout) = workspace();
        let rooms = Arc::new(Rooms::default());
        rooms.open("t1", Some("a"), Default::default(), &[]);

        let tool = InviteTool(seat(&layout, rooms.clone()));
        let out = tool
            .execute(json!({ "agent": "Climate Scientist", "why": "the numbers" }), &ctx())
            .await
            .expect("the tool ran");

        assert_eq!(out.metadata.as_ref().and_then(|m| m.get("joined")), Some(&json!("b")));
        rooms.spoke("t1", "a", &[]);
        assert_eq!(rooms.seated("t1").and_then(|(active, _, _)| active).as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn the_handle_the_model_reads_in_its_own_prompt_resolves() {
        let (_dir, layout) = workspace();
        let rooms = Arc::new(Rooms::default());
        rooms.open("t1", Some("a"), Default::default(), &[]);

        let tool = InviteTool(seat(&layout, rooms));
        // `@climate-scientist` is exactly what the room section writes.
        let out = tool
            .execute(json!({ "agent": "@climate-scientist", "why": "ask them" }), &ctx())
            .await
            .expect("the tool ran");
        assert_eq!(out.metadata.and_then(|m| m.get("joined").cloned()), Some(json!("b")));
    }

    #[tokio::test]
    async fn a_filename_read_out_of_the_agents_folder_still_resolves() {
        let (_dir, layout) = workspace();
        let rooms = Arc::new(Rooms::default());
        rooms.open("t1", Some("a"), Default::default(), &[]);

        // What a model holds after listing the folder. Without the suffix
        // coming off, this is one wasted call and one `read` away from the id
        // it already has in its hand.
        let tool = InviteTool(seat(&layout, rooms));
        let out = tool
            .execute(json!({ "agent": "b.json", "why": "the numbers" }), &ctx())
            .await
            .expect("the tool ran");
        assert_eq!(out.metadata.and_then(|m| m.get("joined").cloned()), Some(json!("b")));
    }

    #[tokio::test]
    async fn a_name_nobody_has_is_answered_with_the_list_rather_than_not_found() {
        let (_dir, layout) = workspace();
        let rooms = Arc::new(Rooms::default());
        rooms.open("t1", Some("a"), Default::default(), &[]);

        let tool = InviteTool(seat(&layout, rooms));
        let out = tool
            .execute(json!({ "agent": "the designer", "why": "layout" }), &ctx())
            .await
            .expect("a refusal is still a result");

        assert!(out.output.contains("Climate Scientist"), "{}", out.output);
        // A refusal, not an error: the turn carries on.
        assert_eq!(
            out.metadata.and_then(|m| m.get("refused").cloned()),
            Some(json!(true))
        );
    }

    #[tokio::test]
    async fn handing_over_seats_the_other_agent_when_the_turn_ends() {
        let (_dir, layout) = workspace();
        let rooms = Arc::new(Rooms::default());
        rooms.open("t1", Some("a"), Default::default(), &[]);

        let tool = HandoverTool(seat(&layout, rooms.clone()));
        tool.execute(json!({ "agent": "b", "note": "your area" }), &ctx())
            .await
            .expect("the tool ran");

        rooms.spoke("t1", "a", &[]);
        let snapshot = rooms.snapshot("t1").expect("a room");
        assert_eq!(snapshot["active"], json!("b"));
        assert_eq!(snapshot["reason"], json!("handover"));
    }

    #[tokio::test]
    async fn the_agent_a_conversation_belongs_to_is_told_it_cannot_leave() {
        let (_dir, layout) = workspace();
        let rooms = Arc::new(Rooms::default());
        rooms.open("t1", Some("a"), Default::default(), &[]);

        let tool = PartTool(seat(&layout, rooms));
        let out = tool
            .execute(json!({ "summary": "done here" }), &ctx())
            .await
            .expect("a refusal is still a result");
        assert!(out.output.contains("cannot leave it"), "{}", out.output);
    }

    #[test]
    fn a_room_that_forbids_something_does_not_offer_the_tool_for_it() {
        let (_dir, layout) = workspace();
        let rooms = Arc::new(Rooms::default());
        let seat = seat(&layout, rooms);

        let all = tools_for(seat.clone(), &crate::group::Permissions::default());
        assert_eq!(all.len(), 3);

        let locked = tools_for(
            seat,
            &crate::group::Permissions {
                can_invite: false,
                can_handover: false,
                ..Default::default()
            },
        );
        assert_eq!(locked.iter().map(|t| t.id()).collect::<Vec<_>>(), vec!["part"]);
    }
}

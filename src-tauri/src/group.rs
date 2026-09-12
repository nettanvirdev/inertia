//! Who is in a conversation, and whose turn it is.
//!
//! Delegation is the other thing: a helper is given a brief, goes away, works
//! in a session of its own and comes back with an answer. This is a *room*.
//! Several agents in one conversation, reading the same transcript, taking
//! turns to speak into it, with a person in the middle who can cut in at any
//! point.
//!
//! The distinction that matters is the transcript. A delegated run has its own;
//! a group agent has none, and that is what makes the agents aware of each
//! other without any machinery for it: they read what everyone said because it
//! is the conversation they are in. Nothing is copied, summarised or forwarded,
//! which is also why the feature costs about what one agent costs plus the
//! turns the others actually take.
//!
//! The rules for whose turn it is live in `inertia_agent::floor`, which is
//! pure. This is the table that remembers, and refuses what the settings say an
//! agent may not do.
//!
//! In memory on purpose, like the main process's `team/group.cjs`: a room is
//! about a conversation that is happening, not a record of one that did. What
//! has to survive a restart - who is in the room - does, because the window
//! keeps the last roster on the thread and sends it back with every turn.

use std::collections::HashMap;
use std::sync::Mutex;

use inertia_agent::floor::{self, Floor, Joined, Outcome, Parted};
use serde_json::{json, Value};

/// How much of the who-said-what record is kept.
const MAX_LOG: usize = 200;

/// What a conversation is allowed to do on its own, before settings say
/// otherwise. The same numbers the settings screen shows.
#[derive(Debug, Clone)]
pub struct Permissions {
    pub can_invite: bool,
    pub can_handover: bool,
    pub can_leave: bool,
    pub max_agents: usize,
    pub max_hops: u32,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            can_invite: true,
            can_handover: true,
            can_leave: true,
            max_agents: floor::MAX_AGENTS,
            max_hops: floor::MAX_HOPS,
        }
    }
}

impl Permissions {
    /// Read from `settings/group.json`, which is a `serde_json::Value` like
    /// every other record. A key that is absent or the wrong type keeps the
    /// default rather than failing: a settings file half-written by an older
    /// build must not stop a conversation.
    pub fn from_settings(settings: &Value) -> Self {
        let base = Self::default();
        let Some(map) = settings.get("permissions") else {
            return base;
        };
        let flag = |key: &str, fallback: bool| map.get(key).and_then(Value::as_bool).unwrap_or(fallback);
        let count = |key: &str, fallback: u64| map.get(key).and_then(Value::as_u64).unwrap_or(fallback);
        Self {
            can_invite: flag("canInvite", base.can_invite),
            can_handover: flag("canHandover", base.can_handover),
            can_leave: flag("canLeave", base.can_leave),
            max_agents: count("maxAgents", base.max_agents as u64) as usize,
            max_hops: count("maxHops", base.max_hops as u64) as u32,
        }
    }
}

/// One line of the room's record, for the panel.
#[derive(Debug, Clone)]
struct Note {
    at: u64,
    kind: &'static str,
    agent_id: Option<String>,
    detail: Option<String>,
}

/// One conversation's room.
#[derive(Debug, Clone)]
pub struct Room {
    pub primary: Option<String>,
    pub roster: Vec<String>,
    pub floor: Floor,
    pub permissions: Permissions,
    /// What this turn's group tools asked for, held until the turn ends.
    ///
    /// Held rather than acted on when the tool is called, because the turn may
    /// invite two agents and then say something to both, and a floor settled
    /// mid-turn would seat the first one.
    pending: Option<Outcome>,
    log: Vec<Note>,
}

impl Room {
    fn note(&mut self, kind: &'static str, agent_id: Option<&str>, detail: Option<&str>) {
        self.log.push(Note {
            at: now_ms(),
            kind,
            agent_id: agent_id.map(str::to_string),
            detail: detail.map(str::to_string),
        });
        if self.log.len() > MAX_LOG {
            self.log.drain(..self.log.len() - MAX_LOG);
        }
    }

    fn room_view(&self) -> floor::Room<'_> {
        floor::Room {
            roster: &self.roster,
            primary: self.primary.as_deref(),
            max_hops: self.permissions.max_hops,
        }
    }

    /// What the room is doing, for the panel and for the window.
    ///
    /// `active` is who is expected to speak next, which is null when the floor
    /// is with the person - the state the conversation spends most of its time
    /// in, and the one the window reads to decide whether to start another
    /// turn on its own.
    pub fn snapshot(&self, conversation_id: &str) -> Value {
        json!({
            "conversationId": conversation_id,
            "primary": self.primary,
            "roster": self.roster,
            "active": self.floor.active,
            "queue": self.floor.queue,
            "hops": self.floor.hops,
            "reason": self.floor.reason,
            "permissions": {
                "canInvite": self.permissions.can_invite,
                "canHandover": self.permissions.can_handover,
                "canLeave": self.permissions.can_leave,
                "maxAgents": self.permissions.max_agents,
                "maxHops": self.permissions.max_hops,
            },
            "log": self.log.iter().rev().take(40).rev().map(|note| json!({
                "at": note.at,
                "kind": note.kind,
                "agentId": note.agent_id,
                "detail": note.detail,
            })).collect::<Vec<_>>(),
        })
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Every room, by conversation id.
#[derive(Debug, Default)]
pub struct Rooms {
    rooms: Mutex<HashMap<String, Room>>,
}

/// What came of asking an agent to join or to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moved {
    /// It happened. `already` marks an invitation to somebody who was already
    /// here - not a failure, because the agent asking means "they should answer
    /// next", and refusing it stalls a real conversation.
    Yes { already: bool },
    /// It did not, and this is what to tell the model. A refusal rather than an
    /// error: the agent asking is mid-turn and can work around being told the
    /// room is full, but not around having its turn ended.
    No(String),
}

impl Rooms {
    /// Start, or fetch, the room for a conversation.
    ///
    /// Called at the top of every group turn rather than once, because there is
    /// no setup moment this process can be sure it has seen: the window
    /// reloads, the app restarts, a routine sends into a thread nobody has
    /// opened. Passing the primary and the permissions every time is what makes
    /// those all work without a separate "has this been set up" question.
    ///
    /// A changed primary re-seats the room rather than emptying it - the person
    /// switched which agent the conversation belongs to, and the colleagues
    /// already in it did not stop existing.
    ///
    /// `remembered` is who the window last saw in the room. This table lives in
    /// memory, and without it the first restart between two messages brings the
    /// room back with only the primary in it - the agent that had just been
    /// invited never asked anything again.
    pub fn open(
        &self,
        conversation_id: &str,
        primary: Option<&str>,
        permissions: Permissions,
        remembered: &[String],
    ) {
        let mut rooms = self.lock();
        let room = rooms.entry(conversation_id.to_string()).or_insert_with(|| {
            let mut roster: Vec<String> = primary.map(str::to_string).into_iter().collect();
            for id in remembered {
                if !id.is_empty() && Some(id.as_str()) != primary && !roster.contains(id) {
                    roster.push(id.clone());
                }
            }
            let mut room = Room {
                primary: primary.map(str::to_string),
                roster,
                floor: floor::open_floor(primary),
                permissions: Permissions::default(),
                pending: None,
                log: Vec::new(),
            };
            if primary.is_some() {
                room.note("opened", primary, None);
            }
            room
        });

        room.permissions = permissions;
        if let Some(primary) = primary {
            if room.primary.as_deref() != Some(primary) {
                room.primary = Some(primary.to_string());
                if !room.roster.iter().any(|id| id == primary) {
                    room.roster.insert(0, primary.to_string());
                }
                room.note("primary", Some(primary), None);
            }
        }
    }

    /// Whatever the room looks like now, or `None` if this is not a group.
    pub fn snapshot(&self, conversation_id: &str) -> Option<Value> {
        self.lock()
            .get(conversation_id)
            .map(|room| room.snapshot(conversation_id))
    }

    /// Who the room says should speak, and who is in it.
    pub fn seated(&self, conversation_id: &str) -> Option<(Option<String>, Vec<String>, Permissions)> {
        self.lock().get(conversation_id).map(|room| {
            (
                room.floor.active.clone(),
                room.roster.clone(),
                room.permissions.clone(),
            )
        })
    }

    /// Bring an agent in.
    pub fn invite(&self, conversation_id: &str, agent_id: &str, by: Option<&str>, why: Option<&str>) -> Moved {
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(conversation_id) else {
            return Moved::No("This conversation is not a group.".into());
        };
        if !room.permissions.can_invite && by.is_some() {
            return Moved::No(
                "This conversation does not let agents bring others in. \
                 The person can add one themselves."
                    .into(),
            );
        }

        if room.roster.iter().any(|id| id == agent_id) {
            if by.is_some() {
                room.pending = Some(Outcome::Invite(agent_id.to_string()));
            }
            room.note("asked", Some(agent_id), why);
            return Moved::Yes { already: true };
        }

        match floor::join(&room.roster, agent_id, room.permissions.max_agents) {
            Joined::No(reason) => Moved::No(reason),
            Joined::Yes(roster) => {
                room.roster = roster;
                room.note("joined", Some(agent_id), why);
                // Whoever is speaking says who arrives next; the floor is
                // settled when the turn ends, not here.
                if by.is_some() {
                    room.pending = Some(Outcome::Invite(agent_id.to_string()));
                }
                Moved::Yes { already: false }
            }
        }
    }

    /// Hand the conversation over.
    ///
    /// Different from an invitation in one respect the person can see: the
    /// agent whose name and face the conversation carries changes. The one
    /// handing over stays in the room - it has not finished being useful, it
    /// has finished being in charge.
    pub fn handover(&self, conversation_id: &str, agent_id: &str, by: Option<&str>, why: Option<&str>) -> Moved {
        // Checked and released before `invite` is called: it takes the same
        // lock, and holding one across the call is the deadlock.
        let seated = {
            let rooms = self.lock();
            let Some(room) = rooms.get(conversation_id) else {
                return Moved::No("This conversation is not a group.".into());
            };
            if !room.permissions.can_handover && by.is_some() {
                return Moved::No("This conversation does not let agents hand it over.".into());
            }
            room.roster.iter().any(|id| id == agent_id)
        };

        if !seated {
            if let Moved::No(reason) = self.invite(conversation_id, agent_id, by, why) {
                return Moved::No(reason);
            }
        }

        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(conversation_id) else {
            return Moved::No("This conversation is not a group.".into());
        };
        // The conversation becomes theirs. This is the whole difference from
        // an invitation, it is what the tool's own description promises the
        // person will see - and until now nothing moved: the thread kept the
        // old agent's name and face, and, because the primary is the one agent
        // `floor::part` refuses to let go, the agent that had just handed over
        // could not then step out of a conversation it no longer ran.
        room.primary = Some(agent_id.to_string());
        room.pending = Some(Outcome::Handover(agent_id.to_string()));
        room.note("handover", Some(agent_id), why);
        Moved::Yes { already: false }
    }

    /// Step out, having said what there was to say.
    pub fn leave(&self, conversation_id: &str, agent_id: &str, to: Option<&str>, why: Option<&str>) -> Moved {
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(conversation_id) else {
            return Moved::No("This conversation is not a group.".into());
        };
        if !room.permissions.can_leave {
            return Moved::No("This conversation does not let agents leave it.".into());
        }
        match floor::part(&room.roster, agent_id, room.primary.as_deref()) {
            Parted::No(reason) => Moved::No(reason),
            Parted::Yes(roster) => {
                room.roster = roster;
                room.pending = Some(Outcome::Leave(to.map(str::to_string)));
                room.note("left", Some(agent_id), why);
                Moved::Yes { already: false }
            }
        }
    }

    /// The person has spoken.
    ///
    /// `mentioned` is the agents they named with `@`. Any that are not in the
    /// room are brought in first: naming somebody is asking for them, and
    /// refusing on the grounds that they have not been invited would be a
    /// strange thing to say to the person holding the invitation.
    pub fn asked(&self, conversation_id: &str, mentioned: &[String]) {
        for id in mentioned {
            let missing = self
                .lock()
                .get(conversation_id)
                .is_some_and(|room| !room.roster.iter().any(|seat| seat == id));
            if missing {
                self.invite(conversation_id, id, None, None);
            }
        }

        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(conversation_id) else {
            return;
        };
        room.pending = None;
        room.floor = floor::after_person(&room.floor, room.room_view(), mentioned);
    }

    /// Give one named agent the floor, without treating it as a new question.
    ///
    /// This is Retry. A reply that was interrupted belongs to one agent, and
    /// the person pressing the button on that bubble means "you, again" - not
    /// "here is a fresh message for the room". Asking again would reset the
    /// floor and put every agent back in the queue, which in a room of six is
    /// the whole debate a second time, at the price of the whole debate a
    /// second time.
    ///
    /// The queue and the hop count are left exactly as they were: the retried
    /// turn takes the place of the one that failed rather than adding to it.
    pub fn resume(&self, conversation_id: &str, agent_id: &str) {
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(conversation_id) else {
            return;
        };
        if !room.roster.iter().any(|id| id == agent_id) {
            match floor::join(&room.roster, agent_id, room.permissions.max_agents) {
                Joined::No(_) => return,
                Joined::Yes(roster) => {
                    room.roster = roster;
                    room.note("joined", Some(agent_id), Some("retry"));
                }
            }
        }
        // Whatever the failed turn had asked for is dropped with it.
        room.pending = None;
        room.floor.active = Some(agent_id.to_string());
        room.floor.reason = "retry".into();
    }

    /// An agent has finished speaking. The floor moves to whoever is next.
    ///
    /// The outcome is whatever the turn's last group tool recorded. `mentioned`
    /// is the one thing read out of what the agent wrote: the colleagues it
    /// named with `@`, who are asked next the way a person's `@` asks them -
    /// and brought in first if they were not here, when the room allows agents
    /// to invite. A turn that did neither simply finished, and the floor goes
    /// back to the person, which is the safe direction to be wrong in.
    pub fn spoke(&self, conversation_id: &str, agent_id: &str, mentioned: &[String]) {
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(conversation_id) else {
            return;
        };

        let outcome = room.pending.take().unwrap_or(Outcome::Done);

        let mut named: Vec<String> = Vec::new();
        for id in mentioned {
            if id.is_empty() || id == agent_id {
                continue;
            }
            if !room.roster.iter().any(|seat| seat == id) {
                if !room.permissions.can_invite {
                    continue;
                }
                match floor::join(&room.roster, id, room.permissions.max_agents) {
                    Joined::No(_) => continue,
                    Joined::Yes(roster) => {
                        room.roster = roster;
                        let detail = format!("named by {agent_id}");
                        room.note("joined", Some(id), Some(&detail));
                    }
                }
            }
            named.push(id.clone());
        }

        room.floor = floor::after_agent(
            &room.floor,
            &outcome,
            room.room_view(),
            &named,
            Some(agent_id),
        );
        let reason = room.floor.reason.clone();
        room.note("spoke", Some(agent_id), Some(&reason));
    }

    /// Forget a conversation's room, when the conversation itself is deleted.
    pub fn forget(&self, conversation_id: &str) {
        self.lock().remove(conversation_id);
    }

    /// A poisoned lock is recovered rather than propagated.
    ///
    /// The only thing a panic while holding this can leave behind is a room
    /// whose roster is half-updated, and a conversation that has to be started
    /// again is a far worse answer than one whose panel is a line out of date.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Room>> {
        self.rooms.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    fn opened() -> Rooms {
        let rooms = Rooms::default();
        rooms.open("t1", Some("a"), Permissions::default(), &[]);
        rooms
    }

    fn active(rooms: &Rooms) -> Option<String> {
        rooms.seated("t1").and_then(|(active, _, _)| active)
    }

    fn roster(rooms: &Rooms) -> Vec<String> {
        rooms.seated("t1").map(|(_, roster, _)| roster).unwrap_or_default()
    }

    #[test]
    fn handing_over_makes_the_conversation_theirs() {
        // The tool's own description promises the person will see the name and
        // face at the top of the thread change. Until the primary moves, they
        // do not: the thread keeps the old agent, and the window sends that old
        // agent back as the primary on the very next turn, undoing it.
        let rooms = opened();
        assert_eq!(rooms.snapshot("t1").unwrap()["primary"], json!("a"));

        assert!(matches!(rooms.handover("t1", "b", Some("a"), None), Moved::Yes { .. }));

        let room = rooms.snapshot("t1").unwrap();
        assert_eq!(room["primary"], json!("b"));
        assert_eq!(room["roster"], json!(["a", "b"]));

        // And the floor goes to them when the turn settles.
        rooms.spoke("t1", "a", &[]);
        assert_eq!(active(&rooms).as_deref(), Some("b"));
    }

    #[test]
    fn the_agent_that_handed_over_can_then_step_out() {
        // The whole shape of "give this to somebody else and leave": the
        // primary is the one agent `floor::part` refuses to release, so an
        // agent that handed over and was still the primary was stuck in a
        // conversation it no longer ran.
        let rooms = opened();
        rooms.handover("t1", "b", Some("a"), None);
        assert!(matches!(rooms.leave("t1", "a", None, None), Moved::Yes { .. }));
        assert_eq!(roster(&rooms), ids(&["b"]));

        // And the new one cannot leave, because somebody has to be here.
        assert!(matches!(rooms.leave("t1", "b", None, None), Moved::No(_)));
    }

    #[test]
    fn a_one_to_one_chat_becomes_a_room_by_handing_over() {
        // `open` is what `Seat::ensure_room` calls the first time an agent in
        // an ordinary chat reaches for `handover`. Nothing before that moment
        // has to know the conversation might become a room.
        let rooms = Rooms::default();
        assert!(rooms.snapshot("solo").is_none());
        assert!(matches!(
            rooms.handover("solo", "b", Some("a"), None),
            Moved::No(_)
        ));

        rooms.open("solo", Some("a"), Permissions::default(), &[]);
        assert!(matches!(rooms.handover("solo", "b", Some("a"), None), Moved::Yes { .. }));
        rooms.spoke("solo", "a", &[]);
        let room = rooms.snapshot("solo").unwrap();
        assert_eq!(room["active"], json!("b"));
        assert_eq!(room["primary"], json!("b"));
    }

    #[test]
    fn a_new_room_seats_the_primary_and_nobody_else() {
        let rooms = opened();
        assert_eq!(roster(&rooms), ids(&["a"]));
        assert_eq!(active(&rooms).as_deref(), Some("a"));
    }

    #[test]
    fn the_roster_the_window_remembers_survives_a_restart() {
        let rooms = Rooms::default();
        rooms.open("t1", Some("a"), Permissions::default(), &ids(&["b", "a"]));
        // "a" is not seated twice even though the window sent it back.
        assert_eq!(roster(&rooms), ids(&["a", "b"]));
    }

    #[test]
    fn an_invited_agent_speaks_next_when_the_turn_ends() {
        let rooms = opened();
        assert_eq!(rooms.invite("t1", "b", Some("a"), Some("second opinion")), Moved::Yes { already: false });
        // Still "a": the floor settles at the end of the turn, not mid-sentence.
        assert_eq!(active(&rooms).as_deref(), Some("a"));

        rooms.spoke("t1", "a", &[]);
        assert_eq!(active(&rooms).as_deref(), Some("b"));
    }

    #[test]
    fn inviting_somebody_already_here_asks_them_rather_than_failing() {
        let rooms = opened();
        rooms.invite("t1", "b", Some("a"), None);
        rooms.spoke("t1", "a", &[]);
        rooms.spoke("t1", "b", &[]);

        // "a" asks for "b" again. Refusing this is what stalls a real
        // conversation: the invite fails, the agent says "over to you" in
        // words, and words move nothing.
        assert_eq!(rooms.invite("t1", "b", Some("a"), None), Moved::Yes { already: true });
    }

    #[test]
    fn a_handover_brings_the_agent_in_if_it_was_not_here() {
        let rooms = opened();
        assert_eq!(rooms.handover("t1", "b", Some("a"), Some("your area now")), Moved::Yes { already: false });
        assert_eq!(roster(&rooms), ids(&["a", "b"]));
        rooms.spoke("t1", "a", &[]);
        assert_eq!(active(&rooms).as_deref(), Some("b"));
    }

    #[test]
    fn naming_a_colleague_in_the_reply_brings_them_in_and_asks_them() {
        let rooms = opened();
        rooms.spoke("t1", "a", &ids(&["b"]));
        assert_eq!(roster(&rooms), ids(&["a", "b"]));
        assert_eq!(active(&rooms).as_deref(), Some("b"));
    }

    #[test]
    fn a_room_that_forbids_inviting_refuses_with_a_reason_a_model_can_act_on() {
        let rooms = Rooms::default();
        rooms.open(
            "t1",
            Some("a"),
            Permissions {
                can_invite: false,
                ..Default::default()
            },
            &[],
        );
        match rooms.invite("t1", "b", Some("a"), None) {
            Moved::No(reason) => assert!(reason.contains("does not let agents")),
            Moved::Yes { .. } => panic!("the setting was ignored"),
        }
    }

    #[test]
    fn a_forbidden_room_still_lets_the_person_add_somebody() {
        let rooms = Rooms::default();
        rooms.open(
            "t1",
            Some("a"),
            Permissions {
                can_invite: false,
                ..Default::default()
            },
            &[],
        );
        // `by: None` is the person, and the person decides who is in their room.
        assert_eq!(rooms.invite("t1", "b", None, None), Moved::Yes { already: false });
    }

    #[test]
    fn the_person_naming_agents_settles_who_answers() {
        let rooms = opened();
        rooms.asked("t1", &ids(&["b"]));
        assert_eq!(roster(&rooms), ids(&["a", "b"]));
        assert_eq!(active(&rooms).as_deref(), Some("b"));
    }

    #[test]
    fn retry_seats_one_agent_without_putting_the_room_back_in_the_queue() {
        let rooms = opened();
        rooms.asked("t1", &[]);
        rooms.invite("t1", "b", None, None);
        rooms.asked("t1", &[]);
        let before = rooms.snapshot("t1").expect("a room");
        let queue = before["queue"].clone();

        rooms.resume("t1", "b");
        let after = rooms.snapshot("t1").expect("a room");
        assert_eq!(after["active"], json!("b"));
        assert_eq!(after["reason"], json!("retry"));
        assert_eq!(after["queue"], queue);
    }

    #[test]
    fn leaving_hands_the_conversation_back() {
        let rooms = opened();
        rooms.invite("t1", "b", None, None);
        assert_eq!(rooms.leave("t1", "b", None, Some("done")), Moved::Yes { already: false });
        rooms.spoke("t1", "b", &[]);
        assert_eq!(active(&rooms).as_deref(), Some("a"));
        assert_eq!(roster(&rooms), ids(&["a"]));
    }

    #[test]
    fn the_snapshot_carries_what_the_panel_draws() {
        let rooms = opened();
        rooms.invite("t1", "b", Some("a"), Some("why"));
        let snap = rooms.snapshot("t1").expect("a room");
        assert_eq!(snap["primary"], json!("a"));
        assert_eq!(snap["roster"], json!(["a", "b"]));
        assert_eq!(snap["permissions"]["maxAgents"], json!(floor::MAX_AGENTS));
        assert!(snap["log"].as_array().is_some_and(|log| !log.is_empty()));
    }

    #[test]
    fn settings_override_the_defaults_and_a_half_written_file_does_not() {
        let settings = json!({ "permissions": { "canLeave": false, "maxHops": 3 } });
        let permissions = Permissions::from_settings(&settings);
        assert!(!permissions.can_leave);
        assert_eq!(permissions.max_hops, 3);
        assert!(permissions.can_invite);
        assert_eq!(permissions.max_agents, floor::MAX_AGENTS);

        assert_eq!(
            Permissions::from_settings(&json!({})).max_hops,
            floor::MAX_HOPS
        );
    }

    #[test]
    fn a_conversation_that_is_not_a_group_refuses_rather_than_panicking() {
        let rooms = Rooms::default();
        match rooms.invite("nope", "b", Some("a"), None) {
            Moved::No(reason) => assert!(reason.contains("not a group")),
            Moved::Yes { .. } => panic!("a room appeared out of nowhere"),
        }
    }
}

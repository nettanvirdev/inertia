//! Who speaks next in a group conversation.
//!
//! A group chat has a roster of agents in it and, at any moment, one of them
//! holding the floor. This module decides who that is. It is deliberately plain
//! functions over plain data: no model call is made to choose a speaker,
//! because a round of "who should go next?" before every actual turn would
//! roughly double the cost of the feature to answer a question the last turn
//! has usually already answered by handing over, inviting, or stopping.
//!
//! The rules in one paragraph. What the person says wins: name agents with `@`
//! and those are the ones who speak, in the order they were named. Otherwise
//! the last turn decides - an agent that handed over gives the floor to whoever
//! it named, one that invited somebody lets the newcomer answer the invitation,
//! one that left gives the floor back to whoever brought it in, and one that
//! simply finished gives the floor back to the person. And running underneath
//! all of it is a hop count, because five agents that can each call the next
//! have no natural end and the bill is real.
//!
//! A port of the main process's `team/floor.cjs`, function for function. The
//! two shells share a renderer that drives the chain from the window, so a room
//! that seated a different agent here than it does there would show as the same
//! conversation behaving differently depending on which build it was opened in.

use serde::{Deserialize, Serialize};

/// How many agent turns may pass without the person, before the floor returns.
pub const MAX_HOPS: u32 = 8;

/// How many agents may be in one conversation at once.
pub const MAX_AGENTS: usize = 6;

/// An agent's name as it is written after `@`.
///
/// Lowercased and hyphenated so "Folder Organizer" is one token. The same rule
/// the composer uses for its pills, ported here because this side reads the
/// agents' own replies for these and both halves must agree on the spelling.
pub fn slug_of(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in name.trim().chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
            dash = false;
        } else if !out.is_empty() && !dash {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Whether the character before an `@` allows it to be a mention.
///
/// A handle glued to a letter or a digit is not a mention: an email address is
/// not asking anybody anything.
fn opens_mention(before: Option<char>) -> bool {
    match before {
        None => true,
        Some(ch) => !ch.is_alphanumeric(),
    }
}

/// The ids of the agents `text` names with `@`, in the order they are named,
/// without repeats.
///
/// The whole hyphenated word after the `@` is the handle, so `@nova-reyes` is
/// Nova Reyes and not Nova followed by a word. `roster` is `(id, name)` pairs;
/// the name is slugged the way the composer slugs it.
pub fn mentioned_agents<'a>(
    text: &str,
    roster: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<String> {
    if !text.contains('@') {
        return Vec::new();
    }

    let mut by_handle: Vec<(String, &str)> = Vec::new();
    for (id, name) in roster {
        let slug = slug_of(name);
        if id.is_empty() || slug.is_empty() {
            continue;
        }
        if !by_handle.iter().any(|(existing, _)| *existing == slug) {
            by_handle.push((slug, id));
        }
    }

    let chars: Vec<char> = text.chars().collect();
    let mut found: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        if !opens_mention(i.checked_sub(1).map(|p| chars[p])) {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '-') {
            end += 1;
        }
        if end == i + 1 {
            i += 1;
            continue;
        }
        let handle: String = chars[i + 1..end]
            .iter()
            .flat_map(|c| c.to_lowercase())
            .collect();
        let handle = handle.trim_end_matches('-');
        if let Some((_, id)) = by_handle.iter().find(|(slug, _)| slug == handle) {
            let id = (*id).to_string();
            if !found.contains(&id) {
                found.push(id);
            }
        }
        i = end;
    }
    found
}

/// What the last turn did with its final move.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Outcome {
    /// Name another agent and stop; the conversation is theirs now.
    Handover(String),
    /// Bring somebody in; they answer next, because being invited and then not
    /// being asked anything is a strange way to arrive.
    Invite(String),
    /// Step out; the floor goes back to whoever brought them in.
    Leave(Option<String>),
    /// Say something and stop.
    #[default]
    Done,
}

impl Outcome {
    /// The agent this outcome names, if it names one.
    fn wanted(&self) -> Option<&str> {
        match self {
            Self::Handover(id) | Self::Invite(id) => Some(id),
            Self::Leave(to) => to.as_deref(),
            Self::Done => None,
        }
    }
}

/// Where the floor is, and how it got there.
///
/// `reason` travels to the window, which shows it in the room panel - "handed
/// over", "hop limit" - so a chain that stopped can be told from one that is
/// still going.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Floor {
    /// Who is expected to speak next. `None` means the floor is with the
    /// person, which is the state a conversation spends most of its time in.
    pub active: Option<String>,
    pub queue: Vec<String>,
    pub hops: u32,
    pub reason: String,
    /// Who spoke last. Not the same as `active`: this is history, that is
    /// intent, and "yes, do that" has to land on the agent that just proposed
    /// it even though the floor had already gone back to the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
}

/// Nothing has been said yet: the primary has the floor and no hops are spent.
pub fn open_floor(primary: Option<&str>) -> Floor {
    Floor {
        active: primary.map(str::to_string),
        queue: Vec::new(),
        hops: 0,
        reason: "start".into(),
        last: None,
    }
}

/// What a roster and a primary say about a room, for the two functions below.
#[derive(Debug, Clone, Copy)]
pub struct Room<'a> {
    pub roster: &'a [String],
    pub primary: Option<&'a str>,
    pub max_hops: u32,
}

/// The floor after a person has spoken.
///
/// Resets the hop count - the cap exists to stop agents talking among
/// themselves indefinitely, and a person saying something is exactly the event
/// it was counting up to. Agents named in the message speak in the order they
/// were named. An unnamed message is to the room, and the room answers the way
/// a group chat does: whoever was speaking goes first and then everyone else
/// gets a turn, each reading what the ones before it said. That is what "you
/// two discuss it" means, and a version where only one agent ever answered an
/// open question left the others sitting there with their opinions.
///
/// Names that are not in the room are dropped rather than refused, because the
/// caller invites them first: by the time this runs they are either in the
/// roster or they were never joinable.
pub fn after_person(floor: &Floor, room: Room<'_>, mentioned: &[String]) -> Floor {
    let present: Vec<String> = mentioned
        .iter()
        .filter(|id| room.roster.contains(id))
        .cloned()
        .collect();

    if let Some((first, rest)) = present.split_first() {
        return Floor {
            active: Some(first.clone()),
            queue: rest.to_vec(),
            hops: 0,
            reason: "asked".into(),
            last: None,
        };
    }

    // Whoever was mid-sentence, else whoever spoke last, else the primary: what
    // makes "yes, do that" land on the agent that just proposed it.
    let first = [
        floor.active.as_deref(),
        floor.last.as_deref(),
        room.primary,
        room.roster.first().map(String::as_str),
    ]
    .into_iter()
    .flatten()
    .find(|id| room.roster.iter().any(|seat| seat == id))
    .map(str::to_string);

    let queue = room
        .roster
        .iter()
        .filter(|id| Some(id.as_str()) != first.as_deref())
        .cloned()
        .collect();

    Floor {
        active: first,
        queue,
        hops: 0,
        reason: "asked".into(),
        last: floor.last.clone(),
    }
}

/// The floor after an agent's turn.
///
/// `mentioned` is who the turn named with `@` in what it wrote. Naming a
/// colleague is asking them, the way it is in any group chat, so they speak
/// next - after whoever the tools chose, and after anyone the person queued.
/// This is the cheap way for agents to talk to each other: no tool call, just a
/// name in a sentence.
///
/// A queue set by the person outranks all of it. If they asked three agents,
/// the second one speaks when the first has finished, whatever the first
/// thought should happen next - it was not asked to run the meeting. What the
/// first asked for is not lost, though: it goes to the back of the line.
pub fn after_agent(
    floor: &Floor,
    outcome: &Outcome,
    room: Room<'_>,
    mentioned: &[String],
    speaker: Option<&str>,
) -> Floor {
    let hops = floor.hops + 1;
    let last = speaker.map(str::to_string).or_else(|| floor.last.clone());
    let seated = |id: &str| room.roster.iter().any(|seat| seat == id);

    // Who this turn wants next, in order: the tool it called, then the names it
    // wrote. Nobody speaks twice in a row on their own say-so.
    let mut asked: Vec<String> = Vec::new();
    if matches!(outcome, Outcome::Handover(_) | Outcome::Invite(_)) {
        if let Some(wanted) = outcome.wanted().filter(|id| seated(id)) {
            asked.push(wanted.to_string());
        }
    }
    for id in mentioned {
        if seated(id) && Some(id.as_str()) != speaker && !asked.contains(id) {
            asked.push(id.clone());
        }
    }

    let queued: Vec<String> = floor
        .queue
        .iter()
        .filter(|id| seated(id) && Some(id.as_str()) != speaker)
        .cloned()
        .collect();

    if !queued.is_empty() {
        // The person's list first, then this turn's, without repeats.
        let mut line = queued;
        for id in &asked {
            if !line.contains(id) {
                line.push(id.clone());
            }
        }
        // `queued` was non-empty on the way in, so this always matches; written
        // as a match rather than an index so it cannot become the one line that
        // panics if the filter above ever changes.
        if let Some((first, rest)) = line.split_first() {
            return Floor {
                active: Some(first.clone()),
                queue: rest.to_vec(),
                hops,
                reason: "asked".into(),
                last,
            };
        }
    }

    // Spent. Whatever the turn wanted next, it waits for the person - who can
    // say "carry on" and get another `max_hops` of it.
    if hops >= room.max_hops {
        return Floor {
            active: None,
            queue: Vec::new(),
            hops,
            reason: "hop-limit".into(),
            last,
        };
    }

    if let Some((first, rest)) = asked.split_first() {
        let reason = match outcome {
            Outcome::Handover(_) => "handover",
            Outcome::Invite(_) => "invited",
            _ => "mentioned",
        };
        return Floor {
            active: Some(first.clone()),
            queue: rest.to_vec(),
            hops,
            reason: reason.into(),
            last,
        };
    }

    if let Outcome::Leave(to) = outcome {
        let back = to
            .as_deref()
            .filter(|id| seated(id))
            .or(room.primary)
            .or_else(|| room.roster.first().map(String::as_str))
            .map(str::to_string);
        // The one who left is the one who was speaking, so this is a real
        // change of speaker even though nobody was named.
        return Floor {
            active: back,
            queue: Vec::new(),
            hops,
            reason: "left".into(),
            last,
        };
    }

    Floor {
        active: None,
        queue: Vec::new(),
        hops,
        reason: "done".into(),
        last,
    }
}

/// What happened when an agent was asked to join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Joined {
    Yes(Vec<String>),
    No(String),
}

/// Add an agent to the roster.
///
/// Refuses past the cap and refuses a duplicate, both with a reason a model can
/// act on rather than an error - an agent that tries to invite a seventh
/// colleague should be told the room is full and carry on, not have its turn
/// end.
pub fn join(roster: &[String], agent_id: &str, max: usize) -> Joined {
    if agent_id.is_empty() {
        return Joined::No("No agent was named.".into());
    }
    if roster.iter().any(|id| id == agent_id) {
        return Joined::No(format!("{agent_id} is already here."));
    }
    if roster.len() >= max {
        return Joined::No(format!(
            "This conversation already has {} agents in it, which is the limit. \
             Someone has to leave first.",
            roster.len()
        ));
    }
    let mut next = roster.to_vec();
    next.push(agent_id.to_string());
    Joined::Yes(next)
}

/// What happened when an agent was asked to leave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parted {
    Yes(Vec<String>),
    No(String),
}

/// Take an agent out of the roster.
///
/// The primary cannot leave. It is the one agent the conversation is guaranteed
/// to have, the one the person started talking to, and a room that empties
/// itself has nobody left to hand the floor back to.
pub fn part(roster: &[String], agent_id: &str, primary: Option<&str>) -> Parted {
    if primary == Some(agent_id) {
        return Parted::No(
            "You are the agent this conversation belongs to; you cannot leave it.".into(),
        );
    }
    if !roster.iter().any(|id| id == agent_id) {
        return Parted::No(format!("{agent_id} is not here."));
    }
    Parted::Yes(roster.iter().filter(|id| *id != agent_id).cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    fn room<'a>(roster: &'a [String], primary: &'a str) -> Room<'a> {
        Room {
            roster,
            primary: Some(primary),
            max_hops: MAX_HOPS,
        }
    }

    #[test]
    fn a_handle_is_the_name_lowercased_and_hyphenated() {
        assert_eq!(slug_of("Folder Organizer"), "folder-organizer");
        assert_eq!(slug_of("  Nova   Reyes  "), "nova-reyes");
        assert_eq!(slug_of("Climate Scientist!"), "climate-scientist");
    }

    #[test]
    fn mentions_are_read_in_order_without_repeats() {
        let roster = [("a", "Iris"), ("b", "Nova Reyes"), ("c", "Atlas")];
        assert_eq!(
            mentioned_agents("@nova-reyes and @iris - also @nova-reyes again", roster),
            ids(&["b", "a"])
        );
    }

    #[test]
    fn an_email_address_is_not_asking_anybody_anything() {
        let roster = [("a", "Iris")];
        assert!(mentioned_agents("write to tanvir@iris.example", roster).is_empty());
        assert_eq!(mentioned_agents("ask @iris please", roster), ids(&["a"]));
    }

    #[test]
    fn the_person_naming_agents_settles_the_order_and_resets_the_hops() {
        let roster = ids(&["a", "b", "c"]);
        let spent = Floor {
            hops: 7,
            ..open_floor(Some("a"))
        };
        let next = after_person(&spent, room(&roster, "a"), &ids(&["c", "b"]));
        assert_eq!(next.active.as_deref(), Some("c"));
        assert_eq!(next.queue, ids(&["b"]));
        assert_eq!(next.hops, 0);
    }

    #[test]
    fn an_unnamed_message_goes_round_the_room_starting_with_whoever_was_speaking() {
        let roster = ids(&["a", "b", "c"]);
        let floor = Floor {
            active: Some("b".into()),
            ..open_floor(Some("a"))
        };
        let next = after_person(&floor, room(&roster, "a"), &[]);
        assert_eq!(next.active.as_deref(), Some("b"));
        assert_eq!(next.queue, ids(&["a", "c"]));
    }

    #[test]
    fn a_handover_seats_the_agent_it_named() {
        let roster = ids(&["a", "b"]);
        let next = after_agent(
            &open_floor(Some("a")),
            &Outcome::Handover("b".into()),
            room(&roster, "a"),
            &[],
            Some("a"),
        );
        assert_eq!(next.active.as_deref(), Some("b"));
        assert_eq!(next.reason, "handover");
        assert_eq!(next.last.as_deref(), Some("a"));
    }

    #[test]
    fn naming_a_colleague_in_the_reply_asks_them_without_a_tool_call() {
        let roster = ids(&["a", "b"]);
        let next = after_agent(
            &open_floor(Some("a")),
            &Outcome::Done,
            room(&roster, "a"),
            &ids(&["b"]),
            Some("a"),
        );
        assert_eq!(next.active.as_deref(), Some("b"));
        assert_eq!(next.reason, "mentioned");
    }

    #[test]
    fn nobody_gives_themselves_the_floor_twice_in_a_row() {
        let roster = ids(&["a", "b"]);
        let next = after_agent(
            &open_floor(Some("a")),
            &Outcome::Done,
            room(&roster, "a"),
            &ids(&["a"]),
            Some("a"),
        );
        assert_eq!(next.active, None);
        assert_eq!(next.reason, "done");
    }

    #[test]
    fn the_persons_queue_outranks_what_the_turn_wanted_and_the_turns_wish_goes_to_the_back() {
        let roster = ids(&["a", "b", "c"]);
        let floor = Floor {
            active: Some("a".into()),
            queue: ids(&["b"]),
            ..open_floor(Some("a"))
        };
        let next = after_agent(
            &floor,
            &Outcome::Handover("c".into()),
            room(&roster, "a"),
            &[],
            Some("a"),
        );
        assert_eq!(next.active.as_deref(), Some("b"));
        assert_eq!(next.queue, ids(&["c"]));
    }

    #[test]
    fn the_hop_cap_hands_the_floor_back_to_the_person() {
        let roster = ids(&["a", "b"]);
        let floor = Floor {
            hops: MAX_HOPS - 1,
            ..open_floor(Some("a"))
        };
        let next = after_agent(
            &floor,
            &Outcome::Handover("b".into()),
            room(&roster, "a"),
            &[],
            Some("a"),
        );
        assert_eq!(next.active, None);
        assert_eq!(next.reason, "hop-limit");
    }

    #[test]
    fn leaving_hands_back_to_the_primary_when_nobody_was_named() {
        let roster = ids(&["a", "b"]);
        let next = after_agent(
            &open_floor(Some("b")),
            &Outcome::Leave(None),
            room(&roster, "a"),
            &[],
            Some("b"),
        );
        assert_eq!(next.active.as_deref(), Some("a"));
        assert_eq!(next.reason, "left");
    }

    #[test]
    fn a_turn_that_simply_finished_gives_the_floor_back_to_the_person() {
        let roster = ids(&["a", "b"]);
        let next = after_agent(
            &open_floor(Some("a")),
            &Outcome::Done,
            room(&roster, "a"),
            &[],
            Some("a"),
        );
        assert_eq!(next.active, None);
    }

    #[test]
    fn the_room_refuses_a_seventh_agent_with_a_reason_rather_than_an_error() {
        let roster = ids(&["a", "b", "c", "d", "e", "f"]);
        match join(&roster, "g", MAX_AGENTS) {
            Joined::No(reason) => assert!(reason.contains("limit")),
            Joined::Yes(_) => panic!("the cap was not enforced"),
        }
    }

    #[test]
    fn the_agent_a_conversation_belongs_to_cannot_leave_it() {
        let roster = ids(&["a", "b"]);
        match part(&roster, "a", Some("a")) {
            Parted::No(reason) => assert!(reason.contains("cannot leave")),
            Parted::Yes(_) => panic!("the primary was allowed to leave"),
        }
    }
}

//! Working out what, if anything, a conversation was worth remembering.
//!
//! This is the part that decides whether the memory store becomes useful or
//! becomes landfill, and the difference is almost entirely about restraint. A
//! pass that writes down five things per conversation produces a thousand
//! records in a month, retrieval gets worse with every one of them, and the
//! user ends up with a screen full of "ran the tests" to prune by hand. So the
//! prompt says what not to keep in as much detail as what to keep, the output
//! is capped, and returning nothing is stated as a good answer.
//!
//! Two other decisions worth naming:
//!
//!   · **Strict parsing, silent dropping.** A malformed reply stores nothing
//!     rather than failing. This runs in the background, after the person has
//!     stopped watching, so a failure has nowhere to be reported and must not
//!     take anything else down with it.
//!   · **The user's instructions go in the middle, not at the front.** They say
//!     what is worth keeping. The rules about format, one fact per record and
//!     never writing down a secret sit around them and are not negotiable - a
//!     custom instruction that could switch off the secret check would be a way
//!     to leak a key by editing a text box in settings.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::record;

/// Longer than this and the pass is reading a novel to write a sentence.
pub const MAX_TRANSCRIPT: usize = 24_000;

/// More than a handful from one conversation is a pass that has lost its nerve.
pub const MAX_MEMORIES: usize = 5;

/// The instructions a capture pass follows.
///
/// Editable in settings, because what is worth remembering is a matter of taste
/// and of what the person is doing: a team lead wants decisions and conventions,
/// someone debugging wants symptoms and dead ends. This is the default, and it
/// is written to be good enough that nobody has to touch it.
///
/// The shape of the advice matters more than its length. "Save important things"
/// produces a store full of nothing; naming what to skip is what keeps it small,
/// so the exclusions here are as specific as the inclusions - and the durability
/// test near the end is there because the exclusions alone were losing. What
/// gets written down without it is the afternoon's debugging: which endpoint
/// returned a 401, how many rows an API gave back. Every one of those is true,
/// none of them is worth carrying into a conversation next month, and a store
/// full of them is how recall stops working.
pub const DEFAULT_INSTRUCTIONS: &str = "\
Write down what someone returning to this project in a month would need to
know, and nothing else.

Worth remembering:
- Decisions, and the reasoning behind them. A decision without its reason
  gets reversed by the next person who sees it.
- Conventions this project follows that are not obvious from one file.
- What a component or subsystem is for, when the name does not say it.
- Bugs that were found and how they were actually caused.
- Work that is unfinished, and what is left to do.
- How this person wants to be worked with, when they say so.
- Corrections they made to you. Those are the most valuable of all.

Not worth remembering:
- Anything readable from the code, the file listing or the git history.
- What happened step by step. The outcome is the memory, not the path.
- Anything already written in AGENTS.md. It is read on every turn.
- Transient state: what is currently failing, what a command just printed,
  what is on screen.
- The shape of a third-party API you happened to call today - which parameter
  it wanted, which status it returned, how many rows came back. That is
  findable, it changes without telling you, and it is the single most common
  thing a pass like this writes down that nobody ever wants again.
- One run's numbers, ids, or which items in a batch failed.
- Pleasantries, restatements and anything you are unsure was ever agreed.

Before writing anything, ask: would this still be true in a month, and would
somebody be glad to be told it at the start of an unrelated conversation? If
either answer is no, leave it out.

One fact per memory. A memory holding three things cannot be corrected when
one of them changes. Write titles somebody could pick out of a list of a
hundred, and bodies that still make sense with none of this conversation
around them. Prefer saving nothing to saving something vague.";

const FORMAT: &str = "\
Reply with a single JSON object and nothing else. No prose, no code fence.

{\"memories\": [{\"title\": \"...\", \"body\": \"...\", \"scope\": \"project\" | \"global\", \"kind\": \"fact\" | \"preference\" | \"project\", \"tags\": [\"...\"], \"replaces\": \"id or null\"}], \"note\": \"...\"}

At most 5 memories, and fewer is better. An empty list is a good
answer and the right one for most conversations. Do not invent something to
fill the array.

`scope` is `project` when the fact is about this codebase and `global` when it
is about the person and would still be true in an unrelated project. When in
doubt use `project`: a fact filed too narrowly is merely missed, a fact filed
too widely turns up in somebody else's work.

The bar for `global` is higher than for `project`, because a global memory is
read in every conversation the person ever has. It is for how they want to be
worked with, what they call things, and standing decisions that hold
everywhere - not for anything you learned about today's codebase.

`replaces` names an existing memory this one corrects, from the list you were
given. Use it whenever the new fact contradicts an old one. Never write a
memory that disagrees with an existing memory without replacing it.

Never write down a password, key, token, or the contents of a credential. Say
what a secret is for and where it lives, never its value.";

const NOTE: &str = "\
`note` is a short handover for whoever opens this project next, replacing the
one below rather than adding to it. Two hundred words at most: what this
project is, what was being worked on, what was decided, and what is left
unfinished. Write it so somebody with no memory of this conversation could
pick the work up. If nothing meaningful happened, repeat the previous note
unchanged.";

/// What to send, assembled.
#[derive(Debug, Clone)]
pub struct Prompt {
    pub system: String,
    pub text: String,
}

/// One memory the pass already knows about: enough to notice a contradiction
/// and name what it replaces, and no more. The bodies would cost more than they
/// are worth here.
#[derive(Debug, Clone)]
pub struct Known {
    pub id: String,
    pub title: String,
}

pub fn known_of(records: &[Value]) -> Vec<Known> {
    records
        .iter()
        .map(|row| Known {
            id: record::text(row, "id"),
            title: record::text(row, "title"),
        })
        .filter(|one| !one.id.is_empty())
        .collect()
}

pub fn build_prompt(
    transcript: &str,
    instructions: &str,
    existing: &[Known],
    note: &str,
    folder: Option<&str>,
) -> Prompt {
    let mut parts: Vec<String> = vec![
        "You are deciding what is worth remembering from a coding session, so that a\n\
         conversation weeks from now starts out knowing it."
            .into(),
    ];

    let own = instructions.trim();
    parts.push(if own.is_empty() { DEFAULT_INSTRUCTIONS.into() } else { own.into() });

    if let Some(folder) = folder.filter(|f| !f.is_empty()) {
        parts.push(format!("The project is at {folder}."));
    }

    if !existing.is_empty() {
        let mut block = String::from(
            "You already remember these. Do not write any of them down again; name one\n\
             in `replaces` if the conversation corrected it.\n",
        );
        for one in existing.iter().take(60) {
            block.push_str(&format!("\n- {}: {}", one.id, one.title));
        }
        parts.push(block);
    }

    if !note.trim().is_empty() {
        parts.push(format!("The current handover note, to be replaced:\n\n{note}"));
    }

    parts.push(FORMAT.into());
    parts.push(NOTE.into());

    // The end of the conversation, not the beginning: what was concluded is
    // worth more than how it opened, and the opening is usually the part that
    // got summarised away already.
    let text = if transcript.len() > MAX_TRANSCRIPT {
        let start = transcript
            .char_indices()
            .rev()
            .map(|(at, _)| at)
            .find(|at| transcript.len() - at >= MAX_TRANSCRIPT)
            .unwrap_or(0);
        transcript[start..].to_string()
    } else {
        transcript.to_string()
    };

    Prompt { system: parts.join("\n\n"), text }
}

/// The first JSON object in a reply, tolerating a fence or a sentence around it.
pub fn parse_reply(raw: &str) -> Option<Value> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&raw[start..=end]).ok()
}

/// One memory the pass decided to write, and what it corrects.
#[derive(Debug, Clone)]
pub struct Harvested {
    pub record: Value,
    /// The id of an existing memory this one supersedes.
    pub replaces: Option<String>,
}

/// Turn a parsed reply into records worth storing.
///
/// Everything here is a filter, and every filter drops rather than repairs.
/// This runs unattended: a half-understood record written anyway is one the
/// person finds later on their Memory screen with no idea where it came from.
pub fn harvest(
    parsed: &Value,
    folder: Option<&str>,
    agent_id: Option<&str>,
    existing_ids: &BTreeSet<String>,
) -> Vec<Harvested> {
    let rows = parsed
        .get("memories")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    let mut out = Vec::new();
    for row in rows.iter().take(MAX_MEMORIES) {
        let title: String = record::text(row, "title").chars().take(120).collect();
        let body: String = record::text(row, "body").chars().take(2000).collect();
        if title.is_empty() || body.is_empty() {
            continue;
        }

        let scope = match row.get("scope").and_then(Value::as_str) {
            Some(given) if record::is_scope(given) => given,
            _ => "project",
        };
        let kind = match row.get("kind").and_then(Value::as_str) {
            Some(given) if record::is_kind(given) => given,
            _ => record::DEFAULT_KIND,
        };
        let mut tags = record::tags(row);
        tags.truncate(6);

        let mut built = serde_json::json!({
            "title": title,
            "body": body,
            "kind": kind,
            "tags": tags,
            "scope": scope,
            "source": "learned",
        });
        if scope == "project" {
            if let Some(folder) = folder {
                built["folder"] = Value::String(folder.to_string());
            }
        }
        if let Some(agent) = agent_id {
            built["agentId"] = Value::String(agent.to_string());
        }

        // The same check the tool makes, applied again here because this path
        // never went through the tool. A key written by a background pass is
        // worse than one written by a visible call: nobody saw it happen.
        if !record::is_storable(&built) {
            continue;
        }

        // A replacement must name something that actually exists, or it
        // silently becomes an insert and the contradiction survives.
        let replaces = row
            .get("replaces")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty() && existing_ids.contains(*id))
            .map(str::to_string);

        out.push(Harvested { record: built, replaces });
    }
    out
}

/// The handover note, if the pass wrote a usable one.
pub fn harvest_note(parsed: &Value) -> Option<String> {
    let note = parsed.get("note").and_then(Value::as_str)?.trim();
    if note.chars().count() < 20 {
        return None;
    }
    Some(note.chars().take(4000).collect())
}

/// The name a person calls this project.
pub fn basename(folder: &str) -> String {
    let trimmed = folder.trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or_default();
    if last.is_empty() {
        "this project".into()
    } else {
        last.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_users_instructions_replace_the_default_but_not_the_rules() {
        let prompt = build_prompt("hi", "Only remember cats.", &[], "", None);
        assert!(prompt.system.contains("Only remember cats."));
        assert!(!prompt.system.contains("Worth remembering:"));
        // The rules that are not negotiable are still there.
        assert!(prompt.system.contains("Never write down a password"));
        assert!(prompt.system.contains("single JSON object"));
    }

    #[test]
    fn an_empty_instruction_falls_back_to_the_default() {
        let prompt = build_prompt("hi", "   ", &[], "", None);
        assert!(prompt.system.contains("Worth remembering:"));
    }

    #[test]
    fn what_is_already_known_is_listed_so_it_is_not_written_twice() {
        let known = vec![Known { id: "a".into(), title: "Deploys go to fly.io".into() }];
        let prompt = build_prompt("hi", "", &known, "", Some("D:/work/api"));
        assert!(prompt.system.contains("- a: Deploys go to fly.io"));
        assert!(prompt.system.contains("The project is at D:/work/api."));
    }

    #[test]
    fn only_the_end_of_a_long_conversation_is_read() {
        let long = format!("{}TAIL", "x".repeat(MAX_TRANSCRIPT * 2));
        let prompt = build_prompt(&long, "", &[], "", None);
        assert!(prompt.text.len() <= MAX_TRANSCRIPT + 4);
        assert!(prompt.text.ends_with("TAIL"));
    }

    #[test]
    fn a_reply_wrapped_in_prose_or_a_fence_still_parses() {
        let parsed = parse_reply("Sure!\n```json\n{\"memories\": []}\n```\nDone.");
        assert_eq!(parsed.unwrap(), json!({ "memories": [] }));
        assert!(parse_reply("no json here").is_none());
        assert!(parse_reply("{ not json }").is_none());
    }

    #[test]
    fn a_record_without_a_title_or_a_body_is_dropped() {
        let parsed = json!({ "memories": [
            { "title": "", "body": "x" },
            { "title": "x", "body": "" },
            { "title": "good", "body": "kept" },
        ]});
        let out = harvest(&parsed, None, None, &BTreeSet::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].record["title"], "good");
    }

    #[test]
    fn a_record_carrying_a_credential_is_dropped_here_too() {
        let parsed = json!({ "memories": [
            { "title": "The key", "body": "ghp_0123456789abcdefghij" },
        ]});
        assert!(harvest(&parsed, None, None, &BTreeSet::new()).is_empty());
    }

    #[test]
    fn a_replacement_must_name_a_memory_that_exists() {
        let parsed = json!({ "memories": [
            { "title": "a", "body": "b", "replaces": "gone" },
            { "title": "c", "body": "d", "replaces": "here" },
        ]});
        let existing: BTreeSet<String> = ["here".to_string()].into_iter().collect();
        let out = harvest(&parsed, None, None, &existing);
        assert_eq!(out[0].replaces, None);
        assert_eq!(out[1].replaces.as_deref(), Some("here"));
    }

    #[test]
    fn an_unknown_scope_or_kind_falls_back_rather_than_being_stored() {
        let parsed = json!({ "memories": [
            { "title": "a", "body": "b", "scope": "team", "kind": "invention" },
        ]});
        let out = harvest(&parsed, Some("D:/work/api"), None, &BTreeSet::new());
        assert_eq!(out[0].record["scope"], "project");
        assert_eq!(out[0].record["kind"], "fact");
        assert_eq!(out[0].record["folder"], "D:/work/api");
    }

    #[test]
    fn a_global_memory_claims_no_folder() {
        let parsed = json!({ "memories": [{ "title": "a", "body": "b", "scope": "global" }] });
        let out = harvest(&parsed, Some("D:/work/api"), None, &BTreeSet::new());
        assert!(out[0].record.get("folder").is_none());
    }

    #[test]
    fn no_more_than_a_handful_from_one_conversation() {
        let many: Vec<Value> = (0..12)
            .map(|n| json!({ "title": format!("t{n}"), "body": "b" }))
            .collect();
        let out = harvest(&json!({ "memories": many }), None, None, &BTreeSet::new());
        assert_eq!(out.len(), MAX_MEMORIES);
    }

    #[test]
    fn a_note_too_short_to_mean_anything_is_not_a_note() {
        assert!(harvest_note(&json!({ "note": "ok" })).is_none());
        assert!(harvest_note(&json!({})).is_none());
        assert!(harvest_note(&json!({ "note": "x".repeat(40) })).is_some());
    }

    #[test]
    fn a_project_is_named_the_way_a_person_names_it() {
        assert_eq!(basename("D:\\work\\api\\"), "api");
        assert_eq!(basename("/home/me/web"), "web");
        assert_eq!(basename(""), "this project");
    }
}

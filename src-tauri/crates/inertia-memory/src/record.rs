//! What a memory is, and which ones are in force.
//!
//! A memory is one durable fact worth carrying between conversations: how this
//! person wants to be worked with, what a project is for, a decision and the
//! reason behind it. One fact per record, deliberately - a record holding three
//! things cannot be superseded when one of them changes, and superseding is
//! most of what keeps a memory store from rotting into a pile of
//! contradictions.
//!
//! Records are `Value`, not a struct. The same folder is written by the Electron
//! app and read by this one, records carry fields neither side has an opinion
//! about, and a typed mirror is how a save quietly deletes the half it does not
//! know. Everything here reads what it needs and leaves the rest alone.

use serde_json::{Map, Value};

/// The kinds, matching what the Memory screen renders.
///
/// Deliberately few. A vocabulary a person has to think about before writing
/// anything down is a vocabulary that stops them writing anything down, and the
/// kind is a filter in one dropdown, not a taxonomy.
pub const KINDS: &[&str] = &[
    "fact",
    "preference",
    "contact",
    "project",
    "credential-note",
    // One per project, rewritten rather than added to: what was being worked on
    // and what is unfinished. It is the memory that makes a new conversation in
    // a familiar folder start somewhere rather than nowhere.
    "handover",
];

pub const DEFAULT_KIND: &str = "fact";

/// Who a memory is about.
///
/// `global` is about the person: how they want to be worked with, what they
/// call things, decisions that hold everywhere. `project` is about one folder.
///
/// The distinction is on the record rather than in a setting, and that is the
/// whole of the isolation guarantee. A setting could be changed, or read wrongly
/// in one of the several places that ask; a memory that says which folder it
/// belongs to can only ever be wrong about itself.
pub const SCOPES: &[&str] = &["global", "project"];

/// Below this, the screen draws a memory as low confidence.
pub const LOW_CONFIDENCE: f64 = 0.5;

/// A memory nobody has recalled in this long reads as stale.
pub const STALE_AFTER_DAYS: i64 = 21;

/// How much of the prompt every turn may spend on memories it was not asked for.
///
/// Two and a half kilobytes is roughly twenty entries at a title and a line
/// each. Picked against a measurement rather than a feeling.
pub const INJECT_BUDGET_BYTES: usize = 2560;

/// How many entries are worth advertising however small they are.
pub const INJECT_MAX: usize = 24;

/// One line of advertising copy, however long the body is.
pub const MAX_SUMMARY: usize = 160;

pub fn is_kind(value: &str) -> bool {
    KINDS.contains(&value)
}

pub fn is_scope(value: &str) -> bool {
    SCOPES.contains(&value)
}

/// A string field, trimmed, or `""`.
pub fn text(record: &Value, field: &str) -> String {
    record
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// A record's tags, as written.
pub fn tags(record: &Value) -> Vec<String> {
    record
        .get("tags")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(|tag| tag.trim().to_string())
                .filter(|tag| !tag.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn scope_of(record: &Value) -> &str {
    match record.get("scope").and_then(Value::as_str) {
        Some("project") => "project",
        _ => "global",
    }
}

pub fn is_pinned(record: &Value) -> bool {
    record
        .get("pinned")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Waiting for approval is not the same as being known.
pub fn is_pending(record: &Value) -> bool {
    record
        .get("pending")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/* -- folders -------------------------------------------------------------- */

/// Compare two folder paths as the same place.
///
/// Windows makes this less obvious than it looks: the same folder legitimately
/// arrives as `D:\work\api` and `D:/work/api`, and the drive letter's case
/// varies with who produced the string. A memory that fails to match its own
/// project because of a slash would be invisible with no way for a user to see
/// why, which is worse than a memory that is merely missing.
pub fn same_folder(a: &str, b: &str) -> bool {
    fn clean(value: &str) -> String {
        value
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase()
    }
    let left = clean(a);
    !left.is_empty() && left == clean(b)
}

/// The memories that apply while working in `folder`.
///
/// Global memories always apply. A project memory applies only in its own
/// project. A project memory whose folder is missing - written before this
/// existed, or hand-edited - is treated as global, because the alternative is a
/// record that exists, is visible on screen, and can never be recalled.
pub fn applicable<'a>(records: &'a [Value], folder: Option<&str>) -> Vec<&'a Value> {
    records
        .iter()
        .filter(|record| {
            if scope_of(record) != "project" {
                return true;
            }
            let own = text(record, "folder");
            own.is_empty() || folder.is_some_and(|here| same_folder(&own, here))
        })
        .collect()
}

/// Memories that are actually in force.
///
/// A memory waiting for approval exists, is visible on the Memory screen, and
/// is believed by nothing. Somebody who asked to review what gets written down
/// has not agreed to it being used in the meantime, and using it would make the
/// review a formality performed after the fact.
pub fn approved(records: Vec<&Value>) -> Vec<&Value> {
    records
        .into_iter()
        .filter(|record| !is_pending(record))
        .collect()
}

/* -- rendering ------------------------------------------------------------ */

/// The one-liner shown in the prompt and nowhere else.
///
/// Prefers an explicit `description`, because a person who wrote one meant it,
/// and otherwise takes the opening of the body. Whitespace collapses: this sits
/// inside an element in a prompt and a stray newline reads as structure.
pub fn summarise(record: &Value, limit: usize) -> String {
    fn flatten(value: &str) -> String {
        value.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    let written = flatten(&text(record, "description"));
    let line = if written.is_empty() {
        flatten(&text(record, "body"))
    } else {
        written
    };
    if line.is_empty() {
        return String::new();
    }
    if line.chars().count() <= limit {
        return line;
    }
    let cut: String = line.chars().take(limit).collect();
    format!("{}...", cut.trim_end())
}

/* -- hygiene -------------------------------------------------------------- */

/// Things that must never be written into a memory.
///
/// A capture pass reads whatever went past in a conversation, and what goes past
/// in a coding conversation includes keys. A memory is injected into every later
/// prompt and rendered on a screen, so a key that lands in one leaks quietly and
/// repeatedly. These catch the shapes that are unmistakable rather than trying
/// to be clever: a false positive costs one memory nobody needed, a false
/// negative costs a credential.
///
/// Hand-written rather than compiled regexes. Every one of these is "a fixed
/// prefix followed by n opaque characters", which is a scan, and a scan is
/// easier to read here than the escaped pattern that would replace it.
pub fn looks_secret(text: &str) -> bool {
    /// A run of at least `min` characters from the set a token is drawn from.
    fn opaque_run(rest: &str, min: usize, extra: &str) -> bool {
        rest.chars()
            .take_while(|c| c.is_ascii_alphanumeric() || extra.contains(*c))
            .count()
            >= min
    }

    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        return true;
    }

    // A JSON web token: three opaque segments separated by dots.
    for start in text.match_indices("ey").map(|(at, _)| at) {
        let parts: Vec<&str> = text[start..]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .split('.')
            .collect();
        if parts.len() >= 3 && parts.iter().take(3).all(|part| part.len() >= 10) {
            return true;
        }
    }

    const PREFIXES: &[(&str, usize, &str)] = &[
        ("sk-", 16, "_-"),
        ("ghp_", 16, ""),
        ("gho_", 16, ""),
        ("ghu_", 16, ""),
        ("ghs_", 16, ""),
        ("ghr_", 16, ""),
        ("xoxb-", 10, "-"),
        ("xoxa-", 10, "-"),
        ("xoxp-", 10, "-"),
        ("xoxs-", 10, "-"),
        ("AKIA", 16, ""),
        ("AIza", 30, "_-"),
    ];
    for (prefix, min, extra) in PREFIXES {
        for (at, _) in text.match_indices(prefix) {
            if opaque_run(&text[at + prefix.len()..], *min, extra) {
                return true;
            }
        }
    }

    // A named field with a long opaque value after it, which is what a
    // hand-written config line looks like.
    let lower = text.to_lowercase();
    for name in [
        "api_key", "api-key", "apikey", "secret", "token", "password", "passwd",
    ] {
        for (at, _) in lower.match_indices(name) {
            let rest = &text[at + name.len()..];
            let rest = rest.trim_start_matches(['"', '\'']);
            let Some(marker) = rest.find([':', '=']) else {
                continue;
            };
            if rest[..marker].trim().is_empty()
                && opaque_run(
                    rest[marker + 1..]
                        .trim_start()
                        .trim_start_matches(['"', '\'']),
                    16,
                    "_-/+.",
                )
            {
                return true;
            }
        }
    }

    false
}

/// True when this memory is safe to store. Checked before writing, never after.
pub fn is_storable(record: &Value) -> bool {
    let title = text(record, "title");
    let body = text(record, "body");
    !title.is_empty() && !body.is_empty() && !looks_secret(&format!("{title}\n{body}"))
}

/* -- duplicates ----------------------------------------------------------- */

/// How much two pieces of text are the same, from 0 to 1.
///
/// Overlap of the words that carry meaning, ignoring order and repetition.
/// Crude, and right for this: what it has to catch is the same fact written
/// twice, which is a case where the words really are the same.
pub fn similarity(a: &str, b: &str) -> f64 {
    use std::collections::BTreeSet;
    let left: BTreeSet<String> = crate::rank::terms(a).into_iter().collect();
    let right: BTreeSet<String> = crate::rank::terms(b).into_iter().collect();
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let shared = left.intersection(&right).count() as f64;
    shared / (left.len() as f64 + right.len() as f64 - shared)
}

/// Titles compared the way a person would: case and punctuation do not count.
fn normalised_title(record: &Value) -> String {
    text(record, "title")
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// How alike two memories must be before they are the same memory.
///
/// High on purpose, and the asymmetry is deliberate: a near-duplicate that slips
/// through is untidy, while a wrong merge silently destroys a fact. "Allergic to
/// eggs" and "allergic to peanuts" are both true and score about a third,
/// nowhere near this.
pub const SAME_MEMORY: f64 = 0.8;

/// The memory a new one would be a repeat of, if there is one.
///
/// Scoped first: two projects are allowed to hold the same sentence, because in
/// each one it is a fact about that project. Then an identical title is taken as
/// the same fact even when the body has changed - a body that changed under the
/// same title is a correction, and keeping both is how a store starts
/// contradicting itself.
pub fn find_duplicate<'a>(records: &'a [Value], candidate: &Value) -> Option<&'a Value> {
    let title = normalised_title(candidate);
    let text_of = |record: &Value| format!("{} {}", text(record, "title"), text(record, "body"));
    let candidate_text = text_of(candidate);
    let scope = scope_of(candidate);
    let folder = text(candidate, "folder");

    records.iter().find(|record| {
        if scope_of(record) != scope {
            return false;
        }
        if scope == "project" && !same_folder(&text(record, "folder"), &folder) {
            return false;
        }
        if !title.is_empty() && normalised_title(record) == title {
            return true;
        }
        similarity(&candidate_text, &text_of(record)) >= SAME_MEMORY
    })
}

/// An older memory brought up to date by a newer one.
///
/// The wording is replaced, because the newer text is the correction. Everything
/// a memory has *accumulated* is kept: its tags, its pin, when it was first
/// written, and how often it has been useful. Overwriting those was a quiet bug
/// in the app this is ported from - saving the same fact again with no tags
/// wiped the tags somebody had added, so a memory got worse every time it was
/// confirmed.
pub fn merged(previous: &Value, next: &Value) -> Value {
    let mut out: Map<String, Value> = previous.as_object().cloned().unwrap_or_default();
    if let Some(fields) = next.as_object() {
        for (key, value) in fields {
            out.insert(key.clone(), value.clone());
        }
    }

    // Unioned rather than replaced: a new pass that happens to think of one tag
    // should add it, not throw the other four away.
    let mut tags: Vec<String> = Vec::new();
    for tag in tags_of(previous).into_iter().chain(tags_of(next)) {
        if !tags.iter().any(|held| held.eq_ignore_ascii_case(&tag)) {
            tags.push(tag);
        }
    }
    tags.truncate(8);
    out.insert("tags".into(), Value::from(tags));

    // Both of these are the user's, not the writer's: a pin stays pinned, and a
    // memory keeps the day it was first written down.
    out.insert(
        "pinned".into(),
        Value::Bool(is_pinned(previous) || is_pinned(next)),
    );
    let created = previous
        .get("createdAt")
        .or_else(|| next.get("createdAt"))
        .cloned();
    match created {
        Some(at) => {
            out.insert("createdAt".into(), at);
        }
        None => {
            out.remove("createdAt");
        }
    }
    out.insert(
        "useCount".into(),
        previous.get("useCount").cloned().unwrap_or(Value::from(0)),
    );

    Value::Object(out)
}

fn tags_of(record: &Value) -> Vec<String> {
    tags(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_folder_matches_itself_however_it_was_spelled() {
        assert!(same_folder("D:\\work\\api", "d:/work/api/"));
        assert!(!same_folder("D:/work/api", "D:/work/web"));
        // Nothing matches nothing: a memory with no folder must not claim every
        // folder by accident.
        assert!(!same_folder("", ""));
    }

    #[test]
    fn a_project_memory_stays_in_its_project() {
        let records = [
            json!({ "id": "g", "scope": "global" }),
            json!({ "id": "a", "scope": "project", "folder": "D:/work/api" }),
            json!({ "id": "b", "scope": "project", "folder": "D:/work/web" }),
            // Written before scoping existed, so it applies everywhere rather
            // than nowhere.
            json!({ "id": "old", "scope": "project" }),
        ];
        let here: Vec<&str> = applicable(&records, Some("d:\\work\\api"))
            .iter()
            .map(|r| r["id"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(here, ["g", "a", "old"]);
    }

    #[test]
    fn a_memory_waiting_for_approval_is_not_in_force() {
        let records = [json!({ "id": "a" }), json!({ "id": "b", "pending": true })];
        let live = approved(records.iter().collect());
        assert_eq!(live.len(), 1);
        assert_eq!(live[0]["id"], "a");
    }

    #[test]
    fn the_summary_prefers_a_description_and_collapses_whitespace() {
        let record = json!({ "description": "  one\n  line  ", "body": "the long body" });
        assert_eq!(summarise(&record, 160), "one line");
        assert_eq!(summarise(&json!({ "body": "a b" }), 160), "a b");
        assert_eq!(summarise(&json!({}), 160), "");
    }

    #[test]
    fn a_long_summary_is_cut_with_an_ellipsis() {
        let record = json!({ "body": "x".repeat(200) });
        let line = summarise(&record, 20);
        assert_eq!(line.chars().count(), 23);
        assert!(line.ends_with("..."));
    }

    #[test]
    fn the_shapes_that_are_unmistakably_credentials() {
        assert!(looks_secret("the key is sk-abcdefghijklmnopqrstuv"));
        assert!(looks_secret("ghp_0123456789abcdefghij"));
        assert!(looks_secret("xoxb-1234567890-abc"));
        assert!(looks_secret("AKIAIOSFODNN7EXAMPLE"));
        assert!(looks_secret("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(looks_secret(
            "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM.SflKxwRJSMeKKF2"
        ));
        assert!(looks_secret("api_key: 0123456789abcdefghij"));
        assert!(looks_secret(r#""token" = "0123456789abcdefghij""#));
    }

    #[test]
    fn what_a_memory_may_legitimately_say_about_a_secret() {
        // Saying where a key lives is the whole point of the credential-note
        // kind, and none of these may be refused.
        assert!(!looks_secret(
            "The Composio API key is in the secret store as COMPOSIO_API_KEY."
        ));
        assert!(!looks_secret("sk-"));
        assert!(!looks_secret("Set the token in settings."));
    }

    #[test]
    fn a_record_needs_a_title_a_body_and_no_credential() {
        assert!(is_storable(&json!({ "title": "t", "body": "b" })));
        assert!(!is_storable(&json!({ "title": "", "body": "b" })));
        assert!(!is_storable(&json!({ "title": "t", "body": "  " })));
        assert!(!is_storable(
            &json!({ "title": "t", "body": "ghp_0123456789abcdefghij" })
        ));
    }

    #[test]
    fn the_same_fact_written_twice_is_found() {
        let records = [
            json!({ "id": "a", "title": "Deploys go to fly.io", "body": "not to render" }),
            json!({ "id": "b", "title": "Tests run with vitest", "body": "not jest" }),
        ];
        // The same title, punctuated differently.
        let twin = find_duplicate(
            &records,
            &json!({ "title": "deploys go to fly.io!", "body": "x" }),
        );
        assert_eq!(twin.unwrap()["id"], "a");

        assert!(
            find_duplicate(&records, &json!({ "title": "Something else", "body": "x" })).is_none()
        );
    }

    #[test]
    fn two_projects_may_hold_the_same_sentence() {
        let records = [json!({
            "id": "a", "title": "Uses pnpm", "body": "not npm",
            "scope": "project", "folder": "D:/work/api"
        })];
        let elsewhere = json!({
            "title": "Uses pnpm", "body": "not npm",
            "scope": "project", "folder": "D:/work/web"
        });
        assert!(find_duplicate(&records, &elsewhere).is_none());
    }

    #[test]
    fn merging_keeps_what_a_memory_accumulated() {
        let previous = json!({
            "id": "a", "title": "old", "body": "old body",
            "tags": ["one", "two"], "pinned": true,
            "createdAt": "2024-01-01T00:00:00.000Z", "useCount": 7
        });
        let next = json!({ "title": "new", "body": "new body", "tags": ["Two", "three"] });
        let out = merged(&previous, &next);

        // The wording is the correction.
        assert_eq!(out["title"], "new");
        assert_eq!(out["body"], "new body");
        // Everything the record earned survives it.
        assert_eq!(out["tags"], json!(["one", "two", "three"]));
        assert_eq!(out["pinned"], true);
        assert_eq!(out["createdAt"], "2024-01-01T00:00:00.000Z");
        assert_eq!(out["useCount"], 7);
    }
}

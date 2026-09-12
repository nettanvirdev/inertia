//! The SKILL.md codec.
//!
//! A skill is a folder, not a row: `skills/<slug>/SKILL.md` holds the metadata
//! in frontmatter and the instructions as the body, and anything else the skill
//! needs (scripts, references) sits beside it. That shape is deliberate - a
//! skill stays readable and editable in any editor, and stays portable to any
//! other tool that reads the same convention, instead of being locked inside
//! our JSON.

use serde_json::{json, Map, Value};

use crate::frontmatter::{self, Value as Meta};

/// The keys this codec owns. Everything else in the frontmatter is the user's.
pub const KNOWN: &[&str] = &[
    "name",
    "description",
    "enabled",
    "tags",
    "version",
    "createdAt",
    "updatedAt",
];

/// What is wrong with this skill, in the user's terms.
///
/// A skill fails silently by nature: a missing description means no agent ever
/// reaches for it, and nothing about the app looks broken. So the faults are
/// collected here, once, and both the screen and the loader read the same list
/// rather than each deciding for itself what counts as usable.
fn problems(meta: &std::collections::BTreeMap<String, Meta>, body: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let text = |key: &str| {
        meta.get(key)
            .map(Meta::to_text)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    if text("name").is_empty() {
        problems.push("No name in the frontmatter, so the folder name is being used instead.".into());
    }
    if text("description").is_empty() {
        problems.push("No description, so an agent has nothing to decide by.".into());
    }
    if body.trim().is_empty() {
        problems.push("Nothing below the frontmatter, so there are no instructions to load.".into());
    }
    problems
}

/// File on disk -> the record shape the renderer works with.
pub fn to_record(id: &str, text: &str) -> Value {
    let document = frontmatter::parse(text);
    let meta = &document.meta;

    let string = |key: &str| meta.get(key).map(Meta::to_text).unwrap_or_default();

    // Keys we do not know about are carried through rather than dropped. The
    // form only edits the fields it shows, and a save that quietly deleted an
    // `allowed-tools:` the user hand-wrote would be the app losing their work.
    let extra: Map<String, Value> = meta
        .iter()
        .filter(|(key, _)| !KNOWN.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.to_json()))
        .collect();

    let tags = match meta.get("tags") {
        Some(Meta::List(items)) => Value::Array(items.iter().map(Meta::to_json).collect()),
        _ => Value::Array(vec![]),
    };

    let name = string("name");
    let optional = |value: String| {
        if value.is_empty() {
            Value::Null
        } else {
            Value::String(value)
        }
    };

    json!({
        "id": id,
        "name": if name.is_empty() { id.to_string() } else { name },
        "description": string("description"),
        // Absent means on. A skill in the folder is a skill the user put there.
        "enabled": meta.get("enabled").and_then(Meta::as_bool).unwrap_or(true),
        "tags": tags,
        "version": string("version"),
        "instructions": document.body,
        "createdAt": optional(string("createdAt")),
        "updatedAt": optional(string("updatedAt")),
        "extra": Value::Object(extra),
        "problems": problems(meta, &document.body),
    })
}

/// A folder in `skills/` with no SKILL.md in it.
///
/// It gets a record rather than being skipped, because a folder that silently
/// does nothing is the hardest kind of broken to find. Disabled, so no agent is
/// offered a skill that has no instructions.
pub fn missing_record(id: &str) -> Value {
    json!({
        "id": id,
        "name": id,
        "description": "",
        "enabled": false,
        "tags": [],
        "version": "",
        "instructions": "",
        "createdAt": Value::Null,
        "updatedAt": Value::Null,
        "extra": {},
        "missing": true,
        "problems": [format!("No SKILL.md in skills/{id}, so there is nothing to load.")],
    })
}

/// Record -> the file. Ordered so the important keys read first.
pub fn to_file(record: &Value) -> String {
    let get = |key: &str| record.get(key).cloned().unwrap_or(Value::Null);

    let mut meta: Vec<(String, Meta)> = Vec::new();
    let mut push = |key: &str, value: Value| {
        if let Some(converted) = Meta::from_json(&value) {
            meta.push((key.to_string(), converted));
        }
    };

    push("name", get("name"));
    push("description", get("description"));
    // Written explicitly rather than dropped when true: `enabled: false` is the
    // whole point of the field, and a reader should not have to know that its
    // absence means on.
    push(
        "enabled",
        Value::Bool(get("enabled").as_bool().unwrap_or(true)),
    );
    push("tags", get("tags"));
    push("version", get("version"));
    push("createdAt", get("createdAt"));
    push("updatedAt", get("updatedAt"));

    if let Some(Value::Object(extra)) = record.get("extra") {
        for (key, value) in extra {
            push(key, value.clone());
        }
    }

    let body = record
        .get("instructions")
        .and_then(Value::as_str)
        .unwrap_or_default();

    frontmatter::serialize(&meta, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nname: Customize Inertia\ndescription: Set it up.\nenabled: true\ntags:\n  - inertia\n  - setup\nversion: \"2\"\n---\n\n# Setting up\n\nBody text.";

    #[test]
    fn a_real_skill_file_parses() {
        let record = to_record("customize-inertia", SAMPLE);
        assert_eq!(record["name"], "Customize Inertia");
        assert_eq!(record["description"], "Set it up.");
        assert_eq!(record["enabled"], true);
        assert_eq!(record["tags"][0], "inertia");
        assert_eq!(record["version"], "2");
        assert!(record["instructions"].as_str().unwrap().starts_with("# Setting up"));
        assert!(record["problems"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_skill_with_no_frontmatter_falls_back_to_its_folder_name() {
        let record = to_record("hand-made", "Just instructions.");
        assert_eq!(record["name"], "hand-made");
        assert_eq!(record["instructions"], "Just instructions.");
        assert_eq!(record["problems"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_skill_with_nothing_below_the_fence_says_so() {
        let record = to_record("empty", "---\nname: Empty\ndescription: x\n---\n");
        let problems = record["problems"].as_array().unwrap();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].as_str().unwrap().contains("no instructions"));
    }

    #[test]
    fn absent_enabled_means_on() {
        let record = to_record("x", "---\nname: X\n---\n\nBody.");
        assert_eq!(record["enabled"], true);
    }

    /// The form only edits the fields it shows; a save that dropped a
    /// hand-written key would be the app losing the user's work.
    #[test]
    fn unknown_frontmatter_keys_survive_a_round_trip() {
        let record = to_record(
            "x",
            "---\nname: X\ndescription: d\nallowed-tools: [read, write]\n---\n\nBody.",
        );
        assert_eq!(record["extra"]["allowed-tools"][0], "read");

        let written = to_file(&record);
        assert!(written.contains("allowed-tools: [read, write]"), "{written}");
        assert_eq!(to_record("x", &written)["extra"]["allowed-tools"][1], "write");
    }

    #[test]
    fn a_record_round_trips_through_the_file() {
        let record = to_record("customize-inertia", SAMPLE);
        let again = to_record("customize-inertia", &to_file(&record));
        assert_eq!(again["name"], record["name"]);
        assert_eq!(again["tags"], record["tags"]);
        assert_eq!(again["instructions"], record["instructions"]);
        assert_eq!(again["version"], record["version"]);
    }

    #[test]
    fn a_disabled_skill_stays_disabled() {
        let record = to_record("x", "---\nname: X\ndescription: d\nenabled: false\n---\n\nB.");
        assert_eq!(record["enabled"], false);
        assert!(to_file(&record).contains("enabled: false"));
    }

    #[test]
    fn a_missing_record_is_disabled_and_explains_itself() {
        let record = missing_record("hollow");
        assert_eq!(record["enabled"], false);
        assert_eq!(record["missing"], true);
        assert!(record["problems"][0].as_str().unwrap().contains("hollow"));
    }
}

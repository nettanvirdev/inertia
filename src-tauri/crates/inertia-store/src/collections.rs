//! One CRUD surface over every collection in the workspace.
//!
//! Adding a resource type - a new plugin kind, a new history log - should cost
//! a line in [`crate::layout`] and nothing here. That is why this file knows
//! about folder-backed versus file-backed records and not about MCP servers or
//! skills: the codec varies, the operations never do.
//!
//! Records are files, one per row. No index, no database. A folder you can
//! read, diff and copy is the whole product requirement, and a 200-row list is
//! far below the point where a stat-per-file starts to matter.
//!
//! # Why records are `serde_json::Value` and not typed structs
//!
//! Because the window owns the record shape, and a typed mirror here can only
//! be wrong in two directions. It rejects what it does not recognise - a real
//! thread carrying `"mode": "group"` was quarantined as damaged by a Rust enum
//! that knew only three modes - and it drops what it does not model, so saving
//! that thread from Rust would have silently deleted the room roster the user
//! had built. Round-tripping the whole document costs nothing and cannot lose
//! a field that was added on the other side of the bridge.

use serde_json::{Map, Value};

use crate::fsx;
use crate::layout::{Collection, Layout};

pub type Result<T> = std::result::Result<T, CollectionError>;

#[derive(Debug, thiserror::Error)]
pub enum CollectionError {
    #[error("Unknown collection: {0}")]
    UnknownCollection(String),
    #[error("Invalid id: {0}")]
    InvalidId(String),
    #[error("No such {collection}: {id}")]
    NotFound { collection: String, id: String },
    #[error("{0} already exists.")]
    AlreadyExists(String),
    #[error(transparent)]
    Store(#[from] fsx::StoreError),
}

/// Resolves the name the renderer used.
pub fn collection(name: &str) -> Result<Collection> {
    Collection::from_key(name).ok_or_else(|| CollectionError::UnknownCollection(name.to_string()))
}

/// Ids are filenames, so the constraint that matters is that one stays a single
/// path segment: no separator, no traversal, no root of its own.
///
/// Deliberately not a slug test. Ids for records this app creates are slugs,
/// but a skill is a folder the user is invited to make by hand, and `My Skill`
/// with a space in it is a perfectly good one. Refusing it here used to throw
/// out of `list`, which took every other skill down with it - one hand-made
/// folder and the agent had no skills at all, with nothing on screen to say
/// why.
pub fn check_id(id: &str) -> Result<&str> {
    let bad = id.trim().is_empty()
        || id == "."
        || id == ".."
        || id.contains('\0')
        || id.contains('/')
        || id.contains('\\')
        || id.contains(':');
    if bad {
        return Err(CollectionError::InvalidId(id.to_string()));
    }
    Ok(id)
}

/// Every id in a collection.
pub fn ids(layout: &Layout, collection: Collection) -> Vec<String> {
    let dir = layout.collection_dir(collection);
    if collection.is_folder_backed() {
        return fsx::list_dir(
            &dir,
            fsx::ListOptions {
                only_dirs: true,
                ..Default::default()
            },
        );
    }
    fsx::list_dir(
        &dir,
        fsx::ListOptions {
            ext: Some("json"),
            ..Default::default()
        },
    )
    .into_iter()
    .map(|name| name.trim_end_matches(".json").to_string())
    .collect()
}

/// One record, or `None` when there is no such id.
pub fn get(layout: &Layout, collection: Collection, id: &str) -> Result<Option<Value>> {
    check_id(id)?;

    if collection.is_folder_backed() {
        let folder = layout.collection_dir(collection).join(id);
        let file = folder.join("SKILL.md");
        if let Some(text) = fsx::read_text(&file) {
            return Ok(Some(crate::skills::to_record(id, &text)));
        }
        // A folder that is there but has no SKILL.md is a different answer from
        // no folder at all, and the user needs to be told which one they have.
        return Ok(if folder.is_dir() {
            Some(crate::skills::missing_record(id))
        } else {
            None
        });
    }

    let path = layout.record(collection, id);
    let (value, outcome) = fsx::read_json::<Value>(&path);
    match outcome {
        fsx::ReadOutcome::Absent => Ok(None),
        fsx::ReadOutcome::Damaged { .. } => Ok(None),
        fsx::ReadOutcome::Loaded => {
            let mut object = match value {
                Value::Object(map) => map,
                // A file holding a bare array or string is not a record. Read
                // as absent rather than wrapped into something invented.
                _ => return Ok(None),
            };
            // The id is the filename, so the filename wins. A copied file whose
            // `id` field still names the original would otherwise be
            // unreachable: listed under one id, saving under another.
            object.insert("id".into(), Value::String(id.to_string()));
            Ok(Some(Value::Object(object)))
        }
    }
}

/// Every record in a collection.
///
/// One unreadable record costs the user that record, not the collection: a read
/// that gave up here would empty the whole screen and, for skills, strip every
/// skill out of the system prompt.
pub fn list(layout: &Layout, collection: Collection) -> Vec<Value> {
    ids(layout, collection)
        .into_iter()
        .filter_map(|id| get(layout, collection, &id).ok().flatten())
        .collect()
}

/// A readable id derived from the record's own name, made unique by suffix.
///
/// Readable because these are filenames the user will see in the folder - a
/// `github-issues.json` says what it is; a `mcp_01JQ...` says only that a
/// machine wrote it.
fn unique_id(layout: &Layout, collection: Collection, seed: &str) -> String {
    let base = fsx::slugify(seed, collection.singular());
    let taken = ids(layout, collection);
    if !taken.iter().any(|id| id == &base) {
        return base;
    }
    for n in 2..500 {
        let candidate = format!("{base}-{n}");
        if !taken.iter().any(|id| id == &candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", jiff::Timestamp::now().as_millisecond())
}

fn now() -> String {
    // The renderer writes `new Date().toISOString()`, which is UTC with
    // milliseconds and a `Z`. Matching it keeps a folder's timestamps
    // comparable as plain strings no matter which side wrote them.
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn string_field(record: &Map<String, Value>, key: &str) -> Option<String> {
    record.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Create or replace.
///
/// The caller sends the whole record; we own only the three fields it should
/// never have to maintain by hand.
pub fn put(layout: &Layout, collection: Collection, input: Value) -> Result<Value> {
    let mut record = match input {
        Value::Object(map) => map,
        _ => Map::new(),
    };

    let id = match string_field(&record, "id").filter(|id| !id.trim().is_empty()) {
        Some(id) => check_id(&id)?.to_string(),
        None => {
            let seed = string_field(&record, "name")
                .or_else(|| string_field(&record, "title"))
                .unwrap_or_default();
            unique_id(layout, collection, &seed)
        }
    };

    let previous = get(layout, collection, &id)?;
    let created = previous
        .as_ref()
        .and_then(|p| p.get("createdAt"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| string_field(&record, "createdAt"))
        .unwrap_or_else(now);

    record.insert("id".into(), Value::String(id.clone()));
    record.insert("createdAt".into(), Value::String(created));
    record.insert("updatedAt".into(), Value::String(now()));

    let value = Value::Object(record);

    if collection.is_folder_backed() {
        let folder = layout.collection_dir(collection).join(&id);
        fsx::ensure_dir(&folder)?;
        fsx::write_text(&folder.join("SKILL.md"), &crate::skills::to_file(&value))?;
    } else {
        fsx::write_json(&layout.record(collection, &id), &value)?;
    }
    Ok(value)
}

/// Shallow merge onto what is stored. A missing record is an error, not a
/// create: the caller asked to change something that was supposed to be there,
/// and inventing it would hide the fact that it was not.
pub fn patch(layout: &Layout, collection: Collection, id: &str, changes: Value) -> Result<Value> {
    check_id(id)?;
    let current = get(layout, collection, id)?.ok_or_else(|| CollectionError::NotFound {
        collection: collection.key().to_string(),
        id: id.to_string(),
    })?;

    let mut merged = match current {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Value::Object(changes) = changes {
        for (key, value) in changes {
            merged.insert(key, value);
        }
    }
    merged.insert("id".into(), Value::String(id.to_string()));
    put(layout, collection, Value::Object(merged))
}

pub fn remove(layout: &Layout, collection: Collection, id: &str) -> Result<()> {
    check_id(id)?;
    if collection.is_folder_backed() {
        fsx::remove_dir_all(&layout.collection_dir(collection).join(id))?;
    } else {
        fsx::remove(&layout.record(collection, id))?;
    }
    Ok(())
}

/// Renaming a record means renaming its file, so it gets an explicit operation
/// rather than being a `put` the caller has to follow with a `remove`.
pub fn rename(layout: &Layout, collection: Collection, id: &str, next_id: &str) -> Result<Value> {
    check_id(id)?;
    check_id(next_id)?;
    if id == next_id {
        return get(layout, collection, id)?.ok_or_else(|| CollectionError::NotFound {
            collection: collection.key().to_string(),
            id: id.to_string(),
        });
    }
    if get(layout, collection, next_id)?.is_some() {
        return Err(CollectionError::AlreadyExists(next_id.to_string()));
    }
    let record = get(layout, collection, id)?.ok_or_else(|| CollectionError::NotFound {
        collection: collection.key().to_string(),
        id: id.to_string(),
    })?;

    let mut map = match record {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert("id".into(), Value::String(next_id.to_string()));
    let saved = put(layout, collection, Value::Object(map))?;
    remove(layout, collection, id)?;
    Ok(saved)
}

/* -- documents: the single files that are not collections ---------------- */

/// Reads a document, falling back to what the caller supplied.
pub fn read_document(layout: &Layout, document: crate::layout::Document, fallback: Value) -> Value {
    let (value, outcome) = fsx::read_json::<Value>(&layout.document(document));
    match outcome {
        fsx::ReadOutcome::Loaded => value,
        _ => fallback,
    }
}

pub fn write_document(
    layout: &Layout,
    document: crate::layout::Document,
    value: &Value,
) -> Result<()> {
    fsx::write_json(&layout.document(document), value)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        layout.scaffold().unwrap();
        (dir, layout)
    }

    #[test]
    fn a_record_round_trips() {
        let (_dir, layout) = workspace();
        let saved = put(
            &layout,
            Collection::Agents,
            json!({ "id": "agent-x", "name": "X" }),
        )
        .unwrap();
        assert_eq!(saved["id"], "agent-x");
        let read = get(&layout, Collection::Agents, "agent-x")
            .unwrap()
            .unwrap();
        assert_eq!(read["name"], "X");
    }

    /// The bug this whole module exists for: a real thread carrying a mode no
    /// Rust enum knew about was quarantined as damaged, and every field the
    /// struct did not model would have been dropped on the next save.
    #[test]
    fn a_record_keeps_fields_rust_knows_nothing_about() {
        let (_dir, layout) = workspace();
        let original = json!({
            "id": "thr-1",
            "mode": "group",
            "room": { "roster": ["a", "b"], "hops": 1 },
            "context": { "used": 215460, "window": 1000000 },
            "temporary": false,
        });
        put(&layout, Collection::Threads, original.clone()).unwrap();

        let read = get(&layout, Collection::Threads, "thr-1").unwrap().unwrap();
        assert_eq!(read["mode"], "group");
        assert_eq!(read["room"]["roster"][1], "b");
        assert_eq!(read["context"]["used"], 215460);

        // And a round trip through patch keeps them too.
        patch(
            &layout,
            Collection::Threads,
            "thr-1",
            json!({ "unread": 2 }),
        )
        .unwrap();
        let again = get(&layout, Collection::Threads, "thr-1").unwrap().unwrap();
        assert_eq!(again["room"]["roster"][1], "b");
        assert_eq!(again["unread"], 2);
    }

    #[test]
    fn an_id_is_derived_from_the_name_when_absent() {
        let (_dir, layout) = workspace();
        let saved = put(
            &layout,
            Collection::Agents,
            json!({ "name": "Code Review" }),
        )
        .unwrap();
        assert_eq!(saved["id"], "code-review");
    }

    #[test]
    fn a_clashing_derived_id_gains_a_suffix() {
        let (_dir, layout) = workspace();
        put(&layout, Collection::Agents, json!({ "name": "Review" })).unwrap();
        let second = put(&layout, Collection::Agents, json!({ "name": "Review" })).unwrap();
        assert_eq!(second["id"], "review-2");
    }

    #[test]
    fn a_record_with_no_name_falls_back_to_the_collections_own_word() {
        let (_dir, layout) = workspace();
        let saved = put(&layout, Collection::Routines, json!({})).unwrap();
        assert_eq!(saved["id"], "routine");
    }

    #[test]
    fn created_at_survives_a_replace() {
        let (_dir, layout) = workspace();
        let first = put(&layout, Collection::Agents, json!({ "id": "a" })).unwrap();
        let second = put(&layout, Collection::Agents, json!({ "id": "a" })).unwrap();
        assert_eq!(first["createdAt"], second["createdAt"]);
    }

    /// A copied file whose `id` field still names the original would otherwise
    /// be listed under one id and saved under another.
    #[test]
    fn the_filename_wins_over_a_stale_id_field() {
        let (_dir, layout) = workspace();
        fsx::write_json(
            &layout.record(Collection::Agents, "copy"),
            &json!({ "id": "original", "name": "X" }),
        )
        .unwrap();
        let read = get(&layout, Collection::Agents, "copy").unwrap().unwrap();
        assert_eq!(read["id"], "copy");
    }

    #[test]
    fn patching_something_absent_is_an_error_not_a_create() {
        let (_dir, layout) = workspace();
        assert!(patch(&layout, Collection::Agents, "ghost", json!({})).is_err());
        assert!(get(&layout, Collection::Agents, "ghost").unwrap().is_none());
    }

    #[test]
    fn ids_may_not_escape_their_collection() {
        for bad in ["..", ".", "", "a/b", "a\\b", "C:x"] {
            assert!(check_id(bad).is_err(), "{bad} should be refused");
        }
    }

    /// A skill folder made by hand is a perfectly good record, and refusing the
    /// space used to take every other skill down with it.
    #[test]
    fn an_id_with_a_space_is_allowed() {
        assert!(check_id("My Skill").is_ok());
    }

    #[test]
    fn renaming_moves_the_record_and_frees_the_old_id() {
        let (_dir, layout) = workspace();
        put(
            &layout,
            Collection::Agents,
            json!({ "id": "old", "name": "X" }),
        )
        .unwrap();
        let moved = rename(&layout, Collection::Agents, "old", "new").unwrap();
        assert_eq!(moved["id"], "new");
        assert!(get(&layout, Collection::Agents, "old").unwrap().is_none());
        assert_eq!(
            get(&layout, Collection::Agents, "new").unwrap().unwrap()["name"],
            "X"
        );
    }

    #[test]
    fn renaming_onto_an_existing_id_is_refused() {
        let (_dir, layout) = workspace();
        put(&layout, Collection::Agents, json!({ "id": "a" })).unwrap();
        put(&layout, Collection::Agents, json!({ "id": "b" })).unwrap();
        assert!(rename(&layout, Collection::Agents, "a", "b").is_err());
    }

    #[test]
    fn one_unreadable_record_does_not_empty_the_collection() {
        let (_dir, layout) = workspace();
        put(&layout, Collection::Agents, json!({ "id": "good" })).unwrap();
        fsx::write_text(&layout.record(Collection::Agents, "bad"), "{ not json").unwrap();
        let all = list(&layout, Collection::Agents);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0]["id"], "good");
    }

    #[test]
    fn a_skill_is_stored_as_a_folder_with_frontmatter() {
        let (_dir, layout) = workspace();
        put(
            &layout,
            Collection::Skills,
            json!({
                "id": "code-review",
                "name": "Code review",
                "description": "Review a diff.",
                "instructions": "Read it closely.",
            }),
        )
        .unwrap();

        let file = layout
            .collection_dir(Collection::Skills)
            .join("code-review/SKILL.md");
        let text = fsx::read_text(&file).unwrap();
        assert!(text.starts_with("---\n"), "{text}");
        assert!(text.contains("name: Code review"));
        assert!(text.contains("Read it closely."));

        let read = get(&layout, Collection::Skills, "code-review")
            .unwrap()
            .unwrap();
        assert_eq!(read["instructions"], "Read it closely.");
        assert_eq!(read["enabled"], true);
    }

    /// A folder that silently does nothing is the hardest kind of broken to
    /// find, so it gets a record saying what is wrong with it.
    #[test]
    fn a_skill_folder_without_a_skill_file_still_reports_itself() {
        let (_dir, layout) = workspace();
        fsx::ensure_dir(&layout.collection_dir(Collection::Skills).join("hollow")).unwrap();
        let read = get(&layout, Collection::Skills, "hollow").unwrap().unwrap();
        assert_eq!(read["missing"], true);
        assert_eq!(read["enabled"], false);
        assert!(!read["problems"].as_array().unwrap().is_empty());
    }

    #[test]
    fn removing_a_skill_takes_its_whole_folder() {
        let (_dir, layout) = workspace();
        put(
            &layout,
            Collection::Skills,
            json!({ "id": "s", "name": "S" }),
        )
        .unwrap();
        let extra = layout
            .collection_dir(Collection::Skills)
            .join("s/helper.py");
        fsx::write_text(&extra, "print()").unwrap();
        remove(&layout, Collection::Skills, "s").unwrap();
        assert!(!extra.exists());
        assert!(get(&layout, Collection::Skills, "s").unwrap().is_none());
    }

    #[test]
    fn documents_round_trip_and_fall_back() {
        let (_dir, layout) = workspace();
        let doc = crate::layout::Document::App;
        assert_eq!(read_document(&layout, doc, json!({ "a": 1 }))["a"], 1);
        write_document(&layout, doc, &json!({ "theme": "dark" })).unwrap();
        assert_eq!(read_document(&layout, doc, json!({}))["theme"], "dark");
    }

    #[test]
    fn collection_names_resolve_to_their_directories() {
        assert_eq!(collection("plugins.mcp").unwrap(), Collection::Mcp);
        assert_eq!(collection("sessions").unwrap(), Collection::Turns);
        assert_eq!(collection("memory").unwrap(), Collection::Memories);
        assert!(collection("nope").is_err());
    }
}

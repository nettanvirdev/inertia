//! Secrets.
//!
//! Stored in the clear, on purpose and only for now: encryption without a key
//! the user actually controls is theatre, and choosing that key (OS keychain, a
//! passphrase, a hardware token) is a decision worth making once, properly.
//! Until then the file says what it is and the UI says it too.
//!
//! The shape it protects even so: a secret's VALUE never leaves this module in
//! bulk. [`list`] returns names and a masked hint; reading a real value takes an
//! explicit [`get`] for one named key, which keeps a careless render of the
//! secrets pane from putting every token on screen at once.
//!
//! Everything else in the app refers to a secret by name - `{secret:GITHUB_PAT}`
//! in an MCP header, say - so the values live in exactly one file and a plugin
//! export can hand out its config without handing out its credentials.

use serde_json::{json, Map, Value};

use crate::collections::{self, CollectionError};
use crate::layout::{Document, Layout};

pub type Result<T> = std::result::Result<T, CollectionError>;

/// A secret's name must be usable as an environment variable, because that is
/// where most of them end up.
fn check_name(name: &str) -> Result<&str> {
    let mut bytes = name.bytes();
    let valid = match bytes.next() {
        Some(first) => {
            (first.is_ascii_alphabetic() || first == b'_')
                && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        }
        None => false,
    };
    if !valid {
        return Err(CollectionError::InvalidId(
            "A secret's name must look like GITHUB_TOKEN: letters, digits and underscores."
                .to_string(),
        ));
    }
    Ok(name)
}

/// Enough to recognise a key you already stored, not enough to use it.
pub fn hint(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "•".repeat(chars.len().max(4));
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 3..].iter().collect();
    format!("{head}…{tail}")
}

fn read_all(layout: &Layout) -> Map<String, Value> {
    let doc = collections::read_document(layout, Document::Secrets, json!({ "entries": {} }));
    match doc.get("entries") {
        Some(Value::Object(entries)) => entries.clone(),
        _ => Map::new(),
    }
}

fn write_all(layout: &Layout, entries: Map<String, Value>) -> Result<()> {
    collections::write_document(
        layout,
        Document::Secrets,
        &json!({ "entries": Value::Object(entries) }),
    )
}

fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Every secret's name and a masked hint, sorted. Never a value.
pub fn list(layout: &Layout) -> Vec<Value> {
    let entries = read_all(layout);
    let mut rows: Vec<Value> = entries
        .iter()
        .map(|(name, entry)| {
            let field = |key: &str| entry.get(key).cloned().unwrap_or(Value::Null);
            json!({
                "name": name,
                "label": entry.get("label").and_then(Value::as_str).unwrap_or(""),
                "hint": hint(entry.get("value").and_then(Value::as_str).unwrap_or("")),
                "createdAt": field("createdAt"),
                "updatedAt": field("updatedAt"),
            })
        })
        .collect();
    rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    rows
}

/// One secret's value, by name.
pub fn get(layout: &Layout, name: &str) -> Result<Option<String>> {
    check_name(name)?;
    Ok(read_all(layout)
        .get(name)
        .and_then(|entry| entry.get("value"))
        .and_then(Value::as_str)
        .map(str::to_string))
}

pub fn set(layout: &Layout, name: &str, value: &str, label: &str) -> Result<Value> {
    check_name(name)?;
    let mut entries = read_all(layout);
    let stamp = now();
    let created = entries
        .get(name)
        .and_then(|e| e.get("createdAt"))
        .cloned()
        .unwrap_or_else(|| Value::String(stamp.clone()));

    entries.insert(
        name.to_string(),
        json!({
            "value": value,
            "label": label,
            "createdAt": created,
            "updatedAt": stamp.clone(),
        }),
    );
    write_all(layout, entries)?;

    Ok(json!({
        "name": name,
        "label": label,
        "hint": hint(value),
        "updatedAt": stamp,
    }))
}

pub fn remove(layout: &Layout, name: &str) -> Result<Value> {
    check_name(name)?;
    let mut entries = read_all(layout);
    entries.remove(name);
    write_all(layout, entries)?;
    Ok(json!({ "name": name, "removed": true }))
}

/// What a secret's value is replaced with wherever it would have been written.
pub const REDACTED: &str = "[REDACTED]";

/// Shorter than this and a value is too likely to occur by chance - a PIN, a
/// word - for replacing it everywhere to be anything but damage.
const MIN_SCRUBBED: usize = 6;

/// Every stored value worth looking for, longest first so a secret that
/// contains another is taken out whole.
///
/// For the places that write what an agent saw to disk - its turn traces, its
/// transcripts, the failure log. A key the agent was handed in an environment
/// variable comes back in a command's output, and the output is kept; this is
/// what keeps the key in the one file meant to hold it.
pub fn values(layout: &Layout) -> Vec<String> {
    let mut found: Vec<String> = read_all(layout)
        .values()
        .filter_map(|entry| entry.get("value").and_then(Value::as_str))
        .filter(|value| value.len() >= MIN_SCRUBBED)
        .map(str::to_string)
        .collect();
    found.sort_by_key(|value| std::cmp::Reverse(value.len()));
    found
}

/// `text`, with every one of `values` replaced by [`REDACTED`].
pub fn scrub(text: &str, values: &[String]) -> String {
    let mut out = text.to_string();
    for value in values {
        if value.len() >= MIN_SCRUBBED && out.contains(value.as_str()) {
            out = out.replace(value.as_str(), REDACTED);
        }
    }
    out
}

/// [`scrub`] through every string in a record, keys left alone.
///
/// String by string rather than over the serialised text, because a value
/// with a quote or a backslash in it is spelled differently once encoded and
/// would be missed.
pub fn scrub_value(value: &mut Value, values: &[String]) {
    if values.is_empty() {
        return;
    }
    match value {
        Value::String(text) => {
            if values.iter().any(|secret| text.contains(secret.as_str())) {
                *text = scrub(text, values);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| scrub_value(item, values)),
        Value::Object(map) => map.values_mut().for_each(|item| scrub_value(item, values)),
        _ => {}
    }
}

/// A record about to be written, with this workspace's secrets taken out.
pub fn scrubbed(layout: &Layout, mut record: Value) -> Value {
    scrub_value(&mut record, &values(layout));
    record
}

/// Resolves `{secret:NAME}` anywhere in a config value, so a stored MCP header
/// or OpenAPI auth field can name a secret instead of carrying one.
///
/// An unknown name is left exactly as written rather than replaced with an
/// empty string: a header that silently became `Bearer ` looks like an
/// authentication failure at the far end, while one that still reads
/// `Bearer {secret:GITHUB_PAT}` says what is actually wrong.
pub fn resolve(layout: &Layout, value: &Value) -> Value {
    let entries = read_all(layout);
    swap(value, &entries)
}

fn swap(value: &Value, entries: &Map<String, Value>) -> Value {
    match value {
        Value::String(text) => Value::String(substitute(text, entries)),
        Value::Array(items) => Value::Array(items.iter().map(|v| swap(v, entries)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), swap(v, entries)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Hand-scanned rather than a regex: the pattern is fixed, this runs on every
/// header of every tool call, and the name rules are the same ones
/// [`check_name`] enforces.
fn substitute(text: &str, entries: &Map<String, Value>) -> String {
    const OPEN: &str = "{secret:";
    if !text.contains(OPEN) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find('}') else {
            // An unterminated `{secret:` is just text.
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &after[..end];
        let replacement = if check_name(name).is_ok() {
            entries
                .get(name)
                .and_then(|entry| entry.get("value"))
                .and_then(Value::as_str)
        } else {
            None
        };
        match replacement {
            Some(value) => out.push_str(value),
            None => out.push_str(&rest[start..start + OPEN.len() + end + 1]),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        layout.scaffold().unwrap();
        (dir, layout)
    }

    #[test]
    fn a_secret_round_trips() {
        let (_dir, layout) = workspace();
        set(&layout, "GITHUB_TOKEN", "ghp_abcdefghijkl", "GitHub").unwrap();
        assert_eq!(
            get(&layout, "GITHUB_TOKEN").unwrap().unwrap(),
            "ghp_abcdefghijkl"
        );
    }

    /// The shape the module exists to protect: a careless render of the pane
    /// must not be able to put every token on screen.
    #[test]
    fn listing_never_returns_a_value() {
        let (_dir, layout) = workspace();
        set(&layout, "GITHUB_TOKEN", "ghp_abcdefghijkl", "GitHub").unwrap();
        let rows = list(&layout);
        assert_eq!(rows[0]["name"], "GITHUB_TOKEN");
        assert_eq!(rows[0]["hint"], "ghp…jkl");
        assert!(!serde_json::to_string(&rows).unwrap().contains("abcdefghijkl"));
    }

    #[test]
    fn a_stored_value_is_taken_out_of_a_record_wherever_it_appears() {
        let (_dir, layout) = workspace();
        set(&layout, "API_KEY", "key-with\"quote-123", "").unwrap();
        set(&layout, "PIN", "1234", "").unwrap();
        let record = json!({
            "id": "turn-1",
            "events": [{ "output": "printenv: API_KEY=key-with\"quote-123 PIN=1234" }],
            "count": 3,
        });
        let clean = scrubbed(&layout, record);
        let text = serde_json::to_string(&clean).unwrap();
        assert!(!text.contains("quote-123"), "{text}");
        assert!(text.contains("API_KEY=[REDACTED]"), "{text}");
        // Too short to look for without wrecking ordinary text.
        assert!(text.contains("PIN=1234"), "{text}");
        assert_eq!(clean["count"], json!(3));
    }

    #[test]
    fn a_secret_inside_a_longer_one_does_not_leave_the_longer_one_half_shown() {
        let (_dir, layout) = workspace();
        set(&layout, "SHORT", "abcdef", "").unwrap();
        set(&layout, "LONG", "abcdef-ghijkl", "").unwrap();
        assert_eq!(scrub("x abcdef-ghijkl y", &values(&layout)), "x [REDACTED] y");
    }

    #[test]
    fn a_short_value_is_masked_entirely() {
        assert_eq!(hint("abc"), "••••");
        assert_eq!(hint(""), "••••");
        assert_eq!(hint("12345678"), "••••••••");
    }

    #[test]
    fn created_at_survives_an_update() {
        let (_dir, layout) = workspace();
        set(&layout, "K", "one", "").unwrap();
        let first = list(&layout)[0]["createdAt"].clone();
        set(&layout, "K", "two", "").unwrap();
        assert_eq!(list(&layout)[0]["createdAt"], first);
        assert_eq!(get(&layout, "K").unwrap().unwrap(), "two");
    }

    #[test]
    fn a_name_that_is_not_an_identifier_is_refused() {
        let (_dir, layout) = workspace();
        for bad in ["", "1TOKEN", "my-token", "a b", "TOKEN!"] {
            assert!(set(&layout, bad, "x", "").is_err(), "{bad} should be refused");
        }
        assert!(set(&layout, "_OK9", "x", "").is_ok());
    }

    #[test]
    fn an_absent_secret_reads_as_none() {
        let (_dir, layout) = workspace();
        assert!(get(&layout, "NOPE").unwrap().is_none());
    }

    #[test]
    fn removing_is_idempotent() {
        let (_dir, layout) = workspace();
        set(&layout, "K", "v", "").unwrap();
        remove(&layout, "K").unwrap();
        remove(&layout, "K").unwrap();
        assert!(get(&layout, "K").unwrap().is_none());
    }

    #[test]
    fn placeholders_resolve_through_nested_config() {
        let (_dir, layout) = workspace();
        set(&layout, "TOKEN", "s3cret", "").unwrap();
        let resolved = resolve(
            &layout,
            &json!({
                "headers": { "Authorization": "Bearer {secret:TOKEN}" },
                "args": ["--key", "{secret:TOKEN}"],
                "port": 8080,
            }),
        );
        assert_eq!(resolved["headers"]["Authorization"], "Bearer s3cret");
        assert_eq!(resolved["args"][1], "s3cret");
        assert_eq!(resolved["port"], 8080);
    }

    /// A header that silently became `Bearer ` looks like an authentication
    /// failure at the far end; one that still names the secret says what is
    /// actually wrong.
    #[test]
    fn an_unknown_placeholder_is_left_alone() {
        let (_dir, layout) = workspace();
        let resolved = resolve(&layout, &json!("Bearer {secret:MISSING}"));
        assert_eq!(resolved, "Bearer {secret:MISSING}");
    }

    #[test]
    fn two_placeholders_in_one_string_both_resolve() {
        let (_dir, layout) = workspace();
        set(&layout, "A", "1", "").unwrap();
        set(&layout, "B", "2", "").unwrap();
        assert_eq!(
            resolve(&layout, &json!("{secret:A}-{secret:B}")),
            "1-2"
        );
    }

    #[test]
    fn an_unterminated_placeholder_is_just_text() {
        let (_dir, layout) = workspace();
        assert_eq!(resolve(&layout, &json!("{secret:A")), "{secret:A");
    }
}

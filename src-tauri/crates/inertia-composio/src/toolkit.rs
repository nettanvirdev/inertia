//! The one toolkit shape the rest of the app knows about.
//!
//! Composio's `/api/v3/toolkits` payload is not what a picker wants to read:
//! the logo moved to `meta.logo` at some point and the old spelling is still
//! what some toolkits carry, categories arrive as either strings or objects,
//! and whether an app can be connected at all is spread across three fields
//! and a separate endpoint. Normalising here means the screen reads one shape
//! and the knowledge of Composio's spellings lives in one place.
//!
//! **Everything comes back, connectable or not.** Two filters used to cut this
//! down - asking the server for `managed_by=composio`, then dropping whatever
//! survived that - and between them a catalogue of hundreds was shown as 89
//! apps with no indication anything was missing. An app that is absent because
//! we could not connect it is indistinguishable, to the person looking for it,
//! from an app Composio does not carry, and they go looking elsewhere.

use serde_json::{json, Value};
use std::collections::BTreeSet;

const MARKETPLACE: &str = "https://platform.composio.dev/marketplace/";

/// Whether a toolkit can be connected from here, and if not, what is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connectability {
    pub connectable: bool,
    pub reason: String,
}

fn upper_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|entry| entry.as_str().unwrap_or_default().to_uppercase())
                .collect()
        })
        .unwrap_or_default()
}

/// The auth schemes Composio both offers for a toolkit and holds the client for.
///
/// Both lists matter. A toolkit can advertise OAUTH2 while Composio manages
/// only its API key, in which case offering Connect would ask the user for a
/// client id they do not have.
fn managed_schemes(item: &Value) -> Vec<String> {
    let offered = upper_list(item.get("auth_schemes"));
    upper_list(item.get("composio_managed_auth_schemes"))
        .into_iter()
        .filter(|scheme| offered.contains(scheme))
        .collect()
}

/// A scheme name a person did not have to learn to read.
fn scheme_words(scheme: &str) -> String {
    match scheme {
        "API_KEY" => "an API key".into(),
        "BEARER_TOKEN" => "a bearer token".into(),
        "BASIC" => "a username and password".into(),
        "BASIC_WITH_JWT" => "a username, a password and a JWT".into(),
        "BILLCOM_AUTH" => "Bill.com credentials".into(),
        "GOOGLE_SERVICE_ACCOUNT" => "a Google service account file".into(),
        "CALCOM_AUTH" => "a Cal.com key".into(),
        "SNOWFLAKE" => "Snowflake credentials".into(),
        other => other.to_lowercase().replace('_', " "),
    }
}

/// `configured` is the set of toolkit slugs this Composio project already has
/// an auth config for. It is the answer to "I configured the app at Composio,
/// why is it still in the bottom list": the toolkit payload never changes when
/// somebody creates an auth config - what changes is that a config now exists -
/// so the classification has to read the project, not just the catalogue.
pub fn connectability(item: &Value, configured: &BTreeSet<String>) -> Connectability {
    let yes = || Connectability {
        connectable: true,
        reason: String::new(),
    };
    let no = |reason: &str| Connectability {
        connectable: false,
        reason: reason.to_string(),
    };

    let schemes = upper_list(item.get("auth_schemes"));
    let slug = item
        .get("slug")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();

    if item.get("no_auth") == Some(&Value::Bool(true)) || schemes.iter().any(|s| s == "NO_AUTH") {
        return yes();
    }
    if !managed_schemes(item).is_empty() || configured.contains(&slug) {
        return yes();
    }

    if schemes.iter().any(|scheme| scheme.starts_with("OAUTH")) {
        return no("Composio has no OAuth client for this app, so it needs one of yours from the vendor's developer console.");
    }
    if !schemes.is_empty() {
        // Deduplicated because two OAuth schemes can map to the same words, and
        // "Needs an API key or an API key" reads as a bug.
        let wanted: Vec<String> = schemes
            .iter()
            .map(|scheme| scheme_words(scheme))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        return no(&format!(
            "Needs {}, saved as an auth config at Composio.",
            wanted.join(" or ")
        ));
    }
    no("Composio publishes no way to sign in to this app.")
}

/// Categories arrive as bare strings from one endpoint and as objects from
/// another, so both are read and anything nameless is dropped.
fn categories(meta: &Value) -> Vec<String> {
    meta.get("categories")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|category| match category {
                    Value::String(text) => Some(text.clone()),
                    other => other
                        .get("name")
                        .or_else(|| other.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// One catalogue row, in the shape the picker reads.
pub fn normalise(item: &Value, configured: &BTreeSet<String>) -> Value {
    let empty = json!({});
    let meta = item.get("meta").unwrap_or(&empty);
    let text = |value: Option<&Value>| value.and_then(Value::as_str).unwrap_or_default().to_string();

    let slug = text(item.get("slug"));
    let names = categories(meta);
    let Connectability {
        connectable,
        reason,
    } = connectability(item, configured);

    let logo = {
        // `meta.logo` is the current field; the top-level one is the older
        // spelling and is still what some toolkits carry.
        let current = text(meta.get("logo"));
        if current.is_empty() {
            text(item.get("logo"))
        } else {
            current
        }
    };

    let description = {
        let from_meta = text(meta.get("description"));
        if from_meta.is_empty() {
            text(item.get("description"))
        } else {
            from_meta
        }
    };

    json!({
        "slug": slug,
        "name": if item.get("name").and_then(Value::as_str).unwrap_or_default().is_empty() {
            slug.clone()
        } else {
            text(item.get("name"))
        },
        "description": description,
        "logo": logo,
        "categories": names,
        "tags": names,
        "connectable": connectable,
        "reason": reason,
        "setupUrl": if slug.is_empty() {
            MARKETPLACE.to_string()
        } else {
            format!("{MARKETPLACE}{}", urlencoding(&slug))
        },
    })
}

/// Percent-encoding for a slug, which is the only thing this ever escapes.
fn urlencoding(slug: &str) -> String {
    slug.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// Case-insensitive search over the fields a person would actually type.
pub fn matches_search(toolkit: &Value, search: &str) -> bool {
    let needle = search.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    let field = |key: &str| {
        toolkit
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let tags = toolkit
        .get("tags")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    format!("{} {} {tags}", field("name"), field("description"))
        .to_lowercase()
        .contains(&needle)
}

/// Whether a normalised toolkit carries a category, compared case-insensitively
/// because the filter comes from a chip the user clicked and the category came
/// from Composio.
pub fn matches_category(toolkit: &Value, category: &str) -> bool {
    let wanted = category.trim().to_lowercase();
    if wanted.is_empty() {
        return true;
    }
    toolkit
        .get("categories")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .any(|name| name.to_lowercase() == wanted)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn no_auth_is_connectable() {
        let item = json!({ "slug": "hackernews", "no_auth": true });
        assert!(connectability(&item, &none()).connectable);
    }

    #[test]
    fn managed_oauth_is_connectable() {
        let item = json!({
            "slug": "gmail",
            "auth_schemes": ["OAUTH2"],
            "composio_managed_auth_schemes": ["OAUTH2"],
        });
        assert!(connectability(&item, &none()).connectable);
    }

    /// A toolkit offering OAuth that Composio manages only the API key for is
    /// the case the two lists exist to separate.
    #[test]
    fn oauth_composio_does_not_manage_is_not_connectable() {
        let item = json!({
            "slug": "someapp",
            "auth_schemes": ["OAUTH2"],
            "composio_managed_auth_schemes": ["API_KEY"],
        });
        let answer = connectability(&item, &none());
        assert!(!answer.connectable);
        assert!(answer.reason.contains("no OAuth client"));
    }

    /// The user made an auth config at Composio; the payload does not change,
    /// so only the project read can tell us.
    #[test]
    fn a_configured_slug_is_connectable() {
        let item = json!({ "slug": "Notion", "auth_schemes": ["OAUTH2"] });
        let configured = BTreeSet::from(["notion".to_string()]);
        assert!(connectability(&item, &configured).connectable);
    }

    #[test]
    fn key_schemes_are_named_in_words() {
        let item = json!({ "slug": "x", "auth_schemes": ["API_KEY", "BEARER_TOKEN"] });
        let answer = connectability(&item, &none());
        assert!(!answer.connectable);
        assert_eq!(
            answer.reason,
            "Needs a bearer token or an API key, saved as an auth config at Composio."
        );
    }

    #[test]
    fn an_unknown_scheme_is_spelled_readably() {
        assert_eq!(scheme_words("SOME_NEW_THING"), "some new thing");
    }

    #[test]
    fn no_schemes_at_all_says_so() {
        let answer = connectability(&json!({ "slug": "x" }), &none());
        assert_eq!(
            answer.reason,
            "Composio publishes no way to sign in to this app."
        );
    }

    #[test]
    fn the_old_logo_spelling_still_works() {
        let row = normalise(&json!({ "slug": "a", "logo": "old.png" }), &none());
        assert_eq!(row["logo"], "old.png");
        let newer = normalise(
            &json!({ "slug": "a", "logo": "old.png", "meta": { "logo": "new.png" } }),
            &none(),
        );
        assert_eq!(newer["logo"], "new.png");
    }

    #[test]
    fn categories_come_as_strings_or_objects() {
        let row = normalise(
            &json!({
                "slug": "a",
                "meta": { "categories": ["crm", { "name": "sales" }, { "id": "ops" }, {}] },
            }),
            &none(),
        );
        assert_eq!(row["categories"], json!(["crm", "sales", "ops"]));
        assert_eq!(row["tags"], row["categories"]);
    }

    #[test]
    fn a_nameless_toolkit_falls_back_to_its_slug() {
        let row = normalise(&json!({ "slug": "asana" }), &none());
        assert_eq!(row["name"], "asana");
        assert_eq!(
            row["setupUrl"],
            "https://platform.composio.dev/marketplace/asana"
        );
    }

    #[test]
    fn a_slug_needing_escaping_is_escaped() {
        let row = normalise(&json!({ "slug": "a b/c" }), &none());
        assert_eq!(
            row["setupUrl"],
            "https://platform.composio.dev/marketplace/a%20b%2Fc"
        );
    }

    #[test]
    fn search_reads_the_description_and_tags_too() {
        let row = normalise(
            &json!({
                "slug": "linear",
                "name": "Linear",
                "meta": { "description": "Issue tracking", "categories": ["productivity"] },
            }),
            &none(),
        );
        assert!(matches_search(&row, "LINE"));
        assert!(matches_search(&row, "issue"));
        assert!(matches_search(&row, "productivity"));
        assert!(!matches_search(&row, "spreadsheet"));
        assert!(matches_search(&row, "   "));
    }

    #[test]
    fn category_matching_ignores_case() {
        let row = normalise(
            &json!({ "slug": "a", "meta": { "categories": ["CRM"] } }),
            &none(),
        );
        assert!(matches_category(&row, "crm"));
        assert!(matches_category(&row, ""));
        assert!(!matches_category(&row, "ops"));
    }
}

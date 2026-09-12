//! Reading an OpenAPI document.
//!
//! Parsed into `serde_json::Value` and walked, rather than deserialised into a
//! typed OpenAPI model. Specs in the wild are frequently invalid in ways that
//! do not matter - a missing `info.version`, a vendor extension where an
//! object was expected, a Swagger 2 document with an OpenAPI 3 header. A
//! strict parser rejects the whole file over any of them, and the user is left
//! with an import that will not work and no way to find out why.
//!
//! Walking the tree means one malformed operation costs that operation, not
//! the document.

use serde_json::Value;

/// How deep `$ref` resolution will go before giving up.
///
/// A self-referential schema (a comment with replies, a tree node) would
/// otherwise inline forever.
const MAX_DEPTH: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("that does not look like an OpenAPI document: {0}")]
    NotASpec(String),
    #[error("could not parse the document: {0}")]
    Parse(String),
}

/// Parses a spec from text, accepting JSON or YAML.
///
/// JSON is tried first because it is unambiguous and cheap; every JSON
/// document is also valid YAML, so the order only matters for speed.
pub fn parse(text: &str) -> Result<Value, SpecError> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return validate(value);
    }

    match serde_yaml_ng::from_str::<Value>(text) {
        Ok(value) => validate(value),
        Err(e) => Err(SpecError::Parse(e.to_string())),
    }
}

/// Checks the document is recognisably a spec before anything else reads it.
///
/// The point is to fail on "the user pasted a URL that returned an HTML error
/// page" with a message that says so, rather than on "no operations found"
/// three steps later.
fn validate(value: Value) -> Result<Value, SpecError> {
    if !value.is_object() {
        return Err(SpecError::NotASpec("it is not an object".into()));
    }

    let version = value
        .get("openapi")
        .or_else(|| value.get("swagger"))
        .and_then(Value::as_str);

    if version.is_none() {
        return Err(SpecError::NotASpec(
            "it has no `openapi` or `swagger` version field".into(),
        ));
    }

    if !value.get("paths").is_some_and(Value::is_object) {
        return Err(SpecError::NotASpec("it has no `paths` section".into()));
    }

    Ok(value)
}

/// Inlines local `$ref`s so the resulting schema stands alone.
///
/// Three rules, each protecting against something real:
///
///   - **A reference outside the document is never fetched.** Resolving it
///     would mean an HTTP request, at import time, to a host the user never
///     named. It becomes an untyped object instead.
///   - **A self-reference collapses** rather than recursing forever.
///   - **Depth is capped**, because a chain of references can be deep without
///     ever repeating.
pub fn resolve_refs(schema: &Value, document: &Value) -> Resolved {
    let mut unresolved = false;
    let value = inline(schema, document, &mut Vec::new(), 0, &mut unresolved);
    Resolved { value, unresolved }
}

#[derive(Debug, Clone)]
pub struct Resolved {
    pub value: Value,
    /// Some part of the schema pointed outside the document. The tool is still
    /// usable; that part is simply undescribed.
    pub unresolved: bool,
}

fn inline(
    schema: &Value,
    document: &Value,
    path: &mut Vec<String>,
    depth: usize,
    unresolved: &mut bool,
) -> Value {
    if depth > MAX_DEPTH {
        return serde_json::json!({});
    }

    match schema {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref") {
                // Anything not starting `#/` addresses another document.
                let Some(pointer) = reference.strip_prefix('#') else {
                    *unresolved = true;
                    return serde_json::json!({ "type": "object" });
                };

                if path.contains(reference) {
                    // Self-referential. Describing the shape is more useful to
                    // a model than an empty object.
                    return serde_json::json!({
                        "type": "object",
                        "description": "A value of the same shape."
                    });
                }

                let Some(target) = document.pointer(pointer) else {
                    *unresolved = true;
                    return serde_json::json!({ "type": "object" });
                };

                path.push(reference.clone());
                let resolved = inline(target, document, path, depth + 1, unresolved);
                path.pop();
                return resolved;
            }

            let mut cleaned = serde_json::Map::new();
            for (key, child) in map {
                cleaned.insert(key.clone(), inline(child, document, path, depth + 1, unresolved));
            }
            Value::Object(cleaned)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| inline(item, document, path, depth + 1, unresolved))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The base URL a spec's operations are served from.
pub fn base_url(document: &Value) -> Option<String> {
    // OpenAPI 3.
    if let Some(url) = document
        .pointer("/servers/0/url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
    {
        return Some(url.trim_end_matches('/').to_string());
    }

    // Swagger 2, assembled from its parts.
    let host = document.get("host").and_then(Value::as_str)?;
    let scheme = document
        .pointer("/schemes/0")
        .and_then(Value::as_str)
        .unwrap_or("https");
    let base = document
        .get("basePath")
        .and_then(Value::as_str)
        .unwrap_or("");

    Some(format!("{scheme}://{host}{base}").trim_end_matches('/').to_string())
}

pub fn title(document: &Value) -> String {
    document
        .pointer("/info/title")
        .and_then(Value::as_str)
        .unwrap_or("API")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal() -> Value {
        json!({
            "openapi": "3.0.0",
            "info": { "title": "Test API", "version": "1.0" },
            "servers": [{ "url": "https://api.example.com/v1/" }],
            "paths": {}
        })
    }

    // ── parsing ─────────────────────────────────────────────────────────

    #[test]
    fn a_json_spec_parses() {
        let parsed = parse(&minimal().to_string()).unwrap();
        assert_eq!(title(&parsed), "Test API");
    }

    #[test]
    fn a_yaml_spec_parses() {
        let yaml = r#"
openapi: 3.0.0
info:
  title: YAML API
  version: "1.0"
servers:
  - url: https://api.example.com
paths:
  /things:
    get:
      operationId: listThings
"#;
        let parsed = parse(yaml).unwrap();
        assert_eq!(title(&parsed), "YAML API");
        assert!(parsed["paths"]["/things"]["get"].is_object());
    }

    /// The common failure: a URL that returned an error page. Saying so beats
    /// "no operations found" three steps later.
    #[test]
    fn an_html_error_page_is_rejected_clearly() {
        let error = parse("<html><body>404 Not Found</body></html>").unwrap_err();
        assert!(matches!(error, SpecError::Parse(_) | SpecError::NotASpec(_)));
    }

    #[test]
    fn a_document_with_no_version_field_is_refused() {
        let error = parse(&json!({ "paths": {} }).to_string()).unwrap_err();
        assert!(error.to_string().contains("openapi"), "got {error}");
    }

    #[test]
    fn a_document_with_no_paths_is_refused() {
        let error = parse(&json!({ "openapi": "3.0.0" }).to_string()).unwrap_err();
        assert!(error.to_string().contains("paths"), "got {error}");
    }

    #[test]
    fn a_swagger_2_document_is_accepted() {
        let spec = json!({
            "swagger": "2.0",
            "host": "api.example.com",
            "basePath": "/v2",
            "schemes": ["https"],
            "paths": {}
        });
        let parsed = parse(&spec.to_string()).unwrap();
        assert_eq!(base_url(&parsed).as_deref(), Some("https://api.example.com/v2"));
    }

    // ── base URL ────────────────────────────────────────────────────────

    #[test]
    fn a_trailing_slash_is_trimmed_from_the_base_url() {
        assert_eq!(
            base_url(&minimal()).as_deref(),
            Some("https://api.example.com/v1")
        );
    }

    #[test]
    fn a_document_with_no_server_has_no_base_url() {
        let spec = json!({ "openapi": "3.0.0", "paths": {} });
        assert_eq!(base_url(&spec), None);
    }

    // ── reference resolution ────────────────────────────────────────────

    #[test]
    fn a_local_reference_is_inlined() {
        let document = json!({
            "components": {
                "schemas": {
                    "Thing": { "type": "object", "properties": { "id": { "type": "string" } } }
                }
            }
        });
        let schema = json!({ "$ref": "#/components/schemas/Thing" });

        let resolved = resolve_refs(&schema, &document);
        assert!(!resolved.unresolved);
        assert_eq!(resolved.value["properties"]["id"]["type"], "string");
    }

    #[test]
    fn nested_references_resolve() {
        let document = json!({
            "components": { "schemas": {
                "Outer": { "type": "object", "properties": {
                    "inner": { "$ref": "#/components/schemas/Inner" }
                }},
                "Inner": { "type": "string" }
            }}
        });
        let resolved = resolve_refs(&json!({ "$ref": "#/components/schemas/Outer" }), &document);
        assert_eq!(resolved.value["properties"]["inner"]["type"], "string");
    }

    /// Resolving it would mean an HTTP request, at import time, to a host the
    /// user never named.
    #[test]
    fn an_external_reference_is_never_fetched() {
        let schema = json!({ "$ref": "https://example.com/other.json#/Thing" });
        let resolved = resolve_refs(&schema, &json!({}));

        assert!(resolved.unresolved);
        assert_eq!(resolved.value["type"], "object");
    }

    #[test]
    fn a_reference_to_something_absent_does_not_fail_the_import() {
        let resolved = resolve_refs(&json!({ "$ref": "#/nowhere" }), &json!({}));
        assert!(resolved.unresolved);
        assert_eq!(resolved.value["type"], "object");
    }

    /// A comment with replies, a tree node - these are common, and inlining
    /// one forever is not an option.
    #[test]
    fn a_self_reference_collapses_rather_than_recursing() {
        let document = json!({
            "components": { "schemas": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "children": {
                            "type": "array",
                            "items": { "$ref": "#/components/schemas/Node" }
                        }
                    }
                }
            }}
        });

        let resolved = resolve_refs(&json!({ "$ref": "#/components/schemas/Node" }), &document);
        let items = &resolved.value["properties"]["children"]["items"];
        assert_eq!(items["type"], "object");
        assert!(items["description"].as_str().unwrap().contains("same shape"));
    }

    #[test]
    fn ordinary_schemas_pass_through_untouched() {
        let schema = json!({
            "type": "object",
            "properties": { "name": { "type": "string", "description": "a name" } },
            "required": ["name"]
        });
        let resolved = resolve_refs(&schema, &json!({}));
        assert_eq!(resolved.value, schema);
        assert!(!resolved.unresolved);
    }
}

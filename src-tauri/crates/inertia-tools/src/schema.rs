//! Argument schemas, and a validator built for model output.
//!
//! Deliberately **not** a general JSON Schema implementation. Tool schemas
//! here are flat by policy - no `$ref`, no `anyOf`, no `oneOf` - because
//! providers are strict and mutually inconsistent about all three, and a
//! schema one provider rejects is a tool that silently does not exist.
//!
//! More importantly, a conforming validator would be the wrong tool. Models
//! produce `"3"` where a number was asked for and a bare string where an array
//! was, constantly. Rejecting those costs a full round trip to fix something
//! unambiguous. So this validator coerces what is safe to coerce, drops what
//! is safe to drop, and reserves failure for things the model genuinely has to
//! be told about.
//!
//! Error messages are written **for a model to act on**: an English sentence
//! naming the field. `serde`'s path-based errors are aimed at a developer
//! reading a stack trace, and read as noise to the thing that has to fix them.

use serde_json::{Map, Value};

/// Validates and cleans tool arguments.
///
/// Returns a *copy* with defaults filled in, coercions applied, and unknown
/// keys dropped - or one English sentence explaining what the model must fix.
pub fn validate(schema: &Value, value: Value) -> Result<Value, String> {
    walk(schema, value, "")
}

/// The JSON type name used in error messages.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// How a field is referred to in an error message. The root has no name, so
/// errors there speak about "the arguments" instead.
fn field(path: &str) -> String {
    if path.is_empty() {
        "the arguments".to_string()
    } else {
        format!("`{path}`")
    }
}

fn child_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn walk(schema: &Value, value: Value, path: &str) -> Result<Value, String> {
    let declared = schema.get("type").and_then(Value::as_str).unwrap_or("");

    // An enum constrains the value regardless of its declared type.
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        return check_enum(allowed, value, path);
    }

    match declared {
        "object" => walk_object(schema, value, path),
        "array" => walk_array(schema, value, path),
        "string" => match value {
            Value::String(s) => Ok(Value::String(s)),
            other => Err(format!(
                "{} must be a string, got {}.",
                field(path),
                type_name(&other)
            )),
        },
        "number" => coerce_number(value, path, false),
        "integer" => coerce_number(value, path, true),
        "boolean" => coerce_bool(value, path),
        // No declared type means no constraint - pass it through. Schemas
        // arriving from MCP servers and OpenAPI documents often omit it.
        _ => Ok(value),
    }
}

fn check_enum(allowed: &[Value], value: Value, path: &str) -> Result<Value, String> {
    if allowed.contains(&value) {
        return Ok(value);
    }
    let options: Vec<String> = allowed
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();
    Err(format!(
        "{} must be one of: {}.",
        field(path),
        options.join(", ")
    ))
}

fn walk_object(schema: &Value, value: Value, path: &str) -> Result<Value, String> {
    let Value::Object(incoming) = value else {
        return Err(format!(
            "{} must be an object, got {}.",
            field(path),
            type_name(&value)
        ));
    };

    // `additionalProperties: true` marks a payload whose keys are not fixed in
    // advance. Dropping unknown keys there would let a caller believe it wrote
    // data that was silently discarded, so everything is kept as-is.
    let open = schema
        .get("additionalProperties")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Ok(Value::Object(incoming));
    };

    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut cleaned = Map::new();

    for (key, property) in properties {
        let here = child_path(path, key);
        let supplied = incoming.get(key);

        // An absent field and an explicit null mean the same thing. Models
        // send both for "I am not using this", and treating them differently
        // rejects half of a perfectly good call.
        let missing = matches!(supplied, None | Some(Value::Null));

        if missing {
            if required.contains(&key.as_str()) {
                return Err(format!("Missing required argument `{here}`."));
            }
            if let Some(default) = property.get("default") {
                cleaned.insert(key.clone(), default.clone());
            }
            continue;
        }

        let value = supplied.cloned().unwrap_or(Value::Null);
        cleaned.insert(key.clone(), walk(property, value, &here)?);
    }

    if open {
        for (key, value) in incoming {
            cleaned.entry(key).or_insert(value);
        }
    }

    Ok(Value::Object(cleaned))
}

fn walk_array(schema: &Value, value: Value, path: &str) -> Result<Value, String> {
    // A scalar where a list was asked for is the single most common shape
    // mistake a model makes, and its intent is never ambiguous.
    let items = match value {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        scalar => vec![scalar],
    };

    let Some(item_schema) = schema.get("items") else {
        return Ok(Value::Array(items));
    };

    let cleaned = items
        .into_iter()
        .enumerate()
        .map(|(i, item)| walk(item_schema, item, &format!("{path}[{i}]")))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Value::Array(cleaned))
}

fn coerce_number(value: Value, path: &str, integer: bool) -> Result<Value, String> {
    let wanted = if integer { "whole number" } else { "number" };

    let number = match value {
        n @ Value::Number(_) => n,
        // Numeric strings are parsed: providers that stringify every argument
        // are common enough that refusing this would break them wholesale.
        Value::String(s) => match s.trim().parse::<f64>() {
            Ok(parsed) => serde_json::Number::from_f64(parsed)
                .map(Value::Number)
                .ok_or_else(|| format!("{} must be a number.", field(path)))?,
            Err(_) => {
                return Err(format!(
                    "{} must be a {wanted}, got the string {s:?}.",
                    field(path),
                ))
            }
        },
        other => {
            return Err(format!(
                "{} must be a {wanted}, got {}.",
                field(path),
                type_name(&other)
            ))
        }
    };

    if integer {
        let as_f64 = number.as_f64().unwrap_or_default();
        if as_f64.fract() != 0.0 {
            return Err(format!(
                "{} must be a whole number, got {as_f64}.",
                field(path)
            ));
        }
        return Ok(Value::Number((as_f64 as i64).into()));
    }

    Ok(number)
}

fn coerce_bool(value: Value, path: &str) -> Result<Value, String> {
    match value {
        b @ Value::Bool(_) => Ok(b),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(format!(
                "{} must be true or false, got the string {s:?}.",
                field(path),
            )),
        },
        other => Err(format!(
            "{} must be true or false, got {}.",
            field(path),
            type_name(&other)
        )),
    }
}

/// Recovers the arguments string a provider sent.
///
/// Nominally JSON, but in practice sometimes wrapped in a code fence by a
/// model that has been writing markdown all day. Trying the obvious repairs
/// costs nothing and saves a round trip.
pub fn parse_arguments(raw: &str) -> Result<Value, String> {
    const FAILED: &str = "The arguments were not valid JSON.";

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Map::new()));
    }

    if let Ok(value @ Value::Object(_)) = serde_json::from_str::<Value>(trimmed) {
        return Ok(value);
    }

    // Strip a ```json ... ``` fence and retry.
    if let Some(inner) = strip_fence(trimmed) {
        if let Ok(value @ Value::Object(_)) = serde_json::from_str::<Value>(inner) {
            return Ok(value);
        }
    }

    Err(FAILED.to_string())
}

fn strip_fence(text: &str) -> Option<&str> {
    let without_open = text.strip_prefix("```")?;
    // Drop an optional language tag on the opening line.
    let body = match without_open.find('\n') {
        Some(newline) => &without_open[newline + 1..],
        None => without_open,
    };
    Some(body.trim_end().strip_suffix("```").unwrap_or(body).trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(properties: Value, required: &[&str]) -> Value {
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    }

    // ── the happy path ──────────────────────────────────────────────────

    #[test]
    fn a_valid_call_passes_through() {
        let schema = object(json!({ "path": { "type": "string" } }), &["path"]);
        let out = validate(&schema, json!({ "path": "a.txt" })).unwrap();
        assert_eq!(out["path"], "a.txt");
    }

    // ── required and absent ─────────────────────────────────────────────

    #[test]
    fn a_missing_required_field_names_itself() {
        let schema = object(json!({ "path": { "type": "string" } }), &["path"]);
        let err = validate(&schema, json!({})).unwrap_err();
        assert_eq!(err, "Missing required argument `path`.");
    }

    /// Models send explicit nulls for "not using this" as often as they omit
    /// the key. Treating those differently rejects good calls.
    #[test]
    fn an_explicit_null_is_the_same_as_absent() {
        let schema = object(json!({ "flag": { "type": "boolean" } }), &[]);
        let from_null = validate(&schema, json!({ "flag": null })).unwrap();
        let from_absent = validate(&schema, json!({})).unwrap();
        assert_eq!(from_null, from_absent);

        // ...but a required field cannot be satisfied by null either.
        let required = object(json!({ "flag": { "type": "boolean" } }), &["flag"]);
        assert!(validate(&required, json!({ "flag": null })).is_err());
    }

    #[test]
    fn defaults_are_filled_in() {
        let schema = object(
            json!({ "deep": { "type": "boolean", "default": true } }),
            &[],
        );
        let out = validate(&schema, json!({})).unwrap();
        assert_eq!(out["deep"], true);
    }

    // ── coercion ────────────────────────────────────────────────────────

    #[test]
    fn numeric_strings_are_parsed() {
        let schema = object(json!({ "limit": { "type": "integer" } }), &[]);
        let out = validate(&schema, json!({ "limit": "42" })).unwrap();
        assert_eq!(out["limit"], 42);
    }

    #[test]
    fn boolean_strings_are_parsed() {
        let schema = object(json!({ "deep": { "type": "boolean" } }), &[]);
        assert_eq!(
            validate(&schema, json!({ "deep": "true" })).unwrap()["deep"],
            true
        );
        assert_eq!(
            validate(&schema, json!({ "deep": "FALSE" })).unwrap()["deep"],
            false
        );
    }

    /// The most common shape mistake a model makes, and never an ambiguous
    /// one.
    #[test]
    fn a_scalar_is_wrapped_where_a_list_was_asked_for() {
        let schema = object(
            json!({ "paths": { "type": "array", "items": { "type": "string" } } }),
            &[],
        );
        let out = validate(&schema, json!({ "paths": "a.txt" })).unwrap();
        assert_eq!(out["paths"], json!(["a.txt"]));
    }

    #[test]
    fn a_non_whole_number_is_refused_for_an_integer() {
        let schema = object(json!({ "limit": { "type": "integer" } }), &[]);
        let err = validate(&schema, json!({ "limit": 1.5 })).unwrap_err();
        assert!(err.contains("whole number"), "got {err}");
    }

    #[test]
    fn a_non_numeric_string_is_refused() {
        let schema = object(json!({ "limit": { "type": "number" } }), &[]);
        let err = validate(&schema, json!({ "limit": "lots" })).unwrap_err();
        assert!(err.contains("`limit`"), "got {err}");
    }

    // ── unknown keys ────────────────────────────────────────────────────

    /// Models improvise fields. Dropping them beats failing a call that is
    /// otherwise entirely correct.
    #[test]
    fn unknown_keys_are_dropped_not_rejected() {
        let schema = object(json!({ "path": { "type": "string" } }), &["path"]);
        let out = validate(&schema, json!({ "path": "a.txt", "encoding": "utf8" })).unwrap();
        assert_eq!(out, json!({ "path": "a.txt" }));
    }

    /// ...except where the key set is genuinely open, in which case dropping
    /// would silently discard data the caller believes it wrote.
    #[test]
    fn an_open_object_keeps_everything() {
        let schema = json!({
            "type": "object",
            "properties": {},
            "additionalProperties": true,
        });
        let out = validate(&schema, json!({ "anything": 1, "else": "two" })).unwrap();
        assert_eq!(out, json!({ "anything": 1, "else": "two" }));
    }

    // ── errors are for a model to read ──────────────────────────────────

    #[test]
    fn errors_name_the_field_and_what_arrived() {
        let schema = object(json!({ "path": { "type": "string" } }), &[]);
        let err = validate(&schema, json!({ "path": 3 })).unwrap_err();
        assert_eq!(err, "`path` must be a string, got number.");
    }

    #[test]
    fn nested_errors_name_the_full_path() {
        let schema = object(
            json!({
                "target": object(json!({ "path": { "type": "string" } }), &["path"]),
            }),
            &[],
        );
        let err = validate(&schema, json!({ "target": { "path": 3 } })).unwrap_err();
        assert_eq!(err, "`target.path` must be a string, got number.");
    }

    #[test]
    fn an_enum_lists_its_options() {
        let schema = object(
            json!({ "mode": { "type": "string", "enum": ["read", "write"] } }),
            &[],
        );
        let err = validate(&schema, json!({ "mode": "delete" })).unwrap_err();
        assert_eq!(err, "`mode` must be one of: read, write.");
    }

    // ── schemas from elsewhere ──────────────────────────────────────────

    /// MCP servers and OpenAPI documents routinely omit `type`. Refusing them
    /// would make every such tool unusable.
    #[test]
    fn an_untyped_schema_accepts_anything() {
        let schema = json!({ "type": "object", "properties": { "x": {} } });
        let out = validate(&schema, json!({ "x": [1, 2] })).unwrap();
        assert_eq!(out["x"], json!([1, 2]));
    }

    // ── argument recovery ───────────────────────────────────────────────

    #[test]
    fn plain_json_arguments_parse() {
        assert_eq!(parse_arguments(r#"{"a":1}"#).unwrap(), json!({"a": 1}));
    }

    #[test]
    fn empty_arguments_mean_no_arguments() {
        assert_eq!(parse_arguments("").unwrap(), json!({}));
        assert_eq!(parse_arguments("   ").unwrap(), json!({}));
    }

    /// A model that has been writing markdown all day sometimes fences its
    /// tool arguments too.
    #[test]
    fn fenced_arguments_are_recovered() {
        let raw = "```json\n{\"a\":1}\n```";
        assert_eq!(parse_arguments(raw).unwrap(), json!({"a": 1}));

        let untagged = "```\n{\"a\":1}\n```";
        assert_eq!(parse_arguments(untagged).unwrap(), json!({"a": 1}));
    }

    #[test]
    fn unrecoverable_arguments_say_so_plainly() {
        let err = parse_arguments("{not json").unwrap_err();
        assert_eq!(err, "The arguments were not valid JSON.");
    }

    /// A bare array is valid JSON but not a valid argument object, and letting
    /// it through would fail later somewhere far less obvious.
    #[test]
    fn a_non_object_payload_is_refused() {
        assert!(parse_arguments("[1,2,3]").is_err());
        assert!(parse_arguments("\"hello\"").is_err());
    }
}

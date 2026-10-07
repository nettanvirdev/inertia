//! Turning a spec's operations into tool definitions.
//!
//! One operation becomes one tool. The interesting work is collapsing three
//! different kinds of input - path parameters, query parameters, and a request
//! body - into the single flat object schema a model calls a tool with, and
//! then taking it apart again at request time.

use serde_json::{json, Map, Value};

use crate::spec;

/// Where a parameter goes in the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum In {
    Path,
    Query,
    Header,
    /// Not a parameter: the request body, flattened into the same argument
    /// object so the model sees one flat set of inputs.
    Body,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub location: In,
    pub required: bool,
    pub schema: Value,
}

#[derive(Debug, Clone)]
pub struct Operation {
    /// The tool id, already namespaced and sanitised.
    pub id: String,
    pub method: String,
    /// The path template, still containing `{placeholders}`.
    pub path: String,
    pub description: String,
    pub parameters: Vec<Parameter>,
    /// Some part of the schema pointed outside the document.
    pub unresolved: bool,
}

impl Operation {
    /// The flat object schema the model calls this tool with.
    pub fn schema(&self) -> Value {
        let mut properties = Map::new();
        let mut required = Vec::new();

        for parameter in &self.parameters {
            properties.insert(parameter.name.clone(), parameter.schema.clone());
            if parameter.required {
                required.push(Value::String(parameter.name.clone()));
            }
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            // Strict, so a model improvising a field gets it dropped rather
            // than sent to an API that will reject the whole call.
            "additionalProperties": false
        })
    }
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// The HTTP methods an operation can be under. Anything else in a path item -
/// `parameters`, `summary`, vendor extensions - is not an operation.
const METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options"];

/// Extracts every operation in a document.
///
/// A malformed operation is skipped rather than failing the import: a spec
/// with forty good endpoints and one broken one should give forty tools.
pub fn extract(document: &Value, namespace: &str) -> Vec<Operation> {
    let Some(paths) = document.get("paths").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut operations = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };

        // Parameters declared on the path item apply to every operation under
        // it, and are frequently where the path parameters actually live.
        let shared = item.get("parameters").and_then(Value::as_array);

        for method in METHODS {
            let Some(details) = item.get(*method) else {
                continue;
            };

            let mut unresolved = false;
            let mut parameters = Vec::new();

            for source in [shared, details.get("parameters").and_then(Value::as_array)]
                .into_iter()
                .flatten()
                .flatten()
            {
                if let Some(parameter) = read_parameter(source, document, &mut unresolved) {
                    parameters.push(parameter);
                }
            }

            parameters.extend(read_body(details, document, &mut unresolved));

            // `operationId` is the author's own name for it and reads far
            // better to a model than a mangled method-and-path.
            let base = details
                .get("operationId")
                .and_then(Value::as_str)
                .map(sanitize)
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| sanitize(&format!("{method}_{path}")));

            let mut id = format!("{}_{}", sanitize(namespace), base);
            // Two operations can share an operationId in a sloppy spec.
            // Suffixing keeps both callable rather than silently dropping one.
            if !seen.insert(id.clone()) {
                let mut suffix = 2;
                while !seen.insert(format!("{id}_{suffix}")) {
                    suffix += 1;
                }
                id = format!("{id}_{suffix}");
            }

            operations.push(Operation {
                id,
                method: method.to_ascii_uppercase(),
                path: path.clone(),
                description: describe(details, method, path, unresolved),
                parameters,
                unresolved,
            });
        }
    }

    operations.sort_by(|a, b| a.id.cmp(&b.id));
    operations
}

fn describe(details: &Value, method: &str, path: &str, unresolved: bool) -> String {
    let summary = details.get("summary").and_then(Value::as_str).unwrap_or("");
    let long = details
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");

    let mut description = match (summary.trim(), long.trim()) {
        ("", "") => format!("{} {}", method.to_ascii_uppercase(), path),
        (summary, "") => summary.to_string(),
        ("", long) => long.to_string(),
        (summary, long) => format!("{summary}\n\n{long}"),
    };

    if unresolved {
        // Said plainly, because the model will otherwise wonder why an
        // argument it can see documented elsewhere is missing here.
        description.push_str(
            "\n\nPart of its argument schema points outside the document the server \
             sent, so that part is undescribed here. Pass what the server documents \
             and it will validate the call.",
        );
    }

    description
}

fn read_parameter(source: &Value, document: &Value, unresolved: &mut bool) -> Option<Parameter> {
    // A parameter can itself be a reference.
    let resolved = spec::resolve_refs(source, document);
    if resolved.unresolved {
        *unresolved = true;
    }
    let source = resolved.value;

    let name = source.get("name").and_then(Value::as_str)?.to_string();
    let location = match source.get("in").and_then(Value::as_str)? {
        "path" => In::Path,
        "query" => In::Query,
        "header" => In::Header,
        // Cookie parameters, and anything a future spec adds. Sending one
        // wrongly is worse than not offering it.
        _ => return None,
    };

    let mut schema = source
        .get("schema")
        .cloned()
        .unwrap_or_else(|| json!({ "type": "string" }));

    // The parameter's own description is the useful one; the schema's is
    // usually about the type.
    if let Some(description) = source.get("description").and_then(Value::as_str) {
        if let Some(map) = schema.as_object_mut() {
            map.entry("description")
                .or_insert(Value::String(description.to_string()));
        }
    }

    Some(Parameter {
        name,
        location,
        // A path parameter is required whether or not the spec says so: the
        // URL cannot be built without it.
        required: location == In::Path
            || source
                .get("required")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        schema,
    })
}

/// Flattens a JSON request body's top-level properties into the argument list.
///
/// A model calls a tool with one flat object. Nesting the body under a `body`
/// key would be closer to the HTTP shape and worse to use - the model would
/// have to know which arguments are "really" body fields, which is exactly the
/// detail a tool exists to hide.
fn read_body(details: &Value, document: &Value, unresolved: &mut bool) -> Vec<Parameter> {
    let Some(content) = details.pointer("/requestBody/content") else {
        return Vec::new();
    };

    // JSON only. A tool that submits multipart or form-encoded bodies needs
    // more than a schema to do it correctly.
    let Some(schema) = content
        .get("application/json")
        .and_then(|json| json.get("schema"))
    else {
        return Vec::new();
    };

    let resolved = spec::resolve_refs(schema, document);
    if resolved.unresolved {
        *unresolved = true;
    }

    let Some(properties) = resolved.value.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };

    let required: Vec<&str> = resolved
        .value
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    properties
        .iter()
        .map(|(name, schema)| Parameter {
            name: name.clone(),
            location: In::Body,
            required: required.contains(&name.as_str()),
            schema: schema.clone(),
        })
        .collect()
}

/// Splits validated arguments into the parts of an HTTP request.
#[derive(Debug, Default)]
pub struct RequestParts {
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

impl Operation {
    /// Builds the request this call becomes.
    pub fn build_request(&self, args: &Value) -> RequestParts {
        let mut parts = RequestParts {
            path: self.path.clone(),
            ..Default::default()
        };
        let mut body = Map::new();

        for parameter in &self.parameters {
            let Some(value) = args.get(&parameter.name) else {
                continue;
            };
            if value.is_null() {
                continue;
            }

            match parameter.location {
                In::Path => {
                    // Percent-encoded: a path parameter containing a slash
                    // would otherwise silently address a different endpoint.
                    let encoded = encode_path_segment(&as_text(value));
                    parts.path = parts
                        .path
                        .replace(&format!("{{{}}}", parameter.name), &encoded);
                }
                In::Query => parts.query.push((parameter.name.clone(), as_text(value))),
                In::Header => parts.headers.push((parameter.name.clone(), as_text(value))),
                In::Body => {
                    body.insert(parameter.name.clone(), value.clone());
                }
            }
        }

        if !body.is_empty() {
            parts.body = Some(Value::Object(body));
        }

        parts
    }
}

/// Scalars go in as themselves; anything structured is sent as JSON.
fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn encode_path_segment(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Value {
        json!({
            "openapi": "3.0.0",
            "info": { "title": "Test" },
            "servers": [{ "url": "https://api.example.com" }],
            "paths": {
                "/things/{id}": {
                    "parameters": [
                        { "name": "id", "in": "path", "schema": { "type": "string" } }
                    ],
                    "get": {
                        "operationId": "getThing",
                        "summary": "Fetch one thing",
                        "parameters": [
                            { "name": "verbose", "in": "query", "schema": { "type": "boolean" } }
                        ]
                    },
                    "delete": { "summary": "Remove it" }
                },
                "/things": {
                    "post": {
                        "operationId": "createThing",
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "name": { "type": "string" },
                                            "tags": { "type": "array", "items": { "type": "string" } }
                                        },
                                        "required": ["name"]
                                    }
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    fn operation(id: &str) -> Operation {
        extract(&document(), "test")
            .into_iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("no operation called {id}"))
    }

    // ── extraction ──────────────────────────────────────────────────────

    #[test]
    fn every_operation_becomes_a_tool() {
        let operations = extract(&document(), "test");
        let ids: Vec<&str> = operations.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "test_creatething",
                "test_delete__things__id",
                "test_getthing"
            ]
        );
    }

    /// The author's own name reads far better to a model than a mangled
    /// method-and-path.
    #[test]
    fn an_operation_id_is_preferred_for_the_name() {
        assert_eq!(operation("test_getthing").method, "GET");
        assert_eq!(operation("test_getthing").path, "/things/{id}");
    }

    #[test]
    fn an_operation_without_an_id_falls_back_to_its_method_and_path() {
        let fallback = operation("test_delete__things__id");
        assert_eq!(fallback.method, "DELETE");
    }

    #[test]
    fn tools_are_namespaced_by_the_import() {
        assert!(extract(&document(), "GitHub API")
            .iter()
            .all(|o| o.id.starts_with("github_api_")));
    }

    /// Path-item parameters apply to every operation under that path, and are
    /// frequently where path parameters actually live.
    #[test]
    fn shared_path_parameters_reach_every_operation() {
        let get = operation("test_getthing");
        assert!(get.parameters.iter().any(|p| p.name == "id"));

        let delete = operation("test_delete__things__id");
        assert!(delete.parameters.iter().any(|p| p.name == "id"));
    }

    /// The URL cannot be built without it, whatever the spec claims.
    #[test]
    fn a_path_parameter_is_always_required() {
        let id = operation("test_getthing")
            .parameters
            .into_iter()
            .find(|p| p.name == "id")
            .unwrap();
        assert!(id.required);
        assert_eq!(id.location, In::Path);
    }

    /// A model calls a tool with one flat object; making it distinguish body
    /// fields from query fields would leak the detail a tool exists to hide.
    #[test]
    fn body_fields_are_flattened_into_the_arguments() {
        let create = operation("test_creatething");
        let names: Vec<&str> = create.parameters.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"tags"));

        let schema = create.schema();
        assert_eq!(schema["properties"]["name"]["type"], "string");
        assert_eq!(schema["required"][0], "name");
    }

    #[test]
    fn the_schema_refuses_improvised_fields() {
        assert_eq!(
            operation("test_getthing").schema()["additionalProperties"],
            false
        );
    }

    #[test]
    fn a_summary_becomes_the_description() {
        assert_eq!(operation("test_getthing").description, "Fetch one thing");
    }

    #[test]
    fn an_operation_with_no_prose_describes_itself_by_route() {
        let spec = json!({
            "openapi": "3.0.0",
            "paths": { "/ping": { "get": { "operationId": "ping" } } }
        });
        let operations = extract(&spec, "x");
        assert_eq!(operations[0].description, "GET /ping");
    }

    /// Forty good endpoints and one broken one should give forty tools.
    #[test]
    fn a_malformed_operation_does_not_break_the_import() {
        let spec = json!({
            "openapi": "3.0.0",
            "paths": {
                "/good": { "get": { "operationId": "good" } },
                "/bad": { "get": { "parameters": [ { "no_name": true } ] } }
            }
        });
        let operations = extract(&spec, "x");
        assert_eq!(operations.len(), 2);
        // The unusable parameter is dropped; the operation survives.
        let bad = operations.iter().find(|o| o.path == "/bad").unwrap();
        assert!(bad.parameters.is_empty());
    }

    #[test]
    fn duplicate_operation_ids_both_stay_callable() {
        let spec = json!({
            "openapi": "3.0.0",
            "paths": {
                "/a": { "get": { "operationId": "same" } },
                "/b": { "get": { "operationId": "same" } }
            }
        });
        let ids: Vec<String> = extract(&spec, "x").into_iter().map(|o| o.id).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[test]
    fn a_reference_outside_the_document_is_flagged_in_the_description() {
        let spec = json!({
            "openapi": "3.0.0",
            "paths": { "/x": { "get": {
                "operationId": "x",
                "parameters": [{ "$ref": "https://elsewhere/param.json" }]
            }}}
        });
        let operation = &extract(&spec, "x")[0];
        assert!(operation.unresolved);
        assert!(operation
            .description
            .contains("points outside the document"));
    }

    // ── request building ────────────────────────────────────────────────

    #[test]
    fn path_parameters_are_substituted() {
        let parts = operation("test_getthing").build_request(&json!({ "id": "abc" }));
        assert_eq!(parts.path, "/things/abc");
    }

    /// A slash in a path parameter would otherwise silently address a
    /// different endpoint.
    #[test]
    fn a_path_parameter_is_percent_encoded() {
        let parts = operation("test_getthing").build_request(&json!({ "id": "a/b c" }));
        assert_eq!(parts.path, "/things/a%2Fb%20c");
    }

    #[test]
    fn query_parameters_are_collected_separately() {
        let parts =
            operation("test_getthing").build_request(&json!({ "id": "1", "verbose": true }));
        assert_eq!(
            parts.query,
            vec![("verbose".to_string(), "true".to_string())]
        );
        assert!(parts.body.is_none());
    }

    #[test]
    fn body_fields_are_reassembled_into_a_body() {
        let parts = operation("test_creatething")
            .build_request(&json!({ "name": "thing", "tags": ["a", "b"] }));

        let body = parts.body.unwrap();
        assert_eq!(body["name"], "thing");
        assert_eq!(body["tags"][1], "b");
    }

    #[test]
    fn absent_arguments_contribute_nothing() {
        let parts = operation("test_getthing").build_request(&json!({ "id": "1" }));
        assert!(parts.query.is_empty());
    }

    #[test]
    fn a_null_argument_is_treated_as_absent() {
        let parts =
            operation("test_getthing").build_request(&json!({ "id": "1", "verbose": null }));
        assert!(parts.query.is_empty());
    }
}

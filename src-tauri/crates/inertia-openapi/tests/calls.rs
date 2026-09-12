//! Imported API tools against a real HTTP server.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;

use inertia_core::tool::{ToolContext, ToolProvider};
use inertia_core::{SessionId, ToolCallId};
use inertia_openapi::provider::{Auth, Credential, ImportRecord, OpenApiProvider};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn document(base: &str) -> serde_json::Value {
    json!({
        "openapi": "3.0.0",
        "info": { "title": "Things" },
        "servers": [{ "url": base }],
        "paths": {
            "/things/{id}": {
                "get": {
                    "operationId": "getThing",
                    "summary": "Fetch one thing",
                    "parameters": [
                        { "name": "id", "in": "path", "schema": { "type": "string" } },
                        { "name": "verbose", "in": "query", "schema": { "type": "boolean" } }
                    ]
                }
            },
            "/things": {
                "post": {
                    "operationId": "createThing",
                    "requestBody": { "content": { "application/json": { "schema": {
                        "type": "object",
                        "properties": { "name": { "type": "string" } },
                        "required": ["name"]
                    }}}}
                }
            }
        }
    })
}

fn context() -> ToolContext {
    ToolContext {
        root: std::path::PathBuf::from("."),
        session: SessionId::new(),
        call_id: ToolCallId::new(),
        permissions: Arc::new(inertia_mock::MockGate::allow_all()),
    }
}

async fn provider_for(server: &MockServer, auth: Auth, key: &str) -> OpenApiProvider {
    let provider = OpenApiProvider::new();
    provider
        .add(
            ImportRecord {
                id: "things".into(),
                name: "Things".into(),
                enabled: true,
                auth,
                ..Default::default()
            },
            &document(&server.uri()),
            Credential {
                value: key.to_string(),
            },
        )
        .unwrap();
    provider
}

async fn call(
    provider: &OpenApiProvider,
    id: &str,
    args: serde_json::Value,
) -> inertia_core::tool::ToolOutcome {
    let tools = provider.tools(Path::new(".")).await.unwrap();
    let tool = tools.iter().find(|t| t.id() == id).unwrap();
    tool.execute(args, &context()).await.unwrap()
}

#[tokio::test]
async fn a_get_substitutes_the_path_and_sends_the_query() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/things/abc"))
        .and(query_param("verbose", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "abc" })))
        .mount(&server)
        .await;

    let provider = provider_for(&server, Auth::None, "").await;
    let outcome = call(
        &provider,
        "things_getthing",
        json!({ "id": "abc", "verbose": true }),
    )
    .await;

    assert!(outcome.output.contains("200"), "got {}", outcome.output);
    assert!(outcome.output.contains("\"id\""));
}

#[tokio::test]
async fn a_post_sends_the_body_fields_as_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/things"))
        .and(wiremock::matchers::body_json(json!({ "name": "a thing" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "created": true })))
        .mount(&server)
        .await;

    let provider = provider_for(&server, Auth::None, "").await;
    let outcome = call(&provider, "things_creatething", json!({ "name": "a thing" })).await;

    assert!(outcome.output.contains("201"), "got {}", outcome.output);
}

#[tokio::test]
async fn a_bearer_credential_is_attached() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let provider = provider_for(
        &server,
        Auth::Bearer {
            secret: "TOKEN".into(),
        },
        "secret-token",
    )
    .await;

    let outcome = call(&provider, "things_getthing", json!({ "id": "1" })).await;
    assert!(outcome.output.contains("200"), "got {}", outcome.output);
}

#[tokio::test]
async fn an_api_key_goes_in_the_named_header() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("x-api-key", "abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let provider = provider_for(
        &server,
        Auth::ApiKey {
            name: "X-API-Key".into(),
            location: "header".into(),
            secret: "KEY".into(),
        },
        "abc123",
    )
    .await;

    let outcome = call(&provider, "things_getthing", json!({ "id": "1" })).await;
    assert!(outcome.output.contains("200"), "got {}", outcome.output);
}

#[tokio::test]
async fn an_api_key_can_go_in_the_query_string() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(query_param("api_key", "abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let provider = provider_for(
        &server,
        Auth::ApiKey {
            name: "api_key".into(),
            location: "query".into(),
            secret: "KEY".into(),
        },
        "abc123",
    )
    .await;

    let outcome = call(&provider, "things_getthing", json!({ "id": "1" })).await;
    assert!(outcome.output.contains("200"), "got {}", outcome.output);
}

/// A 404 or a 422 with a validation message is exactly what the model needs to
/// correct its call. Burying it in an error envelope hides the body.
#[tokio::test]
async fn a_non_2xx_response_is_the_answer_not_a_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(422)
                .set_body_json(json!({ "error": "id must be a number" })),
        )
        .mount(&server)
        .await;

    let provider = provider_for(&server, Auth::None, "").await;
    // Not `unwrap_err` - this succeeds as a call and reports the status.
    let outcome = call(&provider, "things_getthing", json!({ "id": "abc" })).await;

    assert!(outcome.output.contains("422"), "got {}", outcome.output);
    assert!(outcome.output.contains("must be a number"));
    assert_eq!(outcome.metadata.unwrap()["status"], 422);
}

#[tokio::test]
async fn an_empty_response_body_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let provider = provider_for(&server, Auth::None, "").await;
    let outcome = call(&provider, "things_getthing", json!({ "id": "1" })).await;
    assert!(outcome.output.contains("(no body)"), "got {}", outcome.output);
}

/// Following a redirect would carry the credential to a host the user never
/// named.
#[tokio::test]
async fn a_redirect_is_not_followed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "https://evil.example.com/steal"),
        )
        .mount(&server)
        .await;

    let provider = provider_for(
        &server,
        Auth::Bearer {
            secret: "TOKEN".into(),
        },
        "secret-token",
    )
    .await;

    let outcome = call(&provider, "things_getthing", json!({ "id": "1" })).await;
    // The redirect itself is reported; the credential never left.
    assert!(outcome.output.contains("302"), "got {}", outcome.output);
}

#[tokio::test]
async fn an_unreachable_api_fails_with_the_url() {
    let provider = OpenApiProvider::new();
    provider
        .add(
            ImportRecord {
                id: "dead".into(),
                name: "Dead".into(),
                enabled: true,
                ..Default::default()
            },
            &document("http://127.0.0.1:1"),
            Credential::default(),
        )
        .unwrap();

    let tools = provider.tools(Path::new(".")).await.unwrap();
    let tool = tools.iter().find(|t| t.id() == "dead_getthing").unwrap();

    let error = tool
        .execute(json!({ "id": "1" }), &context())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("127.0.0.1:1"), "got {error}");
}

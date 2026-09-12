//! An imported OpenAPI document as a source of tools.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{
    PermissionRequest, Tool, ToolContext, ToolOutcome, ToolProvider, ToolSource,
};
use inertia_core::{Error, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::operation::{self, Operation};
use crate::spec;

/// How an API is authenticated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Auth {
    #[default]
    None,
    /// A key in a named header or query parameter.
    ApiKey {
        name: String,
        /// `"header"` or `"query"`.
        #[serde(rename = "in")]
        location: String,
        /// The **name** of a secret, never the value.
        secret: String,
    },
    Bearer {
        secret: String,
    },
}

/// One imported document, as stored in the workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportRecord {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Where the spec came from, so it can be refreshed.
    pub source: String,
    /// Overrides the `servers` entry in the document, for an API served from
    /// somewhere other than the spec claims.
    pub base_url: String,
    pub auth: Auth,
    /// Which operations to offer, by tool id. Empty means all of them.
    ///
    /// Empty rather than a full list on import, because a spec gains and loses
    /// operations when it is refreshed: a stored list would silently freeze the
    /// import at the shape the document had on the day it arrived.
    #[serde(default)]
    pub operations: Vec<String>,
    #[serde(default)]
    pub tool_count: usize,
    #[serde(default)]
    pub error: String,
}

/// A resolved credential, ready to attach to a request.
///
/// Resolved from the secret store at call time and never stored on the record,
/// so a workspace folder carries the reference and not the key.
#[derive(Debug, Clone, Default)]
pub struct Credential {
    pub value: String,
}

/// One operation, callable.
pub struct ApiTool {
    operation: Operation,
    base_url: String,
    auth: Auth,
    credential: Credential,
    api_name: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for ApiTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiTool")
            .field("id", &self.operation.id)
            .finish()
    }
}

#[async_trait]
impl Tool for ApiTool {
    fn id(&self) -> &str {
        &self.operation.id
    }

    fn description(&self) -> &str {
        &self.operation.description
    }

    fn parameters(&self) -> Value {
        self.operation.schema()
    }

    fn source(&self) -> ToolSource {
        ToolSource::OpenApi
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        // Keyed on `openapi` with the tool id as target, so a rule can name
        // one operation, one import (`github_*`), or every imported API.
        PermissionRequest::new("openapi", self.operation.id.clone())
            .with_always(self.operation.id.clone())
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some(format!(
            "{} · {} {}",
            self.api_name, self.operation.method, self.operation.path
        ))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        self.call(args).await
    }
}

impl ApiTool {
    /// The call itself, with no turn around it.
    ///
    /// Split out of `execute` so the settings screen can make the same request
    /// the agent would. The "Call it" button exists to prove a base URL and a
    /// key before a turn depends on them, and proving it with a second,
    /// separately written request would prove the wrong thing - the bug it is
    /// meant to catch lives in exactly the code an independent copy would not
    /// share.
    pub async fn call(&self, args: Value) -> Result<ToolOutcome> {
        let parts = self.operation.build_request(&args);
        let url = format!("{}{}", self.base_url, parts.path);

        let method = reqwest::Method::from_bytes(self.operation.method.as_bytes())
            .map_err(|_| Error::InvalidInput(format!("{} is not an HTTP method.", self.operation.method)))?;

        let mut request = self.http.request(method, &url);

        for (name, value) in &parts.query {
            request = request.query(&[(name, value)]);
        }
        for (name, value) in &parts.headers {
            request = request.header(name, value);
        }

        // Auth last, so a header parameter in the spec cannot accidentally
        // overwrite the credential.
        match &self.auth {
            Auth::None => {}
            Auth::Bearer { .. } => {
                request = request.bearer_auth(&self.credential.value);
            }
            Auth::ApiKey { name, location, .. } => {
                if location == "query" {
                    request = request.query(&[(name.as_str(), self.credential.value.as_str())]);
                } else {
                    request = request.header(name.as_str(), self.credential.value.as_str());
                }
            }
        }

        if let Some(body) = &parts.body {
            request = request.json(body);
        }

        let response = request
            .send()
            .await
            .map_err(|e| Error::Network(format!("{url} could not be reached: {e}")))?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        // A non-2xx is *the answer*, not a tool failure. A 404 or a 422 with a
        // validation message is exactly what the model needs to correct its
        // call, and burying it in an error envelope hides the body.
        let mut output = format!("{} {}\n", status.as_u16(), status.canonical_reason().unwrap_or(""));
        if body.trim().is_empty() {
            output.push_str("(no body)");
        } else {
            output.push_str(body.trim());
        }

        Ok(ToolOutcome {
            title: Some(format!("{} {}", self.operation.method, parts.path)),
            output,
            metadata: Some(serde_json::json!({
                "status": status.as_u16(),
                "url": url,
            })),
            images: Vec::new(),
        })
    }
}

/// Every imported document, as one source of tools.
#[derive(Default)]
pub struct OpenApiProvider {
    imports: RwLock<Vec<Import>>,
    http: reqwest::Client,
}

struct Import {
    record: ImportRecord,
    operations: Vec<Operation>,
    base_url: String,
    credential: Credential,
}

impl std::fmt::Debug for OpenApiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenApiProvider")
            .field("imports", &self.imports.read().len())
            .finish()
    }
}

impl OpenApiProvider {
    pub fn new() -> Self {
        Self {
            imports: RwLock::new(Vec::new()),
            // Redirects are refused: following one would carry the
            // Authorization header to a host the user never named.
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
        }
    }

    /// Registers a parsed document.
    ///
    /// Returns how many operations it yielded, so the caller can tell the user
    /// whether the import actually did anything.
    pub fn add(
        &self,
        record: ImportRecord,
        document: &Value,
        credential: Credential,
    ) -> std::result::Result<usize, String> {
        let namespace = if record.name.is_empty() {
            record.id.clone()
        } else {
            record.name.clone()
        };

        // Operations first. A document with nothing callable in it is useless
        // whatever its base URL, and asking the user to configure a server for
        // an empty spec sends them down the wrong path entirely.
        let operations = operation::extract(document, &namespace);
        if operations.is_empty() {
            return Err("The document declares no operations.".into());
        }
        let count = operations.len();

        let base_url = if !record.base_url.is_empty() {
            record.base_url.trim_end_matches('/').to_string()
        } else {
            spec::base_url(document).ok_or_else(|| {
                "The document does not say where the API is served from. Set a base URL."
                    .to_string()
            })?
        };

        self.imports.write().retain(|i| i.record.id != record.id);
        self.imports.write().push(Import {
            record,
            operations,
            base_url,
            credential,
        });

        Ok(count)
    }

    pub fn remove(&self, id: &str) {
        self.imports.write().retain(|i| i.record.id != id);
    }

    /// Calls one operation of a stored import, without registering it.
    ///
    /// Deliberately not routed through the live provider. The provider holds
    /// only imports that are switched on and that parsed cleanly, and the one
    /// import somebody wants to test is very often neither - it is off because
    /// it does not work yet, which is why they are on this screen. Building the
    /// operation from the record and the document, the way `add` does, means
    /// the test says something about the import in front of them rather than
    /// about whether it happens to be loaded.
    pub async fn call_operation(
        &self,
        record: &ImportRecord,
        document: &Value,
        credential: Credential,
        operation_id: &str,
        args: Value,
    ) -> std::result::Result<ToolOutcome, String> {
        let namespace = if record.name.is_empty() {
            record.id.clone()
        } else {
            record.name.clone()
        };

        let operation = operation::extract(document, &namespace)
            .into_iter()
            .find(|op| op.id == operation_id)
            .ok_or_else(|| format!("{namespace} has no operation called {operation_id}."))?;

        let base_url = if record.base_url.is_empty() {
            spec::base_url(document).ok_or_else(|| {
                "The document does not say where the API is served from. Set a base URL."
                    .to_string()
            })?
        } else {
            record.base_url.trim_end_matches('/').to_string()
        };

        ApiTool {
            operation,
            base_url,
            auth: record.auth.clone(),
            credential,
            api_name: record.name.clone(),
            http: self.http.clone(),
        }
        .call(args)
        .await
        .map_err(|e| e.to_string())
    }

    pub fn imported(&self) -> Vec<ImportRecord> {
        self.imports
            .read()
            .iter()
            .map(|i| i.record.clone())
            .collect()
    }
}

#[async_trait]
impl ToolProvider for OpenApiProvider {
    fn name(&self) -> &'static str {
        "openapi"
    }

    async fn tools(&self, _root: &Path) -> Result<Vec<Arc<dyn Tool>>> {
        let imports = self.imports.read();
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

        for import in imports.iter() {
            if !import.record.enabled {
                continue;
            }
            for operation in &import.operations {
                tools.push(Arc::new(ApiTool {
                    operation: operation.clone(),
                    base_url: import.base_url.clone(),
                    auth: import.record.auth.clone(),
                    credential: import.credential.clone(),
                    api_name: import.record.name.clone(),
                    http: self.http.clone(),
                }));
            }
        }

        Ok(tools)
    }

    fn invalidate(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> Value {
        json!({
            "openapi": "3.0.0",
            "info": { "title": "Test" },
            "servers": [{ "url": "https://api.example.com" }],
            "paths": {
                "/things": { "get": { "operationId": "listThings" } }
            }
        })
    }

    fn record() -> ImportRecord {
        ImportRecord {
            id: "test".into(),
            name: "Test".into(),
            enabled: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn an_import_contributes_its_operations() {
        let provider = OpenApiProvider::new();
        let count = provider
            .add(record(), &document(), Credential::default())
            .unwrap();
        assert_eq!(count, 1);

        let tools = provider.tools(Path::new(".")).await.unwrap();
        assert_eq!(tools[0].id(), "test_listthings");
    }

    #[tokio::test]
    async fn a_disabled_import_contributes_nothing() {
        let provider = OpenApiProvider::new();
        provider
            .add(
                ImportRecord {
                    enabled: false,
                    ..record()
                },
                &document(),
                Credential::default(),
            )
            .unwrap();

        assert!(provider.tools(Path::new(".")).await.unwrap().is_empty());
    }

    /// The name in the message is the one on screen. "No operation called
    /// test_missing" is findable in the dropdown the person just used; a bare
    /// "not found" is not.
    #[tokio::test]
    async fn calling_an_operation_the_document_does_not_have_says_which() {
        let error = OpenApiProvider::new()
            .call_operation(
                &record(),
                &document(),
                Credential::default(),
                "test_missing",
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("test_missing"), "got {error}");
        assert!(error.contains("Test"), "got {error}");
    }

    /// The test call is the whole reason this screen exists: somebody is on it
    /// because the import does not work, and an import that does not work is
    /// usually one they switched off. Refusing to test it would refuse exactly
    /// the case the button is for.
    #[tokio::test]
    async fn a_disabled_import_can_still_be_tested() {
        let off = ImportRecord {
            enabled: false,
            // Port 1 refuses at once rather than hanging, so the call gets all
            // the way to the socket without this test touching the network.
            base_url: "http://127.0.0.1:1".into(),
            ..record()
        };

        // Never registered, and switched off - yet it reaches the request. The
        // failure is the connection, not the import's state.
        let error = OpenApiProvider::new()
            .call_operation(
                &off,
                &document(),
                Credential::default(),
                "test_listthings",
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("127.0.0.1:1"), "got {error}");
    }

    /// The record's base URL overrides the document's, and the override is the
    /// field people actually get wrong - so it has to be the one that wins here
    /// too, or the test would pass against a host the agent will never call.
    #[tokio::test]
    async fn a_document_with_no_server_and_no_override_says_so() {
        let no_server = json!({
            "openapi": "3.0.0",
            "info": { "title": "Test" },
            "paths": { "/things": { "get": { "operationId": "listThings" } } }
        });
        let error = OpenApiProvider::new()
            .call_operation(
                &record(),
                &no_server,
                Credential::default(),
                "test_listthings",
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("base URL"), "got {error}");
    }

    /// An import with nothing callable in it is a failed import, not a silent
    /// success the user has to discover later.
    #[tokio::test]
    async fn a_document_with_no_operations_is_refused() {
        let empty = json!({ "openapi": "3.0.0", "paths": {} });
        let error = OpenApiProvider::new()
            .add(record(), &empty, Credential::default())
            .unwrap_err();
        assert!(error.contains("no operations"), "got {error}");
    }

    #[tokio::test]
    async fn a_document_with_no_server_asks_for_a_base_url() {
        let no_server = json!({
            "openapi": "3.0.0",
            "paths": { "/x": { "get": { "operationId": "x" } } }
        });
        let error = OpenApiProvider::new()
            .add(record(), &no_server, Credential::default())
            .unwrap_err();
        assert!(error.contains("base URL"), "got {error}");
    }

    #[tokio::test]
    async fn a_configured_base_url_overrides_the_document() {
        let provider = OpenApiProvider::new();
        provider
            .add(
                ImportRecord {
                    base_url: "https://staging.example.com/".into(),
                    ..record()
                },
                &document(),
                Credential::default(),
            )
            .unwrap();

        assert_eq!(
            provider.imports.read()[0].base_url,
            "https://staging.example.com"
        );
    }

    #[tokio::test]
    async fn re_importing_replaces_rather_than_duplicates() {
        let provider = OpenApiProvider::new();
        provider
            .add(record(), &document(), Credential::default())
            .unwrap();
        provider
            .add(record(), &document(), Credential::default())
            .unwrap();

        assert_eq!(provider.tools(Path::new(".")).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn removing_an_import_removes_its_tools() {
        let provider = OpenApiProvider::new();
        provider
            .add(record(), &document(), Credential::default())
            .unwrap();
        provider.remove("test");
        assert!(provider.tools(Path::new(".")).await.unwrap().is_empty());
    }

    /// The key is a reference on the record and a value only at call time.
    #[test]
    fn the_stored_record_carries_a_secret_name_not_a_key() {
        let record = ImportRecord {
            auth: Auth::Bearer {
                secret: "GITHUB_TOKEN".into(),
            },
            ..record()
        };
        let stored = serde_json::to_string(&record).unwrap();
        assert!(stored.contains("GITHUB_TOKEN"));
        assert!(stored.contains("bearer"));
    }
}

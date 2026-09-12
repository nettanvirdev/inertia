//! The Composio HTTP API.
//!
//! Nine endpoints over `reqwest`, no SDK. Composio's model is three-layered
//! and worth stating once, because the names are not self-explanatory:
//!
//!   - a **toolkit** is an app (`gmail`, `slack`),
//!   - an **auth config** is Composio's record of *how* to log into that app,
//!     usually its own managed OAuth client,
//!   - a **connected account** is one person's actual credential for it.
//!
//! Connecting an app means finding or creating the auth config, then walking
//! the user through an OAuth link to produce the connected account.

use serde_json::Value;

pub const DEFAULT_BASE_URL: &str = "https://backend.composio.dev";

/// Every request is abandoned after this.
const REQUEST_TIMEOUT_SECS: u64 = 30;

/// Rows asked for per page on a cursor-paged endpoint.
///
/// A cursor-paged list cannot be fetched in parallel - page two's cursor is in
/// page one - so the only lever on how long the catalogue takes is how few
/// round trips it costs. At 100 a catalogue of several hundred apps was six or
/// more serial requests, each of which can sit for seconds. A server that caps
/// the limit lower simply returns fewer rows and the cursor still walks, so
/// asking for more is never wrong, only sometimes ignored.
const PAGE_SIZE: &str = "500";

// Hard ceilings on paged endpoints. A server that never stops paging cannot
// hold the app hostage.
const MAX_TOOLKITS: usize = 3_000;
const MAX_TOOLS_PER_TOOLKIT: usize = 500;
const MAX_ACCOUNTS: usize = 1_000;

#[derive(Debug, thiserror::Error)]
pub enum ComposioError {
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    Network(String),
}

type Result<T> = std::result::Result<T, ComposioError>;

/// Turns a failed response into a sentence the user can act on.
pub fn describe_failure(status: u16, url: &str, body: &str) -> String {
    let headline = match status {
        401 => "Composio rejected the API key.".to_string(),
        403 => "That key is not allowed to do that.".to_string(),
        404 => format!("Composio has nothing at {url}."),
        429 => "Rate limited by Composio.".to_string(),
        500 => "Composio had an internal error.".to_string(),
        502 => "Composio was unreachable.".to_string(),
        503 => "Composio is overloaded.".to_string(),
        other => format!("Composio answered {other}."),
    };

    // The server's own message is usually the specific part.
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|d| !d.trim().is_empty());

    match detail {
        Some(detail) => format!("{headline} {detail}"),
        None => headline,
    }
}

/// Walks a cursor-paged endpoint.
///
/// The repeated-cursor check is not defensive programming for its own sake: a
/// server echoing back the cursor it was given caused unbounded re-fetching
/// once already. Treating a repeat as the end is always safe - the worst case
/// is one page short, against an infinite loop.
pub async fn collect_pages<F, Fut>(mut fetch: F, max: usize) -> Result<Vec<Value>>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<Value>, Option<String>)>>,
{
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen: Vec<String> = Vec::new();

    loop {
        let (page, next) = fetch(cursor.clone()).await?;
        items.extend(page);

        let Some(next) = next.filter(|c| !c.is_empty()) else {
            break;
        };
        if seen.contains(&next) {
            break;
        }
        if items.len() >= max {
            break;
        }

        seen.push(next.clone());
        cursor = Some(next);
    }

    items.truncate(max);
    Ok(items)
}

#[derive(Clone)]
pub struct Composio {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for Composio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Composio")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl Composio {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(DEFAULT_BASE_URL, api_key)
    }

    pub fn with_base_url(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
                // The key must never be replayed to a redirect target.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
        }
    }

    async fn send(
        &self,
        method: reqwest::Method,
        route: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value> {
        let url = format!("{}{}", self.base_url, route);
        let mut request = self
            .http
            .request(method, &url)
            .header("x-api-key", &self.api_key);

        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request
            .send()
            .await
            .map_err(|e| ComposioError::Network(describe_network_error(&e, &url)))?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            return Err(ComposioError::Failed(describe_failure(
                status.as_u16(),
                &url,
                &text,
            )));
        }

        if text.trim().is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&text)
            .map_err(|e| ComposioError::Failed(format!("Composio sent something unreadable: {e}")))
    }

    /// Every app Composio knows about.
    pub async fn toolkits(&self) -> Result<Vec<Value>> {
        collect_pages(
            |cursor| async move {
                let mut query = vec![("limit", PAGE_SIZE.to_string())];
                if let Some(cursor) = cursor {
                    query.push(("cursor", cursor));
                }
                let page = self
                    .send(reqwest::Method::GET, "/api/v3/toolkits", &query, None)
                    .await?;
                Ok((items_of(&page), cursor_of(&page)))
            },
            MAX_TOOLKITS,
        )
        .await
    }

    /// Every toolkit slug this project already has an auth config for.
    ///
    /// Fetched once per catalogue read so every toolkit in the pass is
    /// classified against the same snapshot of the project. Doing it per
    /// toolkit would be hundreds of round trips to answer one question.
    pub async fn auth_config_slugs(&self) -> Result<std::collections::BTreeSet<String>> {
        let items = collect_pages(
            |cursor| async move {
                let mut query = vec![("limit", PAGE_SIZE.to_string())];
                if let Some(cursor) = cursor {
                    query.push(("cursor", cursor));
                }
                let page = self
                    .send(reqwest::Method::GET, "/api/v3/auth_configs", &query, None)
                    .await?;
                Ok((items_of(&page), cursor_of(&page)))
            },
            MAX_TOOLKITS,
        )
        .await?;

        Ok(items
            .iter()
            .filter_map(|item| {
                item.pointer("/toolkit/slug")
                    .or_else(|| item.get("toolkit_slug"))
                    .or_else(|| item.get("toolkit"))
                    .and_then(Value::as_str)
            })
            .map(|slug| slug.trim().to_lowercase())
            .filter(|slug| !slug.is_empty())
            .collect())
    }

    /// Every operation one app exposes.
    pub async fn tools(&self, toolkit: &str) -> Result<Vec<Value>> {
        let toolkit = toolkit.to_string();
        collect_pages(
            |cursor| {
                let toolkit = toolkit.clone();
                async move {
                    let mut query = vec![
                        ("toolkit_slug", toolkit),
                        ("limit", "100".to_string()),
                    ];
                    if let Some(cursor) = cursor {
                        query.push(("cursor", cursor));
                    }
                    let page = self
                        .send(reqwest::Method::GET, "/api/v3/tools", &query, None)
                        .await?;
                    Ok((items_of(&page), cursor_of(&page)))
                }
            },
            MAX_TOOLS_PER_TOOLKIT,
        )
        .await
    }

    /// The accounts this key has connected.
    pub async fn connected_accounts(&self) -> Result<Vec<Value>> {
        collect_pages(
            |cursor| async move {
                let mut query = vec![("limit", PAGE_SIZE.to_string())];
                if let Some(cursor) = cursor {
                    query.push(("cursor", cursor));
                }
                let page = self
                    .send(
                        reqwest::Method::GET,
                        "/api/v3/connected_accounts",
                        &query,
                        None,
                    )
                    .await?;
                Ok((items_of(&page), cursor_of(&page)))
            },
            MAX_ACCOUNTS,
        )
        .await
    }

    pub async fn connected_account(&self, id: &str) -> Result<Value> {
        self.send(
            reqwest::Method::GET,
            &format!("/api/v3/connected_accounts/{id}"),
            &[],
            None,
        )
        .await
    }

    /// Finds the auth config for a toolkit, creating one if there is none.
    ///
    /// **Filtered by `toolkit_slug`, deliberately.** An unfiltered list returns
    /// any app's config, and using the wrong one meant connecting Gmail
    /// sometimes opened Instagram's consent page.
    pub async fn auth_config_for(&self, toolkit: &str) -> Result<String> {
        let existing = self
            .send(
                reqwest::Method::GET,
                "/api/v3/auth_configs",
                &[("toolkit_slug", toolkit.to_string())],
                None,
            )
            .await?;

        if let Some(id) = items_of(&existing)
            .first()
            .and_then(|config| config.get("id"))
            .and_then(Value::as_str)
        {
            return Ok(id.to_string());
        }

        let created = self
            .send(
                reqwest::Method::POST,
                "/api/v3/auth_configs",
                &[],
                Some(serde_json::json!({
                    "toolkit": { "slug": toolkit },
                    "auth_config": { "type": "use_composio_managed_auth" }
                })),
            )
            .await?;

        created
            .pointer("/auth_config/id")
            .or_else(|| created.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                ComposioError::Failed("Composio created no auth config for that app.".into())
            })
    }

    /// One app's own row: its display name and its logo.
    ///
    /// Read when a connection is made rather than when a row is drawn, so the
    /// list of connected apps stays readable with the network off.
    pub async fn toolkit(&self, slug: &str) -> Result<Value> {
        self.send(
            reqwest::Method::GET,
            &format!("/api/v3/toolkits/{slug}"),
            &[],
            None,
        )
        .await
    }

    /// Starts an OAuth handshake, returning the URL the user must visit.
    ///
    /// `user_id` is Composio's idea of whose account this is. It is carried
    /// rather than left to Composio so that reconnecting names the same person
    /// as the original link did - anything scoped to that id on Composio's side
    /// goes on referring to one account instead of accumulating one per repair.
    pub async fn start_connection(
        &self,
        auth_config_id: &str,
        user_id: Option<&str>,
    ) -> Result<Connection> {
        let mut body = serde_json::json!({ "auth_config_id": auth_config_id });
        if let (Some(map), Some(user)) = (body.as_object_mut(), user_id) {
            map.insert("user_id".into(), Value::String(user.to_string()));
        }

        let started = self
            .send(
                reqwest::Method::POST,
                "/api/v3/connected_accounts/link",
                &[],
                Some(body),
            )
            .await?;

        let id = started
            .get("id")
            .or_else(|| started.pointer("/connectedAccount/id"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let redirect_url = started
            .get("redirect_url")
            .or_else(|| started.get("redirectUrl"))
            .and_then(Value::as_str)
            .map(str::to_string);

        Ok(Connection { id, redirect_url })
    }

    pub async fn revoke(&self, account_id: &str) -> Result<()> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/api/v3/connected_accounts/{account_id}"),
            &[],
            None,
        )
        .await
        .map(|_| ())
    }

    /// Runs one tool.
    pub async fn execute(
        &self,
        slug: &str,
        arguments: Value,
        connected_account_id: Option<&str>,
    ) -> Result<Value> {
        let mut body = serde_json::json!({ "arguments": arguments });
        if let (Some(map), Some(account)) = (body.as_object_mut(), connected_account_id) {
            map.insert(
                "connected_account_id".into(),
                Value::String(account.to_string()),
            );
        }

        self.send(
            reqwest::Method::POST,
            &format!("/api/v3/tools/execute/{slug}"),
            &[],
            Some(body),
        )
        .await
    }
}

/// An OAuth handshake in progress.
#[derive(Debug, Clone)]
pub struct Connection {
    pub id: String,
    /// Where the user must go to approve it.
    pub redirect_url: Option<String>,
}

/// The list inside a paged response, whichever key it arrived under.
fn items_of(page: &Value) -> Vec<Value> {
    for key in ["items", "data", "results"] {
        if let Some(items) = page.get(key).and_then(Value::as_array) {
            return items.clone();
        }
    }
    // Some endpoints return a bare array.
    page.as_array().cloned().unwrap_or_default()
}

fn cursor_of(page: &Value) -> Option<String> {
    for key in ["next_cursor", "nextCursor", "cursor"] {
        if let Some(cursor) = page.get(key).and_then(Value::as_str) {
            return Some(cursor.to_string());
        }
    }
    None
}

fn describe_network_error(error: &reqwest::Error, url: &str) -> String {
    if error.is_timeout() {
        return "Composio did not answer in time.".into();
    }
    if error.is_connect() {
        return format!("Could not reach {url}.");
    }
    format!("The request to Composio failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_rejected_key_is_named_plainly() {
        let message = describe_failure(401, "https://x", "");
        assert!(message.contains("rejected the API key"));
    }

    /// Composio's own message is usually the specific part.
    #[test]
    fn the_servers_own_message_is_kept() {
        let message = describe_failure(
            400,
            "https://x",
            r#"{"error":{"message":"toolkit_slug is required"}}"#,
        );
        assert!(message.contains("toolkit_slug is required"), "got {message}");
    }

    #[test]
    fn an_unreadable_error_body_still_gives_a_headline() {
        let message = describe_failure(500, "https://x", "<html>oops</html>");
        assert!(message.contains("internal error"));
    }

    #[test]
    fn items_are_found_under_any_of_the_usual_keys() {
        assert_eq!(items_of(&json!({ "items": [1] })).len(), 1);
        assert_eq!(items_of(&json!({ "data": [1, 2] })).len(), 2);
        assert_eq!(items_of(&json!([1, 2, 3])).len(), 3);
        assert!(items_of(&json!({ "nothing": true })).is_empty());
    }

    #[tokio::test]
    async fn paging_walks_to_the_end() {
        let pages = [
            (vec![json!(1), json!(2)], Some("b".to_string())),
            (vec![json!(3)], None),
        ];
        let index = std::sync::atomic::AtomicUsize::new(0);

        let items = collect_pages(
            |_| {
                let at = index.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let page = pages[at].clone();
                async move { Ok(page) }
            },
            100,
        )
        .await
        .unwrap();

        assert_eq!(items.len(), 3);
    }

    /// A server echoing back the cursor it was given caused unbounded
    /// re-fetching once already.
    #[tokio::test]
    async fn a_repeated_cursor_ends_the_walk() {
        let calls = std::sync::atomic::AtomicUsize::new(0);

        let items = collect_pages(
            |_| {
                calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // Always the same cursor, forever.
                async move { Ok((vec![json!(1)], Some("stuck".to_string()))) }
            },
            10_000,
        )
        .await
        .unwrap();

        assert_eq!(items.len(), 2, "it should stop on the repeat");
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn paging_respects_its_ceiling() {
        let items = collect_pages(
            |cursor| {
                // A fresh cursor every time, so only the ceiling stops it.
                let next = cursor.map(|c| format!("{c}x")).unwrap_or_else(|| "a".into());
                async move { Ok(([json!(1), json!(2)].to_vec(), Some(next))) }
            },
            5,
        )
        .await
        .unwrap();

        assert_eq!(items.len(), 5);
    }

    #[test]
    fn the_key_is_never_printed() {
        let client = Composio::new("sk-composio-secret");
        assert!(!format!("{client:?}").contains("sk-composio-secret"));
    }
}

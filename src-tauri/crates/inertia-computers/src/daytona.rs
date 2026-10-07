//! A sandbox in Daytona's cloud. Keeps running when Inertia is closed.
//!
//! # The two hosts
//!
//! The control plane - make a sandbox, start it, stop it - is on the API host.
//! The **toolbox** - running a command, reading a file - is NOT. It lives on a
//! separate proxy whose address the sandbox object carries in
//! `toolboxProxyUrl`, and the paths under it have no `/toolbox` segment of
//! their own.
//!
//! That is worth writing down because guessing it is expensive: every shape
//! that looks reasonable answers 404 from the API host, and the address was in
//! a field of the sandbox object all along. The proxy is remembered per sandbox
//! so every command does not have to ask, and forgotten when the sandbox goes,
//! because a new one gets a new proxy.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::image;
use crate::{
    machine_name, shell_quote, ComputerError, Created, DirEntry, ExecRequest, ExecResult, Health,
    Provider, Readiness, Result, Snapshot, Spec, Stats, Status, DEFAULT_TIMEOUT,
};

/// The name of the workspace secret holding the key.
pub const KEY_SECRET: &str = "DAYTONA_API_KEY";

/// Overridable so a self-hosted Daytona - or a mock - can be pointed at.
pub const DEFAULT_API_URL: &str = "https://app.daytona.io/api";

/// Long enough for a cold sandbox to come up, short enough to fail visibly.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct DaytonaProvider {
    api_url: String,
    api_key: String,
    http: reqwest::Client,
    /// Where each sandbox's toolbox is. Cleared when a sandbox goes.
    proxies: Arc<Mutex<HashMap<String, String>>>,
}

/// A sandbox, as this app understands it.
#[derive(Debug, Clone)]
pub struct Sandbox {
    pub handle: String,
    pub status: Status,
    pub image: Option<String>,
    pub started_at: Option<String>,
    pub toolbox_proxy_url: Option<String>,
    pub user: Option<String>,
}

/// Daytona's state words, as the app's.
///
/// Anything unrecognised reads as `Error` rather than as `Running`, because a
/// machine in a state we do not understand is one no command should be sent to.
/// An empty state is `Provisioning`: a sandbox that has just been asked for has
/// not got one yet.
pub(crate) fn map_state(raw: &str) -> Status {
    match raw.trim().to_lowercase().as_str() {
        "started" | "running" => Status::Running,
        "creating" | "pending" | "starting" | "restoring" => Status::Provisioning,
        "stopping" | "stopped" | "archived" => Status::Stopped,
        "paused" | "suspended" => Status::Paused,
        "destroyed" | "destroying" | "deleted" => Status::Missing,
        "" => Status::Provisioning,
        _ => Status::Error,
    }
}

/// Reads the parts of a sandbox object this app uses.
///
/// Every field is tried under both of the names Daytona has used for it. The
/// API renamed several between versions, and a self-hosted instance can be on
/// either - reading both costs nothing and is the difference between working
/// and a blank screen.
pub(crate) fn read_sandbox(payload: &Value) -> Option<Sandbox> {
    if payload.is_null() {
        return None;
    }
    let text = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .find_map(|key| payload.get(*key).and_then(Value::as_str))
            .map(str::to_string)
    };

    let raw = text(&["state", "status"]).unwrap_or_default();
    Some(Sandbox {
        handle: text(&["id", "sandboxId"]).unwrap_or_default(),
        status: map_state(&raw),
        image: text(&["image", "snapshot"]),
        started_at: text(&["startedAt", "createdAt"]),
        toolbox_proxy_url: text(&["toolboxProxyUrl"]),
        user: text(&["user"]),
    })
}

impl DaytonaProvider {
    pub fn new(api_key: impl Into<String>, api_url: Option<String>) -> Self {
        let api_url = api_url
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_API_URL.to_string());

        Self {
            api_url: api_url.trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_default(),
            proxies: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Where this provider is pointed, after the default and the trailing
    /// slashes have been settled. The secrets pane says it back to the person
    /// testing their key, because an endpoint override is the setting most
    /// likely to be the thing that is wrong.
    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    /// One control-plane request.
    ///
    /// A 404 answers `None` rather than failing: a sandbox that is gone is an
    /// ordinary state of the world here, not an error.
    async fn api(
        &self,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
    ) -> Result<Option<Value>> {
        if self.api_key.is_empty() {
            return Err(ComputerError::NotReady(format!(
                "No {KEY_SECRET} is set. Add it under Settings, Secrets."
            )));
        }
        self.request(method, format!("{}{route}", self.api_url), body)
            .await
    }

    async fn request(
        &self,
        method: reqwest::Method,
        url: String,
        body: Option<Value>,
    ) -> Result<Option<Value>> {
        let mut request = self
            .http
            .request(method, &url)
            .bearer_auth(&self.api_key)
            .header("accept", "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }

        // A network failure and a refusal are different problems with different
        // fixes, and collapsing them into "request failed" costs the user the
        // difference.
        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                ComputerError::Timeout(REQUEST_TIMEOUT)
            } else {
                ComputerError::Failed(format!("Cannot reach Daytona: {e}"))
            }
        })?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ComputerError::Failed(describe_failure(
                status.as_u16(),
                &body,
            )));
        }

        let text = response.text().await.unwrap_or_default();
        if text.trim().is_empty() {
            return Ok(Some(json!({})));
        }
        Ok(Some(
            serde_json::from_str(&text).unwrap_or(json!({ "raw": text })),
        ))
    }

    /// Where this sandbox's toolbox lives, asking the API only when it must.
    async fn proxy_for(&self, handle: &str) -> Result<String> {
        if let Some(known) = self.proxies.lock().await.get(handle) {
            return Ok(known.clone());
        }

        let payload = self
            .api(reqwest::Method::GET, &format!("/sandbox/{handle}"), None)
            .await?
            .ok_or_else(|| ComputerError::Failed("That sandbox is gone.".into()))?;

        let sandbox = read_sandbox(&payload).ok_or_else(|| {
            ComputerError::Failed("Daytona described a sandbox this app could not read.".into())
        })?;

        let proxy = sandbox.toolbox_proxy_url.ok_or_else(|| {
            ComputerError::Failed(
                "This sandbox has no toolbox address yet. It may still be starting.".into(),
            )
        })?;
        let proxy = proxy.trim_end_matches('/').to_string();
        self.proxies
            .lock()
            .await
            .insert(handle.to_string(), proxy.clone());
        Ok(proxy)
    }

    async fn toolbox(
        &self,
        handle: &str,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
    ) -> Result<Option<Value>> {
        let proxy = self.proxy_for(handle).await?;
        self.request(method, format!("{proxy}{route}"), body).await
    }

    async fn forget_proxy(&self, handle: &str) {
        self.proxies.lock().await.remove(handle);
    }
}

fn describe_failure(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|parsed| {
            parsed
                .get("message")
                .or_else(|| parsed.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string());
    let detail: String = detail.chars().take(300).collect();

    let lead = match status {
        401 | 403 => {
            format!("Daytona rejected the key. Check {KEY_SECRET} under Settings, Secrets.")
        }
        402 => {
            "Daytona refused for billing reasons. Check the account's plan or credit.".to_string()
        }
        429 => "Daytona is rate limiting this key right now.".to_string(),
        other => format!("Daytona answered {other}."),
    };
    if detail.is_empty() {
        lead
    } else {
        format!("{lead} {detail}")
    }
}

/// The snapshot a sandbox was asked to be made from is not on the account.
///
/// The default name is only on an account where someone pushed the sandbox
/// image under it, so this is the first thing anyone new to Daytona meets,
/// and it has to say what to do.
fn missing_snapshot(name: &str, said: Option<&str>) -> ComputerError {
    let lead = said.map(|said| format!("{said} ")).unwrap_or_default();
    ComputerError::Failed(format!(
        "{lead}Daytona has no snapshot called `{name}` on this account. Build the sandbox \
         image, push it to Daytona as a snapshot with that name (see docs/computers.md), or \
         pick a snapshot the account has when making the computer."
    ))
}

/// What an `execute` answer said. The field names differ between versions.
pub(crate) fn read_exec(payload: &Value, duration_ms: u64) -> ExecResult {
    let number = |keys: &[&str]| -> Option<i64> {
        keys.iter()
            .find_map(|key| payload.get(*key).and_then(Value::as_i64))
    };
    let text = |keys: &[&str]| -> String {
        keys.iter()
            .find_map(|key| payload.get(*key).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    };

    // Daytona returns one combined stream on some versions and two on others.
    // `result` is the combined one, and it goes to stdout rather than being
    // dropped.
    let stdout = text(&["stdout", "result", "output"]);
    let stderr = text(&["stderr", "error"]);

    ExecResult {
        // A toolbox version that reports no exit code has not said the command
        // worked. Assuming zero turned every silent failure into an empty
        // success - which is how a missing file reads as an empty file.
        code: number(&["exitCode", "code", "exit_code"]).unwrap_or(if stderr.trim().is_empty() {
            0
        } else {
            1
        }) as i32,
        stdout,
        stderr,
        duration_ms,
        timed_out: false,
    }
}

#[async_trait]
impl Provider for DaytonaProvider {
    fn id(&self) -> &'static str {
        "daytona"
    }

    fn label(&self) -> &'static str {
        "Daytona"
    }

    fn blurb(&self) -> &'static str {
        "A sandbox in the cloud. Keeps running when Inertia is closed."
    }

    async fn available(&self) -> Readiness {
        if self.api_key.is_empty() {
            return Readiness::not(format!("No {KEY_SECRET} is set.")).hint(
                "Add it under Settings, Secrets. A self-hosted Daytona can also set DAYTONA_API_URL.",
            );
        }
        match self.api(reqwest::Method::GET, "/sandbox", None).await {
            Ok(_) => Readiness::ready(),
            Err(e) => Readiness::not(e.to_string()),
        }
    }

    async fn create(&self, spec: &Spec) -> Result<Created> {
        let manifest = image::manifest();
        // What the person picked, then the name the image manifest publishes
        // for this exact purpose, then the local image reference as a last
        // resort. It used to be only the last of those: `inertia-sandbox:1.0.0`
        // is what the container is called on this machine, and Daytona has
        // never heard of it - the snapshot on the account is
        // `inertia-sandbox-1.0.0`, which `daytonaSnapshot` has been carrying
        // in the manifest all along with nothing reading it.
        let snapshot = spec
            .image
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(if manifest.daytona_snapshot.is_empty() {
                &manifest.reference
            } else {
                &manifest.daytona_snapshot
            });

        let mut body = json!({
            "snapshot": snapshot,
            "labels": { "inertia": "1", "inertia-id": spec.id },
        });
        // Only when the record asked. A limit the user did not choose is worse
        // than none, and Daytona's own defaults are sane.
        if let Some(cpu) = spec.cpu.filter(|c| *c > 0.0) {
            body["cpu"] = json!(cpu);
        }
        if let Some(memory) = spec.memory_gb.filter(|m| *m > 0.0) {
            body["memory"] = json!(memory);
        }
        if let Some(disk) = spec.disk_gb.filter(|d| *d > 0.0) {
            body["disk"] = json!(disk);
        }

        // A snapshot that is not on the account comes back as a 404, or as a
        // 400 whose message names it, depending on the version. Either way the
        // person needs to know which name was asked for and how to make it,
        // not that "Daytona did not make a sandbox".
        let payload = match self
            .api(reqwest::Method::POST, "/sandbox", Some(body))
            .await
        {
            Ok(Some(payload)) => payload,
            Ok(None) => return Err(missing_snapshot(snapshot, None)),
            Err(ComputerError::Failed(said)) if said.to_lowercase().contains("snapshot") => {
                return Err(missing_snapshot(snapshot, Some(&said)))
            }
            Err(other) => return Err(other),
        };
        let sandbox = read_sandbox(&payload).ok_or_else(|| {
            ComputerError::Failed("Daytona described a sandbox this app could not read.".into())
        })?;
        if sandbox.handle.is_empty() {
            return Err(ComputerError::Failed(
                "Daytona made a sandbox without giving it an id.".into(),
            ));
        }

        Ok(Created {
            handle: sandbox.handle,
            name: machine_name(&spec.id),
            status: sandbox.status,
            os: "Linux (Daytona sandbox)".into(),
            image: sandbox.image.unwrap_or(manifest.reference),
            workdir: manifest.workdir,
        })
    }

    async fn start(&self, handle: &str) -> Result<()> {
        // A restarted sandbox gets a new proxy, so the old address must not be
        // reused - that was a 404 on the first command after every start.
        self.forget_proxy(handle).await;
        self.api(
            reqwest::Method::POST,
            &format!("/sandbox/{handle}/start"),
            None,
        )
        .await?;
        Ok(())
    }

    async fn stop(&self, handle: &str) -> Result<()> {
        self.forget_proxy(handle).await;
        self.api(
            reqwest::Method::POST,
            &format!("/sandbox/{handle}/stop"),
            None,
        )
        .await?;
        Ok(())
    }

    /// Daytona has no pause, so stopping is the closest honest thing.
    ///
    /// An archived sandbox keeps its disk and costs nothing to keep, which is
    /// what a person means by pause here.
    async fn pause(&self, handle: &str) -> Result<()> {
        self.stop(handle).await
    }

    async fn resume(&self, handle: &str) -> Result<()> {
        self.start(handle).await
    }

    async fn remove(&self, handle: &str, _name: Option<&str>) -> Result<()> {
        self.forget_proxy(handle).await;
        self.api(reqwest::Method::DELETE, &format!("/sandbox/{handle}"), None)
            .await?;
        Ok(())
    }

    async fn status(&self, handle: &str) -> Health {
        let payload = self
            .api(reqwest::Method::GET, &format!("/sandbox/{handle}"), None)
            .await;

        match payload {
            Ok(Some(payload)) => match read_sandbox(&payload) {
                Some(sandbox) => Health {
                    status: sandbox.status,
                    started_at: sandbox.started_at,
                },
                None => Health {
                    status: Status::Missing,
                    started_at: None,
                },
            },
            // A 404 and an unreachable API are different, but neither is a
            // running machine, and polling a dead sandbox every few seconds is
            // what this screen does by design.
            _ => Health {
                status: Status::Missing,
                started_at: None,
            },
        }
    }

    /// Nothing is reported.
    ///
    /// Daytona's API exposes the sandbox's allocation, not its live usage, and
    /// a gauge showing "4 CPUs allocated" as a percentage would be a number
    /// that never moves.
    async fn stats(&self, _handle: &str) -> Stats {
        Stats {
            scope: "sandbox".into(),
            ..Default::default()
        }
    }

    async fn exec(&self, handle: &str, request: &ExecRequest) -> Result<ExecResult> {
        let line = request.command.trim();
        if line.is_empty() {
            return Ok(ExecResult::default());
        }

        // The toolbox takes one command line and no working directory, so the
        // directory is part of the line.
        // The environment goes in front of the command rather than in the
        // request body, because the toolbox takes a command line and nothing
        // else. Without this the `env` map every caller builds was assembled
        // and dropped on the floor for Daytona machines alone.
        let exports: String = request
            .env
            .iter()
            .filter(|(name, _)| !name.trim().is_empty())
            .map(|(name, value)| format!("export {name}={}; ", shell_quote(value)))
            .collect();

        let command = match request.cwd.as_deref().filter(|c| !c.is_empty()) {
            Some(cwd) => format!("{exports}cd {} && {line}", shell_quote(cwd)),
            None => format!("{exports}{line}"),
        };
        let timeout = request.timeout.unwrap_or(DEFAULT_TIMEOUT);

        let started = Instant::now();
        let payload = self
            .toolbox(
                handle,
                reqwest::Method::POST,
                "/process/execute",
                Some(json!({
                    "command": command,
                    // Seconds, which is the unit the API takes and not the one
                    // the rest of this app uses. Converted here so no caller
                    // has to remember.
                    "timeout": timeout.as_secs().max(1),
                })),
            )
            .await?
            .unwrap_or(json!({}));

        Ok(read_exec(&payload, started.elapsed().as_millis() as u64))
    }

    /// Through the shell rather than the toolbox's own `/files` listing.
    ///
    /// One code path for both providers, and `find -printf` answers exactly the
    /// four fields the pane draws - where the files endpoint's shape has
    /// changed between API versions.
    async fn list_dir(&self, handle: &str, path: &str) -> Result<Vec<DirEntry>> {
        let target = if path.trim().is_empty() { "." } else { path };
        let command = format!(
            r"find {} -mindepth 1 -maxdepth 1 -printf '%y\t%s\t%T@\t%f\n' 2>/dev/null | head -c 200000",
            shell_quote(target)
        );
        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command,
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;

        if !result.ok() && result.stdout.is_empty() {
            return Err(ComputerError::Failed(format!("Cannot read {target}")));
        }
        let mut entries: Vec<DirEntry> = result
            .stdout
            .lines()
            .filter_map(crate::docker::parse_entry)
            .collect();
        crate::docker::sort_entries(&mut entries);
        Ok(entries)
    }

    async fn read_file(&self, handle: &str, path: &str) -> Result<String> {
        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command: format!("cat {}", shell_quote(path)),
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;
        if !result.ok() {
            return Err(ComputerError::Failed(format!("Cannot read {path}")));
        }
        Ok(result.stdout)
    }

    async fn read_file_base64(&self, handle: &str, path: &str) -> Result<String> {
        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command: format!("base64 -w0 {}", shell_quote(path)),
                    timeout: Some(Duration::from_secs(60)),
                    ..Default::default()
                },
            )
            .await?;
        if !result.ok() {
            return Err(ComputerError::Failed(format!("Cannot read {path}")));
        }
        Ok(result.stdout.split_whitespace().collect())
    }

    async fn write_file(&self, handle: &str, path: &str, content: &str) -> Result<()> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
        let quoted = shell_quote(path);
        // Piped through base64 for the same reason Docker's write is: the
        // content is arbitrary and would otherwise have to survive shell
        // quoting and whatever the locale does to a byte that is not UTF-8.
        let command = format!(
            "mkdir -p \"$(dirname {quoted})\" && printf %s {} | base64 -d > {quoted}",
            shell_quote(&encoded)
        );

        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command,
                    timeout: Some(Duration::from_secs(60)),
                    ..Default::default()
                },
            )
            .await?;
        if !result.ok() {
            return Err(ComputerError::Failed(format!("Cannot write {path}")));
        }
        Ok(())
    }

    async fn snapshot(&self, handle: &str, name: &str) -> Result<Snapshot> {
        self.api(
            reqwest::Method::POST,
            &format!("/sandbox/{handle}/backup"),
            Some(json!({ "name": name })),
        )
        .await?;

        Ok(Snapshot {
            id: format!("backup-{}", jiff::Timestamp::now().as_millisecond()),
            name: name.to_string(),
            created_at: jiff::Timestamp::now().to_string(),
            size: None,
        })
    }

    async fn snapshots(&self, _handle: &str) -> Result<Vec<Snapshot>> {
        let payload = self
            .api(reqwest::Method::GET, "/snapshots", None)
            .await?
            .unwrap_or(json!([]));

        // The list arrives bare on some versions and under `items` on others.
        let rows = payload
            .as_array()
            .cloned()
            .or_else(|| payload.get("items").and_then(Value::as_array).cloned())
            .unwrap_or_default();

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let name = row.get("name").and_then(Value::as_str)?.to_string();
                Some(Snapshot {
                    id: row
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or(&name)
                        .to_string(),
                    name,
                    created_at: row
                        .get("createdAt")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    size: None,
                })
            })
            .collect())
    }

    /// Daytona restores by making a new sandbox from a snapshot rather than
    /// rewinding the one you have, so the handle changes and the caller has to
    /// store the new one.
    async fn restore(&self, handle: &str, snapshot_id: &str, name: &str) -> Result<Created> {
        let payload = self
            .api(
                reqwest::Method::POST,
                "/sandbox",
                Some(json!({
                    "snapshot": snapshot_id,
                    "labels": { "inertia": "1", "inertia-restored-from": handle },
                })),
            )
            .await?
            .ok_or_else(|| {
                ComputerError::Failed(format!(
                    "There is no snapshot called {snapshot_id} any more."
                ))
            })?;

        let sandbox = read_sandbox(&payload).ok_or_else(|| {
            ComputerError::Failed("Daytona described a sandbox this app could not read.".into())
        })?;

        // The old one goes only once the new one exists, so a failed restore
        // leaves the machine the user had.
        let _ = self.remove(handle, None).await;

        Ok(Created {
            handle: sandbox.handle,
            name: name.to_string(),
            status: sandbox.status,
            os: "Linux (Daytona sandbox)".into(),
            image: sandbox.image.unwrap_or_else(|| snapshot_id.to_string()),
            workdir: image::manifest().workdir,
        })
    }

    async fn screenshot(&self, handle: &str) -> Result<Option<String>> {
        let result = self
            .exec(
                handle,
                &ExecRequest {
                    command: crate::desktop::still(),
                    timeout: Some(Duration::from_secs(30)),
                    ..Default::default()
                },
            )
            .await?;

        let encoded: String = result.stdout.split_whitespace().collect();
        if !result.ok() || encoded.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!("data:image/png;base64,{encoded}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn daytona_state_words_become_the_apps() {
        assert_eq!(map_state("started"), Status::Running);
        assert_eq!(map_state("creating"), Status::Provisioning);
        assert_eq!(map_state("archived"), Status::Stopped);
        assert_eq!(map_state("destroyed"), Status::Missing);
    }

    /// A machine in a state we do not understand is one no command should be
    /// sent to.
    #[test]
    fn an_unknown_state_is_an_error_not_running() {
        assert_eq!(map_state("quantum"), Status::Error);
    }

    /// A sandbox that has just been asked for has no state yet.
    #[test]
    fn no_state_at_all_reads_as_provisioning() {
        assert_eq!(map_state(""), Status::Provisioning);
    }

    /// The API renamed several fields between versions, and a self-hosted
    /// instance can be on either.
    #[test]
    fn both_spellings_of_every_field_are_read() {
        let new =
            read_sandbox(&json!({ "id": "s1", "state": "started", "snapshot": "img" })).unwrap();
        assert_eq!(new.handle, "s1");
        assert_eq!(new.image.as_deref(), Some("img"));

        let old = read_sandbox(&json!({ "sandboxId": "s2", "status": "stopped", "image": "img2" }))
            .unwrap();
        assert_eq!(old.handle, "s2");
        assert_eq!(old.status, Status::Stopped);
        assert_eq!(old.image.as_deref(), Some("img2"));
    }

    /// The snapshot name Daytona is asked for.
    ///
    /// It used to be the local image reference, `inertia-sandbox:1.0.0`, which
    /// no Daytona account has ever heard of - every create failed. The account
    /// has `inertia-sandbox-1.0.0`, and the manifest has been carrying that
    /// name under `daytonaSnapshot` with nothing reading it.
    #[tokio::test]
    async fn a_sandbox_is_made_from_the_snapshot_the_account_has() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "s1", "state": "started",
            })))
            .mount(&server)
            .await;

        let provider = DaytonaProvider::new("dtn-test", Some(server.uri()));
        provider
            .create(&Spec {
                id: "c1".into(),
                ..Default::default()
            })
            .await
            .unwrap();

        let asked: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(asked["snapshot"], json!(image::manifest().daytona_snapshot));
    }

    /// And what the person picked in the dialog wins over either default.
    #[tokio::test]
    async fn the_chosen_snapshot_is_the_one_asked_for() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "s1", "state": "started",
            })))
            .mount(&server)
            .await;

        let provider = DaytonaProvider::new("dtn-test", Some(server.uri()));
        provider
            .create(&Spec {
                id: "c1".into(),
                image: Some("mine-2.0.0".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        let asked: Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(asked["snapshot"], json!("mine-2.0.0"));
    }

    /// The default snapshot exists only where someone pushed it, so its
    /// absence is what a new account meets first and has to be actionable.
    #[tokio::test]
    async fn a_snapshot_the_account_lacks_is_named_with_the_way_out() {
        for refusal in [
            ResponseTemplate::new(404),
            ResponseTemplate::new(400)
                .set_body_json(json!({ "message": "Snapshot inertia-sandbox-1.1.0 not found" })),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/sandbox"))
                .respond_with(refusal)
                .mount(&server)
                .await;

            let provider = DaytonaProvider::new("dtn-test", Some(server.uri()));
            let failure = provider
                .create(&Spec {
                    id: "c1".into(),
                    ..Default::default()
                })
                .await
                .unwrap_err()
                .to_string();
            assert!(
                failure.contains("no snapshot called `inertia-sandbox-1.1.0`"),
                "{failure}"
            );
            assert!(failure.contains("docs/computers.md"), "{failure}");
        }
    }

    #[test]
    fn an_exec_answer_is_read_under_any_of_its_names() {
        let combined = read_exec(&json!({ "exitCode": 0, "result": "hello" }), 5);
        assert_eq!(combined.stdout, "hello");
        assert!(combined.ok());

        let split = read_exec(&json!({ "code": 2, "stdout": "out", "stderr": "bad" }), 5);
        assert_eq!(split.code, 2);
        assert_eq!(split.stderr, "bad");
        assert!(!split.ok(), "a non-zero exit is still a result");
    }

    /// A toolbox version that reports no exit code has not said it worked.
    /// Reading that as zero turned every silent failure into an empty success.
    #[test]
    fn an_answer_with_no_exit_code_is_judged_by_its_stderr() {
        let quiet = read_exec(&json!({ "result": "fine" }), 5);
        assert_eq!(quiet.code, 0);

        let complaining = read_exec(&json!({ "stderr": "wget: not found" }), 5);
        assert_eq!(complaining.code, 1);
        assert!(!complaining.ok());
    }

    #[tokio::test]
    async fn no_key_is_reported_as_something_to_fix() {
        let provider = DaytonaProvider::new("", None);
        let answer = provider.available().await;
        assert!(!answer.ready);
        assert!(
            answer.reason.contains("DAYTONA_API_KEY"),
            "{}",
            answer.reason
        );
    }

    #[tokio::test]
    async fn a_rejected_key_says_which_secret_to_check() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox"))
            .respond_with(
                ResponseTemplate::new(401).set_body_json(json!({ "message": "bad token" })),
            )
            .mount(&server)
            .await;

        let provider = DaytonaProvider::new("dtn-test", Some(server.uri()));
        let answer = provider.available().await;
        assert!(!answer.ready);
        assert!(
            answer.reason.contains("DAYTONA_API_KEY"),
            "{}",
            answer.reason
        );
        assert!(answer.reason.contains("bad token"), "{}", answer.reason);
    }

    #[tokio::test]
    async fn a_sandbox_that_is_gone_reads_as_missing_rather_than_failing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox/ghost"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let provider = DaytonaProvider::new("dtn-test", Some(server.uri()));
        assert_eq!(provider.status("ghost").await.status, Status::Missing);
    }

    #[tokio::test]
    async fn creating_returns_the_handle_daytona_gave_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "sbx-123", "state": "creating",
            })))
            .mount(&server)
            .await;

        let provider = DaytonaProvider::new("dtn-test", Some(server.uri()));
        let made = provider
            .create(&Spec {
                id: "box".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(made.handle, "sbx-123");
        assert_eq!(made.status, Status::Provisioning);
        assert_eq!(made.name, "inertia-box");
    }

    /// The toolbox is on a different host, and its address is in a field of the
    /// sandbox object. Getting this wrong is a 404 on every command.
    #[tokio::test]
    async fn commands_go_to_the_toolbox_proxy_not_the_api_host() {
        let toolbox = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/process/execute"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "exitCode": 0, "result": "hi from the sandbox",
            })))
            .mount(&toolbox)
            .await;

        let api = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox/sbx-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "sbx-1", "state": "started", "toolboxProxyUrl": toolbox.uri(),
            })))
            .mount(&api)
            .await;

        let provider = DaytonaProvider::new("dtn-test", Some(api.uri()));
        let result = provider
            .exec(
                "sbx-1",
                &ExecRequest {
                    command: "echo hi".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(result.stdout, "hi from the sandbox");

        // And the address is remembered, so a second command does not ask again.
        assert_eq!(
            provider
                .proxies
                .lock()
                .await
                .get("sbx-1")
                .map(String::as_str),
            Some(toolbox.uri().trim_end_matches('/'))
        );
    }

    /// A restarted sandbox gets a new proxy, so the remembered one has to go -
    /// otherwise the first command after every start is a 404.
    #[tokio::test]
    async fn restarting_forgets_the_old_toolbox_address() {
        let api = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox/sbx-1/start"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&api)
            .await;

        let provider = DaytonaProvider::new("dtn-test", Some(api.uri()));
        provider
            .proxies
            .lock()
            .await
            .insert("sbx-1".into(), "https://stale.example".into());

        provider.start("sbx-1").await.unwrap();
        assert!(provider.proxies.lock().await.get("sbx-1").is_none());
    }
}

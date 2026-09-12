//! Machines an agent can drive.
//!
//! A "computer" here is something you can run a command in, read and write the
//! files of, see how hard it is working, and take a snapshot of. Where that
//! machine actually is - a container on this laptop, a sandbox in someone's
//! cloud - is the provider's business and nobody else's.
//!
//! # Two rules every provider follows
//!
//! **Nothing fails for a machine that is simply not there.** A container the
//! user removed by hand, a sandbox reaped for idleness - these are ordinary
//! states of the world, and the app has to render them as "gone" rather than as
//! an error dialog. [`Provider::status`] answers [`Status::Missing`]; the rest
//! answer the way they would for a stopped machine.
//!
//! **A command's exit code is not an error.** [`Provider::exec`] succeeds for a
//! command that failed and fails only when the command could not be run at all.
//! An agent reading a non-zero exit code and a stderr can decide what to do; an
//! agent handed an error cannot tell "the test suite failed" from "the machine
//! is unreachable", and those need different reactions.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod daytona;
pub mod desktop;
pub mod docker;
pub mod image;
pub mod local;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Long enough for a package install, short enough that a hung command does not
/// hold a pane forever. Callers override it per command.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Every machine this app makes is named so it can be found again, and so a
/// person looking at `docker ps` can tell what put it there.
pub const NAME_PREFIX: &str = "inertia-";

pub type Result<T> = std::result::Result<T, ComputerError>;

#[derive(Debug, thiserror::Error)]
pub enum ComputerError {
    /// The provider cannot run at all right now, with the reason. Never a bare
    /// "unavailable": "Docker is not running" is actionable and "unavailable"
    /// is not.
    #[error("{0}")]
    NotReady(String),
    #[error("{0}")]
    Failed(String),
    #[error("Timed out after {0:?}.")]
    Timeout(Duration),
}

/// The states a machine can be in, as the app understands them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Provisioning,
    Running,
    Paused,
    Stopped,
    Error,
    /// It is not there any more. Not a failure - a person can remove a
    /// container by hand and the app has to draw that.
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
}

/// How hard a machine is working, as percentages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub cpu_pct: Option<f64>,
    pub mem_pct: Option<f64>,
    /// Often `None`, and honestly so: a volume with no quota has no percentage.
    /// `du` inside the machine answers "how much is used" and still does not
    /// answer "out of what".
    pub disk_pct: Option<f64>,
    pub scope: String,
}

/// What a command did. Always produced, whatever the exit code.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}

impl ExecResult {
    pub fn ok(&self) -> bool {
        self.code == 0 && !self.timed_out
    }
}

/// One command to run inside a machine.
#[derive(Debug, Clone, Default)]
pub struct ExecRequest {
    pub command: String,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub timeout: Option<Duration>,
}

/// One entry in a directory listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    /// `"dir"` or `"file"`.
    #[serde(rename = "type")]
    pub kind: String,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
}

/// What a machine was made from.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    pub id: String,
    pub cpu: Option<f64>,
    pub memory_gb: Option<f64>,
    pub disk_gb: Option<f64>,
    /// What to build it from: a snapshot name on Daytona, an image reference
    /// elsewhere. `None` means the provider's own default.
    ///
    /// The New computer dialog has asked for this since it was written and the
    /// answer had nowhere to go - the field was saved onto the record and the
    /// provider was never told, so picking a snapshot changed nothing.
    pub image: Option<String>,
}

/// A machine, just made.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    /// What the provider calls it. Stored on the record and handed back for
    /// every later operation.
    pub handle: String,
    pub name: String,
    pub status: Status,
    pub os: String,
    pub image: String,
    pub workdir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub size: Option<u64>,
}

/// Where a machine's screen can be watched, and where it can be driven.
///
/// Two addresses rather than one with a flag, because "you are only watching"
/// should be true of the connection and not merely of the page that opened it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    /// A URL that renders the screen and ignores input.
    pub view: String,
    /// A URL that renders it and sends mouse and keyboard through.
    pub control: String,
}

/// Whether a provider can run at all right now.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Readiness {
    pub ready: bool,
    /// Why not, in words a person can act on.
    pub reason: String,
    /// Something true of a provider that *is* ready.
    ///
    /// Its own field because the screen shows a reason only when a provider
    /// cannot run, and the local machine's caveat has to be read by somebody
    /// who is about to pick it precisely because it works.
    pub warning: String,
    /// What to do about it, when the reason alone does not say.
    ///
    /// Separate from the reason because the screen joins them: "No
    /// DAYTONA_API_KEY is set." is the fact, "Add it under Settings, Secrets."
    /// is the way out, and a provider that only knows the first leaves a person
    /// reading a dead end.
    pub hint: String,
}

impl Readiness {
    pub fn ready() -> Self {
        Self {
            ready: true,
            reason: String::new(),
            warning: String::new(),
            hint: String::new(),
        }
    }

    pub fn not(reason: impl Into<String>) -> Self {
        Self {
            ready: false,
            reason: reason.into(),
            warning: String::new(),
            hint: String::new(),
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }

    pub fn warning(mut self, warning: impl Into<String>) -> Self {
        self.warning = warning.into();
        self
    }
}

/// What a computer provider has to be able to do.
///
/// This is the whole seam. Everything above it - the command surface, the
/// panes, the tool an agent calls - is written against these methods and has no
/// idea which provider answered, which is what makes adding another one a file
/// plus a line in the registry rather than a change to the product.
#[async_trait]
pub trait Provider: Send + Sync + std::fmt::Debug {
    /// What it is, for the settings screen.
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn blurb(&self) -> &'static str;

    async fn available(&self) -> Readiness;

    async fn create(&self, spec: &Spec) -> Result<Created>;
    async fn start(&self, handle: &str) -> Result<()>;
    async fn stop(&self, handle: &str) -> Result<()>;
    async fn pause(&self, handle: &str) -> Result<()>;
    async fn resume(&self, handle: &str) -> Result<()>;
    async fn remove(&self, handle: &str, name: Option<&str>) -> Result<()>;

    async fn status(&self, handle: &str) -> Health;
    async fn stats(&self, handle: &str) -> Stats;

    async fn exec(&self, handle: &str, request: &ExecRequest) -> Result<ExecResult>;

    async fn list_dir(&self, handle: &str, path: &str) -> Result<Vec<DirEntry>>;
    async fn read_file(&self, handle: &str, path: &str) -> Result<String>;
    async fn read_file_base64(&self, handle: &str, path: &str) -> Result<String>;
    async fn write_file(&self, handle: &str, path: &str, content: &str) -> Result<()>;

    async fn snapshot(&self, handle: &str, name: &str) -> Result<Snapshot>;
    async fn snapshots(&self, handle: &str) -> Result<Vec<Snapshot>>;
    async fn restore(&self, handle: &str, snapshot_id: &str, name: &str) -> Result<Created>;

    /// A PNG data URL, or `None` when the machine has no display.
    async fn screenshot(&self, handle: &str) -> Result<Option<String>>;

    /// Where this machine's screen can be watched live, or `None`.
    ///
    /// Defaulted to `None` rather than required, because most providers have no
    /// such thing and a pane that falls back to still frames is a complete
    /// answer for them. Asked of the provider on every look rather than stored
    /// on the record: for Docker it is a host port picked fresh on every
    /// `docker start`, and a cached one points at nothing after the first
    /// restart.
    async fn screen(&self, _handle: &str) -> Result<Option<Screen>> {
        Ok(None)
    }
}

/// The machine name for an id, so a person looking at `docker ps` can tell what
/// put it there and the app can find its own containers again.
pub fn machine_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("{NAME_PREFIX}{}", safe.trim_matches('-'))
}

/// Wraps a value so a shell inside the machine reads it as one argument.
///
/// Single quotes, with the one escape that single quotes need. Not optional
/// politeness: a path with a space in it is ordinary, and one with a `;` in it
/// is how a directory listing becomes a command.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_is_named_after_its_id() {
        assert_eq!(machine_name("test"), "inertia-test");
    }

    /// The id comes from a record the user named, so it is not assumed to be a
    /// legal container name.
    #[test]
    fn an_awkward_id_is_made_safe() {
        assert_eq!(machine_name("my box!"), "inertia-my-box");
        assert_eq!(machine_name("a/b"), "inertia-a-b");
    }

    /// A path with a `;` in it is how a directory listing becomes a command.
    #[test]
    fn quoting_survives_the_characters_that_matter() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("a;rm -rf /"), "'a;rm -rf /'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn a_failed_command_is_a_result_not_an_error() {
        let failed = ExecResult {
            code: 1,
            stderr: "no such file".into(),
            ..Default::default()
        };
        assert!(!failed.ok());
        assert_eq!(failed.code, 1);
    }

    #[test]
    fn a_timed_out_command_is_not_ok_even_at_zero() {
        let timed_out = ExecResult {
            code: 0,
            timed_out: true,
            ..Default::default()
        };
        assert!(!timed_out.ok());
    }

    #[test]
    fn not_being_ready_always_carries_a_reason() {
        let answer = Readiness::not("Docker is not running.");
        assert!(!answer.ready);
        assert!(!answer.reason.is_empty());
    }
}

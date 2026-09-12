//! The error type that crosses trait boundaries.
//!
//! Each implementation crate keeps its own rich error enum internally. What
//! travels through a `dyn Provider` or `dyn Tool` has to be one shared type,
//! and this is it: broad enough to classify a failure, narrow enough that
//! callers can actually match on it.
//!
//! The classification exists to answer two questions the agent loop asks about
//! every failure: *should this be retried*, and *should the model be told*.
//! A rate limit is retried and hidden; a tool that rejected its arguments is
//! not retried and is handed straight back to the model to correct.

use std::time::Duration;

use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The caller passed something invalid - bad tool arguments, a malformed
    /// request. Retrying verbatim cannot help.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// The thing being addressed does not exist.
    #[error("{kind} not found: {id}")]
    NotFound { kind: &'static str, id: String },

    /// Credentials are missing, expired, or rejected.
    #[error("authentication failed: {0}")]
    Auth(String),

    /// Rate limited or over quota. Carries the server's own retry hint when it
    /// gave one, because guessing a backoff when the server has told us the
    /// answer is how you get banned for longer.
    #[error("rate limited{}", .retry_after.map(|d| format!(", retry after {}s", d.as_secs())).unwrap_or_default())]
    RateLimited { retry_after: Option<Duration> },

    /// The remote is reachable but unhappy, or unreachable entirely.
    #[error("upstream error{}: {message}", .status.map(|s| format!(" ({s})")).unwrap_or_default())]
    Upstream {
        status: Option<u16>,
        message: String,
    },

    /// Network-level failure: DNS, TLS, connection reset.
    #[error("network error: {0}")]
    Network(String),

    /// Ran out of time.
    #[error("timed out after {}ms", .0.as_millis())]
    Timeout(Duration),

    /// Deliberately stopped - the user hit stop, or a parent task was dropped.
    /// Distinct from every other variant because it is not a failure and must
    /// never be reported to the user as one.
    #[error("cancelled")]
    Cancelled,

    /// A permission prompt came back as a refusal.
    #[error("denied: {0}")]
    Denied(String),

    /// Reading or writing the workspace failed.
    #[error("storage error: {0}")]
    Storage(String),

    /// Could not make sense of a payload - malformed JSON, a response shaped
    /// differently than the protocol promises.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Anything that does not fit above. Kept last and used sparingly; a
    /// failure that lands here cannot be handled intelligently by anyone.
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn not_found(kind: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            kind,
            id: id.into(),
        }
    }

    pub fn upstream(status: Option<u16>, message: impl Into<String>) -> Self {
        Self::Upstream {
            status,
            message: message.into(),
        }
    }

    /// Whether retrying the identical request could plausibly succeed.
    ///
    /// Deliberately conservative: a 500 might be transient, but a 400 will be
    /// a 400 forever, and retrying it just burns quota and makes the user
    /// wait.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Network(_) | Self::Timeout(_) => true,
            // 408 Request Timeout and 429 are retryable; so are the 5xx that
            // are not "we will never accept this".
            Self::Upstream { status, .. } => matches!(status, Some(408 | 429 | 500..=599)),
            _ => false,
        }
    }

    /// Whether the failure is the model's to fix.
    ///
    /// When true, the agent loop feeds the message back as a tool result and
    /// lets the model try again - a bad path or a malformed argument is
    /// something it can correct. When false, the turn fails and the user is
    /// told, because no amount of rewording fixes an expired API key.
    pub fn is_model_correctable(&self) -> bool {
        matches!(
            self,
            Self::InvalidInput(_) | Self::NotFound { .. } | Self::Denied(_)
        )
    }

    /// Cancellation is routine, not exceptional, and callers need to tell the
    /// difference before they log anything at error level.
    pub fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Protocol(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => Self::not_found("file", e.to_string()),
            std::io::ErrorKind::TimedOut => Self::Network(e.to_string()),
            _ => Self::Storage(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_failures_are_retryable() {
        assert!(Error::RateLimited { retry_after: None }.is_retryable());
        assert!(Error::Network("reset".into()).is_retryable());
        assert!(Error::upstream(Some(503), "unavailable").is_retryable());
    }

    #[test]
    fn permanent_failures_are_not() {
        assert!(!Error::InvalidInput("bad path".into()).is_retryable());
        assert!(!Error::Auth("expired".into()).is_retryable());
        assert!(!Error::upstream(Some(400), "malformed").is_retryable());
        assert!(!Error::Cancelled.is_retryable());
    }

    // The distinction the agent loop depends on: these go back to the model
    // as a tool result, everything else ends the turn.
    #[test]
    fn only_the_models_mistakes_go_back_to_the_model() {
        assert!(Error::InvalidInput("no such flag".into()).is_model_correctable());
        assert!(Error::not_found("file", "/nope").is_model_correctable());
        assert!(!Error::Auth("expired key".into()).is_model_correctable());
        assert!(!Error::Cancelled.is_model_correctable());
    }

    #[test]
    fn a_missing_file_reads_as_not_found() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        assert!(matches!(Error::from(io), Error::NotFound { .. }));
    }

    #[test]
    fn rate_limits_carry_the_servers_own_hint() {
        let e = Error::RateLimited {
            retry_after: Some(Duration::from_secs(30)),
        };
        assert!(e.to_string().contains("30s"));
    }
}

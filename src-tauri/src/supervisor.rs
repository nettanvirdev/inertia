//! Keeping the MCP servers up.
//!
//! Every server in the workspace was configured, saved, and then never started:
//! `mcp_connect_all` existed as a command and nothing in the app or the window
//! ever called it. So a workspace with five working servers and sixteen tools
//! ran every turn with none of them, and the agent's own instructions - "look
//! that up with Context7", "search with Tavily" - named tools it did not have.
//!
//! A tick rather than a one-shot connect at startup, which is what the other
//! shell settled on for three reasons that are each real on their own:
//!
//!   - **The workspace may be chosen later.** There is nothing to connect to at
//!     launch if nobody has opened a folder yet.
//!   - **Servers die.** A stdio server is a child process; it can crash, or be
//!     killed with the terminal it inherited.
//!   - **Laptops sleep.** Coming back from suspend should not need a restart.
//!
//! And a backoff, because a server that will never start must not respawn a
//! process every fifteen seconds for as long as the app is open: five seconds,
//! then ten, then twenty, up to five minutes - slow enough to be free, fast
//! enough that a machine waking up reconnects before anyone notices.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};
use tokio::time::Instant;

/// How often to look at what is down.
pub const TICK: Duration = Duration::from_secs(15);

/// The first wait after a failure.
const RETRY_BASE: Duration = Duration::from_secs(5);

/// And the longest. Past this it is not coming back on its own.
const RETRY_MAX: Duration = Duration::from_secs(5 * 60);

/// When each failing server may next be tried.
#[derive(Debug, Default)]
pub struct Backoff {
    waits: Mutex<HashMap<String, Wait>>,
}

#[derive(Debug, Clone, Copy)]
struct Wait {
    failures: u32,
    next_at: Instant,
}

impl Backoff {
    /// Whether this server may be tried now.
    ///
    /// A server that has never failed is always due, which is what makes the
    /// first tick after launch connect everything at once.
    pub fn due(&self, id: &str, now: Instant) -> bool {
        match self.waits.lock().get(id) {
            None => true,
            Some(wait) => now >= wait.next_at,
        }
    }

    /// It came up. The next failure starts counting from scratch.
    pub fn succeeded(&self, id: &str) {
        self.waits.lock().remove(id);
    }

    /// It did not. Returns how long until the next attempt.
    pub fn failed(&self, id: &str, now: Instant) -> Duration {
        let mut waits = self.waits.lock();
        let failures = waits.get(id).map(|wait| wait.failures).unwrap_or(0) + 1;
        let delay = Self::delay(failures);
        waits.insert(
            id.to_string(),
            Wait {
                failures,
                next_at: now + delay,
            },
        );
        delay
    }

    /// Doubling from five seconds, capped at five minutes.
    ///
    /// Saturating rather than shifting: a server that has been down for days
    /// has a failure count that would overflow the shift, and the answer for it
    /// is the cap, not a panic or a wrap back round to five seconds.
    fn delay(failures: u32) -> Duration {
        let doublings = failures.saturating_sub(1).min(16);
        RETRY_BASE.saturating_mul(1u32 << doublings).min(RETRY_MAX)
    }

    /// Forget a server, when its record is deleted or the workspace closes.
    pub fn forget(&self, id: &str) {
        self.waits.lock().remove(id);
    }
}

/// Starts the supervisor. Runs for the life of the app.
///
/// Failures are recorded against their records rather than propagated: one
/// misconfigured server must not stop the others coming up, and the Integrations
/// screen reads the reason off the record.
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // A first pass immediately, so a workspace already open when the window
        // appears does not wait fifteen seconds for its tools.
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            tick(&handle).await;
        }
    });
}

/// One pass: connect anything enabled that is down and out of backoff.
async fn tick(app: &AppHandle) {
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return;
    };
    // No workspace yet is the normal state at launch, not a problem.
    let Ok(workspace) = state.workspace() else {
        return;
    };

    let live: Vec<String> = workspace
        .mcp
        .connected()
        .into_iter()
        .map(|record| record.id)
        .collect();

    let now = Instant::now();
    for mut record in workspace.mcp_records() {
        if !record.enabled {
            // A server the person turned off is not down, and must not be
            // retried forever behind their back.
            state.mcp_backoff.forget(&record.id);
            continue;
        }
        if live.contains(&record.id) || !state.mcp_backoff.due(&record.id, now) {
            continue;
        }

        match workspace
            .mcp
            .add(crate::integrations::with_secrets(
                &workspace.layout,
                &record,
            ))
            .await
        {
            Ok(tool_count) => {
                state.mcp_backoff.succeeded(&record.id);
                record.status = "connected".into();
                record.error = String::new();
                record.tool_count = tool_count;
                tracing::info!(server = %record.id, tools = tool_count, "MCP server connected");
            }
            Err(message) => {
                let wait = state.mcp_backoff.failed(&record.id, now);
                tracing::warn!(
                    server = %record.id,
                    error = %message,
                    retry_in_s = wait.as_secs(),
                    "MCP server did not start"
                );
                record.status = "failed".into();
                record.error = message;
                record.tool_count = 0;
            }
        }

        let _ = workspace.save_mcp_record(&record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_that_has_never_failed_is_tried_at_once() {
        let backoff = Backoff::default();
        assert!(backoff.due("context7", Instant::now()));
    }

    #[test]
    fn a_failure_holds_it_back_and_the_wait_doubles() {
        let backoff = Backoff::default();
        let now = Instant::now();

        assert_eq!(backoff.failed("apify", now), Duration::from_secs(5));
        assert!(!backoff.due("apify", now));
        // Not yet.
        assert!(!backoff.due("apify", now + Duration::from_secs(4)));
        // Now.
        assert!(backoff.due("apify", now + Duration::from_secs(5)));

        assert_eq!(backoff.failed("apify", now), Duration::from_secs(10));
        assert_eq!(backoff.failed("apify", now), Duration::from_secs(20));
        assert_eq!(backoff.failed("apify", now), Duration::from_secs(40));
    }

    #[test]
    fn the_wait_is_capped_so_a_dead_server_is_checked_every_five_minutes() {
        let backoff = Backoff::default();
        let now = Instant::now();
        let mut last = Duration::ZERO;
        for _ in 0..40 {
            last = backoff.failed("gone", now);
        }
        assert_eq!(last, RETRY_MAX);
    }

    #[test]
    fn a_huge_failure_count_saturates_rather_than_wrapping_back_to_five_seconds() {
        // The shift overflows long before this; the answer is the cap.
        assert_eq!(Backoff::delay(u32::MAX), RETRY_MAX);
        assert_eq!(Backoff::delay(1), RETRY_BASE);
    }

    #[test]
    fn coming_back_up_clears_the_wait() {
        let backoff = Backoff::default();
        let now = Instant::now();
        backoff.failed("tavily", now);
        backoff.failed("tavily", now);
        assert!(!backoff.due("tavily", now));

        backoff.succeeded("tavily");
        assert!(backoff.due("tavily", now));
        // And the next failure starts from five seconds again, not from forty.
        assert_eq!(backoff.failed("tavily", now), Duration::from_secs(5));
    }

    #[test]
    fn turning_a_server_off_stops_it_being_retried_behind_the_persons_back() {
        let backoff = Backoff::default();
        let now = Instant::now();
        backoff.failed("exa", now);
        backoff.forget("exa");
        assert!(backoff.due("exa", now));
    }
}

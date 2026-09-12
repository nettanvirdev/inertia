//! Time, behind a seam.
//!
//! Anything that reads the wall clock directly is untestable by construction:
//! session timestamps, retry backoff, prompt-injected dates, routine
//! schedules. Taking a `&dyn Clock` instead makes those deterministic, which
//! matters because several of them end up in the prompt and therefore in
//! snapshot tests.

use std::sync::Arc;

use jiff::Timestamp;

/// The current time, as far as the caller is concerned.
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> Timestamp;
}

/// Reads the real clock. What the app runs with.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A clock that does not move unless told to. What tests run with.
#[derive(Debug, Clone)]
pub struct FixedClock {
    // Not a `Cell`, because `Clock` is `Sync` and callers share it across
    // tasks.
    now: Arc<std::sync::Mutex<Timestamp>>,
}

impl FixedClock {
    pub fn new(now: Timestamp) -> Self {
        Self {
            now: Arc::new(std::sync::Mutex::new(now)),
        }
    }

    /// A fixed, arbitrary instant for tests that need *a* time but do not care
    /// which. Having one shared value keeps snapshot expectations stable.
    pub fn for_tests() -> Self {
        Self::new(
            "2024-01-01T00:00:00Z"
                .parse()
                .unwrap_or(Timestamp::UNIX_EPOCH),
        )
    }

    /// Moves the clock forward, for testing anything that measures elapsed
    /// time without actually waiting for it.
    pub fn advance(&self, by: std::time::Duration) {
        if let Ok(mut now) = self.now.lock() {
            let span = jiff::Span::new().nanoseconds(by.as_nanos() as i64);
            if let Ok(next) = now.checked_add(span) {
                *now = next;
            }
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.now
            .lock()
            .map(|t| *t)
            .unwrap_or(Timestamp::UNIX_EPOCH)
    }
}

impl Clock for Arc<dyn Clock> {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_moves() {
        let clock = SystemClock;
        assert!(clock.now() > Timestamp::UNIX_EPOCH);
    }

    #[test]
    fn a_fixed_clock_does_not() {
        let clock = FixedClock::for_tests();
        assert_eq!(clock.now(), clock.now());
    }

    #[test]
    fn advancing_moves_it_by_exactly_that_much() {
        let clock = FixedClock::for_tests();
        let before = clock.now();
        clock.advance(std::time::Duration::from_secs(90));
        let after = clock.now();
        assert_eq!((after - before).get_seconds(), 90);
    }
}

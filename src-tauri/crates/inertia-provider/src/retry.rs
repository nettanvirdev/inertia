//! Waiting out a provider that is having a bad minute.
//!
//! A long session dies of small things. A 529 while the provider sheds load, a
//! 429 two messages into a burst, a socket closed by a laptop that changed
//! networks - each of them is a turn that ends with a red line in the
//! transcript and work the person has to ask for again. None of them is a
//! mistake anyone made, and all of them come back on their own within seconds.
//!
//! So every provider is wrapped in this before the agent loop ever sees it.
//! [`StreamEvent::Retry`] was already in the vocabulary and already rendered by
//! the window - "Retrying (2/10): ..." - and nothing had ever emitted one. This
//! is what emits it.
//!
//! # The rule that makes it safe
//!
//! **Only a stream that has said nothing may be retried.** A request that has
//! already streamed nine hundred tokens and then loses the connection cannot be
//! taken back: the tokens are on screen, and running the request again would
//! append a second copy of the same paragraph - or worse, a second copy of the
//! same tool call, which is a file written twice. So the moment anything
//! substantive is yielded - prose, thinking, or tool calls - this wrapper
//! becomes a pass-through for the rest of that stream, and its failure is
//! reported exactly as the provider reported it.
//!
//! That is a narrower promise than "retries everything", and it is the only one
//! that is honest. It still covers the great majority of what ends long
//! sessions, because a provider that is overloaded, rate-limiting or
//! unreachable says so before it streams a word.
//!
//! # Which failures
//!
//! Transient ones, by status: 408, 409, 425, 429, and the 5xx family, of which
//! 529 is Anthropic's own "overloaded". A failure with no status at all never
//! reached a server, which is the most retryable thing there is. A 400 is a
//! malformed request and will be malformed forever; a 401 is a key only a
//! person can fix. Retrying those burns quota and delays the one error the
//! user actually needs to read.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use inertia_core::provider::{ChatRequest, ModelInfo, Provider, StreamEvent};

/// How many times a turn may be attempted in total, the first try included.
///
/// Ten, with the backoff below, is a little over four minutes of waiting. That
/// is the right order of magnitude for a provider shedding load: the outages
/// that clear at all clear inside it, and a person who walked away from a long
/// turn would rather come back to a finished one than to a red line from four
/// minutes ago.
pub const MAX_ATTEMPTS: u32 = 10;

/// The wait before the second attempt. Doubles from here.
const BASE_DELAY: Duration = Duration::from_secs(1);

/// And the longest single wait. Past half a minute the doubling stops being
/// politeness to the provider and is only the person waiting.
const MAX_DELAY: Duration = Duration::from_secs(30);

/// The wait after attempt `n`, where the first attempt is 1.
///
/// Exponential, capped, and deterministic: a test can assert the whole
/// schedule, and there is no jitter because this is one desktop app making one
/// request, not a fleet stampeding a service on the same clock tick.
pub fn delay_for(attempt: u32) -> Duration {
    let step = attempt.saturating_sub(1).min(16);
    let scaled = BASE_DELAY.saturating_mul(1u32 << step);
    scaled.min(MAX_DELAY)
}

/// Is this failure worth waiting out?
pub fn is_transient(status: Option<u16>) -> bool {
    match status {
        None => true,
        Some(code) => matches!(code, 408 | 409 | 425 | 429 | 500..=599),
    }
}

/// Anything that means this attempt can no longer be taken back.
fn is_committing(event: &StreamEvent) -> bool {
    matches!(
        event,
        StreamEvent::Delta { .. }
            | StreamEvent::Thinking { .. }
            | StreamEvent::Tool { .. }
            | StreamEvent::Done { .. }
    )
}

/// Sleeping, as a seam.
///
/// The tests must not actually wait four minutes to prove the schedule, and a
/// provider crate should not have to reach for a runtime to express "wait". So
/// the waiting is a trait object: the app fills it in with `tokio::time::sleep`
/// and the tests fill it in with a recorder.
#[async_trait]
pub trait Wait: Send + Sync + std::fmt::Debug {
    async fn wait(&self, how_long: Duration);
}

/// The real one.
#[derive(Debug, Clone, Copy, Default)]
pub struct SleepWait;

#[async_trait]
impl Wait for SleepWait {
    async fn wait(&self, how_long: Duration) {
        tokio::time::sleep(how_long).await;
    }
}

/// A provider that waits out the failures which clear on their own.
#[derive(Debug, Clone)]
pub struct Resilient {
    inner: Arc<dyn Provider>,
    attempts: u32,
    wait: Arc<dyn Wait>,
}

impl Resilient {
    /// Wrap a provider with the shipping policy.
    pub fn new(inner: Arc<dyn Provider>) -> Self {
        Self {
            inner,
            attempts: MAX_ATTEMPTS,
            wait: Arc::new(SleepWait),
        }
    }

    /// The same, with a different ceiling and a different way of waiting.
    pub fn with(inner: Arc<dyn Provider>, attempts: u32, wait: Arc<dyn Wait>) -> Self {
        Self {
            inner,
            attempts: attempts.max(1),
            wait,
        }
    }
}

#[async_trait]
impl Provider for Resilient {
    fn id(&self) -> &str {
        self.inner.id()
    }

    async fn list_models(&self) -> inertia_core::Result<Vec<ModelInfo>> {
        self.inner.list_models().await
    }

    fn stream_chat(&self, request: ChatRequest) -> BoxStream<'static, StreamEvent> {
        let inner = self.inner.clone();
        let wait = self.wait.clone();
        let attempts = self.attempts;

        Box::pin(async_stream::stream! {
            for attempt in 1..=attempts {
                let mut stream = inner.stream_chat(request.clone());
                // Every attempt emits its own `Start`, and the window treats
                // one as "the reply begins here". Only the first may through.
                let mut started = attempt > 1;
                // Nothing has been said yet, so nothing has been committed to.
                let mut committed = false;
                let mut again: Option<(Option<u16>, String)> = None;

                while let Some(event) = stream.next().await {
                    if is_committing(&event) {
                        committed = true;
                    }
                    if matches!(event, StreamEvent::Start { .. }) {
                        if started {
                            continue;
                        }
                        started = true;
                    }
                    if let StreamEvent::Error { status, message } = &event {
                        if !committed && attempt < attempts && is_transient(*status) {
                            again = Some((*status, message.clone()));
                            break;
                        }
                    }
                    yield event;
                }

                let Some((status, message)) = again else { return };

                let delay = delay_for(attempt);
                yield StreamEvent::Retry {
                    attempt,
                    of: attempts,
                    delay_ms: delay.as_millis() as u64,
                    status,
                    message,
                };
                wait.wait(delay).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::provider::FinishReason;
    use parking_lot::Mutex;

    #[derive(Debug, Default)]
    struct Recorder {
        waited: Mutex<Vec<Duration>>,
    }

    #[async_trait]
    impl Wait for Recorder {
        async fn wait(&self, how_long: Duration) {
            self.waited.lock().push(how_long);
        }
    }

    /// A provider that replays one scripted stream per attempt.
    #[derive(Debug)]
    struct Scripted {
        runs: Mutex<Vec<Vec<StreamEvent>>>,
        seen: Mutex<u32>,
    }

    impl Scripted {
        fn new(runs: Vec<Vec<StreamEvent>>) -> Arc<Self> {
            Arc::new(Self {
                runs: Mutex::new(runs),
                seen: Mutex::new(0),
            })
        }
    }

    #[async_trait]
    impl Provider for Scripted {
        fn id(&self) -> &str {
            "scripted"
        }
        async fn list_models(&self) -> inertia_core::Result<Vec<ModelInfo>> {
            Ok(vec![])
        }
        fn stream_chat(&self, _request: ChatRequest) -> BoxStream<'static, StreamEvent> {
            *self.seen.lock() += 1;
            let mut runs = self.runs.lock();
            let events = if runs.is_empty() {
                vec![]
            } else {
                runs.remove(0)
            };
            Box::pin(futures::stream::iter(events))
        }
    }

    fn start() -> StreamEvent {
        StreamEvent::Start { model: "m".into() }
    }

    fn fail(status: Option<u16>) -> StreamEvent {
        StreamEvent::Error {
            message: "overloaded".into(),
            status,
        }
    }

    fn done() -> StreamEvent {
        StreamEvent::Done {
            finish: Some(FinishReason::Stop),
            usage: None,
        }
    }

    async fn drain(provider: &Resilient) -> Vec<StreamEvent> {
        provider
            .stream_chat(ChatRequest::default())
            .collect::<Vec<_>>()
            .await
    }

    #[tokio::test]
    async fn an_overloaded_provider_is_waited_out_and_the_turn_still_finishes() {
        let inner = Scripted::new(vec![
            vec![start(), fail(Some(529))],
            vec![start(), fail(Some(529))],
            vec![start(), StreamEvent::Delta { text: "hi".into() }, done()],
        ]);
        let waits = Arc::new(Recorder::default());
        let provider = Resilient::with(inner.clone(), 10, waits.clone());

        let events = drain(&provider).await;

        // One `Start`, both waits announced, and the reply that finally worked.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, StreamEvent::Start { .. }))
                .count(),
            1,
            "{events:?}"
        );
        let retries: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Retry { attempt, of, .. } => Some((*attempt, *of)),
                _ => None,
            })
            .collect();
        assert_eq!(retries, vec![(1, 10), (2, 10)]);
        assert!(
            matches!(events.last(), Some(StreamEvent::Done { .. })),
            "{events:?}"
        );
        assert_eq!(*inner.seen.lock(), 3);
        assert_eq!(
            *waits.waited.lock(),
            vec![Duration::from_secs(1), Duration::from_secs(2)]
        );
    }

    #[tokio::test]
    async fn a_request_the_provider_will_always_refuse_is_not_retried() {
        let inner = Scripted::new(vec![vec![start(), fail(Some(400))]]);
        let waits = Arc::new(Recorder::default());
        let provider = Resilient::with(inner.clone(), 10, waits.clone());

        let events = drain(&provider).await;

        assert!(
            matches!(events.last(), Some(StreamEvent::Error { .. })),
            "{events:?}"
        );
        assert_eq!(*inner.seen.lock(), 1, "a 400 is malformed forever");
        assert!(waits.waited.lock().is_empty());
    }

    #[tokio::test]
    async fn a_stream_that_already_said_something_is_never_run_twice() {
        let inner = Scripted::new(vec![
            vec![
                start(),
                StreamEvent::Delta {
                    text: "half a ".into(),
                },
                fail(Some(529)),
            ],
            vec![start(), done()],
        ]);
        let provider = Resilient::with(inner.clone(), 10, Arc::new(Recorder::default()));

        let events = drain(&provider).await;

        // The failure is reported as it stands. Retrying would print the
        // paragraph twice, which is worse than the error.
        assert!(
            matches!(events.last(), Some(StreamEvent::Error { .. })),
            "{events:?}"
        );
        assert_eq!(*inner.seen.lock(), 1);
    }

    #[tokio::test]
    async fn an_unreachable_provider_is_retried() {
        let inner = Scripted::new(vec![vec![start(), fail(None)], vec![start(), done()]]);
        let provider = Resilient::with(inner.clone(), 10, Arc::new(Recorder::default()));

        let events = drain(&provider).await;
        assert!(
            matches!(events.last(), Some(StreamEvent::Done { .. })),
            "{events:?}"
        );
        assert_eq!(*inner.seen.lock(), 2);
    }

    #[tokio::test]
    async fn the_last_attempt_reports_the_failure_rather_than_swallowing_it() {
        let inner = Scripted::new(vec![
            vec![start(), fail(Some(529))],
            vec![start(), fail(Some(529))],
        ]);
        let provider = Resilient::with(inner.clone(), 2, Arc::new(Recorder::default()));

        let events = drain(&provider).await;
        assert!(
            matches!(events.last(), Some(StreamEvent::Error { .. })),
            "{events:?}"
        );
        assert_eq!(*inner.seen.lock(), 2);
    }

    #[test]
    fn the_wait_doubles_and_then_stops_growing() {
        assert_eq!(delay_for(1), Duration::from_secs(1));
        assert_eq!(delay_for(2), Duration::from_secs(2));
        assert_eq!(delay_for(3), Duration::from_secs(4));
        assert_eq!(delay_for(6), Duration::from_secs(30));
        assert_eq!(delay_for(10), MAX_DELAY);
        assert_eq!(delay_for(64), MAX_DELAY);
    }

    #[test]
    fn only_the_failures_that_clear_on_their_own_are_waited_out() {
        for code in [408, 409, 425, 429, 500, 502, 503, 529] {
            assert!(is_transient(Some(code)), "{code}");
        }
        for code in [400, 401, 403, 404, 422] {
            assert!(!is_transient(Some(code)), "{code}");
        }
        assert!(is_transient(None));
    }
}

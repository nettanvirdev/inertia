//! Delta coalescing.
//!
//! A fast provider emits a token every few milliseconds. Forwarding each one
//! individually means an IPC message, a React state update and a full markdown
//! reparse per token, and on a long reply that is the difference between a
//! transcript that scrolls smoothly and one that stutters. Over Tauri's
//! transport the per-message cost is higher than Electron's, not lower: every
//! event is serialised to JSON and evaluated as script in the webview, so a
//! reply arriving at three hundred tokens a second is three hundred script
//! evaluations a second competing with React for the same thread.
//!
//! So text is buffered and flushed on a short interval instead. Two things keep
//! it from feeling laggy:
//!
//! - The interval is short enough to read as continuous. Sixteen updates a
//!   second is below the eye's threshold for a break in motion and well above
//!   the point where re-rendering starts to cost.
//! - A newline flushes immediately. Block boundaries are where markdown changes
//!   shape - a fence opening, a list starting, a table row landing - and holding
//!   one back is exactly the case where the reader sees the layout jump.
//!
//! Everything that is not text flushes first and then passes through, so a tool
//! card, an error or a `done` can never arrive ahead of the prose that preceded
//! it. That ordering is the reason this is a buffer with a rule rather than a
//! timer somebody remembers to cancel.
//!
//! This is a port of the main process's `llm/coalesce.cjs`, deliberately down to
//! the interval: the two shells stream the same replies and a person moving
//! between them should not be able to tell which one they are in.

use std::time::Duration;

use serde_json::Value;
use tokio::time::Instant;

/// Sixteen updates a second.
pub const FLUSH: Duration = Duration::from_millis(60);

/// Text held back, and when it is due.
///
/// Reasoning and prose are buffered separately and flushed together, in that
/// order: they are two different blocks on screen, and interleaving them would
/// put a thought in the middle of a sentence.
#[derive(Debug)]
pub struct Coalescer {
    prose: String,
    reasoning: String,
    due: Option<Instant>,
    interval: Duration,
}

impl Default for Coalescer {
    fn default() -> Self {
        Self::new(FLUSH)
    }
}

impl Coalescer {
    pub fn new(interval: Duration) -> Self {
        Self {
            prose: String::new(),
            reasoning: String::new(),
            due: None,
            interval,
        }
    }

    /// When the buffer is next due out, if anything is waiting.
    ///
    /// The caller selects on this. `None` means nothing is held and the loop
    /// can wait on the provider alone.
    pub fn due(&self) -> Option<Instant> {
        self.due
    }

    /// One event in, whatever should go to the window now out.
    ///
    /// Returns a list rather than an `Option` because a tool call arriving
    /// mid-sentence produces three: the reasoning so far, the prose so far, and
    /// then the card.
    pub fn push(&mut self, event: Value) -> Vec<Value> {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let text = event.get("text").and_then(Value::as_str);

        match (kind, text) {
            ("delta", Some(text)) => {
                self.prose.push_str(text);
                self.hold_or_flush(text)
            }
            ("reasoning", Some(text)) => {
                self.reasoning.push_str(text);
                self.hold_or_flush(text)
            }
            _ => {
                let mut out = self.flush();
                out.push(event);
                out
            }
        }
    }

    /// Everything held, and the timer stood down.
    ///
    /// Also the abort path: a turn that was cancelled is not coming back for
    /// its tail, and half a sentence dropped on the floor reads as a bug in the
    /// model rather than in the shell.
    pub fn flush(&mut self) -> Vec<Value> {
        self.due = None;
        let mut out = Vec::new();
        if !self.reasoning.is_empty() {
            out.push(serde_json::json!({
                "type": "reasoning",
                "text": std::mem::take(&mut self.reasoning),
            }));
        }
        if !self.prose.is_empty() {
            out.push(serde_json::json!({
                "type": "delta",
                "text": std::mem::take(&mut self.prose),
            }));
        }
        out
    }

    /// A newline goes now; anything else waits for the interval.
    fn hold_or_flush(&mut self, text: &str) -> Vec<Value> {
        if text.contains('\n') {
            return self.flush();
        }
        if self.due.is_none() {
            self.due = Some(Instant::now() + self.interval);
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn delta(text: &str) -> Value {
        json!({ "type": "delta", "text": text })
    }

    fn texts(events: &[Value]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|e| e.get("text").and_then(Value::as_str))
            .collect()
    }

    #[test]
    fn tokens_are_held_rather_than_forwarded_one_at_a_time() {
        let mut c = Coalescer::default();
        assert!(c.push(delta("Hel")).is_empty());
        assert!(c.push(delta("lo ")).is_empty());
        assert!(c.push(delta("there")).is_empty());

        let out = c.flush();
        assert_eq!(texts(&out), vec!["Hello there"]);
    }

    #[test]
    fn holding_anything_sets_a_deadline_and_flushing_clears_it() {
        let mut c = Coalescer::default();
        assert!(c.due().is_none());
        c.push(delta("a"));
        assert!(c.due().is_some());
        c.flush();
        assert!(c.due().is_none());
    }

    #[test]
    fn a_newline_goes_immediately() {
        let mut c = Coalescer::default();
        c.push(delta("# Heading"));
        let out = c.push(delta("\n\nBody"));
        // The whole block boundary lands at once rather than a beat later, so
        // the reader never sees the heading redrawn as a heading.
        assert_eq!(texts(&out), vec!["# Heading\n\nBody"]);
        assert!(c.due().is_none());
    }

    #[test]
    fn anything_that_is_not_text_flushes_what_came_before_it() {
        let mut c = Coalescer::default();
        c.push(delta("Let me look. "));
        let out = c.push(json!({ "type": "tool-start", "name": "read" }));

        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["text"], "Let me look. ");
        // The card cannot arrive ahead of the sentence that introduced it.
        assert_eq!(out[1]["type"], "tool-start");
    }

    #[test]
    fn reasoning_and_prose_are_separate_buffers_flushed_in_order() {
        let mut c = Coalescer::default();
        c.push(json!({ "type": "reasoning", "text": "weighing it up" }));
        c.push(delta("Here is the answer"));

        let out = c.flush();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["type"], "reasoning");
        assert_eq!(out[1]["type"], "delta");
    }

    #[test]
    fn a_done_that_arrives_with_text_still_held_does_not_overtake_it() {
        let mut c = Coalescer::default();
        c.push(delta("the last word"));
        let out = c.push(json!({ "type": "done", "stopped": "complete" }));

        assert_eq!(texts(&out), vec!["the last word"]);
        assert_eq!(out.last().and_then(|e| e.get("type")), Some(&json!("done")));
    }

    #[test]
    fn flushing_an_empty_buffer_says_nothing() {
        let mut c = Coalescer::default();
        assert!(c.flush().is_empty());
    }
}

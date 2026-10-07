//! Talking to a turn that is already running.
//!
//! The most common thing anyone wants to say to a working agent is "no, not
//! like that", and the only two answers a loop without this can give are
//! "stop everything" and "wait until it has finished being wrong". So a
//! running turn has a queue: the window hands it a message, the loop folds
//! that message into what the model sees at the next safe boundary, and the
//! turn carries on.
//!
//! Two halves, deliberately separated, because they happen at different
//! moments:
//!
//! - **Accepting** is immediate and synchronous. [`Steer::send`] answers
//!   `true` or `false` there and then, so a window whose turn has already
//!   finished learns that at once and can send the message as a new turn
//!   instead of dropping it.
//! - **Delivering** happens only at a step boundary, in
//!   [`crate::turn::Agent::run`]. That is the only place a user message can
//!   go without cutting a tool call away from its result, which strict
//!   providers reject outright.
//!
//! [`Steer::close`] is what keeps a turn from accepting a message it will
//! never answer. Nothing awaits between the loop's last look at this queue
//! and the close, so a message is either taken by a turn that is still going
//! or refused by one that is not.

use std::sync::{Arc, Mutex};

/// The queue one running turn is steered through.
///
/// Cheap to clone: every clone is the same queue. The app keeps one beside
/// the turn's cancellation handle, and the loop keeps the other.
#[derive(Clone, Debug, Default)]
pub struct Steer {
    inner: Arc<Mutex<Queue>>,
}

#[derive(Debug, Default)]
struct Queue {
    waiting: Vec<String>,
    closed: bool,
}

impl Steer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer a message to the turn. `false` means it was not taken - the turn
    /// has ended, or there was nothing but whitespace to say - and the caller
    /// must do something else with the text rather than assume it landed.
    ///
    /// The emptiness check is here rather than at the command edge so every
    /// caller gets it: a window that sends a stray newline must not wake a
    /// turn with an empty user message the model has to interpret.
    pub fn send(&self, text: &str) -> bool {
        let said = text.trim();
        if said.is_empty() {
            return false;
        }
        let mut queue = self.lock();
        if queue.closed {
            return false;
        }
        queue.waiting.push(said.to_string());
        true
    }

    /// Whether anything is waiting to be delivered.
    ///
    /// The loop asks this before it lets a turn end: someone who typed "wait,
    /// not that one" while the final answer was being written said it to
    /// *this* turn, and finishing without showing it to the model would be
    /// accepting their words and then throwing them away.
    pub fn pending(&self) -> bool {
        !self.lock().waiting.is_empty()
    }

    /// Everything said since the last boundary, oldest first.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().waiting)
    }

    /// Refuse anything further. Called when the turn ends, however it ends.
    pub fn close(&self) {
        self.lock().closed = true;
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// A guard that closes the queue when it is dropped.
    ///
    /// The loop holds one for the life of the stream, so a cancelled turn -
    /// where the stream is simply dropped and no code after it runs - stops
    /// accepting messages just as surely as one that ran to the end.
    pub fn closing(&self) -> Closing {
        Closing(self.clone())
    }

    /// A poisoned lock here means some other thread panicked while holding a
    /// `Vec<String>`. The queue is still perfectly usable, and taking the
    /// turn down over it would turn one panic into two.
    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Closes a [`Steer`] on drop. See [`Steer::closing`].
#[derive(Debug)]
pub struct Closing(Steer);

impl Drop for Closing {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_accepted_and_delivered_once() {
        let steer = Steer::new();
        assert!(steer.send("check the tests first"));
        assert!(steer.pending());
        assert_eq!(steer.take(), vec!["check the tests first".to_string()]);
        assert!(!steer.pending());
        assert!(steer.take().is_empty());
    }

    #[test]
    fn messages_keep_the_order_they_were_said_in() {
        let steer = Steer::new();
        steer.send("first");
        steer.send("second");
        assert_eq!(
            steer.take(),
            vec!["first".to_string(), "second".to_string()]
        );
    }

    #[test]
    fn whitespace_is_not_a_message() {
        let steer = Steer::new();
        assert!(!steer.send("   \n "));
        assert!(!steer.send(""));
        assert!(!steer.pending());
    }

    #[test]
    fn a_message_is_trimmed_before_it_is_queued() {
        let steer = Steer::new();
        steer.send("  stop  ");
        assert_eq!(steer.take(), vec!["stop".to_string()]);
    }

    #[test]
    fn a_closed_queue_refuses_rather_than_swallowing() {
        let steer = Steer::new();
        steer.close();
        assert!(!steer.send("too late"));
        assert!(steer.is_closed());
        assert!(!steer.pending());
    }

    #[test]
    fn the_guard_closes_the_queue_when_the_turn_goes_away() {
        let steer = Steer::new();
        {
            let _closing = steer.closing();
            assert!(steer.send("in time"));
        }
        assert!(!steer.send("too late"));
    }

    #[test]
    fn every_clone_is_the_same_queue() {
        let steer = Steer::new();
        let other = steer.clone();
        other.send("said to the clone");
        assert_eq!(steer.take(), vec!["said to the clone".to_string()]);
    }
}

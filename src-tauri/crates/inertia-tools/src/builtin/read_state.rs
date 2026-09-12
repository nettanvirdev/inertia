//! Which files the agent has actually looked at.
//!
//! `edit` refuses to touch a file this conversation has not read. That is not
//! bureaucracy: an edit is a claim about what the file currently contains, and
//! a model that has not read it is reconstructing from memory or from an
//! earlier version. The failure mode is silent and destructive - a plausible
//! edit applied to the wrong contents.
//!
//! The recorded mtime handles the other half: a file read ten minutes ago and
//! changed since is no better than one never read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use parking_lot::Mutex;

use inertia_core::SessionId;

#[derive(Debug, Default)]
pub struct ReadState {
    // Keyed by session so one conversation's reads never satisfy another's
    // edits - two agents working in the same workspace must each look first.
    seen: Mutex<HashMap<(String, PathBuf), SystemTime>>,
}

impl ReadState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that a file was read, with the modification time it had then.
    pub fn mark_read(&self, session: &SessionId, path: &Path, modified: SystemTime) {
        self.seen
            .lock()
            .insert((session.to_string(), path.to_path_buf()), modified);
    }

    /// Drops everything remembered for one conversation.
    ///
    /// A deleted thread must not leave its reads behind: ids are reused by
    /// nothing today, but the entries would otherwise live as long as the
    /// process, and "this file was already read" is exactly the wrong thing for
    /// a conversation to inherit.
    pub fn forget(&self, session: &str) {
        self.seen.lock().retain(|(held, _), _| held != session);
    }

    pub fn was_read(&self, session: &SessionId, path: &Path) -> bool {
        self.seen
            .lock()
            .contains_key(&(session.to_string(), path.to_path_buf()))
    }

    /// Whether the file has changed since it was read.
    ///
    /// An unread file is not "stale" - it is unread, which is a different
    /// error with a different fix, and conflating them tells the model to
    /// re-read a file it never read.
    pub fn is_stale(&self, session: &SessionId, path: &Path, modified: SystemTime) -> bool {
        match self
            .seen
            .lock()
            .get(&(session.to_string(), path.to_path_buf()))
        {
            Some(recorded) => modified > *recorded,
            None => false,
        }
    }

    /// Drops everything remembered for one conversation.
    pub fn forget_session(&self, session: &SessionId) {
        self.seen.lock().retain(|(s, _), _| s != session.as_str());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn state() -> (ReadState, SessionId, PathBuf) {
        (
            ReadState::new(),
            SessionId::from_existing("ses_1"),
            PathBuf::from("/ws/a.txt"),
        )
    }

    #[test]
    fn an_unread_file_is_unread() {
        let (state, session, path) = state();
        assert!(!state.was_read(&session, &path));
    }

    #[test]
    fn reading_is_remembered() {
        let (state, session, path) = state();
        state.mark_read(&session, &path, SystemTime::UNIX_EPOCH);
        assert!(state.was_read(&session, &path));
    }

    #[test]
    fn a_file_changed_since_reading_is_stale() {
        let (state, session, path) = state();
        let read_at = SystemTime::UNIX_EPOCH;
        state.mark_read(&session, &path, read_at);

        assert!(!state.is_stale(&session, &path, read_at));
        assert!(state.is_stale(&session, &path, read_at + Duration::from_secs(1)));
    }

    /// Two different errors with two different fixes.
    #[test]
    fn an_unread_file_is_never_reported_as_stale() {
        let (state, session, path) = state();
        assert!(!state.is_stale(&session, &path, SystemTime::now()));
    }

    /// One conversation looking at a file must not license another to edit it.
    #[test]
    fn reads_do_not_leak_between_conversations() {
        let (state, session, path) = state();
        state.mark_read(&session, &path, SystemTime::UNIX_EPOCH);

        let other = SessionId::from_existing("ses_2");
        assert!(!state.was_read(&other, &path));
    }

    #[test]
    fn forgetting_a_session_clears_only_that_session() {
        let (state, session, path) = state();
        let other = SessionId::from_existing("ses_2");
        state.mark_read(&session, &path, SystemTime::UNIX_EPOCH);
        state.mark_read(&other, &path, SystemTime::UNIX_EPOCH);

        state.forget_session(&session);

        assert!(!state.was_read(&session, &path));
        assert!(state.was_read(&other, &path));
    }
}

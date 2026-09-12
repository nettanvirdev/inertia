//! Typed identifiers.
//!
//! Sessions, messages and tool calls all have string ids, and all three get
//! passed through the same functions often enough that mixing them up is a
//! matter of time. Distinct types make that a compile error instead of a bug
//! report about a session that will not load.

use std::fmt;

/// Declares a newtype over `String` that serialises as a bare string.
///
/// A macro rather than a generic `Id<T>` marker: the error messages name the
/// actual type (`SessionId`, not `Id<SessionMarker>`), which is most of the
/// value of having the types at all.
macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Mints a new id.
            ///
            /// UUIDv7 rather than v4: the first 48 bits are a millisecond
            /// timestamp, so ids sort chronologically as plain strings. That
            /// makes a directory listing of session files, or a lexical sort
            /// of message ids, already in the right order - no separate index
            /// and no parsing to compare two of them.
            pub fn new() -> Self {
                Self(format!("{}{}", $prefix, uuid::Uuid::now_v7().simple()))
            }

            /// Wraps an id that already exists - read from disk, or handed to
            /// us by the frontend. Deliberately not validated: ids written by
            /// an older build are still ids, and rejecting them here would
            /// turn a cosmetic format change into unreadable history.
            pub fn from_existing(raw: impl Into<String>) -> Self {
                Self(raw.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        // Debug prints the bare id rather than `SessionId("ses_...")`. These
        // show up in log lines constantly and the wrapper is noise.
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&self.0, f)
            }
        }

        impl From<String> for $name {
            fn from(raw: String) -> Self {
                Self(raw)
            }
        }

        impl From<&str> for $name {
            fn from(raw: &str) -> Self {
                Self(raw.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(
    /// One conversation.
    SessionId,
    "ses_"
);

string_id!(
    /// One message within a session.
    MessageId,
    "msg_"
);

string_id!(
    /// One tool invocation. Minted by the provider, not by us, whenever the
    /// model is the one asking - so this is usually built with
    /// [`ToolCallId::from_existing`].
    ToolCallId,
    "tc_"
);

string_id!(
    /// One agent turn: a user message and everything the model did in
    /// response, across however many tool round-trips that took.
    TurnId,
    "turn_"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let a = SessionId::new();
        let b = SessionId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn ids_carry_their_prefix() {
        assert!(SessionId::new().as_str().starts_with("ses_"));
        assert!(MessageId::new().as_str().starts_with("msg_"));
    }

    // The whole reason for choosing v7. If this ever fails, every place that
    // relies on lexical ordering standing in for chronological ordering is
    // silently wrong.
    #[test]
    fn ids_sort_chronologically() {
        let mut minted: Vec<MessageId> = (0..50).map(|_| MessageId::new()).collect();
        let in_creation_order = minted.clone();
        minted.sort();
        assert_eq!(minted, in_creation_order);
    }

    #[test]
    fn serialises_as_a_bare_string() {
        let id = SessionId::from_existing("ses_abc");
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"ses_abc\"");
        let back: SessionId = serde_json::from_str("\"ses_abc\"").unwrap();
        assert_eq!(back, id);
    }

    // Ids minted by other tools will not look like ours, and history written
    // by an older build must keep loading.
    #[test]
    fn accepts_foreign_id_formats() {
        let id = ToolCallId::from_existing("toolu_01ABC");
        assert_eq!(id.as_str(), "toolu_01ABC");
    }
}

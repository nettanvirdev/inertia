//! The workspace on disk.
//!
//! The folder is the product: it is meant to be opened in a file manager, kept
//! in git, and read by a person. That constraint drives the whole design -
//! plain JSON, stable directory names, and byte formatting chosen so a diff
//! stays readable.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::needless_pass_by_value
    )
)]

pub mod collections;
pub mod conversations;
pub mod frontmatter;
pub mod fsx;
pub mod layout;
pub mod secrets;
pub mod settings;
pub mod skills;
pub mod transcript;

pub use conversations::{Conversations, Message, Thread};
pub use fsx::{ReadOutcome, StoreError};
pub use layout::{Collection, Document, Layout};
pub use settings::{Models, Permissions, Protocol, ProviderRecord, Secrets, Settings};

//! Tools: the registry that runs them, the schemas that describe them, and
//! the builtins themselves.
//!
//! The registry is the interesting part. It is the only way to invoke a tool,
//! which is what makes the permission check unskippable rather than merely
//! conventional.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::needless_pass_by_value
    )
)]

pub mod builtin;
pub mod registry;
pub mod replace;
pub mod schema;
pub mod truncate;

pub use builtin::{all as builtin_tools, file_tools, Lists, ReadState};
pub use registry::{shared, Limits, Registry};

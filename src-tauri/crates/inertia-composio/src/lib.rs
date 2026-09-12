//! Composio-connected apps, exposed to the agent as ordinary tools.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::needless_pass_by_value
    )
)]

pub mod api;
pub mod provider;
pub mod toolkit;

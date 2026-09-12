//! Fakes for every seam in `inertia-core`.
//!
//! This crate exists so the agent loop can be run - fully, with tool calls and
//! permission prompts - without a network, an API key, or a window. That is
//! what `inertia-devkit` does, and what most of the workspace's tests do.
//!
//! These are *fakes*, not stubs: they record what they were asked and answer
//! from a script, so a test can assert on the prompt that was assembled and
//! the permission target that was requested, not merely on the final output.
//! Those are the things a port breaks silently.
//!
//! They deliberately do **not** reimplement any pipeline. Mock tools plug into
//! the real registry and mock providers feed the real loop, so what the tests
//! exercise is the shipping code path.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod permission;
pub mod provider;
pub mod tool;

pub use permission::{MockGate, Policy};
pub use provider::MockProvider;
pub use tool::{Behaviour, MockTool};

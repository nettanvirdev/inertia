//! The agent turn loop.
//!
//! This crate is the reason the workspace is split the way it is. It holds the
//! orchestration - when to call the model, when to run a tool, when to stop -
//! and it holds no implementation of any of those things. It depends on
//! `inertia-core` and nothing else from the workspace.
//!
//! The practical consequence: the loop in [`turn::Agent::run`] is the same
//! code whether it is driving a real provider against a real filesystem or a
//! scripted fake against a temp directory. The tests below and the headless
//! harness in `inertia-devkit` both exercise the shipping path, not a
//! parallel one written for testing.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod event;
pub mod floor;
pub mod perspective;
pub mod prompt;
pub mod steer;
pub mod turn;

pub use event::{AgentEvent, StopReason};
pub use floor::{Floor, Outcome as FloorOutcome, MAX_AGENTS, MAX_HOPS};
pub use perspective::{perspective, Seat};
pub use prompt::{Approval, Context as PromptContext, Mode};
pub use steer::{Closing, Steer};
pub use turn::{Agent, AgentConfig, Turn, LOOP_THRESHOLD, MAX_STEPS, STEER_NOTE};

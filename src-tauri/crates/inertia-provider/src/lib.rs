//! Provider implementations.
//!
//! One module per wire protocol, each implementing `inertia_core::Provider`
//! and each emitting the identical event vocabulary, so the agent loop cannot
//! tell them apart.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::needless_pass_by_value
    )
)]

pub mod anthropic;
pub mod base;
pub mod config;
pub mod models;
pub mod openai;
pub mod retry;
pub mod sse;

pub use anthropic::client::AnthropicProvider;
pub use config::ProviderConfig;
pub use openai::client::OpenAiProvider;
pub use retry::Resilient;

//! Everything that talks to the host OS directly rather than through Tauri.
//!
//! Kept behind this module so the rest of the app crate contains no `#[cfg]`
//! ladders: each submodule exposes one cross-platform API and hides the fact
//! that the Windows path goes through Win32 while the others no-op or fall
//! back to Tauri's own calls.

pub mod window;

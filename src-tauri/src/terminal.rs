//! The terminal, as the window and the turn reach it.
//!
//! Everything with logic in it is in `inertia-terminal`: the pty, the screen it
//! paints, the shell integration that gets a folder out of a prompt. This file
//! is the two edges that crate deliberately does not have - Tauri commands for
//! the pane, and the seam `terminal_read` is built on - and nothing else.
//!
//! ## Why the terminals are process-wide rather than on `AppState`
//!
//! A shell belongs to a conversation and outlives every turn in it: a build
//! started in one turn has to still be running in the next, and a pane closed
//! and reopened has to find the same shell. That is process state, the same as
//! the background commands and the room rosters. Keeping it in a `OnceLock`
//! here rather than on the app state also keeps it reachable from a tool that
//! was handed nothing but a conversation id, and keeps the tests in this file
//! from having to build an `AppState` - which on Windows links the window
//! chrome into the test binary and stops the whole harness from starting.
//!
//! ## What is not offered
//!
//! No command writes into the person's shell on an agent's behalf. `write`,
//! `run` and `interrupt` exist for the PANE, which is the person typing; the
//! tool below is read-only. Two writers on one stdin interleave, and a command
//! the agent sent landing in the middle of one the person was typing loses them
//! the one surface in the app that was unambiguously theirs.

use std::sync::{Arc, OnceLock};

use inertia_core::tool::Tool;
use inertia_terminal::{Capability, Listed, OpenOptions, Reading, Snapshot, Terminals};
use inertia_tools::builtin::terminal::{terminal_tool as build_tool, TerminalTab};
use serde::Deserialize;
use tauri::AppHandle;

/// The channel the pane listens on. Twin of `terminal:event` in
/// `src/bridge/terminal.js`; the renderer was written against this name.
const TERMINAL_EVENT: &str = "terminal:event";

/// Every terminal this process has open.
pub fn terminals() -> &'static Terminals {
    static TERMINALS: OnceLock<Terminals> = OnceLock::new();
    TERMINALS.get_or_init(Terminals::new)
}

/// The window, when there is one.
#[derive(Debug)]
struct Window(AppHandle);

impl inertia_terminal::Events for Window {
    fn emit(&self, payload: serde_json::Value) {
        use tauri::Emitter as _;
        // Output that lands while the window is closing has nobody to tell.
        let _ = self.0.emit(TERMINAL_EVENT, payload);
    }
}

/// Start pushing terminal output to the window. Called once, at setup.
///
/// Output is pushed rather than polled: a build writes hundreds of lines and a
/// pane that asked for them on a timer would be a pane that stutters.
pub fn start(app: &AppHandle) {
    terminals().attach(Arc::new(Window(app.clone())));
}

/// Kill every shell and everything it started. Called when the app is closing.
///
/// Without this a `npm run dev` started in a terminal tab survives the app that
/// opened it, holding a port with nothing left to show it.
pub fn shutdown() {
    terminals().close_all();
}

// ── the pane ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenInput {
    pub cwd: Option<String>,
    pub cols: Option<usize>,
    pub rows: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadInput {
    pub chars: Option<usize>,
}

#[tauri::command]
pub fn terminal_capability() -> Capability {
    terminals().capability()
}

#[tauri::command]
pub fn terminal_open(id: String, options: Option<OpenInput>) -> Result<Snapshot, String> {
    let options = options.unwrap_or_default();
    terminals().open(
        &id,
        &OpenOptions {
            cwd: options.cwd,
            cols: options.cols,
            rows: options.rows,
        },
    )
}

/// Keystrokes, on a pty. The hot path of the whole pane: every character the
/// person types is one of these, so it does nothing but hand the bytes on.
#[tauri::command]
pub fn terminal_write(id: String, data: String) -> Result<usize, String> {
    terminals().write(&id, &data)
}

#[tauri::command]
pub fn terminal_resize(id: String, cols: Option<usize>, rows: Option<usize>) -> Result<(usize, usize), String> {
    terminals().resize(&id, cols, rows)
}

#[tauri::command]
pub fn terminal_run(id: String, command: String) -> Result<Snapshot, String> {
    terminals().run(&id, &command)
}

#[tauri::command]
pub fn terminal_interrupt(id: String) -> Result<Snapshot, String> {
    terminals().interrupt(&id)
}

#[tauri::command]
pub fn terminal_read(id: String, options: Option<ReadInput>) -> Reading {
    terminals().read(&id, options.unwrap_or_default().chars)
}

#[tauri::command]
pub fn terminal_set_cwd(id: String, cwd: String) -> Result<Snapshot, String> {
    terminals().set_cwd(&id, &cwd)
}

#[tauri::command]
pub fn terminal_close(id: String) -> bool {
    terminals().close(&id)
}

#[tauri::command]
pub fn terminal_list() -> Vec<Listed> {
    terminals().list()
}

// ── the tool ────────────────────────────────────────────────────────────

/// What `terminal_read` is given: the terminals, and a way to show one.
///
/// The tool lives in `inertia-tools` and knows nothing about Tauri or about a
/// pty. This is the whole of what it needs, and it is here because bringing a
/// tab forward means emitting a window event, which is this side's business.
#[derive(Debug)]
pub struct AppTerminals;

impl inertia_tools::builtin::terminal::Terminals for AppTerminals {
    fn read_thread(&self, thread: &str, chars: usize) -> Vec<TerminalTab> {
        terminals()
            .read_thread(thread, Some(chars))
            .into_iter()
            .map(|reading| TerminalTab {
                id: reading.id,
                cwd: reading.cwd,
                busy: reading.busy,
                running: reading.running,
                text: reading.text,
                front: reading.front,
            })
            .collect()
    }

    fn reveal(&self, id: &str) {
        terminals().reveal(id);
    }
}

/// The read-only tool, for the turn to hold.
pub fn terminal_tool() -> Arc<dyn Tool> {
    build_tool(Arc::new(AppTerminals))
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::tool::{ToolContext, ToolOutcome};
    use inertia_mock::MockGate;

    /// A mirrored tab rather than a shell: this is a test of the seam between
    /// the crate and the tool, and a pty would only add a second thing that can
    /// fail. The pty itself is exercised in `inertia-terminal`.
    async fn read_of(thread: &str) -> ToolOutcome {
        let ctx = ToolContext {
            root: std::env::temp_dir(),
            session: SessionId::from_existing(thread),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(MockGate::allow_all()),
        };
        terminal_tool()
            .execute(serde_json::json!({}), &ctx)
            .await
            .expect("reading a terminal is never a failure")
    }

    #[tokio::test]
    async fn the_tool_reads_the_terminals_this_process_actually_has() {
        // Named for this test alone: the table is process-wide, so two tests
        // sharing a conversation id would read each other's tabs.
        let thread = "app-terminal-test-1";
        let id = format!("chat:{thread}:job:1");
        terminals().mirror(&id, "D:\\work", "npm run dev", None);
        terminals().feed(&id, "ready on :3000\n", false);

        let out = read_of(thread).await;
        assert!(out.output.contains("ready on :3000"), "{}", out.output);
        assert!(out.output.contains("Working directory: D:\\work"));
        assert!(out.output.contains("Currently running: npm run dev"));
        assert_eq!(out.title.as_deref(), Some("the terminal (D:\\work)"));

        terminals().close(&id);
    }

    #[tokio::test]
    async fn a_conversation_with_no_pane_open_is_told_so_rather_than_failing() {
        let out = read_of("app-terminal-test-2").await;
        assert_eq!(
            out.output,
            "The terminal pane is not open, or nothing has been run in it yet."
        );
    }

    /// The pane asks this before it decides which of its two shapes to draw, so
    /// a build that answered `false` would draw the fallback for ever.
    #[test]
    fn this_build_has_a_real_terminal() {
        assert!(terminal_capability().pty);
    }

    #[test]
    fn closing_a_tab_nobody_opened_is_false_rather_than_an_error() {
        assert!(!terminal_close("chat:app-terminal-test-3:sh:1".to_string()));
        assert!(terminal_read("chat:app-terminal-test-3:sh:1".to_string(), None)
            .text
            .is_empty());
    }
}

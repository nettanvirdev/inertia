//! The tools that ship with the app.
//!
//! Each is an ordinary `Tool` implementation with no special standing in the
//! registry: a builtin and an MCP tool go through the same validate, ask,
//! execute, truncate pipeline.

pub mod background;
pub mod browser;
pub mod fence;
pub mod files;
pub mod look;
pub mod lsp;
pub mod manage;
pub mod patch;
pub mod present;
pub mod read_state;
pub mod search;
pub mod shell;
pub mod terminal;
pub mod worktree;

pub use background::{background_tools, Background};
pub use files::{file_tools, EditTool, ReadTool, WriteTool};
pub use look::{look_tools, Lists, LsTool, PresentPlanTool, TodoTool};
pub use lsp::{lsp_tool, LspTool};
pub use manage::manage_tools;
pub use patch::patch_tool;
pub use present::present_tool;
pub use read_state::ReadState;
pub use search::{search_tools, GlobTool, GrepTool};
pub use shell::{shell_name, shell_tools, ShellTool};
pub use worktree::worktree_tools;

use std::sync::Arc;

use inertia_core::tool::Tool;

/// Every builtin, with the per-conversation state some of them share.
///
/// `background` is shared with `shell`: a command started in the background by
/// one tool is listed, read, typed into and stopped by the others, and two
/// tables would mean a turn could start a server it could never see again.
pub fn all(
    state: Arc<ReadState>,
    lists: Arc<Lists>,
    background: Arc<Background>,
    lsp: Arc<inertia_lsp::Lsp>,
) -> Vec<Arc<dyn Tool>> {
    let mut tools = file_tools(state.clone(), lsp.clone());
    tools.extend(search_tools());
    tools.extend(shell_tools(background.clone()));
    tools.extend(background_tools(background));
    tools.extend(look_tools(lists));
    // Moving, copying and organising, which used to be a shell command the
    // model had to write differently on every platform.
    tools.extend(manage_tools());
    // Entering a worktree, so a long change happens on a branch of its own.
    tools.extend(worktree_tools());
    tools.push(patch_tool(state, lsp.clone()));
    // Handing the finished thing over, as distinct from the log of everything
    // touched on the way to it.
    tools.push(present_tool());
    // Asking the language server instead of guessing. `grep` finds the word;
    // this understands the symbol, which is a different question and usually
    // the one being asked.
    tools.push(lsp_tool(lsp));
    tools
}

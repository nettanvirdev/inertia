//! Memory: what an agent knows before the conversation starts.
//!
//! Two halves that must not be confused, because confusing them is what makes a
//! memory system quietly stop behaving like itself:
//!
//!   · **Above the seam, and always ours**: when to capture, what is worth
//!     keeping, what reaches the prompt and within what budget. This is the
//!     product. It does not change because the storage changed.
//!   · **Below the seam, and replaceable**: storing a record and finding it
//!     again. Folders of JSON files by default - the workspace for what is about
//!     the person, the project's own `.inertia/memory/` for what is about the
//!     code - or any MCP memory server instead, which is what lets Inertia and
//!     another coding agent pointed at the same server know the same things.
//!     `store::Backend` is that seam; `mcp` is the other side of it.
//!
//! Everything here degrades rather than fails. A memory store is an improvement
//! to a conversation, never a precondition for one: if it is unreadable or slow,
//! the turn goes ahead without it.
//!
//! The crate knows nothing about Tauri and nothing about any particular model.
//! The one thing it cannot do alone is decide what a conversation was worth
//! remembering, which takes a model call, so that is a trait the caller
//! implements - the same seam every other crate here uses.

// A test that cannot set up its own fixture has nothing to say, so the two
// lints that forbid a panic are lifted for tests only.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod capture;
pub mod extract;
pub mod mcp;
pub mod rank;
pub mod record;
pub mod store;
pub mod tools;

pub use capture::{Capture, Complete, Captured};
pub use record::{INJECT_BUDGET_BYTES, LOW_CONFIDENCE, STALE_AFTER_DAYS};
pub use store::{Backend, Capabilities, Local, Store};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How often the app looks for something worth keeping.
pub const CAPTURE_MODES: &[&str] = &["session", "turn", "off"];

/// The storage choice that needs nothing, and the one everything falls back to.
/// The Memory screen writes this exact string for "This workspace".
pub const LOCAL_BACKEND: &str = "local";

/// What the preferences say about memory.
///
/// Read from the identity document the settings screen writes, and checked
/// rather than trusted: a zero or a missing budget typed into a settings box
/// would otherwise switch memory off silently, with no message anywhere saying
/// why.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub enabled: bool,
    /// Where memories are kept: `local`, or `mcp:<server id>` for a memory
    /// server shared with whatever else is pointed at it.
    pub backend: String,
    /// `session`, `turn` or `off`.
    pub capture: String,
    /// New memories wait on the Memory screen instead of being believed at once.
    pub review: bool,
    pub budget: usize,
    pub project_memory: bool,
    pub agent_scoped: bool,
    pub instructions: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: LOCAL_BACKEND.into(),
            capture: "session".into(),
            review: false,
            budget: INJECT_BUDGET_BYTES,
            project_memory: true,
            agent_scoped: false,
            instructions: String::new(),
        }
    }
}

impl Settings {
    /// The preferences as the settings screen stores them.
    ///
    /// Everything is optional and everything has a default, because this
    /// document is written by a screen that only writes the fields somebody
    /// touched.
    pub fn from_preferences(prefs: &Value) -> Self {
        let flag = |name: &str, fallback: bool| {
            prefs.get(name).and_then(Value::as_bool).unwrap_or(fallback)
        };
        let text = |name: &str| {
            prefs
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };

        let budget = prefs.get("memoryBudget").and_then(Value::as_u64).unwrap_or(0) as usize;
        let capture = match text("memoryCapture") {
            given if CAPTURE_MODES.contains(&given.as_str()) => given,
            _ => "session".into(),
        };

        let backend = text("memoryBackend");
        Self {
            enabled: flag("memory", true),
            backend: if backend.is_empty() { LOCAL_BACKEND.into() } else { backend },
            capture,
            review: flag("memoryReview", false),
            budget: if budget > 0 { budget } else { INJECT_BUDGET_BYTES },
            project_memory: flag("memoryProject", true),
            agent_scoped: flag("memoryPerAgent", false),
            instructions: text("memoryInstructions").trim().to_string(),
        }
    }
}

/// What the prompt should carry, for one turn.
///
/// Built once at the top of a turn and never rebuilt inside it. Tool schemas and
/// the system prompt sit at the front of the cacheable prefix, so a block that
/// changed between turns would break every cache read behind it and cost more
/// than the memories are worth. A fact learned mid-conversation therefore lands
/// in the next one, which is a deliberate trade and the right way round.
///
/// `query` is what was just said. The memories that answer it are the ones that
/// make the budget, rather than the ones used most.
pub fn for_turn(
    store: &Store,
    settings: &Settings,
    agent_id: Option<&str>,
    query: &str,
) -> Vec<Value> {
    if !settings.enabled {
        return Vec::new();
    }

    let rows = store.list();
    // The common case for a new workspace and for every turn before anyone has
    // written anything down: one directory read and no more.
    if rows.is_empty() {
        return Vec::new();
    }

    let mut usable = store.in_scope(&rows);
    if !settings.project_memory {
        usable.retain(|row| record::scope_of(row) != "project");
    }
    if settings.agent_scoped {
        if let Some(agent) = agent_id {
            usable.retain(|row| {
                let own = record::text(row, "agentId");
                own.is_empty() || own == agent
            });
        }
    }

    let refs: Vec<&Value> = usable.iter().collect();
    let now = jiff::Timestamp::now().as_millisecond();
    rank::for_prompt(&refs, query, settings.budget, now)
        .into_iter()
        .cloned()
        .collect()
}

/// The block of memories as the prompt carries it.
///
/// Titles and one line each, which is the bargain skills make for the same
/// reason: the body costs bytes on every turn and arrives only when the model
/// asks for it. So a title has to be worth reading on its own.
pub fn to_prompt_block(memories: &[Value]) -> String {
    if memories.is_empty() {
        return String::new();
    }

    let mut out = String::from(
        "<memories>\n\
         What you already know about this person and this project. Each line is a\n\
         title and a summary; use `memory_recall` to read one in full.\n",
    );
    for memory in memories {
        let title = record::text(memory, "title");
        let line = record::summarise(memory, record::MAX_SUMMARY);
        let id = record::text(memory, "id");
        out.push_str(&format!("\n- [{id}] {title}: {line}"));
    }
    out.push_str("\n</memories>");
    out
}

/// Read what is already known, for a query.
pub fn recall(store: &Store, query: &str, limit: usize) -> Vec<Value> {
    let rows = store.list();
    let usable = store.in_scope(&rows);
    let refs: Vec<&Value> = usable.iter().collect();
    let now = jiff::Timestamp::now().as_millisecond();
    let found: Vec<Value> = rank::search(&refs, query, limit, now)
        .into_iter()
        .cloned()
        .collect();

    let ids: Vec<String> = found.iter().map(|row| record::text(row, "id")).collect();
    if !ids.is_empty() {
        store.touch(&ids);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_settings_document_that_says_nothing_leaves_memory_on() {
        let settings = Settings::from_preferences(&json!({}));
        assert!(settings.enabled);
        assert!(settings.project_memory);
        assert_eq!(settings.capture, "session");
        assert_eq!(settings.budget, INJECT_BUDGET_BYTES);
    }

    /// A zero or a nonsense budget would otherwise switch memory off with
    /// nothing on screen saying why.
    #[test]
    fn a_budget_of_zero_is_not_a_budget() {
        let settings = Settings::from_preferences(&json!({ "memoryBudget": 0 }));
        assert_eq!(settings.budget, INJECT_BUDGET_BYTES);
        let chosen = Settings::from_preferences(&json!({ "memoryBudget": 4096 }));
        assert_eq!(chosen.budget, 4096);
    }

    #[test]
    fn an_unknown_capture_mode_falls_back_to_the_default() {
        let settings = Settings::from_preferences(&json!({ "memoryCapture": "hourly" }));
        assert_eq!(settings.capture, "session");
        let off = Settings::from_preferences(&json!({ "memoryCapture": "off" }));
        assert_eq!(off.capture, "off");
    }

    #[test]
    fn the_block_carries_titles_and_one_line_each() {
        let block = to_prompt_block(&[json!({
            "id": "a", "title": "Deploys go to fly.io", "body": "Never render. The\nreason is..."
        })]);
        assert!(block.contains("- [a] Deploys go to fly.io: Never render."));
        assert!(block.starts_with("<memories>"));
        assert!(block.ends_with("</memories>"));
        // Nothing at all when there is nothing to say: an empty element would
        // still cost bytes on every turn and tell the model nothing.
        assert_eq!(to_prompt_block(&[]), "");
    }
}

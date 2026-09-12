//! Finding the agent a model named.
//!
//! Shared by everything that takes a colleague's name as an argument - `task`,
//! `invite`, `handover`, `part`. One implementation on purpose: these are the
//! tools where getting the wrong agent is not a failed call but the wrong agent
//! doing the work, and two resolvers would eventually disagree about which
//! spelling means whom.

use inertia_store::Layout;
use serde_json::Value;

/// One agent, as much of it as a caller needs.
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    /// The display name, falling back to the id for a record without one.
    pub name: String,
    /// The whole record, because callers want different fields out of it and a
    /// typed mirror of an agent file is a list of ways to read one back empty.
    pub record: Value,
}

impl Agent {
    /// A trimmed string field, or `None` for missing and blank alike.
    ///
    /// Blank is what a field somebody never filled in actually looks like on
    /// disk, and letting it through is how "You are Inertia Dev, the  on this
    /// person's team" reaches a model.
    pub fn text(&self, key: &str) -> Option<String> {
        self.record
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    /// Whether this agent has been stood down.
    ///
    /// A paused agent is not a broken one: the person turned it off, and the
    /// right answer is to say so rather than to quietly run it anyway.
    pub fn is_paused(&self) -> bool {
        matches!(self.text("status").as_deref(), Some("paused"))
    }
}

/// The thinking dials an agent record carries, as the loop wants them.
///
/// Two shapes because the providers have two: Anthropic takes a token budget,
/// OpenAI-compatible ones take `"low" | "medium" | "high"`. A record can name
/// either; whichever the provider ignores costs nothing.
///
/// This is the link that was missing. The editor writes `thinkingBudget` to the
/// record, the Anthropic client builds a `thinking` block whenever the request
/// carries one - and nothing in between ever read the record, so an agent asking
/// for twenty-four thousand tokens of reasoning silently got none, and the
/// person watching saw a model that never thinks.
pub fn thinking_of(record: &Value) -> (Option<u32>, Option<String>) {
    let budget = record
        .get("thinkingBudget")
        .and_then(Value::as_u64)
        // Zero is how the editor spells "off", and sending `budget_tokens: 0`
        // is not off - it is a request the API rejects.
        .filter(|budget| *budget > 0)
        .map(|budget| budget.min(u32::MAX as u64) as u32);

    let effort = record
        .get("reasoningEffort")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|effort| matches!(*effort, "low" | "medium" | "high"))
        .map(str::to_string);

    (budget, effort)
}

/// Every agent in the workspace.
pub fn list(layout: &Layout) -> Vec<Agent> {
    inertia_store::collections::list(layout, inertia_store::Collection::Agents)
        .into_iter()
        .filter_map(|record| {
            let id = record
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())?
                .to_string();
            let name = record
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(&id)
                .to_string();
            Some(Agent { id, name, record })
        })
        .collect()
}

/// Find the agent the model named, by whatever it called them.
///
/// Models write a colleague's name the way a person would - "Iris", "@iris",
/// sometimes the id it saw in a roster, and sometimes a filename it read out of
/// the agents folder. All of those resolve.
///
/// A near miss is answered with the list rather than with "not found", because
/// the fix is always to pick one of these and the model should not have to spend
/// a turn asking which.
pub fn resolve(layout: &Layout, wanted: &str) -> Result<Agent, String> {
    let agents = list(layout);
    // `.json` comes off because a model that has listed the agents folder is
    // holding filenames; `agent-` because the ids are prefixed and a model that
    // has seen one id guesses at the others.
    let needle = wanted
        .trim()
        .trim_start_matches('@')
        .trim_end_matches(".json")
        .to_lowercase();

    if !needle.is_empty() {
        let matches = |value: Option<String>| value.as_deref() == Some(needle.as_str());
        let lower = |agent: &Agent, key: &str| {
            agent
                .text(key)
                .map(|value| value.trim_start_matches('@').to_lowercase())
        };

        let found = agents
            .iter()
            .find(|agent| agent.id.to_lowercase() == needle)
            .or_else(|| agents.iter().find(|agent| matches(lower(agent, "name"))))
            .or_else(|| agents.iter().find(|agent| matches(lower(agent, "handle"))))
            // The handle as the composer spells it: "Nova Reyes" is
            // `@nova-reyes`, and that is what a model reading a roster in its
            // own prompt will write back.
            .or_else(|| {
                agents
                    .iter()
                    .find(|agent| inertia_agent::floor::slug_of(&agent.name) == needle)
            });

        if let Some(agent) = found {
            return Ok(agent.clone());
        }
    }

    Err(if agents.is_empty() {
        "There are no agents configured, so there is nobody to delegate to.".to_string()
    } else {
        format!(
            "There is no agent called {wanted}. The agents on this team are: {}.",
            agents
                .iter()
                .map(|agent| agent.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        for record in [
            json!({ "id": "agent-inertia-dev", "name": "Inertia Dev", "handle": "@inertiadev" }),
            json!({ "id": "agent-climate", "name": "Climate Scientist", "status": "paused" }),
        ] {
            inertia_store::collections::put(&layout, inertia_store::Collection::Agents, record)
                .expect("the agent was written");
        }
        (dir, layout)
    }

    #[test]
    fn an_agent_resolves_by_id_name_handle_and_composer_slug() {
        let (_dir, layout) = workspace();
        for spelling in [
            "agent-inertia-dev",
            "Inertia Dev",
            "inertia dev",
            "@inertiadev",
            "inertiadev",
            "@inertia-dev",
        ] {
            let found = resolve(&layout, spelling)
                .unwrap_or_else(|e| panic!("{spelling} did not resolve: {e}"));
            assert_eq!(found.id, "agent-inertia-dev", "{spelling}");
        }
    }

    #[test]
    fn a_filename_from_the_agents_folder_resolves() {
        let (_dir, layout) = workspace();
        let found = resolve(&layout, "agent-climate.json").expect("it resolved");
        assert_eq!(found.id, "agent-climate");
    }

    #[test]
    fn a_name_nobody_has_comes_back_with_the_list() {
        let (_dir, layout) = workspace();
        let error = resolve(&layout, "the designer").expect_err("nobody by that name");
        assert!(error.contains("Inertia Dev"), "{error}");
        assert!(error.contains("Climate Scientist"), "{error}");
    }

    #[test]
    fn an_empty_workspace_says_there_is_nobody_rather_than_naming_nobody() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let error = resolve(&Layout::new(dir.path()), "anyone").expect_err("no agents");
        assert!(error.contains("no agents configured"), "{error}");
    }

    #[test]
    fn a_stood_down_agent_is_reported_as_paused() {
        let (_dir, layout) = workspace();
        assert!(resolve(&layout, "Climate Scientist").expect("found").is_paused());
        assert!(!resolve(&layout, "Inertia Dev").expect("found").is_paused());
    }

    #[test]
    fn the_thinking_budget_reaches_the_loop_and_zero_means_off() {
        assert_eq!(thinking_of(&json!({ "thinkingBudget": 24576 })).0, Some(24576));
        // Zero is how the editor spells "off", and `budget_tokens: 0` is a
        // request the API refuses rather than a request that does not think.
        assert_eq!(thinking_of(&json!({ "thinkingBudget": 0 })).0, None);
        assert_eq!(thinking_of(&json!({})).0, None);
    }

    #[test]
    fn only_the_three_efforts_a_provider_understands_are_passed_on() {
        assert_eq!(thinking_of(&json!({ "reasoningEffort": "high" })).1.as_deref(), Some("high"));
        assert_eq!(thinking_of(&json!({ "reasoningEffort": "default" })).1, None);
        assert_eq!(thinking_of(&json!({ "reasoningEffort": null })).1, None);
    }

    #[test]
    fn a_blank_field_reads_as_absent() {
        let agent = Agent {
            id: "a".into(),
            name: "A".into(),
            record: json!({ "role": "   ", "description": "real" }),
        };
        assert_eq!(agent.text("role"), None);
        assert_eq!(agent.text("description").as_deref(), Some("real"));
    }
}

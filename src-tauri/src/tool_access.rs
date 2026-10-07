//! Loading tools when they are needed.
//!
//! Every tool a turn holds is its schema in the request, and the model reads
//! all of them before it writes a word. The core dozen cost well under a second
//! to first token; the same request with a couple of connected apps costs two
//! to three. That is paid on every turn, including the ones that only wanted
//! `read`. The renderer offers the person a choice (`toolAccess`, in
//! `src/shared/tool-access.js`), and until this module existed the Tauri build
//! honoured neither answer: the control saved a value nothing read.
//!
//! [`Deferred`] is that answer. It wraps the turn's real registry and shows the
//! model the families it reaches for on nearly every message plus one gate,
//! `load_tools`, whose description names every other group and what is in it.
//! Three things make the deferred half as usable as the loaded half, and they
//! are the whole design, copied from the Electron app on purpose:
//!
//!   - the gate's description is generated per turn from the tools that
//!     actually exist, never written by hand;
//!   - loading never asks and never refuses. Permission is enforced when a
//!     tool runs, by the inner registry, exactly as it would have been had the
//!     tool never left. Gating the act of looking at a schema would only teach
//!     a model that its tools are unreliable;
//!   - a deferred tool called by exact name is loaded and run in the same
//!     step, so a group the model already knows the names of costs nothing.
//!
//! The loaded set lives in the wrapper, and the wrapper is built per turn next
//! to the registry it wraps, so "for the rest of the turn" falls out of the
//! object's lifetime rather than a clock.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use inertia_core::message::ToolCall;
use inertia_core::provider::ToolSpec;
use inertia_core::tool::{
    ProviderProblem, Tool, ToolContext, ToolRegistry, ToolResult, ToolSource,
};
use parking_lot::Mutex;
use serde_json::{json, Value};

/// The gate's id. The renderer already renders a card for this name.
pub const GATE: &str = "load_tools";

/// What the renderer writes when the person picks "Load tools when they are
/// needed". `all` is the default and everything else the app has ever done.
const ON_DEMAND: &str = "on-demand";

/// The tools that are never behind the gate.
///
/// The test is not "is this important" but "is this used on nearly every
/// turn": a family loaded on the first step of every turn costs a round trip
/// for nothing.
const ALWAYS_LOADED: &[&str] = &[
    "read",
    "write",
    "edit",
    "patch",
    "ls",
    "glob",
    "grep",
    "lsp",
    "shell",
    "todowrite",
    "question",
    "skill",
    "failures",
    "later",
    // Everything the store does. Reading and writing are constant, and
    // forgetting is not: it is here anyway, because a memory that turned out
    // to be wrong is worth deleting the moment it is noticed, and a turn that
    // must load a tool first will leave it standing.
    "memory_recall",
    "memory_save",
    "memory_forget",
    // Moving files around. The same work as `write` and `edit`, done to whole
    // files instead of their contents, and reached for on the same turns.
    "file_copy",
    "file_move",
    "file_folder",
    "file_delete",
    // The other half of `shell`. A command left running in the background is
    // unreadable without these, and the turn that started it is the turn that
    // needs them.
    "shell_list",
    "shell_logs",
    "shell_write",
    "shell_kill",
    // This workspace's own setup: agents, routines, skills, servers, rules.
    // Inertia configuring itself is close to the core of what it is for, and
    // the model has to be able to see that it can.
    "inertia_list",
    "inertia_get",
    "inertia_save",
    "inertia_set_picture",
    "inertia_connect_app",
    "inertia_remove",
    "inertia_set_rules",
    // Handing over the result. A turn that would have to fetch this tool
    // before it could show you what it built will simply not show you.
    "present",
    "present_plan",
    // The one delegation tool that is always there; the asynchronous half is
    // a group.
    "task",
    "worktree_enter",
    "worktree_exit",
    GATE,
];

/// A built-in family: what a turn reaches for together.
///
/// Written out rather than derived from the permission keys, because the keys
/// answer "what would a person write a rule about" and this answers "what does
/// a turn reach for together", and those are not the same cut: `file_*` share
/// a key with `write` and are always loaded, while the browser tools are one
/// key and one family at once.
///
/// Short list on purpose. A family here is one a turn either lives in or never
/// opens at all - a browser session, a crew of agents - and everything that
/// merely *might* come up is cheaper in front of the gate than behind it.
struct Family {
    id: &'static str,
    label: &'static str,
    summary: &'static str,
    tools: &'static [&'static str],
}

const FAMILIES: &[Family] = &[
    Family {
        id: "browser",
        label: "Driving the browser pane",
        summary: "Open a page in the pane beside the conversation and work it: browser_navigate opens a URL and returns the page's outline, browser_read_page and browser_read_text re-read it, browser_click, browser_type and browser_press act on it, browser_evaluate runs script in it, and browser_console and browser_network read what it logged and fetched.",
        tools: &[
            "browser_navigate",
            "browser_read_page",
            "browser_read_text",
            "browser_click",
            "browser_type",
            "browser_press",
            "browser_evaluate",
            "browser_console",
            "browser_network",
        ],
    },
    Family {
        id: "crew",
        label: "Working alongside other agents",
        summary: "Start helpers without waiting for them, see what they have finished, send them a note, and stop one. This is the difference between delegating and blocking.",
        tools: &["spawn", "team", "collect", "agent_send", "wait", "followup", "interrupt"],
    },
];

/// How many tool names to print for a group before printing a count instead.
///
/// The names are the point for a small family: a model that can write the
/// name can call it, which is what makes a deferred tool as reachable as a
/// loaded one. For a connected app with forty actions the names would cost
/// more than the schemas they replaced.
const NAMES_SHOWN: usize = 8;

/// Whether the person asked for tools to be loaded on demand.
///
/// Takes the preferences bag the way the renderer stores it in
/// `settings/identity.json` (`{ user: { preferences: { toolAccess } } }`),
/// and accepts either the bag itself or the whole document, because the two
/// callers that have one in hand hold different halves.
pub fn wanted(prefs: &Value) -> bool {
    let chosen = prefs
        .get("toolAccess")
        .or_else(|| prefs.pointer("/user/preferences/toolAccess"))
        .or_else(|| prefs.pointer("/preferences/toolAccess"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    chosen.eq_ignore_ascii_case(ON_DEMAND)
}

/// What the grouping needs to know about one tool, and nothing more.
///
/// A [`ToolSpec`] carries a name, a description and a schema: the model's view.
/// Grouping needs the source too, and for a runtime tool the thing it came
/// from - the MCP server, the Composio toolkit, the imported API - which the
/// registry does not put in a spec. So the wrapper is handed these alongside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolFact {
    pub id: String,
    pub source: ToolSource,
    /// The server, toolkit or API this tool belongs to. `None` for a builtin.
    pub hint: Option<String>,
}

impl ToolFact {
    /// Reads the facts off a live tool.
    ///
    /// The `Tool` trait has no "what do you belong to" method, so the hint is
    /// read from what the adapters already expose: a Composio tool's
    /// permission target is its toolkit, and the MCP and OpenAPI adapters both
    /// render as `"<origin> · <what>"`. Reading a label is the weakest seam in
    /// this module - see the Integration note about `Tool::group_hint` - but it
    /// degrades to `mcp`/`api` rather than to a missing tool.
    pub fn of(tool: &dyn Tool) -> Self {
        let source = tool.source();
        let hint = match source {
            ToolSource::Composio => Some(tool.permission(&Value::Null).target),
            ToolSource::Mcp | ToolSource::OpenApi => tool
                .render(&Value::Null)
                .and_then(|label| label.split(" · ").next().map(str::to_string)),
            ToolSource::Builtin => None,
        }
        .map(|hint| hint.trim().to_string())
        .filter(|hint| !hint.is_empty());
        Self {
            id: tool.id().to_string(),
            source,
            hint,
        }
    }
}

/// Where the wrapper learns each tool's source and origin.
///
/// Implemented next to the real registry by the code that builds it, and by a
/// fake in the tests. Called on every step, so it must be as cheap as
/// `ToolRegistry::specs`: cached state, no network.
#[async_trait]
pub trait Facts: Send + Sync {
    async fn facts(&self) -> Vec<ToolFact>;
}

/// One group of deferred tools, as the gate describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Group {
    id: String,
    label: String,
    summary: String,
    tools: Vec<String>,
}

/// A group id that is safe to type back and unique per source.
fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in text.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch);
        } else {
            pending_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    trimmed.chars().take(48).collect()
}

/// Turns a slug back into something a person and a model both read easily.
fn title(text: &str) -> String {
    let words: Vec<&str> = text
        .split(['-', '_'])
        .filter(|word| !word.is_empty())
        .collect();
    let clean = words.join(" ");
    let mut chars = clean.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Which group a tool belongs to: `(id, label)`, or `None` when it is always
/// loaded.
///
/// A tool nobody classified comes back `None` on purpose. A family added later
/// and not listed here should cost a little latency, never its own existence.
fn group_for(fact: &ToolFact) -> Option<(String, String)> {
    let id = fact.id.as_str();
    if id.is_empty() || ALWAYS_LOADED.contains(&id) {
        return None;
    }
    // An agent that still has these was given a machine, and for that agent
    // the family is the core loop.
    if id.starts_with("computer") {
        return None;
    }

    let hint = fact.hint.as_deref().map(slug).filter(|s| !s.is_empty());
    match fact.source {
        ToolSource::Composio => {
            let app = hint.unwrap_or_else(|| "composio".to_string());
            Some((format!("app.{app}"), format!("{}, through Composio", title(&app))))
        }
        ToolSource::Mcp => {
            let server = hint.unwrap_or_else(|| "mcp".to_string());
            Some((format!("mcp.{server}"), format!("{}, an MCP server", title(&server))))
        }
        ToolSource::OpenApi => {
            let api = hint.unwrap_or_else(|| "api".to_string());
            Some((format!("api.{api}"), format!("{}, an imported API", title(&api))))
        }
        ToolSource::Builtin => FAMILIES
            .iter()
            .find(|family| family.tools.contains(&id))
            .map(|family| (family.id.to_string(), family.label.to_string())),
    }
}

/// What a group of runtime tools does, said in the only terms we have.
fn summarise(tools: &[String]) -> String {
    let count = tools.len();
    let noun = if count == 1 { "action" } else { "actions" };
    if count <= NAMES_SHOWN {
        format!("{count} {noun}: {}.", tools.join(", "))
    } else {
        format!(
            "{count} {noun}, including {}.",
            tools[..NAMES_SHOWN].join(", ")
        )
    }
}

/// The gate's description: every group, by name, with what is in it.
///
/// This text is the whole reason deferring tools does not cost the model
/// anything. A tool it cannot see is a tool it will not use, so what it cannot
/// see has to be described precisely enough that it knows the group exists,
/// knows what the group can do, and - for the small ones - knows the exact
/// names.
fn describe(groups: &[Group]) -> String {
    if groups.is_empty() {
        return "Load the tools for one or more groups into this conversation. Every tool \
                you have is already here, so there is nothing to load."
            .to_string();
    }

    let mut lines = vec![
        "Load the tools for one or more groups into this conversation.".to_string(),
        String::new(),
        "Not every tool you have is in this request. The ones used on almost every".to_string(),
        "turn are; the rest are grouped below and arrive when you ask for them. They".to_string(),
        "are as available as anything else - loading is one step, it never fails for".to_string(),
        "permission reasons, and once a group is loaded it stays for the rest of the".to_string(),
        "turn. Ask for every group you expect to need in a single call rather than".to_string(),
        "one at a time.".to_string(),
        String::new(),
        "The groups:".to_string(),
    ];
    for group in groups {
        // The exact names for a small group, always - unless the summary
        // already lists them, which a runtime group's does.
        let names = if group.tools.len() <= NAMES_SHOWN
            && !group
                .tools
                .first()
                .is_some_and(|first| group.summary.contains(first))
        {
            format!(" Tools: {}.", group.tools.join(", "))
        } else {
            String::new()
        };
        lines.push(format!(
            "- {} ({}): {}{}",
            group.id, group.label, group.summary, names
        ));
    }
    lines.push(String::new());
    lines.push("If you already know the exact name of a tool in a group, you may simply call".to_string());
    lines.push("it: it will be loaded and run in one step. Use this tool when you want to see".to_string());
    lines.push("what a group contains first.".to_string());
    lines.join("\n")
}

fn gate_spec(deferred: &[Group]) -> ToolSpec {
    ToolSpec::new(
        GATE,
        describe(deferred),
        json!({
            "type": "object",
            "properties": {
                "groups": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "description": "A group id, exactly as written in this tool's description."
                    },
                    "description": "The groups to load. Ask for every group you expect to need at once."
                }
            },
            "required": ["groups"]
        }),
    )
}

/// The turn's tools, cut in two.
struct Cut {
    /// What the request carries, sorted by name, gate included.
    open: Vec<ToolSpec>,
    /// Everything else, by id, in an order stable across turns.
    deferred: Vec<Group>,
    /// Whether any group exists at all, loaded or not. When none does the
    /// gate is not offered: a gate with nothing behind it is noise.
    grouped: bool,
}

impl Cut {
    fn deferred_group_of(&self, tool: &str) -> Option<&Group> {
        self.deferred
            .iter()
            .find(|group| group.tools.iter().any(|id| id == tool))
    }
}

/// What one `load_tools` call did.
struct Loaded {
    loaded: Vec<Group>,
    already: Vec<String>,
    unknown: Vec<String>,
    available: Vec<String>,
}

/// A registry that shows the model its core tools and a gate to the rest.
pub struct Deferred {
    inner: Arc<dyn ToolRegistry>,
    facts: Arc<dyn Facts>,
    /// Group ids brought in so far this turn.
    loaded: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for Deferred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deferred")
            .field("loaded", &*self.loaded.lock())
            .finish_non_exhaustive()
    }
}

impl Deferred {
    pub fn new(inner: Arc<dyn ToolRegistry>, facts: Arc<dyn Facts>) -> Self {
        Self {
            inner,
            facts,
            loaded: Mutex::new(HashSet::new()),
        }
    }

    /// The trait object the agent loop takes.
    pub fn wrap(inner: Arc<dyn ToolRegistry>, facts: Arc<dyn Facts>) -> Arc<dyn ToolRegistry> {
        Arc::new(Self::new(inner, facts))
    }

    /// Splits what the inner registry would offer right now.
    ///
    /// Starts from the inner `specs()` rather than the facts so that a tool
    /// the permission rules hide stays hidden: this wrapper narrows the list,
    /// it never widens it.
    async fn cut(&self) -> Cut {
        let specs = self.inner.specs().await;
        let facts: BTreeMap<String, ToolFact> = self
            .facts
            .facts()
            .await
            .into_iter()
            .map(|fact| (fact.id.clone(), fact))
            .collect();
        let loaded = self.loaded.lock().clone();

        let mut open = Vec::with_capacity(specs.len());
        let mut deferred: BTreeMap<String, Group> = BTreeMap::new();
        let mut grouped = false;

        for spec in specs {
            // A spec with no fact is treated as a builtin known only by id,
            // so the built-in families still work when the facts are partial.
            let fact = facts.get(&spec.name).cloned().unwrap_or(ToolFact {
                id: spec.name.clone(),
                source: ToolSource::Builtin,
                hint: None,
            });
            let Some((group_id, label)) = group_for(&fact) else {
                open.push(spec);
                continue;
            };
            grouped = true;
            if loaded.contains(&group_id) {
                open.push(spec);
                continue;
            }
            deferred
                .entry(group_id.clone())
                .or_insert_with(|| Group {
                    id: group_id,
                    label,
                    summary: String::new(),
                    tools: Vec::new(),
                })
                .tools
                .push(spec.name);
        }

        let mut deferred: Vec<Group> = deferred.into_values().collect();
        for group in &mut deferred {
            group.summary = FAMILIES
                .iter()
                .find(|family| family.id == group.id)
                .map(|family| family.summary.to_string())
                .unwrap_or_else(|| summarise(&group.tools));
        }

        if grouped {
            open.push(gate_spec(&deferred));
        }
        // The inner list was sorted; the gate has to land in its place. Two
        // turns whose tool lists differ only in order share no cached prefix.
        open.sort_by(|a, b| a.name.cmp(&b.name));

        Cut {
            open,
            deferred,
            grouped,
        }
    }

    /// The group a loosely written name means, because a model will write it
    /// loosely: `Files`, `my-server`, `mcp/my-server`, or the start of a label.
    fn group_named<'a>(deferred: &'a [Group], name: &str) -> Option<&'a Group> {
        let mut wanted = String::new();
        let mut pending_dot = false;
        for ch in name.trim().to_lowercase().chars() {
            if ch.is_whitespace() || matches!(ch, '/' | ':') {
                pending_dot = true;
            } else {
                if pending_dot {
                    wanted.push('.');
                    pending_dot = false;
                }
                wanted.push(ch);
            }
        }
        if wanted.is_empty() {
            return None;
        }
        deferred
            .iter()
            .find(|group| group.id.to_lowercase() == wanted)
            .or_else(|| {
                deferred.iter().find(|group| {
                    group.id.to_lowercase().rsplit('.').next() == Some(wanted.as_str())
                })
            })
            .or_else(|| {
                deferred
                    .iter()
                    .find(|group| group.id.to_lowercase().ends_with(&format!(".{wanted}")))
            })
            .or_else(|| {
                deferred
                    .iter()
                    .find(|group| group.label.to_lowercase().starts_with(&wanted))
            })
    }

    /// Brings one or more groups into this turn. Never refuses; never asks.
    fn load(&self, cut: &Cut, names: &[String]) -> Loaded {
        let mut remaining: Vec<Group> = cut.deferred.clone();
        let mut loaded = Vec::new();
        let mut already = Vec::new();
        let mut unknown = Vec::new();

        for name in names {
            let Some(index) = Self::group_named(&remaining, name)
                .map(|group| group.id.clone())
                .and_then(|id| remaining.iter().position(|group| group.id == id))
            else {
                let wanted = name.trim().to_lowercase();
                let seen = self.loaded.lock().iter().any(|id| {
                    id == &wanted || id.rsplit('.').next() == Some(wanted.as_str())
                });
                (if seen { &mut already } else { &mut unknown }).push(name.clone());
                continue;
            };
            let group = remaining.remove(index);
            self.loaded.lock().insert(group.id.clone());
            loaded.push(group);
        }

        Loaded {
            loaded,
            already,
            unknown,
            available: remaining.into_iter().map(|group| group.id).collect(),
        }
    }

    /// The gate itself.
    ///
    /// Handled here rather than by a `Tool` in the inner registry because the
    /// inner pipeline asks permission, and loading must never ask: the tools
    /// that arrive are asked about under their own rules exactly as they would
    /// have been had they never left.
    async fn run_gate(&self, call: &ToolCall) -> ToolResult {
        let started = Instant::now();
        let args = call.parsed_arguments();
        let asked: Vec<String> = match args.get("groups") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            // A single name where a list was asked for is a near miss, not a
            // mistake worth a round trip.
            Some(Value::String(one)) if !one.trim().is_empty() => vec![one.trim().to_string()],
            _ => Vec::new(),
        };

        if asked.is_empty() {
            return ToolResult {
                title: Some("Load tools: none".to_string()),
                duration_ms: started.elapsed().as_millis() as u64,
                ..ToolResult::failed(call.id.clone(), GATE, "Name at least one group to load.")
            };
        }

        let cut = self.cut().await;
        let result = self.load(&cut, &asked);

        let mut lines = Vec::new();
        for group in &result.loaded {
            lines.push(format!(
                "{} ({}): {}",
                group.id,
                group.label,
                group.tools.join(", ")
            ));
        }
        if !result.already.is_empty() {
            lines.push(format!("Already loaded: {}.", result.already.join(", ")));
        }
        if !result.unknown.is_empty() {
            let where_else = if result.available.is_empty() {
                "Every group is already loaded.".to_string()
            } else {
                format!("The groups are: {}.", result.available.join(", "))
            };
            lines.push(format!(
                "No group called {}. {where_else}",
                result.unknown.join(", ")
            ));
        }

        let count: usize = result.loaded.iter().map(|group| group.tools.len()).sum();
        let mut output = Vec::new();
        if count > 0 {
            output.push(format!(
                "{count} tool{} now available and will stay for the rest of this conversation. Use them directly.",
                if count == 1 { " is" } else { "s are" }
            ));
        }
        output.extend(lines);

        ToolResult {
            call_id: call.id.clone(),
            tool: GATE.to_string(),
            ok: true,
            title: Some(if count > 0 {
                format!("Loaded {count} tool{}", if count == 1 { "" } else { "s" })
            } else {
                "Nothing to load".to_string()
            }),
            output: output.join("\n"),
            metadata: Some(json!({
                "groups": result.loaded.iter().map(|group| group.id.clone()).collect::<Vec<_>>(),
                "count": count,
            })),
            images: Vec::new(),
            duration_ms: started.elapsed().as_millis() as u64,
        }
    }
}

#[async_trait]
impl ToolRegistry for Deferred {
    async fn specs(&self) -> Vec<ToolSpec> {
        self.cut().await.open
    }

    async fn run(&self, call: &ToolCall, ctx: &ToolContext) -> inertia_core::Result<ToolResult> {
        if call.name == GATE {
            return Ok(self.run_gate(call).await);
        }

        let cut = self.cut().await;

        // A deferred tool called by name loads its group and runs, in one
        // step. This is what stops loading on demand from costing capability:
        // the gate names the tools in the small groups, so a model that reads
        // it can write `browser_type` directly, and that has to just work.
        if let Some(group) = cut.deferred_group_of(&call.name) {
            self.loaded.lock().insert(group.id.clone());
        }

        let mut result = self.inner.run(call, ctx).await?;

        // A model that guessed a name close to a deferred one is one sentence
        // away from getting it right, and that sentence is free. Recognised by
        // the inner registry's own wording; if that changes the hint is
        // merely missing, nothing breaks.
        if !result.ok && cut.grouped && result.output.starts_with("No tool named") {
            let have: Vec<&str> = cut.open.iter().map(|spec| spec.name.as_str()).collect();
            result.output = format!(
                "{} The tools you have are: {}.",
                result.output,
                have.join(", ")
            );
            if !cut.deferred.is_empty() {
                let groups: Vec<&str> = cut.deferred.iter().map(|group| group.id.as_str()).collect();
                result.output.push_str(&format!(
                    " You can also load more tools with {GATE}; the groups are: {}.",
                    groups.join(", ")
                ));
            }
        }

        Ok(result)
    }

    async fn problems(&self) -> Vec<ProviderProblem> {
        self.inner.problems().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::tool::{PermissionGate, PermissionRequest, ToolOutcome};
    use inertia_core::{SessionId, ToolCallId};
    use inertia_mock::{MockGate, MockTool};
    use inertia_tools::Registry;

    /// A tool from an MCP server, as the adapter presents one: source `Mcp`,
    /// rendered as `"<server> · <tool>"`.
    #[derive(Debug)]
    struct FakeMcp {
        id: String,
        server: String,
    }

    #[async_trait]
    impl Tool for FakeMcp {
        fn id(&self) -> &str {
            &self.id
        }
        fn description(&self) -> &str {
            "searches"
        }
        fn parameters(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }
        fn source(&self) -> ToolSource {
            ToolSource::Mcp
        }
        fn permission(&self, _args: &Value) -> PermissionRequest {
            PermissionRequest::new("mcp", self.id.clone())
        }
        fn render(&self, _args: &Value) -> Option<String> {
            Some(format!("{} · search", self.server))
        }
        async fn execute(&self, _args: Value, _ctx: &ToolContext) -> inertia_core::Result<ToolOutcome> {
            Ok(ToolOutcome::text("found"))
        }
    }

    /// The real registry underneath, so `run` goes through the shipping
    /// validate/ask/execute pipeline, plus the facts the wrapper is owed.
    struct Fake {
        registry: Registry,
        tools: Vec<Arc<dyn Tool>>,
    }

    #[async_trait]
    impl ToolRegistry for Fake {
        async fn specs(&self) -> Vec<ToolSpec> {
            self.registry.specs().await
        }
        async fn run(&self, call: &ToolCall, ctx: &ToolContext) -> inertia_core::Result<ToolResult> {
            self.registry.run(call, ctx).await
        }
        async fn problems(&self) -> Vec<ProviderProblem> {
            self.registry.problems().await
        }
    }

    #[async_trait]
    impl Facts for Fake {
        async fn facts(&self) -> Vec<ToolFact> {
            self.tools.iter().map(|tool| ToolFact::of(tool.as_ref())).collect()
        }
    }

    struct Rig {
        deferred: Deferred,
        browser_click: Arc<MockTool>,
        read: Arc<MockTool>,
    }

    fn rig() -> Rig {
        let browser_click = Arc::new(MockTool::new("browser_click"));
        let read = Arc::new(MockTool::new("read"));
        let tools: Vec<Arc<dyn Tool>> = vec![
            read.clone(),
            browser_click.clone(),
            Arc::new(MockTool::new("browser_type")),
            Arc::new(MockTool::new("write")),
            Arc::new(MockTool::new("spawn")),
            Arc::new(FakeMcp {
                id: "my_server_search".into(),
                server: "My Server".into(),
            }),
        ];
        let gate: Arc<dyn PermissionGate> = Arc::new(MockGate::allow_all());
        let fake = Arc::new(Fake {
            registry: Registry::new(gate).with_tools(tools.clone()),
            tools,
        });
        Rig {
            deferred: Deferred::new(fake.clone(), fake),
            browser_click,
            read,
        }
    }

    fn ctx() -> ToolContext {
        ToolContext {
            root: std::path::PathBuf::from("."),
            session: SessionId::from_existing("thread-1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(MockGate::allow_all()),
        }
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: ToolCallId::from_existing("tc_1"),
            name: name.to_string(),
            arguments: args.to_string(),
        }
    }

    async fn names(deferred: &Deferred) -> Vec<String> {
        deferred.specs().await.into_iter().map(|spec| spec.name).collect()
    }

    #[tokio::test]
    async fn on_demand_hides_a_family_and_keeps_the_core() {
        let rig = rig();
        let have = names(&rig.deferred).await;
        assert!(have.contains(&"read".to_string()), "{have:?}");
        assert!(have.contains(&"write".to_string()), "{have:?}");
        assert!(have.contains(&GATE.to_string()), "{have:?}");
        assert!(!have.contains(&"browser_click".to_string()), "{have:?}");
        assert!(!have.contains(&"spawn".to_string()), "{have:?}");
        assert!(!have.contains(&"my_server_search".to_string()), "{have:?}");

        let mut sorted = have.clone();
        sorted.sort();
        assert_eq!(have, sorted, "specs must stay sorted by name");
    }

    #[tokio::test]
    async fn the_gate_names_each_family_and_its_tools() {
        let rig = rig();
        let specs = rig.deferred.specs().await;
        let gate = specs.iter().find(|spec| spec.name == GATE).expect("a gate");
        assert!(gate.description.contains("- crew (Working alongside other agents)"), "{}", gate.description);
        assert!(gate.description.contains("Tools: spawn."), "{}", gate.description);
        // The browser family's own summary names its tools, so they are not
        // printed a second time.
        assert!(gate.description.contains("- browser (Driving the browser pane)"), "{}", gate.description);
        assert!(gate.description.contains("browser_click"), "{}", gate.description);
        assert_eq!(gate.parameters["required"], json!(["groups"]));
    }

    #[tokio::test]
    async fn an_mcp_tool_groups_under_its_server() {
        let rig = rig();
        let specs = rig.deferred.specs().await;
        let gate = specs.iter().find(|spec| spec.name == GATE).expect("a gate");
        assert!(
            gate.description.contains("- mcp.my-server (My server, an MCP server): 1 action: my_server_search."),
            "{}",
            gate.description
        );
        // The runtime group's summary already lists the names, so they are
        // not printed twice.
        assert!(!gate.description.contains("Tools: my_server_search"), "{}", gate.description);
    }

    #[tokio::test]
    async fn loading_the_browser_makes_its_tools_appear() {
        let rig = rig();
        let result = rig
            .deferred
            .run(&call(GATE, json!({ "groups": ["browser"] })), &ctx())
            .await
            .expect("not cancelled");
        assert!(result.ok, "{}", result.output);
        assert_eq!(result.title.as_deref(), Some("Loaded 2 tools"));
        assert!(result.output.contains("browser (Driving the browser pane): browser_click, browser_type"), "{}", result.output);
        assert_eq!(result.metadata.as_ref().and_then(|m| m.get("groups")), Some(&json!(["browser"])));

        let have = names(&rig.deferred).await;
        assert!(have.contains(&"browser_click".to_string()), "{have:?}");
        assert!(have.contains(&"browser_type".to_string()), "{have:?}");
        // The others stay behind, and the gate stays offered for them.
        assert!(!have.contains(&"spawn".to_string()), "{have:?}");
        assert!(have.contains(&GATE.to_string()), "{have:?}");
        let mut sorted = have.clone();
        sorted.sort();
        assert_eq!(have, sorted);
    }

    #[tokio::test]
    async fn a_group_named_loosely_still_loads() {
        let rig = rig();
        let result = rig
            .deferred
            .run(&call(GATE, json!({ "groups": ["Browser", "mcp/my-server"] })), &ctx())
            .await
            .expect("not cancelled");
        assert!(result.ok);
        let have = names(&rig.deferred).await;
        assert!(have.contains(&"browser_click".to_string()), "{have:?}");
        assert!(have.contains(&"my_server_search".to_string()), "{have:?}");
    }

    #[tokio::test]
    async fn a_deferred_tool_called_by_name_runs_in_one_step() {
        let rig = rig();
        let result = rig
            .deferred
            .run(&call("browser_click", json!({ "input": "a.txt" })), &ctx())
            .await
            .expect("not cancelled");
        assert!(result.ok, "{}", result.output);
        assert_eq!(rig.browser_click.call_count(), 1);
        assert_eq!(rig.browser_click.calls()[0]["input"], "a.txt");
        // And its group is in the next request, so a second call needs nothing.
        assert!(names(&rig.deferred).await.contains(&"browser_type".to_string()));
    }

    #[tokio::test]
    async fn an_open_tool_runs_as_before() {
        let rig = rig();
        let result = rig
            .deferred
            .run(&call("read", json!({ "input": "x" })), &ctx())
            .await
            .expect("not cancelled");
        assert!(result.ok);
        assert_eq!(rig.read.call_count(), 1);
    }

    #[tokio::test]
    async fn an_unknown_group_is_told_what_exists() {
        let rig = rig();
        let result = rig
            .deferred
            .run(&call(GATE, json!({ "groups": ["email"] })), &ctx())
            .await
            .expect("not cancelled");
        assert!(result.ok);
        assert_eq!(result.title.as_deref(), Some("Nothing to load"));
        assert!(result.output.contains("No group called email."), "{}", result.output);
        assert!(
            result.output.contains("The groups are: browser, crew, mcp.my-server."),
            "{}",
            result.output
        );
    }

    #[tokio::test]
    async fn loading_the_same_group_twice_says_so() {
        let rig = rig();
        rig.deferred
            .run(&call(GATE, json!({ "groups": ["browser"] })), &ctx())
            .await
            .expect("not cancelled");
        let again = rig
            .deferred
            .run(&call(GATE, json!({ "groups": ["browser"] })), &ctx())
            .await
            .expect("not cancelled");
        assert!(again.output.contains("Already loaded: browser."), "{}", again.output);
    }

    #[tokio::test]
    async fn naming_no_group_is_a_failure_the_model_can_fix() {
        let rig = rig();
        let result = rig
            .deferred
            .run(&call(GATE, json!({ "groups": [] })), &ctx())
            .await
            .expect("not cancelled");
        assert!(!result.ok);
        assert_eq!(result.output, "Name at least one group to load.");
    }

    #[tokio::test]
    async fn a_guessed_tool_name_is_pointed_at_the_gate() {
        let rig = rig();
        let result = rig
            .deferred
            .run(&call("send_an_email", json!({})), &ctx())
            .await
            .expect("not cancelled");
        assert!(!result.ok);
        assert!(result.output.contains(GATE), "{}", result.output);
        assert!(result.output.contains("crew"), "{}", result.output);
    }

    #[test]
    fn the_preference_is_read_the_way_the_renderer_writes_it() {
        assert!(wanted(&json!({ "toolAccess": "on-demand" })));
        assert!(wanted(&json!({ "user": { "preferences": { "toolAccess": "on-demand" } } })));
        assert!(!wanted(&json!({ "toolAccess": "all" })));
        assert!(!wanted(&json!({})));
        assert!(!wanted(&Value::Null));
    }

    #[test]
    fn group_ids_come_from_the_source_and_its_origin() {
        let composio = ToolFact {
            id: "gmail_fetch_emails".into(),
            source: ToolSource::Composio,
            hint: Some("gmail".into()),
        };
        assert_eq!(
            group_for(&composio),
            Some(("app.gmail".into(), "Gmail, through Composio".into()))
        );
        let api = ToolFact {
            id: "petstore_list_pets".into(),
            source: ToolSource::OpenApi,
            hint: Some("Pet Store".into()),
        };
        assert_eq!(
            group_for(&api),
            Some(("api.pet-store".into(), "Pet store, an imported API".into()))
        );
        let unclassified = ToolFact {
            id: "something_new".into(),
            source: ToolSource::Builtin,
            hint: None,
        };
        assert_eq!(group_for(&unclassified), None);
        let core = ToolFact {
            id: "read".into(),
            source: ToolSource::Builtin,
            hint: None,
        };
        assert_eq!(group_for(&core), None);
    }
}

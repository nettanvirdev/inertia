//! Filling in the prompt context from the running app.
//!
//! The assembly itself lives in `inertia-agent`, where it can be tested
//! without a window. This is only the part that has to look at the actual
//! machine: what today's date is, which folder we are in, whether it is a git
//! repository.

use inertia_agent::prompt::{self, Context};

use crate::state::Workspace;

/// A shallow listing of the working folder.
///
/// Shallow on purpose: the point is to tell the model what kind of project
/// this is so it does not have to spend a turn discovering it. A recursive
/// listing of a real repository would be thousands of lines and would displace
/// the conversation.
/// Whether this turn holds any of these tools.
///
/// Any rather than all: a family is described by the paragraph as a whole, and
/// an agent holding half of one still needs to be told what it is for.
fn holds(turn: &Turn, wanted: &[&str]) -> bool {
    turn.tools
        .iter()
        .any(|held| wanted.contains(&held.as_str()))
}

/// The machine an agent was given, as its `computers/` record.
fn computer_of(workspace: &Workspace, agent_id: Option<&str>) -> Option<serde_json::Value> {
    let record = inertia_store::collections::get(
        &workspace.layout,
        inertia_store::Collection::Agents,
        agent_id?,
    )
    .ok()
    .flatten()?;
    let machine = record
        .get("computerId")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    inertia_store::collections::get(
        &workspace.layout,
        inertia_store::Collection::Computers,
        machine,
    )
    .ok()
    .flatten()
}

/// The apps the person connected through Composio.
fn apps_of(workspace: &Workspace) -> Vec<prompt::App> {
    inertia_store::collections::list(&workspace.layout, inertia_store::Collection::Composio)
        .iter()
        .filter_map(prompt::App::from_record)
        .collect()
}

fn is_git_repository(cwd: &std::path::Path) -> bool {
    cwd.join(".git").exists()
}

/// Today, in the format the prompt asks for.
fn today() -> String {
    jiff::Zoned::now().strftime("%a %b %d %Y").to_string()
}

/// What this particular turn is, as opposed to what the workspace is.
///
/// Every field here was hardcoded to its default until now, and the effect was
/// the one people noticed first: an agent with a name, a role and pages of
/// standing instructions on disk introduced itself as having no name set,
/// because none of it was ever sent. The conversation mode and the approval
/// posture were the same story - the composer's two pills changed the label
/// above the message box and nothing else.
#[derive(Debug, Clone, Default)]
pub struct Turn {
    /// Which agent is answering. `None` in a conversation with no agent set.
    pub agent_id: Option<String>,
    /// The composer's mode pill: `chat`, `plan` or `autonomous`.
    pub mode: Option<String>,
    /// The composer's approval pill: `ask`, `edits` or `auto`.
    pub approval: Option<String>,
    /// The model reference, as the person chose it.
    pub model: String,
    pub is_subagent: bool,
    /// The worktree the conversation has entered, as the window remembers it.
    pub worktree: Option<serde_json::Value>,
    /// Where the previous turn in this conversation ran, when it was somewhere
    /// else. Said out loud so a model does not read the difference between its
    /// own earlier answer and this prompt as evidence that it hallucinates.
    pub previous_cwd: Option<String>,
    /// Every tool id this turn actually holds, as the registry answered.
    ///
    /// The prompt describes capabilities - delegating, driving a machine,
    /// changing this app's own setup - and each of those paragraphs is a lie
    /// when the tools behind it were withheld. Passed rather than guessed
    /// because the registry is the only thing that knows: the mode withholds
    /// some, the permission rules deny others, and an agent without a computer
    /// never had that family at all.
    pub tools: Vec<String>,
    /// The room, in a group conversation. Assembled by whoever seated it -
    /// `turn` - because deciding who speaks and describing who is there are the
    /// same piece of knowledge, and two places working it out separately is two
    /// places that can disagree about who is in the conversation.
    pub room: Option<prompt::Room>,
}

/// Builds the system prompt for a turn in this workspace.
///
/// `cwd` is the folder the turn actually runs in, not the workspace's own
/// scratch folder: the listing, the git check and the project instructions all
/// describe where the work is happening, and describing somewhere else is worse
/// than describing nothing.
///
/// `memories` is appended rather than woven in. It sits at the end of the
/// cacheable prefix because it is the part most likely to differ between two
/// turns, and anything placed before it would lose its cache entry every time a
/// memory was written.
pub fn build(workspace: &Workspace, cwd: &std::path::Path, memories: &str, turn: &Turn) -> String {
    let assembled = build_for(workspace, cwd, turn);
    if memories.is_empty() {
        return assembled;
    }
    format!("{assembled}\n\n{memories}")
}

/// The agent's own record, as the prompt needs it.
///
/// A missing agent is not an error: a conversation can be pointed at nobody in
/// particular, and the prompt simply opens on the core instructions instead.
fn identity_of(workspace: &Workspace, agent_id: Option<&str>) -> Option<prompt::AgentIdentity> {
    let id = agent_id?;
    let record =
        inertia_store::collections::get(&workspace.layout, inertia_store::Collection::Agents, id)
            .ok()
            .flatten()?;
    prompt::AgentIdentity::from_record(&record)
}

/// The person, from `settings/identity.json`.
fn person_of(workspace: &Workspace) -> Option<String> {
    let identity = inertia_store::collections::read_document(
        &workspace.layout,
        inertia_store::layout::Document::Identity,
        serde_json::json!({}),
    );
    prompt::describe_person(identity.get("user")?)
}

fn build_for(workspace: &Workspace, cwd: &std::path::Path, turn: &Turn) -> String {
    let cwd = cwd.to_path_buf();

    prompt::build(&Context {
        agent: identity_of(workspace, turn.agent_id.as_deref()),
        // Unset means the default in both cases, which is what `parse` answers
        // for anything it does not recognise - so an older thread saved before
        // these were written still gets the behaviour it had.
        mode: turn
            .mode
            .as_deref()
            .map(prompt::Mode::parse)
            .unwrap_or_default(),
        approval: turn
            .approval
            .as_deref()
            .map(prompt::Approval::parse)
            .unwrap_or_default(),
        cwd: cwd.display().to_string(),
        workspace: workspace.layout.root().display().to_string(),
        platform: std::env::consts::OS.to_string(),
        // Named because a model that knows it is writing for PowerShell writes
        // a different command than one assuming bash, and getting that wrong
        // costs a round trip every time.
        shell: Some(inertia_tools::builtin::shell_name().to_string()),
        model: turn.model.clone(),
        previous_cwd: turn.previous_cwd.clone(),
        worktree: turn
            .worktree
            .as_ref()
            .and_then(prompt::Worktree::from_record),
        is_git_repository: is_git_repository(&cwd),
        today: today(),
        layout: prompt::read_layout(&cwd),
        instructions: prompt::read_project_instructions(&cwd),
        person: person_of(workspace),
        is_subagent: turn.is_subagent,
        // The machine this agent was given, by its own record. Read here rather
        // than passed in because the agent is looked up here anyway, and a
        // machine described to an agent that does not have one is a paragraph
        // of instructions it can only fail to follow.
        computer: computer_of(workspace, turn.agent_id.as_deref()),
        apps: apps_of(workspace),
        skills: crate::skills::advertised(&workspace.layout)
            .into_iter()
            .map(|skill| prompt::Skill {
                id: skill.id,
                name: skill.name,
                description: skill.description,
                missing: false,
            })
            .collect(),
        // Said only when the tools behind it are actually in this turn's hands.
        can_delegate: holds(turn, &["task", "spawn"]),
        can_browse: holds(turn, &["browser_navigate"]),
        can_read_terminal: holds(turn, &["terminal_read"]),
        can_set_up: holds(turn, &["inertia_save", "inertia_list"]),
        room: turn.room.clone(),
    })
}

//! The shape of a workspace folder.
//!
//! Every directory name here is load-bearing. The folder is meant to be opened
//! in a file manager and kept in git, and the app resolves records by path -
//! so these are addresses, not implementation detail. The README the app
//! writes into a new workspace says as much: *do not rename the directories.*
//!
//! Adding a resource type should be an entry here, not new code elsewhere.

use std::path::{Path, PathBuf};

/// Marks a folder as a workspace. Its presence is what distinguishes "the user
/// pointed us at their workspace" from "the user pointed us at their home
/// directory by mistake".
pub const MANIFEST: &str = "inertia.json";

/// Written once, for whoever opens the folder without the app.
pub const README: &str = "README.md";

/// One directory the workspace owns, with the line the settings pane renders
/// beside it.
///
/// The description is data rather than something the UI holds because the
/// folder is the product: whoever opens it in a file manager deserves the same
/// explanation the app gives, and two copies of that sentence would drift.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Directory {
    pub path: &'static str,
    pub label: &'static str,
    pub description: &'static str,
}

/// Every directory a workspace contains, in the order the settings pane shows
/// them.
///
/// Listed explicitly rather than created on demand so the pane can show what is
/// in the folder, and so a fresh workspace looks complete rather than growing
/// directories as features are first used.
pub const DIRECTORY_INFO: &[Directory] = &[
    Directory { path: "settings", label: "Settings", description: "App preferences, appearance and model providers." },
    Directory { path: "agents", label: "Agents", description: "Each agent's persona, model and tool grants." },
    Directory { path: "skills", label: "Skills", description: "One folder per skill, each with a SKILL.md." },
    Directory { path: "plugins", label: "Plugins", description: "Everything that adds tools to an agent." },
    Directory { path: "plugins/mcp", label: "MCP servers", description: "Model Context Protocol server definitions." },
    Directory { path: "plugins/openapi", label: "OpenAPI", description: "Imported specs and their auth." },
    Directory { path: "plugins/openapi/specs", label: "OpenAPI specs", description: "The raw spec documents, kept beside their entry." },
    Directory { path: "plugins/composio", label: "Composio", description: "Connected Composio apps and their toolkits." },
    Directory { path: "conversations", label: "Conversations", description: "Threads and their full message history." },
    Directory { path: "conversations/threads", label: "Threads", description: "One file per thread." },
    Directory { path: "conversations/messages", label: "Messages", description: "One file per thread's transcript." },
    Directory { path: "history", label: "History", description: "What ran, when, and what came back." },
    Directory { path: "history/executions", label: "Executions", description: "Tool and routine runs, with inputs and outputs." },
    Directory { path: "history/sessions", label: "Sessions", description: "Each agent turn as it ran, with its tools and its token count." },
    Directory { path: "history/activity", label: "Activity", description: "The workspace event log." },
    Directory { path: "memory", label: "Memory", description: "Long-lived facts agents are allowed to recall." },
    Directory { path: "hooks", label: "Hooks", description: "Programs that run before and after an agent acts, and when it wants to stop." },
    Directory { path: "routines", label: "Routines", description: "Scheduled and triggered playbooks." },
    Directory { path: "computers", label: "Computers", description: "Sandboxes and machines agents can drive." },
    Directory { path: "files", label: "Files", description: "Attachments and anything an agent produced." },
    Directory { path: "secrets", label: "Secrets", description: "API keys and tokens. Stored in the clear for now." },
    Directory { path: "cache", label: "Cache", description: "Copies of things fetched from the internet, so a restart does not fetch them again. Safe to delete." },
    Directory { path: "logs", label: "Logs", description: "Diagnostics. Safe to delete." },
    Directory { path: "backups", label: "Backups", description: "Snapshots you or the app take of this folder." },
];

/// Just the paths, for the scaffold and for the tests that pin them.
pub const DIRECTORIES: &[&str] = &[
    "settings",
    "agents",
    "skills",
    "plugins",
    "plugins/mcp",
    "plugins/openapi",
    "plugins/openapi/specs",
    "plugins/composio",
    "conversations",
    "conversations/threads",
    "conversations/messages",
    "history",
    "history/executions",
    "history/sessions",
    "history/activity",
    "memory",
    "hooks",
    "routines",
    "computers",
    "files",
    "secrets",
    "cache",
    "logs",
    "backups",
];

/// The scratch folder an agent works in when no project has been chosen.
///
/// Created by the scaffold but deliberately absent from `DIRECTORY_INFO`: it is
/// an implementation detail of "no project selected", not a part of the folder
/// the settings pane invites a person to go and look at.
pub const WORK_DIR: &str = "files/work";

/// Directories whose loss costs performance or history, never data. Nothing
/// reads them as the source of truth for a record.
pub const DISPOSABLE: &[&str] = &["cache", "logs", "backups"];

/// A settings document, each its own file so a hand edit or a git conflict is
/// scoped to one concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Document {
    App,
    Appearance,
    Models,
    Permissions,
    Identity,
    Group,
    Projects,
    Voice,
    Computers,
    Secrets,
    Hooks,
    /// Composio's catalogue of apps: hundreds of rows and several seconds of
    /// paged requests for an answer that does not change until Composio ships
    /// something. Kept as a file so opening the app shows the list rather than
    /// fetching it.
    CacheComposio,
}

impl Document {
    /// Path relative to the workspace root.
    pub fn path(&self) -> &'static str {
        match self {
            Self::App => "settings/app.json",
            Self::Appearance => "settings/appearance.json",
            Self::Models => "settings/models.json",
            Self::Permissions => "settings/permissions.json",
            Self::Identity => "settings/identity.json",
            Self::Group => "settings/group.json",
            Self::Projects => "settings/projects.json",
            Self::Voice => "settings/voice.json",
            Self::Computers => "settings/computers.json",
            Self::Secrets => "secrets/secrets.json",
            Self::Hooks => "hooks/hooks.json",
            Self::CacheComposio => "cache/composio-catalogue.json",
        }
    }

    /// The name the renderer asks for. The renderer names documents; it never
    /// names paths, which is what keeps this file the only one that knows the
    /// folder's shape.
    pub fn key(&self) -> &'static str {
        match self {
            Self::App => "settings.app",
            Self::Appearance => "settings.appearance",
            Self::Models => "settings.models",
            Self::Permissions => "settings.permissions",
            Self::Identity => "settings.identity",
            Self::Group => "settings.group",
            Self::Projects => "settings.projects",
            Self::Voice => "settings.voice",
            Self::Computers => "settings.computers",
            Self::Secrets => "secrets",
            Self::Hooks => "hooks",
            Self::CacheComposio => "cache.composio",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|d| d.key() == key)
    }

    pub const ALL: &'static [Document] = &[
        Self::App,
        Self::Appearance,
        Self::Models,
        Self::Permissions,
        Self::Identity,
        Self::Group,
        Self::Projects,
        Self::Voice,
        Self::Computers,
        Self::Secrets,
        Self::Hooks,
        Self::CacheComposio,
    ];
}

/// A collection of records, one file (or folder) per record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collection {
    Agents,
    Threads,
    Messages,
    Memories,
    Routines,
    Computers,
    Mcp,
    OpenApi,
    Composio,
    Turns,
    Executions,
    Activity,
    /// The one collection whose record is a folder rather than a file.
    Skills,
}

impl Collection {
    pub fn dir(&self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Threads => "conversations/threads",
            Self::Messages => "conversations/messages",
            Self::Memories => "memory",
            Self::Routines => "routines",
            Self::Computers => "computers",
            Self::Mcp => "plugins/mcp",
            Self::OpenApi => "plugins/openapi",
            Self::Composio => "plugins/composio",
            Self::Turns => "history/sessions",
            Self::Executions => "history/executions",
            Self::Activity => "history/activity",
            Self::Skills => "skills",
        }
    }

    /// The name the renderer asks for.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Threads => "threads",
            Self::Messages => "messages",
            Self::Memories => "memory",
            Self::Routines => "routines",
            Self::Computers => "computers",
            Self::Mcp => "plugins.mcp",
            Self::OpenApi => "plugins.openapi",
            Self::Composio => "plugins.composio",
            Self::Turns => "sessions",
            Self::Executions => "executions",
            Self::Activity => "activity",
            Self::Skills => "skills",
        }
    }

    /// The prefix a generated id is built from, when a record arrives without
    /// one and has no name to slugify.
    pub fn singular(&self) -> &'static str {
        match self {
            Self::Agents => "agent",
            Self::Threads => "thread",
            Self::Messages => "messages",
            Self::Memories => "memory",
            Self::Routines => "routine",
            Self::Computers => "computer",
            Self::Mcp => "mcp",
            Self::OpenApi => "openapi",
            Self::Composio => "composio",
            Self::Turns => "turn",
            Self::Executions => "run",
            Self::Activity => "event",
            Self::Skills => "skill",
        }
    }

    pub const ALL: &'static [Collection] = &[
        Self::Agents,
        Self::Threads,
        Self::Messages,
        Self::Memories,
        Self::Routines,
        Self::Computers,
        Self::Mcp,
        Self::OpenApi,
        Self::Composio,
        Self::Turns,
        Self::Executions,
        Self::Activity,
        Self::Skills,
    ];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.key() == key)
    }

    /// Whether a record is a folder containing `SKILL.md` rather than a single
    /// JSON file. Anything else placed beside it is preserved untouched.
    pub fn is_folder_backed(&self) -> bool {
        matches!(self, Self::Skills)
    }
}

/// Paths within one workspace.
#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest(&self) -> PathBuf {
        self.root.join(MANIFEST)
    }

    pub fn document(&self, document: Document) -> PathBuf {
        self.root.join(document.path())
    }

    pub fn collection_dir(&self, collection: Collection) -> PathBuf {
        self.root.join(collection.dir())
    }

    /// Where one record lives.
    pub fn record(&self, collection: Collection, id: &str) -> PathBuf {
        let dir = self.collection_dir(collection);
        if collection.is_folder_backed() {
            dir.join(id).join("SKILL.md")
        } else {
            dir.join(format!("{id}.json"))
        }
    }

    /// The day-bucketed activity log for a date, as `YYYY-MM-DD`.
    pub fn activity(&self, date: &str) -> PathBuf {
        self.root.join("history/activity").join(format!("{date}.json"))
    }

    /// The default working directory when no project has been chosen.
    pub fn work_dir(&self) -> PathBuf {
        self.root.join(WORK_DIR)
    }

    /// Creates every directory a workspace is expected to have.
    pub fn scaffold(&self) -> crate::fsx::Result<()> {
        for dir in DIRECTORIES {
            crate::fsx::ensure_dir(&self.root.join(dir))?;
        }
        crate::fsx::ensure_dir(&self.work_dir())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new("/ws")
    }

    /// These strings are addresses. Changing one orphans every existing
    /// workspace, so they are pinned here deliberately.
    #[test]
    fn document_paths_are_stable() {
        assert_eq!(Document::App.path(), "settings/app.json");
        assert_eq!(Document::Models.path(), "settings/models.json");
        assert_eq!(Document::Permissions.path(), "settings/permissions.json");
        assert_eq!(Document::Secrets.path(), "secrets/secrets.json");
        assert_eq!(Document::Hooks.path(), "hooks/hooks.json");
    }

    #[test]
    fn collection_directories_are_stable() {
        assert_eq!(Collection::Threads.dir(), "conversations/threads");
        assert_eq!(Collection::Messages.dir(), "conversations/messages");
        assert_eq!(Collection::Turns.dir(), "history/sessions");
        assert_eq!(Collection::Mcp.dir(), "plugins/mcp");
    }

    #[test]
    fn a_record_is_one_json_file() {
        assert_eq!(
            layout().record(Collection::Threads, "thr_a1"),
            Path::new("/ws/conversations/threads/thr_a1.json")
        );
    }

    /// Skills are the exception: a folder, so a skill can carry scripts and
    /// reference files the app never touches.
    #[test]
    fn a_skill_is_a_folder_with_a_markdown_file() {
        assert!(Collection::Skills.is_folder_backed());
        assert_eq!(
            layout().record(Collection::Skills, "code-review"),
            Path::new("/ws/skills/code-review/SKILL.md")
        );
    }

    #[test]
    fn scaffolding_creates_every_declared_directory() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        layout.scaffold().unwrap();

        for declared in DIRECTORIES {
            assert!(
                dir.path().join(declared).is_dir(),
                "{declared} was not created"
            );
        }
    }

    #[test]
    fn scaffolding_twice_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path());
        layout.scaffold().unwrap();
        layout.scaffold().unwrap();
    }

    /// Every collection must live somewhere the scaffold actually creates, or
    /// the first write to it lands in a directory nothing else knows about.
    #[test]
    fn every_collection_directory_is_scaffolded() {
        let collections = [
            Collection::Agents,
            Collection::Threads,
            Collection::Messages,
            Collection::Memories,
            Collection::Routines,
            Collection::Computers,
            Collection::Mcp,
            Collection::OpenApi,
            Collection::Composio,
            Collection::Turns,
            Collection::Executions,
            Collection::Skills,
        ];
        for collection in collections {
            assert!(
                DIRECTORIES.contains(&collection.dir()),
                "{} is not in DIRECTORIES",
                collection.dir()
            );
        }
    }

    /// Likewise every settings document needs its parent directory to exist.
    #[test]
    fn every_document_has_a_scaffolded_parent() {
        for document in Document::ALL {
            let path = document.path();
            let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            assert!(
                DIRECTORIES.contains(&parent),
                "{path} needs {parent} in DIRECTORIES"
            );
        }
    }
}

//! Remembering, recalling and forgetting, from inside a turn.
//!
//! The prompt already carries the titles of what this agent knows, which is
//! enough to decide a memory is relevant and never enough to act on. These three
//! tools are the other half of that bargain: read one in full, write a new one,
//! throw one away.
//!
//! `memory_recall` also exists for the case the injected list cannot cover - a
//! fact the agent suspects it was told but cannot see listed, because the
//! injected block is budgeted and a workspace can hold far more than fits. A
//! search that finds nothing is a real answer and is reported as one; the
//! alternative, returning the three least-unrelated memories with a confident
//! score, is how a memory system starts lying.
//!
//! Saving is not a decision to make lightly and the tool says so in its own
//! description. A store that accumulates every passing remark retrieves badly
//! and is worse than no store at all.

use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::Result;
use serde_json::{json, Value};

use crate::record;
use crate::store::Store;

/// The three of them, for a registry.
pub fn all(store: Store) -> Vec<Arc<dyn Tool>> {
    let store = Arc::new(store);
    vec![
        Arc::new(RecallTool { store: store.clone() }),
        Arc::new(SaveTool { store: store.clone() }),
        Arc::new(ForgetTool { store }),
    ]
}

/// One memory, rendered for a model rather than for a screen.
fn render(memory: &Value) -> String {
    let mut lines = vec![format!("## {}", record::text(memory, "title"))];
    let kind = record::text(memory, "kind");
    if !kind.is_empty() && kind != record::DEFAULT_KIND {
        lines.push(format!("Kind: {kind}"));
    }
    let tags = record::tags(memory);
    if !tags.is_empty() {
        lines.push(format!("Tags: {}", tags.join(", ")));
    }
    if record::is_pinned(memory) {
        lines.push("Pinned by the user.".into());
    }
    lines.push(String::new());
    lines.push(record::text(memory, "body"));
    lines.join("\n")
}

/* -- recall --------------------------------------------------------------- */

#[derive(Debug)]
struct RecallTool {
    store: Arc<Store>,
}

#[async_trait]
impl Tool for RecallTool {
    fn id(&self) -> &str {
        "memory_recall"
    }

    fn description(&self) -> &str {
        "Read what you already know. The titles of your memories are in your system \
         prompt; this returns the full text of the ones that match a query, and finds \
         memories that were not listed there - the list is budgeted and a workspace can \
         hold far more than fits.\n\n\
         Use it when a title in your memories looks relevant and you need what it \
         actually says, and when you suspect you were told something about this person, \
         project or preference that you cannot see listed. Do not use it for things you \
         were told earlier in this conversation - those are already in front of you.\n\n\
         Finding nothing is a real answer and is reported as one. It means you were not \
         told, not that you should guess."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What you are looking for, in words that would appear in the memory."
                },
                "limit": {
                    "type": "integer",
                    "description": "How many to return. Defaults to 8."
                }
            }
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    /// Reading what the user has already agreed the agent knows is not an act
    /// that needs approving. Writing one is.
    fn permission(&self, _args: &Value) -> PermissionRequest {
        PermissionRequest::new("memory", "recall").with_always("memory")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let query = args.get("query").and_then(Value::as_str)?;
        Some(format!("Recalling \"{query}\""))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let query = args.get("query").and_then(Value::as_str).unwrap_or_default();
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(8)
            .clamp(1, 50) as usize;

        let found = crate::recall(&self.store, query, limit);
        if found.is_empty() {
            return Ok(ToolOutcome::text(format!(
                "Nothing remembered about \"{query}\". You were not told this."
            ))
            .with_title("Nothing found"));
        }

        let body = found.iter().map(render).collect::<Vec<_>>().join("\n\n");
        Ok(ToolOutcome::text(body).with_title(format!(
            "{} {}",
            found.len(),
            if found.len() == 1 { "memory" } else { "memories" }
        )))
    }
}

/* -- save ----------------------------------------------------------------- */

#[derive(Debug)]
struct SaveTool {
    store: Arc<Store>,
}

#[async_trait]
impl Tool for SaveTool {
    fn id(&self) -> &str {
        "memory_save"
    }

    fn description(&self) -> &str {
        "Write down one durable fact, so a conversation weeks from now starts out \
         knowing it.\n\n\
         Save sparingly. A store that accumulates every passing remark retrieves badly \
         and is worse than no store at all, and the user has to prune it by hand. Worth \
         saving: a decision and its reason, a convention this project follows, a \
         correction the person made to you, how they want to be worked with. Not worth \
         saving: anything readable from the code or the git history, what is currently \
         failing, the shape of an API you called today, or anything you are unsure was \
         ever agreed.\n\n\
         One fact per memory - a memory holding three things cannot be corrected when \
         one of them changes. Never write down a key, token or password; say what a \
         secret is for and where it lives, never its value."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Something you could pick out of a list of a hundred."
                },
                "body": {
                    "type": "string",
                    "description": "The fact, written so it still makes sense with none of this conversation around it."
                },
                "scope": {
                    "type": "string",
                    "enum": ["project", "global"],
                    "description": "`project` when it is about this codebase, `global` when it is about the person and would still be true in an unrelated project. When in doubt use `project`."
                },
                "kind": {
                    "type": "string",
                    "enum": ["fact", "preference", "contact", "project", "credential-note"]
                },
                "tags": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["title", "body"]
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let title = args.get("title").and_then(Value::as_str).unwrap_or("a memory");
        PermissionRequest::new("memory", format!("remember {title}")).with_always("memory")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let title = args.get("title").and_then(Value::as_str)?;
        Some(format!("Remembering \"{title}\""))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let scope = match args.get("scope").and_then(Value::as_str) {
            Some(given) if record::is_scope(given) => given,
            _ => "project",
        };

        let mut input = json!({
            "title": record::text(&args, "title"),
            "body": record::text(&args, "body"),
            "kind": match args.get("kind").and_then(Value::as_str) {
                Some(given) if record::is_kind(given) => given,
                _ => record::DEFAULT_KIND,
            },
            "tags": record::tags(&args),
            "scope": scope,
            "source": "agent",
        });
        if scope == "project" {
            if let Some(folder) = self.store.project() {
                input["folder"] = Value::String(folder.to_string_lossy().to_string());
            }
        }

        // A failure the model can act on - a title it left empty, a key it tried
        // to write down - is a successful call with unhappy output, not an error
        // that ends the turn.
        match self.store.remember(&input, true) {
            Ok(saved) => Ok(ToolOutcome::text(format!(
                "Remembered: {}",
                record::text(&saved, "title")
            ))
            .with_title(record::text(&saved, "title"))),
            Err(why) => Ok(ToolOutcome::text(format!("Not remembered. {why}"))),
        }
    }
}

/* -- forget --------------------------------------------------------------- */

#[derive(Debug)]
struct ForgetTool {
    store: Arc<Store>,
}

#[async_trait]
impl Tool for ForgetTool {
    fn id(&self) -> &str {
        "memory_forget"
    }

    fn description(&self) -> &str {
        "Throw away a memory that is wrong or no longer true. Takes the id shown in \
         square brackets beside each memory in your system prompt. If the fact merely \
         changed, write the new one with `memory_save` instead - a corrected memory is \
         more use than a missing one."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The id of the memory to forget." }
            },
            "required": ["id"]
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
        // Deliberately not generalised to "memory": forgetting is the one
        // destructive act here, and an "always allow" on it would hand over
        // every memory the person has on the strength of one click.
        PermissionRequest::new("memory", format!("forget {id}"))
    }

    fn render(&self, args: &Value) -> Option<String> {
        let id = args.get("id").and_then(Value::as_str)?;
        Some(format!("Forgetting {id}"))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let id = record::text(&args, "id");
        match self.store.forget(&id) {
            Ok(()) => Ok(ToolOutcome::text(format!("Forgotten: {id}"))),
            Err(why) => Ok(ToolOutcome::text(format!("Not forgotten. {why}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::tool::{Decision, PermissionGate};
    use inertia_core::{Action, SessionId, ToolCallId};
    use inertia_store::Layout;

    /// The registry has already decided by the time `execute` runs, so these
    /// tests only need something to put in the context.
    #[derive(Debug)]
    struct Allows;

    #[async_trait]
    impl PermissionGate for Allows {
        async fn ask(&self, _request: &PermissionRequest) -> Result<Decision> {
            Ok(Decision::Allow)
        }
        async fn verdict(&self, _key: &str, _target: &str) -> Action {
            Action::Allow
        }
    }

    fn setup() -> (tempfile::TempDir, Store, Vec<Arc<dyn Tool>>) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).expect("a project dir");
        let store = Store::new(Layout::new(dir.path().join("workspace")), Some(project));
        let tools = all(store.clone());
        (dir, store, tools)
    }

    fn ctx(dir: &tempfile::TempDir) -> ToolContext {
        ToolContext {
            root: dir.path().to_path_buf(),
            session: SessionId::from_existing("ses_1"),
            call_id: ToolCallId::from("call_1"),
            permissions: Arc::new(Allows),
        }
    }

    fn tool<'a>(tools: &'a [Arc<dyn Tool>], id: &str) -> &'a Arc<dyn Tool> {
        tools.iter().find(|t| t.id() == id).expect("the tool")
    }

    #[tokio::test]
    async fn finding_nothing_says_so_rather_than_guessing() {
        let (dir, _store, tools) = setup();
        let out = tool(&tools, "memory_recall")
            .execute(json!({ "query": "kubernetes" }), &ctx(&dir))
            .await
            .expect("the call ran");
        assert!(out.output.contains("You were not told this."));
    }

    #[tokio::test]
    async fn a_saved_memory_is_recallable_in_full() {
        let (dir, _store, tools) = setup();
        tool(&tools, "memory_save")
            .execute(
                json!({
                    "title": "Deploys go to fly.io",
                    "body": "Never render. The account is on the team plan.",
                    "scope": "project",
                    "tags": ["deploy"],
                }),
                &ctx(&dir),
            )
            .await
            .expect("the call ran");

        let out = tool(&tools, "memory_recall")
            .execute(json!({ "query": "where do deploys go" }), &ctx(&dir))
            .await
            .expect("the call ran");
        assert!(out.output.contains("## Deploys go to fly.io"));
        assert!(out.output.contains("Never render."));
        assert!(out.output.contains("Tags: deploy"));
    }

    /// The model can act on this, so it is unhappy output rather than an error
    /// that would end the turn.
    #[tokio::test]
    async fn a_memory_carrying_a_credential_is_refused_in_words() {
        let (dir, store, tools) = setup();
        let out = tool(&tools, "memory_save")
            .execute(
                json!({ "title": "The key", "body": "ghp_0123456789abcdefghij" }),
                &ctx(&dir),
            )
            .await
            .expect("the call ran");
        assert!(out.output.starts_with("Not remembered."));
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn forgetting_removes_it() {
        let (dir, store, tools) = setup();
        tool(&tools, "memory_save")
            .execute(json!({ "title": "One", "body": "fact" }), &ctx(&dir))
            .await
            .expect("the call ran");
        let id = record::text(&store.list()[0], "id");

        tool(&tools, "memory_forget")
            .execute(json!({ "id": id }), &ctx(&dir))
            .await
            .expect("the call ran");
        assert!(store.list().is_empty());
    }

    /// An "always allow" on forgetting would hand over every memory the person
    /// has on the strength of one click, so it does not generalise.
    #[test]
    fn only_the_safe_half_of_memory_can_be_approved_forever() {
        let (_dir, _store, tools) = setup();
        assert_eq!(
            tool(&tools, "memory_recall").permission(&json!({})).always.as_deref(),
            Some("memory")
        );
        assert_eq!(
            tool(&tools, "memory_forget").permission(&json!({ "id": "a" })).always,
            None
        );
    }
}

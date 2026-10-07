//! `ls`, `todowrite` and `present_plan`.
//!
//! Three small tools that have nothing to do with each other except the thing
//! that makes them worth writing: each is something the window already knows how
//! to draw and the backend never sent.
//!
//! - `ls` - looking around. A model that has just arrived in a project needs the
//!   shape of it, not its contents, and the fastest way to waste a context
//!   window is to hand it a recursive listing of `node_modules`.
//! - `todowrite` - the task list. A model working through a five-step job
//!   forgets step four, not because it is bad at the job but because by then the
//!   beginning of the conversation is a long way behind it. Rewriting the list
//!   after each step keeps the whole plan in the most recent thing it read - and
//!   the person gets to watch an agent work rather than watch it emit calls.
//! - `present_plan` - how a planning turn ends. Plan mode has no tools that
//!   change anything, so a planning turn ends with a plan or with a wasted turn.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::fence;

/// Generated, vendored or built. Never what anyone meant by "what is in here".
const SKIP: &[&str] = &[
    "node_modules",
    ".git",
    "dist",
    "build",
    ".next",
    "target",
    "__pycache__",
    ".venv",
];

/// Deeper than this and it stops being an overview.
const MAX_DEPTH: usize = 3;

/// Enough to see a project's shape, short of pasting a filesystem into a
/// message.
const MAX_ENTRIES: usize = 500;

fn resolve(root: &Path, supplied: Option<&str>) -> PathBuf {
    match supplied.map(str::trim).filter(|p| !p.is_empty()) {
        None => root.to_path_buf(),
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(path) => root.join(path),
    }
}

// ── ls ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct LsTool;

/// One level of the walk, appended to `out`.
///
/// The skips are reported rather than silent: a listing that quietly omitted a
/// folder is a listing that will send the model looking for a file it was told
/// does not exist.
fn walk(dir: &Path, prefix: &str, depth: usize, all: bool, out: &mut Vec<String>) -> bool {
    let Ok(reader) = std::fs::read_dir(dir) else {
        return false;
    };

    let mut entries: Vec<(String, bool)> = reader
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !all && name.starts_with('.') {
                return None;
            }
            let is_dir = entry.file_type().ok()?.is_dir();
            Some((name, is_dir))
        })
        .collect();

    // Directories first, then alphabetical: the shape of a project reads off a
    // listing grouped that way and does not off a raw readdir order.
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));

    for (name, is_dir) in entries {
        if out.len() >= MAX_ENTRIES {
            return true;
        }
        if is_dir && SKIP.contains(&name.as_str()) {
            out.push(format!("{prefix}{name}/  (skipped)"));
            continue;
        }
        out.push(format!("{prefix}{name}{}", if is_dir { "/" } else { "" }));
        if is_dir && depth > 1 {
            let nested = format!("{prefix}  ");
            if walk(&dir.join(&name), &nested, depth - 1, all, out) {
                return true;
            }
        }
    }
    false
}

#[async_trait]
impl Tool for LsTool {
    fn id(&self) -> &str {
        "ls"
    }

    fn description(&self) -> &str {
        "List the contents of a directory as a tree. Skips node_modules, .git and other \
         generated folders. Use depth for a couple of levels at once."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The path to list. Defaults to the working directory." },
                "depth": { "type": "integer", "description": "How many levels to descend. 1 to 3, default 1." },
                "all": { "type": "boolean", "description": "Include dotfiles." }
            },
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .to_string();
        PermissionRequest::new("read", target).with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(args.get("path").and_then(Value::as_str).unwrap_or(".").to_string())
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let dir = resolve(&ctx.root, args.get("path").and_then(Value::as_str));
        fence::check(ctx, &dir).await?;
        let depth = args
            .get("depth")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .clamp(1, MAX_DEPTH as u64) as usize;
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);

        if !dir.is_dir() {
            return Err(Error::Other(format!(
                "{} is not a directory.",
                dir.display()
            )));
        }

        let mut out = Vec::new();
        let truncated = walk(&dir, "", depth, all, &mut out);

        if out.is_empty() {
            return Ok(ToolOutcome::text(format!("{} is empty.", dir.display()))
                .with_title(dir.display().to_string()));
        }

        let mut body = format!("{}\n{}", dir.display(), out.join("\n"));
        if truncated {
            body.push_str(&format!(
                "\n\n… stopped at {MAX_ENTRIES} entries. List a subdirectory for the rest."
            ));
        }

        Ok(ToolOutcome {
            title: Some(dir.display().to_string()),
            output: body,
            metadata: Some(json!({ "entries": out.len(), "truncated": truncated })),
            images: Vec::new(),
        })
    }
}

// ── todowrite ───────────────────────────────────────────────────────────

const STATUSES: &[&str] = &["pending", "in_progress", "completed", "cancelled"];

/// The lists, by conversation.
///
/// Not persisted: a task list is about this piece of work, not forever, and one
/// restored from disk three days later describes a job nobody is doing.
#[derive(Debug, Default)]
pub struct Lists(Mutex<HashMap<String, Value>>);

impl Lists {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, session: &str) -> Value {
        self.0.lock().get(session).cloned().unwrap_or(json!([]))
    }

    pub fn forget(&self, session: &str) {
        self.0.lock().remove(session);
    }
}

#[derive(Debug)]
pub struct TodoTool(Arc<Lists>);

#[async_trait]
impl Tool for TodoTool {
    fn id(&self) -> &str {
        "todowrite"
    }

    fn description(&self) -> &str {
        "Write or update the task list for the work you are doing now.\n\
         \n\
         Use it when a job has three or more real steps, when the user gave you several \
         things at once, or when you are partway through something long and need to keep \
         track. Do not use it for a single-step task; a one-item list is noise.\n\
         \n\
         Rules that make it useful rather than decorative:\n\
         - Send the WHOLE list every time. This replaces the previous one.\n\
         - Exactly one item may be in_progress at a time.\n\
         - Mark an item completed as soon as it is done, not in a batch at the end.\n\
         - If something turns out not to be needed, mark it cancelled and say why in the\n\
         \x20 content rather than deleting it, so the user can see what you decided.\n\
         \n\
         The list is visible to the user as you write it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The complete task list, replacing whatever was there before.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "A short stable id for this item." },
                            "content": { "type": "string", "description": "What needs doing, in a few words." },
                            "status": { "type": "string", "enum": STATUSES, "description": "pending, in_progress, completed or cancelled." }
                        },
                        "required": ["id", "content", "status"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["todos"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        PermissionRequest::new("todowrite", "list").with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let open = args
            .get("todos")
            .and_then(Value::as_array)
            .map(|todos| {
                todos
                    .iter()
                    .filter(|todo| todo.get("status").and_then(Value::as_str) != Some("completed"))
                    .count()
            })
            .unwrap_or(0);
        Some(if open == 1 {
            "1 task left".to_string()
        } else {
            format!("{open} tasks left")
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let todos = args
            .get("todos")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let status_is = |todo: &Value, want: &str| {
            todo.get("status").and_then(Value::as_str) == Some(want)
        };

        let running = todos.iter().filter(|t| status_is(t, "in_progress")).count();
        if running > 1 {
            return Err(Error::Other(format!(
                "Only one task may be in_progress at a time, and you marked {running}. \
                 Pick the one you are actually doing now and leave the rest pending."
            )));
        }

        let list = Value::Array(todos.clone());
        self.0
            .0
            .lock()
            .insert(ctx.session.to_string(), list.clone());

        let done = todos.iter().filter(|t| status_is(t, "completed")).count();
        // The list goes back as the result so the model reads its own plan
        // again on the next turn. That re-reading is most of the value.
        let body = todos
            .iter()
            .map(|todo| {
                let mark = match todo.get("status").and_then(Value::as_str) {
                    Some("completed") => "x",
                    Some("in_progress") => ">",
                    _ => " ",
                };
                format!(
                    "[{mark}] {}",
                    todo.get("content").and_then(Value::as_str).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolOutcome {
            title: Some(format!("{done}/{} done", todos.len())),
            output: body,
            metadata: Some(json!({ "todos": list })),
            images: Vec::new(),
        })
    }
}

// ── present_plan ────────────────────────────────────────────────────────

/// A plan with no steps is a paragraph, and forty is a shopping list.
const MIN_STEPS: usize = 1;
const MAX_STEPS: usize = 40;

/// Handing a plan to the person who has to approve it.
///
/// ## Why a tool rather than prose
///
/// A plan written as prose is indistinguishable, to the app, from any other
/// reply. There is nothing to attach an Approve button to, nothing to carry into
/// the build as the brief, and nothing to show the person as "this is the thing
/// you are agreeing to" - so approval becomes them typing "ok", and the plan
/// becomes whatever the model remembers of it.
///
/// ## It cannot approve itself
///
/// This records a plan and stops. It does not switch modes and has no way to.
/// The mode changes when the person presses the button, in the window, because
/// the entire point of the step is that a human decided.
#[derive(Debug, Default)]
pub struct PresentPlanTool;

#[async_trait]
impl Tool for PresentPlanTool {
    fn id(&self) -> &str {
        "present_plan"
    }

    fn description(&self) -> &str {
        "Show the user your plan and ask them to approve it. This is how a planning \
         turn ends.\n\
         \n\
         Call it once, when you have looked at enough of the project to write a plan you \
         would be willing to build from. The user sees the steps and chooses: build it, \
         which switches this conversation to Autonomous mode and starts the work, or \
         keep talking, which leaves you here to revise it.\n\
         \n\
         Write steps as work someone could start: \"Add cached-token prices to \
         ModelsField and fold them into costOf\" is a step. \"Pricing\" is a heading, and \
         a heading is not a plan. Order them so each one can begin when the one before \
         it is finished.\n\
         \n\
         Say what you are unsure about in `risks` rather than leaving it out. A plan \
         that hides the part you have not figured out gets approved and then fails at \
         exactly that point, which is worse than being asked a question now.\n\
         \n\
         Do not call this and then keep working - you have no tools to work with in \
         Plan mode, and the turn is over once the plan is on screen."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "A short name for the work, a few words." },
                "summary": { "type": "string", "description": "One or two sentences: what this does and why, in the user's terms." },
                "steps": {
                    "type": "array",
                    "description": "The steps, in the order they should happen.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "title": { "type": "string", "description": "The step, as a piece of work someone could start." },
                            "detail": { "type": "string", "description": "Optional: what it touches, or the thing worth knowing about it." }
                        },
                        "required": ["title"],
                        "additionalProperties": false
                    }
                },
                "risks": {
                    "type": "array",
                    "description": "Optional: what could go wrong or is still open.",
                    "items": { "type": "string" }
                }
            },
            "required": ["title", "summary", "steps"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        PermissionRequest::new("todowrite", "plan").with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let count = args
            .get("steps")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let title = args.get("title").and_then(Value::as_str).unwrap_or("Plan");
        Some(format!(
            "{title} - {count} step{}",
            if count == 1 { "" } else { "s" }
        ))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let text = |value: Option<&Value>| {
            value
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };

        let steps: Vec<Value> = args
            .get("steps")
            .and_then(Value::as_array)
            .map(|steps| {
                steps
                    .iter()
                    .filter_map(|step| {
                        let title = text(step.get("title"))?;
                        let mut out = json!({ "title": title });
                        if let Some(detail) = text(step.get("detail")) {
                            out["detail"] = json!(detail);
                        }
                        Some(out)
                    })
                    .collect()
            })
            .unwrap_or_default();

        if steps.len() < MIN_STEPS {
            return Err(Error::Other(
                "A plan needs at least one step with a title. Write what would actually be \
                 done, not a heading for it."
                    .into(),
            ));
        }
        if steps.len() > MAX_STEPS {
            return Err(Error::Other(format!(
                "That is {} steps. Group them into at most {MAX_STEPS}: a plan nobody can \
                 read is not an approval anyone can give.",
                steps.len()
            )));
        }

        let title = text(args.get("title")).unwrap_or_else(|| "Plan".into());
        let summary = text(args.get("summary")).unwrap_or_default();
        let risks: Vec<String> = args
            .get("risks")
            .and_then(Value::as_array)
            .map(|risks| risks.iter().filter_map(|r| text(Some(r))).collect())
            .unwrap_or_default();

        let plan = json!({
            "title": title,
            "summary": summary,
            "steps": steps,
            "risks": risks,
        });

        // Read back to the model as well, so a follow-up turn revising the plan
        // is revising THIS plan rather than its memory of one.
        let mut body = vec![format!("Plan presented to the user: {title}"), summary];
        for (i, step) in steps.iter().enumerate() {
            let detail = step
                .get("detail")
                .and_then(Value::as_str)
                .map(|d| format!(" - {d}"))
                .unwrap_or_default();
            body.push(format!(
                "{}. {}{detail}",
                i + 1,
                step.get("title").and_then(Value::as_str).unwrap_or_default()
            ));
        }
        if !risks.is_empty() {
            body.push(String::new());
            body.push("Open questions:".into());
            body.extend(risks.iter().map(|risk| format!("- {risk}")));
        }
        body.push(String::new());
        body.push(
            "They will either approve it, which starts the build in Autonomous mode, or \
             reply with changes. Stop here and wait."
                .into(),
        );

        Ok(ToolOutcome {
            title: Some(title),
            output: body.join("\n"),
            metadata: Some(json!({ "plan": plan, "awaitingApproval": true })),
            images: Vec::new(),
        })
    }
}

/// The three, with the task lists they share.
pub fn look_tools(lists: Arc<Lists>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(LsTool),
        Arc::new(TodoTool(lists)),
        Arc::new(PresentPlanTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::permission::Action;
    use inertia_core::tool::{Decision, PermissionGate};

    #[derive(Debug)]
    struct Allow;

    #[async_trait]
    impl PermissionGate for Allow {
        async fn ask(&self, _request: &PermissionRequest) -> Result<Decision> {
            Ok(Decision::Allow)
        }
        async fn verdict(&self, _key: &str, _target: &str) -> Action {
            Action::Allow
        }
    }

    fn ctx(root: &Path) -> ToolContext {
        ToolContext {
            root: root.to_path_buf(),
            session: SessionId::from_existing("t1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(Allow),
        }
    }

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir_all(dir.path().join("src/inner")).expect("dirs");
        std::fs::create_dir_all(dir.path().join("node_modules/left-pad")).expect("dirs");
        std::fs::write(dir.path().join("README.md"), "hi").expect("file");
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").expect("file");
        std::fs::write(dir.path().join("src/inner/deep.rs"), "").expect("file");
        std::fs::write(dir.path().join(".hidden"), "").expect("file");
        dir
    }

    #[tokio::test]
    async fn ls_lists_one_level_with_directories_first() {
        let dir = tree();
        let out = LsTool
            .execute(json!({}), &ctx(dir.path()))
            .await
            .expect("the tool ran");
        let lines: Vec<&str> = out.output.lines().skip(1).collect();
        assert_eq!(lines[0], "node_modules/  (skipped)");
        assert_eq!(lines[1], "src/");
        assert_eq!(lines[2], "README.md");
        // One level only, by default.
        assert!(!out.output.contains("main.rs"));
        // And dotfiles stay out of the way.
        assert!(!out.output.contains(".hidden"));
    }

    #[tokio::test]
    async fn a_skipped_folder_is_reported_rather_than_silently_dropped() {
        // A listing that quietly omitted a folder sends the model looking for a
        // file it was told does not exist.
        let dir = tree();
        let out = LsTool
            .execute(json!({ "depth": 3 }), &ctx(dir.path()))
            .await
            .expect("the tool ran");
        assert!(out.output.contains("node_modules/  (skipped)"));
        assert!(!out.output.contains("left-pad"));
        assert!(out.output.contains("deep.rs"));
    }

    #[tokio::test]
    async fn ls_depth_is_clamped_rather_than_trusted() {
        let dir = tree();
        let out = LsTool
            .execute(json!({ "depth": 99 }), &ctx(dir.path()))
            .await
            .expect("the tool ran");
        assert!(out.output.contains("deep.rs"));
    }

    #[tokio::test]
    async fn ls_says_so_when_the_path_is_not_a_directory() {
        let dir = tree();
        let err = LsTool
            .execute(json!({ "path": "README.md" }), &ctx(dir.path()))
            .await
            .expect_err("a file is not a directory");
        assert!(err.to_string().contains("not a directory"));
    }

    #[tokio::test]
    async fn a_task_list_comes_back_as_the_result_so_the_model_rereads_it() {
        let dir = tree();
        let lists = Arc::new(Lists::new());
        let tool = TodoTool(lists.clone());
        let out = tool
            .execute(
                json!({ "todos": [
                    { "id": "1", "content": "read the code", "status": "completed" },
                    { "id": "2", "content": "write the fix", "status": "in_progress" },
                    { "id": "3", "content": "run the tests", "status": "pending" }
                ] }),
                &ctx(dir.path()),
            )
            .await
            .expect("the tool ran");

        assert_eq!(out.title.as_deref(), Some("1/3 done"));
        assert_eq!(out.output, "[x] read the code\n[>] write the fix\n[ ] run the tests");
        assert_eq!(lists.get("t1").as_array().map(Vec::len), Some(3));
    }

    #[tokio::test]
    async fn two_tasks_in_progress_is_refused_with_the_count() {
        let dir = tree();
        let tool = TodoTool(Arc::new(Lists::new()));
        let err = tool
            .execute(
                json!({ "todos": [
                    { "id": "1", "content": "a", "status": "in_progress" },
                    { "id": "2", "content": "b", "status": "in_progress" }
                ] }),
                &ctx(dir.path()),
            )
            .await
            .expect_err("two at once");
        assert!(err.to_string().contains("you marked 2"), "{err}");
    }

    #[tokio::test]
    async fn the_card_renders_from_the_plan_in_the_metadata() {
        let dir = tree();
        let out = PresentPlanTool
            .execute(
                json!({
                    "title": "Add pricing",
                    "summary": "Show what a turn cost.",
                    "steps": [
                        { "title": "Add the fields", "detail": "ModelsField" },
                        { "title": "Fold them into costOf" }
                    ],
                    "risks": ["The cache columns may be wrong"]
                }),
                &ctx(dir.path()),
            )
            .await
            .expect("the tool ran");

        let plan = &out.metadata.as_ref().expect("metadata")["plan"];
        assert_eq!(plan["title"], json!("Add pricing"));
        assert_eq!(plan["steps"].as_array().map(Vec::len), Some(2));
        assert_eq!(
            out.metadata.as_ref().expect("metadata")["awaitingApproval"],
            json!(true)
        );
        // And the model reads its own plan back, numbered.
        assert!(out.output.contains("1. Add the fields - ModelsField"));
        assert!(out.output.contains("Open questions:"));
    }

    #[tokio::test]
    async fn a_plan_of_headings_is_refused() {
        let dir = tree();
        let err = PresentPlanTool
            .execute(
                json!({ "title": "x", "summary": "y", "steps": [{ "title": "  " }] }),
                &ctx(dir.path()),
            )
            .await
            .expect_err("a step with no title is not a step");
        assert!(err.to_string().contains("at least one step"));
    }

    #[tokio::test]
    async fn a_plan_nobody_could_read_is_refused() {
        let dir = tree();
        let steps: Vec<Value> = (0..MAX_STEPS + 1)
            .map(|i| json!({ "title": format!("step {i}") }))
            .collect();
        let err = PresentPlanTool
            .execute(
                json!({ "title": "x", "summary": "y", "steps": steps }),
                &ctx(dir.path()),
            )
            .await
            .expect_err("too many steps");
        assert!(err.to_string().contains("41 steps"), "{err}");
    }
}

//! Reading the terminal the person is typing in.
//!
//! Read-only, and that is a decision rather than an omission. The agent already
//! has `shell`, which runs a command of its own, cancellably, and reports back.
//! What it did not have was any way to see the terminal the PERSON is using -
//! so "the command I just ran", "this error", "did the build pass" were all
//! questions it had to answer by asking them to paste something, or by running
//! the command a second time and hoping it failed the same way.
//!
//! Writing into their shell was the obvious next step and is the wrong one. Two
//! writers on one stdin interleave, a command typed half a second before the
//! agent's lands in the middle of it, and the person loses the one surface in
//! the app that was unambiguously theirs.
//!
//! ## Why the terminals arrive through a trait
//!
//! The pty lives in `inertia-terminal` and is held by the app, which is also
//! the only thing that can ask a window to bring a tab forward. This crate
//! knows neither. [`Terminals`] is the whole of what the tool needs - read
//! every tab in a conversation, ask for one to be shown - so the tool is
//! testable against a fake and the app supplies the real one when it builds the
//! turn's tools.

use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::Result;
use serde_json::{json, Value};

/// One terminal tab, as the tool needs it.
#[derive(Debug, Clone, Default)]
pub struct TerminalTab {
    pub id: String,
    pub cwd: String,
    /// Whether something is running in this tab. Only a mirrored process can
    /// say so: a shell at a prompt looks exactly like a shell part-way through
    /// a build, from outside.
    pub busy: bool,
    pub running: Option<String>,
    /// The tail of the screen, as a person would read it.
    pub text: String,
    /// The tab the person is looking at.
    pub front: bool,
}

/// The terminals beside a conversation.
pub trait Terminals: Send + Sync + std::fmt::Debug {
    /// Every open tab in this conversation, front tab first, each holding at
    /// most a share of `chars`.
    fn read_thread(&self, thread: &str, chars: usize) -> Vec<TerminalTab>;

    /// Ask the window to bring this tab forward. A request, not a command: see
    /// `features/chat/pane-reveal.js` for who decides what showing it means.
    fn reveal(&self, id: &str);
}

/// How much of the end of the terminals is read when nobody says.
const DEFAULT_CHARS: usize = 8000;

#[derive(Debug)]
pub struct TerminalReadTool(Arc<dyn Terminals>);

#[async_trait]
impl Tool for TerminalReadTool {
    fn id(&self) -> &str {
        "terminal_read"
    }

    fn description(&self) -> &str {
        "Read every terminal tab beside this conversation: the commands the user typed, \
         the output they saw, and where the shell currently is. Use it when they refer to \
         something they ran - \"the command I just ran\", \"this error\", \"did it pass\" - \
         instead of saying you cannot see it, and instead of running the command again \
         yourself. Every open tab is returned, each with its own folder and whether \
         something is running in it. It runs nothing. This is THEIR shell, not yours: use \
         `shell` when you want to run something."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "chars": {
                    "type": "integer",
                    "description": "How much of the end of the terminals to read, shared between the tabs. Defaults to 8000."
                }
            },
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        PermissionRequest::new("terminal_read", "the terminal").with_always("*")
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some("the terminal".to_string())
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        // A subagent's session is `thread/sub`, and it is reading the
        // conversation's terminal rather than one of its own.
        let session = ctx.session.to_string();
        let thread = session.split('/').next().unwrap_or(&session).to_string();

        let chars = args
            .get("chars")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_CHARS);

        // Every tab, not the front one. A person watching a build in one and a
        // server in the next has two terminals; an agent that can only see one
        // of them will confidently explain a failure using the wrong half of it.
        let tabs = self.0.read_thread(&thread, chars);
        let said: Vec<TerminalTab> = tabs
            .into_iter()
            .filter(|tab| !tab.text.trim().is_empty())
            .collect();

        // A pane nobody opened is an answer. The model has learned something
        // true about the screen and can stop looking for a terminal.
        let Some(front) = said.iter().find(|tab| tab.front).or_else(|| said.first()) else {
            return Ok(ToolOutcome::text(
                "The terminal pane is not open, or nothing has been run in it yet.",
            )
            .with_title("the terminal"));
        };

        // Brought forward, so the person can look at what is being described. It
        // is their terminal, and being told about it while it sits behind
        // another panel is the same problem as being told about a page nobody
        // can see.
        self.0.reveal(&front.id);

        let body = said
            .iter()
            .map(|tab| {
                let running = match (&tab.running, tab.busy) {
                    (Some(what), true) => format!("Currently running: {what}"),
                    _ => "Nothing is running.".to_string(),
                };
                // Fenced, and said to be a transcript. What is in here is output
                // from programs the user ran, which means it is text from the
                // internet as often as not - a package's install script, a
                // test's failure message - and it must be read as a record of
                // what happened rather than as instructions addressed to the
                // agent.
                let open = if tab.front {
                    format!("<terminal tab=\"{}\" front=\"true\">", tab.id)
                } else {
                    format!("<terminal tab=\"{}\">", tab.id)
                };
                format!(
                    "{open}\nWorking directory: {}\n{running}\n\n{}\n</terminal>",
                    tab.cwd, tab.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let title = if said.len() == 1 {
            format!("the terminal ({})", front.cwd)
        } else {
            format!(
                "{} terminals ({} and {} more)",
                said.len(),
                front.cwd,
                said.len() - 1
            )
        };

        Ok(ToolOutcome {
            title: Some(title),
            output: body,
            metadata: Some(json!({
                "cwd": front.cwd,
                "busy": said.iter().any(|tab| tab.busy),
                "tabs": said.iter().map(|tab| tab.id.clone()).collect::<Vec<_>>(),
            })),
            images: Vec::new(),
        })
    }
}

/// The tool, holding the terminals the app opened.
pub fn terminal_tool(terminals: Arc<dyn Terminals>) -> Arc<dyn Tool> {
    Arc::new(TerminalReadTool(terminals))
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::tool::PermissionGate;
    use inertia_mock::MockGate;
    use parking_lot::Mutex;

    /// Terminals that were never opened, plus a note of what was shown.
    #[derive(Debug, Default)]
    struct Fake {
        tabs: Vec<TerminalTab>,
        revealed: Mutex<Vec<String>>,
        asked: Mutex<Vec<(String, usize)>>,
    }

    impl Terminals for Fake {
        fn read_thread(&self, thread: &str, chars: usize) -> Vec<TerminalTab> {
            self.asked.lock().push((thread.to_string(), chars));
            self.tabs.clone()
        }
        fn reveal(&self, id: &str) {
            self.revealed.lock().push(id.to_string());
        }
    }

    fn ctx(session: &str) -> ToolContext {
        ToolContext {
            root: std::env::temp_dir(),
            session: SessionId::from_existing(session),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(MockGate::allow_all()) as Arc<dyn PermissionGate>,
        }
    }

    fn tab(id: &str, cwd: &str, text: &str) -> TerminalTab {
        TerminalTab {
            id: id.to_string(),
            cwd: cwd.to_string(),
            text: text.to_string(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_pane_nobody_opened_is_an_answer_rather_than_an_error() {
        let tool = TerminalReadTool(Arc::new(Fake::default()));
        let out = tool
            .execute(json!({}), &ctx("t1"))
            .await
            .expect("a closed pane is not a failure");
        assert_eq!(
            out.output,
            "The terminal pane is not open, or nothing has been run in it yet."
        );
        assert_eq!(out.title.as_deref(), Some("the terminal"));
        assert!(out.metadata.is_none());
    }

    /// A tab that has printed nothing is a tab with nothing to say about it.
    #[tokio::test]
    async fn a_tab_that_has_said_nothing_is_not_reported() {
        let fake = Arc::new(Fake {
            tabs: vec![tab("chat:t1:sh:1", "D:\\work", "   \n  ")],
            ..Default::default()
        });
        let tool = TerminalReadTool(fake.clone());
        let out = tool.execute(json!({}), &ctx("t1")).await.expect("ran");
        assert!(out.output.starts_with("The terminal pane is not open"));
        // And nothing was brought forward, because there was nothing to look at.
        assert!(fake.revealed.lock().is_empty());
    }

    #[tokio::test]
    async fn one_terminal_is_fenced_with_its_folder_and_named_in_the_title() {
        let fake = Arc::new(Fake {
            tabs: vec![TerminalTab {
                front: true,
                ..tab("chat:t1:sh:1", "D:\\work", "npm test\n2 passed")
            }],
            ..Default::default()
        });
        let tool = TerminalReadTool(fake.clone());
        let out = tool.execute(json!({}), &ctx("t1")).await.expect("ran");

        assert_eq!(
            out.output,
            "<terminal tab=\"chat:t1:sh:1\" front=\"true\">\n\
             Working directory: D:\\work\n\
             Nothing is running.\n\
             \n\
             npm test\n2 passed\n\
             </terminal>"
        );
        assert_eq!(out.title.as_deref(), Some("the terminal (D:\\work)"));
        let meta = out.metadata.expect("metadata");
        assert_eq!(meta["cwd"], json!("D:\\work"));
        assert_eq!(meta["busy"], json!(false));
        assert_eq!(meta["tabs"], json!(["chat:t1:sh:1"]));
        // The tab being described is the one brought forward.
        assert_eq!(fake.revealed.lock().as_slice(), ["chat:t1:sh:1"]);
    }

    /// The whole reason this reads every tab: a build in one and a server in
    /// the next are two terminals, and an agent that sees one of them explains
    /// a failure with the wrong half.
    #[tokio::test]
    async fn every_tab_is_read_and_the_front_one_is_the_one_shown() {
        let fake = Arc::new(Fake {
            tabs: vec![
                TerminalTab {
                    front: true,
                    ..tab("chat:t1:sh:2", "D:\\work\\api", "cargo test")
                },
                TerminalTab {
                    busy: true,
                    running: Some("npm run dev".to_string()),
                    ..tab("chat:t1:job:8123", "D:\\work\\web", "ready on :3000")
                },
            ],
            ..Default::default()
        });
        let tool = TerminalReadTool(fake.clone());
        let out = tool.execute(json!({}), &ctx("t1")).await.expect("ran");

        assert!(out.output.contains("<terminal tab=\"chat:t1:sh:2\" front=\"true\">"));
        assert!(out.output.contains("<terminal tab=\"chat:t1:job:8123\">"));
        assert!(out.output.contains("Currently running: npm run dev"));
        assert!(out.output.contains("cargo test"));
        assert!(out.output.contains("ready on :3000"));
        assert_eq!(
            out.title.as_deref(),
            Some("2 terminals (D:\\work\\api and 1 more)")
        );
        let meta = out.metadata.expect("metadata");
        assert_eq!(meta["busy"], json!(true));
        assert_eq!(meta["tabs"], json!(["chat:t1:sh:2", "chat:t1:job:8123"]));
        assert_eq!(fake.revealed.lock().as_slice(), ["chat:t1:sh:2"]);
    }

    /// With no front tab named, the first is described and shown rather than
    /// nothing being shown at all.
    #[tokio::test]
    async fn something_is_brought_forward_even_when_no_tab_claims_to_be_front() {
        let fake = Arc::new(Fake {
            tabs: vec![tab("chat:t1:sh:9", "/srv", "done")],
            ..Default::default()
        });
        let tool = TerminalReadTool(fake.clone());
        tool.execute(json!({}), &ctx("t1")).await.expect("ran");
        assert_eq!(fake.revealed.lock().as_slice(), ["chat:t1:sh:9"]);
    }

    /// A subagent reads the conversation's terminal, not one of its own: its
    /// session is the conversation's with a suffix.
    #[tokio::test]
    async fn a_subagent_reads_the_conversation_it_was_started_from() {
        let fake = Arc::new(Fake {
            tabs: vec![tab("chat:t1:sh:1", "/srv", "hi")],
            ..Default::default()
        });
        let tool = TerminalReadTool(fake.clone());
        tool.execute(json!({ "chars": 2000 }), &ctx("t1/task_2"))
            .await
            .expect("ran");
        assert_eq!(
            fake.asked.lock().as_slice(),
            [("t1".to_string(), 2000usize)]
        );
    }

    #[tokio::test]
    async fn it_asks_for_nothing_and_promises_to_change_nothing() {
        let tool = TerminalReadTool(Arc::new(Fake::default()));
        let request = tool.permission(&json!({}));
        // The renderer already renders a card for this key; see
        // `src/shared/tools.js`.
        assert_eq!(request.key, "terminal_read");
        assert_eq!(request.target, "the terminal");
        assert_eq!(request.always.as_deref(), Some("*"));
        assert_eq!(tool.render(&json!({})).as_deref(), Some("the terminal"));
        assert_eq!(tool.id(), "terminal_read");
    }
}

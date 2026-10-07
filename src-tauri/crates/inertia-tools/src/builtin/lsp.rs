//! Asking the language server, instead of guessing.
//!
//! The servers are already here: one is started when a file is read and
//! consulted after every edit, and its diagnostics go back with the result.
//! That is half of what a language server is for. The other half is the
//! questions - where is this defined, who calls it, what type is it, what is
//! in this file - and until now an agent answered those with `grep`, which
//! finds the word and not the symbol: every comment, every string, every
//! unrelated variable of the same name, and none of the call sites that go
//! through an alias.
//!
//! So this is Claude Code's `LSP` tool and opencode's `lsp`, over the servers
//! this app already runs. One tool with an `operation` rather than six tools,
//! because the six differ only in which question is asked and a model choosing
//! between six near-identical descriptions chooses badly.
//!
//! What it deliberately does not do is replace reading. A definition is a file
//! and a line; the agent still opens it. The tool's job is to say which line,
//! exactly, in a language it actually understands.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use inertia_lsp::{Answer, Lsp, Question};
use serde_json::{json, Value};

use super::fence;

/// How many locations or symbols go back to the model.
const MAX_ROWS: usize = 60;

const DESCRIPTION: &str = "\
Ask the language server about the code: where a symbol is defined, who uses it,
what its type is, what a file contains.

This understands the language; `grep` understands text. Use it whenever the
question is about a symbol rather than a string - `grep` finds the word in
comments and in unrelated names, and misses the call that goes through an alias.

Operations:
  definition        where the symbol at this position is defined
  type_definition   where its type is defined
  implementation    what implements it
  references        everywhere it is used
  hover             its type and documentation, as the editor would show
  symbols           everything declared in this file, with line numbers
  workspace_symbols find a symbol by name across the project (pass `symbol`)

Positions are 1-based, the way `read` numbers its lines: give the line and the
character where the name starts. Read the file first, so you are pointing at
something. If there is no language server for this file's language, you are told
so plainly - fall back to `grep` then, rather than trying again.";

/// A path as the model should read it back: relative to where it is working,
/// unless it is somewhere else entirely.
fn relative(file: &Path, root: &Path) -> String {
    match file.strip_prefix(root) {
        Ok(rest) if !rest.as_os_str().is_empty() => rest.display().to_string(),
        _ => file.display().to_string(),
    }
}

fn basename(file: &Path) -> String {
    file.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string())
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

/// The `lsp` tool, over the servers the app is already running.
#[derive(Debug)]
pub struct LspTool {
    lsp: Arc<Lsp>,
}

impl LspTool {
    pub fn new(lsp: Arc<Lsp>) -> Self {
        Self { lsp }
    }
}

#[async_trait]
impl Tool for LspTool {
    fn id(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": inertia_lsp::operation_names(),
                    "description": "Which question to ask",
                },
                "filePath": {
                    "type": "string",
                    "description": "The file to ask about. For workspace_symbols, any file in the project.",
                },
                "line": {
                    "type": "integer",
                    "description": "1-based line of the symbol. Required except for symbols and workspace_symbols.",
                },
                "character": {
                    "type": "integer",
                    "description": "1-based column where the symbol's name starts. Defaults to 1.",
                },
                "symbol": {
                    "type": "string",
                    "description": "The name to search for, with workspace_symbols.",
                },
            },
            "required": ["operation", "filePath"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("filePath")
            .and_then(Value::as_str)
            .unwrap_or(inertia_core::permission::ANY);
        // The same grant as `read`, because that is exactly what this is: it
        // answers questions about a file the agent may read.
        PermissionRequest::new("read", target).with_always(inertia_core::permission::ANY)
    }

    fn render(&self, args: &Value) -> Option<String> {
        let operation = text(args, "operation").unwrap_or_else(|| "lsp".to_string());
        let what = match text(args, "symbol") {
            Some(symbol) => format!("\"{symbol}\""),
            None => basename(Path::new(&text(args, "filePath").unwrap_or_default())),
        };
        Some(format!("{} {what}", operation.replace('_', " ")))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let operation = text(&args, "operation").unwrap_or_default();
        let Some((_, needs_position)) = inertia_lsp::operation(&operation) else {
            return Err(Error::InvalidInput(format!(
                "`{operation}` is not one of: {}.",
                inertia_lsp::operation_names().join(", ")
            )));
        };
        let supplied = text(&args, "filePath")
            .ok_or_else(|| Error::InvalidInput("filePath is required.".into()))?;
        let file = fence::reach(ctx, &supplied).await?;

        let line = args.get("line").and_then(Value::as_u64).unwrap_or(0);
        let character = args.get("character").and_then(Value::as_u64).unwrap_or(1);
        let symbol = text(&args, "symbol");

        if needs_position && line == 0 {
            return Err(Error::InvalidInput(format!(
                "`{operation}` needs a position. Read the file first and pass the 1-based \
                 `line`, and `character` where the name starts."
            )));
        }
        if operation == "workspace_symbols" && symbol.as_deref().unwrap_or("").trim().is_empty() {
            return Err(Error::InvalidInput(
                "`workspace_symbols` needs a `symbol` to search for.".into(),
            ));
        }

        let question = Question {
            operation: &operation,
            line: line.max(1),
            character: character.max(1),
            symbol: symbol.as_deref(),
            ..Question::new(&operation)
        };
        let answer = self.lsp.query(&file, Some(&ctx.root), &question).await;

        // No language server for this file is an answer, not an error. On a
        // machine with none installed this is the only thing this tool ever
        // says, and it has to say it in a way that sends the model to `grep`
        // rather than back here.
        let Some(answer) = answer else {
            return Ok(ToolOutcome {
                title: Some(format!("no language server for {}", basename(&file))),
                output: format!(
                    "There is no language server for {} on this machine, so this question cannot be answered that way.\nUse `grep` instead, and read what it finds.",
                    file.display()
                ),
                metadata: Some(json!({
                    "operation": operation,
                    "path": file.display().to_string(),
                    "supported": false,
                })),
                images: Vec::new(),
            });
        };

        if let Answer::Unsupported { error } = &answer {
            return Ok(ToolOutcome {
                title: Some("the server could not answer".to_string()),
                output: format!("The language server did not answer that: {error}. Use `grep` instead."),
                metadata: Some(json!({
                    "operation": operation,
                    "path": file.display().to_string(),
                    "supported": false,
                })),
                images: Vec::new(),
            });
        }

        if let Answer::Hover(hover) = &answer {
            return Ok(ToolOutcome {
                title: Some(format!("hover {}:{}", basename(&file), line)),
                output: if hover.is_empty() {
                    "The server has nothing to say about that position.".to_string()
                } else {
                    hover.clone()
                },
                metadata: Some(json!({
                    "operation": operation,
                    "path": file.display().to_string(),
                    "supported": true,
                })),
                images: Vec::new(),
            });
        }

        let symbols = matches!(answer, Answer::Symbols(_));
        let rows: Vec<String> = match &answer {
            Answer::Symbols(rows) => rows
                .iter()
                .take(MAX_ROWS)
                .map(|row| {
                    format!(
                        "{} {}{} - {}:{}",
                        row.kind,
                        row.name,
                        row.container
                            .as_ref()
                            .map(|container| format!(" in {container}"))
                            .unwrap_or_default(),
                        relative(row.file.as_deref().unwrap_or(&file), &ctx.root),
                        row.line
                    )
                })
                .collect(),
            Answer::Locations(rows) => rows
                .iter()
                .take(MAX_ROWS)
                .map(|row| {
                    format!(
                        "{}:{}:{}",
                        relative(&row.file, &ctx.root),
                        row.line,
                        row.character
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
        let total = match &answer {
            Answer::Symbols(all) => all.len(),
            Answer::Locations(all) => all.len(),
            _ => 0,
        };

        if total == 0 {
            return Ok(ToolOutcome {
                title: Some("nothing found".to_string()),
                output: if operation == "references" {
                    format!(
                        "Nothing uses that, according to the language server. Check you pointed at the name itself, at {}:{}:{}.",
                        relative(&file, &ctx.root),
                        line,
                        character
                    )
                } else {
                    "The language server found nothing for that.".to_string()
                },
                metadata: Some(json!({
                    "operation": operation,
                    "path": file.display().to_string(),
                    "supported": true,
                    "count": 0,
                })),
                images: Vec::new(),
            });
        }

        let more = total - rows.len();
        let mut lines = rows;
        if more > 0 {
            lines.push(format!("... and {more} more"));
        }

        Ok(ToolOutcome {
            title: Some(format!(
                "{total} {}{}",
                if symbols { "symbol" } else { "location" },
                if total == 1 { "" } else { "s" }
            )),
            output: lines.join("\n"),
            metadata: Some(json!({
                "operation": operation,
                "path": file.display().to_string(),
                "supported": true,
                "count": total,
            })),
            images: Vec::new(),
        })
    }
}

/// The `lsp` tool, over the language servers the app already runs.
pub fn lsp_tool(lsp: Arc<Lsp>) -> Arc<dyn Tool> {
    Arc::new(LspTool::new(lsp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_lsp::testing::fake_launcher;
    use inertia_mock::MockGate;

    /// A project pointed at the fake language server for `.demo` files, the
    /// same way a user points Inertia at a server it has never heard of.
    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut config = dir.path().to_path_buf();
        config.push(".inertia.json");
        std::fs::write(
            &config,
            r#"{"lsp":{"demo":{"command":["demo-server"],"extensions":[".demo"],"rootFiles":[]}}}"#,
        )
        .unwrap();
        for (name, contents) in files {
            let mut file = dir.path().to_path_buf();
            file.push(name);
            std::fs::write(&file, contents).unwrap();
        }
        inertia_lsp::servers::reset();
        dir
    }

    /// The temp directory as the filesystem spells it. On Windows `TEMP` is
    /// often the 8.3 short form, and a root in one spelling never matches a
    /// path the server answered with in the other - which would fail this test
    /// about nothing.
    fn root(dir: &tempfile::TempDir) -> std::path::PathBuf {
        inertia_lsp::spelling(dir.path())
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext {
            root: root.to_path_buf(),
            session: SessionId::new(),
            call_id: ToolCallId::from_existing("tc_lsp"),
            permissions: Arc::new(MockGate::allow_all()),
        }
    }

    async fn run(tool: &LspTool, args: Value, root: &Path) -> ToolOutcome {
        tool.execute(args, &context(root)).await.unwrap()
    }

    #[tokio::test]
    async fn a_definition_is_one_location_per_line() {
        let dir = project(&[("a.demo", "@@def thing\ncall thing()\n")]);
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let out = run(
            &tool,
            json!({ "operation": "definition", "filePath": "a.demo", "line": 2, "character": 6 }),
            &root(&dir),
        )
        .await;

        assert_eq!(out.output, "a.demo:1:1");
        assert_eq!(out.title.as_deref(), Some("1 location"));
        assert_eq!(out.metadata.unwrap()["count"], 1);
    }

    #[tokio::test]
    async fn references_are_listed_and_counted() {
        let dir = project(&[(
            "a.demo",
            "@@def thing\n@@use thing here\n@@use thing there\ncall thing()\n",
        )]);
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let out = run(
            &tool,
            json!({ "operation": "references", "filePath": "a.demo", "line": 4, "character": 6 }),
            &root(&dir),
        )
        .await;

        assert_eq!(out.output, "a.demo:2:1\na.demo:3:1");
        assert_eq!(out.title.as_deref(), Some("2 locations"));
    }

    #[tokio::test]
    async fn symbols_name_their_kind_and_line() {
        let dir = project(&[("a.demo", "@@sym 12 doTheThing\n@@sym 5 Widget\n")]);
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let out = run(
            &tool,
            json!({ "operation": "symbols", "filePath": "a.demo" }),
            &root(&dir),
        )
        .await;

        assert_eq!(
            out.output,
            "function doTheThing - a.demo:1\nclass Widget - a.demo:2"
        );
        assert_eq!(out.title.as_deref(), Some("2 symbols"));
    }

    #[tokio::test]
    async fn a_long_answer_is_capped_and_says_how_many_were_left_out() {
        let mut source = String::from("@@def thing\n");
        for _ in 0..MAX_ROWS + 5 {
            source.push_str("@@use thing\n");
        }
        source.push_str("call thing()\n");
        let line = source.split('\n').count() - 1;
        let dir = project(&[("a.demo", &source)]);

        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let out = run(
            &tool,
            json!({ "operation": "references", "filePath": "a.demo", "line": line, "character": 6 }),
            &root(&dir),
        )
        .await;

        let lines: Vec<&str> = out.output.split('\n').collect();
        assert_eq!(lines.len(), MAX_ROWS + 1);
        assert_eq!(lines[MAX_ROWS], "... and 5 more");
        assert_eq!(out.title.as_deref(), Some("65 locations"));
    }

    #[tokio::test]
    async fn hover_is_whatever_the_server_said() {
        let dir = project(&[("a.demo", "@@hover const x: number\n")]);
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let out = run(
            &tool,
            json!({ "operation": "hover", "filePath": "a.demo", "line": 1 }),
            &root(&dir),
        )
        .await;
        assert_eq!(out.output, "const x: number");
        assert_eq!(out.title.as_deref(), Some("hover a.demo:1"));
    }

    /// The answer on nearly every machine, and the one that must not read as a
    /// failure.
    #[tokio::test]
    async fn no_language_server_is_an_answer_not_an_error() {
        let dir = project(&[("notes.txt", "hello\n")]);
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let out = run(
            &tool,
            json!({ "operation": "definition", "filePath": "notes.txt", "line": 1 }),
            &root(&dir),
        )
        .await;

        assert!(
            out.output.starts_with("There is no language server for "),
            "got {:?}",
            out.output
        );
        assert!(out.output.contains("Use `grep` instead"));
        assert_eq!(out.metadata.unwrap()["supported"], false);
    }

    #[tokio::test]
    async fn nothing_found_tells_the_model_where_it_pointed() {
        let dir = project(&[("a.demo", "call other()\n")]);
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let out = run(
            &tool,
            json!({ "operation": "references", "filePath": "a.demo", "line": 1, "character": 6 }),
            &root(&dir),
        )
        .await;
        assert!(out.output.contains("a.demo:1:6"), "got {:?}", out.output);
        assert_eq!(out.title.as_deref(), Some("nothing found"));
    }

    #[tokio::test]
    async fn an_operation_that_needs_a_position_says_so() {
        let dir = project(&[("a.demo", "x\n")]);
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let error = tool
            .execute(
                json!({ "operation": "definition", "filePath": "a.demo" }),
                &context(&root(&dir)),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("needs a position"), "got {error}");

        let error = tool
            .execute(
                json!({ "operation": "workspace_symbols", "filePath": "a.demo" }),
                &context(&root(&dir)),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("needs a `symbol`"), "got {error}");
    }

    #[tokio::test]
    async fn an_unknown_operation_lists_the_real_ones() {
        let dir = project(&[("a.demo", "x\n")]);
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let error = tool
            .execute(
                json!({ "operation": "rename", "filePath": "a.demo" }),
                &context(&root(&dir)),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("definition, type_definition"), "got {error}");
    }

    #[test]
    fn the_label_says_what_is_being_asked() {
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        assert_eq!(
            tool.render(&json!({ "operation": "type_definition", "filePath": "src/a.ts" })),
            Some("type definition a.ts".to_string())
        );
        assert_eq!(
            tool.render(&json!({ "operation": "workspace_symbols", "symbol": "Widget" })),
            Some("workspace symbols \"Widget\"".to_string())
        );
    }

    #[test]
    fn the_permission_is_the_one_read_asks_for() {
        let tool = LspTool::new(Arc::new(Lsp::with_launcher(fake_launcher())));
        let request = tool.permission(&json!({ "filePath": "src/a.ts" }));
        assert_eq!(request.key, "read");
        assert_eq!(request.target, "src/a.ts");
        assert_eq!(request.always.as_deref(), Some("*"));
    }
}

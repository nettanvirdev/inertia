//! `present` - handing something over.
//!
//! The Files panel used to be "every path a write tool touched", which for a
//! turn that scaffolds a site is forty rows of intermediate work with the two
//! files worth opening buried somewhere in the middle. The list was a log, and
//! the reader wanted a deliverable.
//!
//! The difference between those two is a judgement nothing but the agent can
//! make. A tool call cannot know whether the file it just wrote is the finished
//! thing or a step on the way to it; the agent knows, because it is the one with
//! the plan. So this is that judgement, said out loud: these are the files worth
//! looking at, and here is why.
//!
//! It changes nothing on disk and reveals nothing the reader could not already
//! open, which is why it is allowed by default. The only thing it costs is the
//! agent's own attention, and the only thing it can get wrong is presenting the
//! wrong file - which the reader sees immediately, because the file is right
//! there.
//!
//! Anything is presentable. A picture is a picture, a component is code, a PDF
//! is a PDF and a `.bin` nobody recognises is a name, a size and a way to open
//! it - the panel is not a viewer for a fixed list of formats, it is a way of
//! saying "this one".
//!
//! The renderer reads `metadata.presented` as `[{ path, name, bytes }]` and
//! `metadata.note`; the card and the side panel both draw from those two keys,
//! so their shape is the contract here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use serde_json::{json, Value};

/// More than this and it is a file listing again, which is the thing being
/// fixed.
const MAX_FILES: usize = 12;

/// The same rule every file tool uses: relative to the turn's folder.
fn resolve(root: &Path, supplied: &str) -> PathBuf {
    let path = Path::new(supplied);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Is `target` the root itself or somewhere beneath it?
fn is_inside(root: &Path, target: &Path) -> bool {
    target.starts_with(root)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

#[derive(Debug, Default)]
pub struct PresentTool;

#[async_trait]
impl Tool for PresentTool {
    fn id(&self) -> &str {
        "present"
    }

    fn description(&self) -> &str {
        "Show the person the files that are the result of your work, so they can open them \
         without reading back through the conversation. Use it when you finish something: the \
         page you built, the report you wrote, the picture you produced. Present the finished \
         things, not every file you touched on the way - a list of everything is what the \
         person already could not read. Any kind of file can be presented."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "description": "The files to present, in the order they are worth looking at.",
                    "items": { "type": "string", "description": "An absolute path to a file worth showing." }
                },
                "note": { "type": "string", "description": "One line saying what these are. Shown above them." }
            },
            "required": ["paths"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let target = args
            .get("paths")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        PermissionRequest::new("read", target).with_always("*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let names: Vec<String> = args
            .get("paths")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|one| file_name(Path::new(one)))
                    .collect()
            })
            .unwrap_or_default();
        Some(if names.is_empty() {
            "Present".to_string()
        } else {
            names.join(", ")
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let asked: Vec<String> = args
            .get("paths")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|one| !one.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        if asked.is_empty() {
            return Err(Error::Other("Name at least one file to present.".into()));
        }
        if asked.len() > MAX_FILES {
            return Err(Error::Other(format!(
                "That is {} files. Present at most {MAX_FILES} - the ones worth opening - \
                 rather than everything the turn touched.",
                asked.len()
            )));
        }

        let mut files: Vec<Value> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        for one in &asked {
            let abs = resolve(&ctx.root, one);

            // The same boundary every other file tool honours. Presenting a
            // file is pointing at it, and pointing outside the working folder
            // is still the user's decision to make. One question per
            // directory, and the "always" pattern is the directory too:
            // agreeing to a folder and then being asked again for the second
            // file in it is how people learn to click through prompts.
            if !is_inside(&ctx.root, &abs) {
                let dir = abs
                    .parent()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|| abs.display().to_string());
                // A trailing separator would make the pattern for a drive root
                // read `D:\/*`, which is not a rule anyone would recognise in
                // the settings list.
                let pattern = format!("{}/*", dir.trim_end_matches(['\\', '/']));
                let decision = ctx
                    .permissions
                    .ask(&PermissionRequest::new("external_directory", pattern.clone()).with_always(pattern))
                    .await?;
                if !decision.is_allowed() {
                    return Err(Error::Denied(format!(
                        "Showing {} was not allowed: it is outside the working folder.",
                        abs.display()
                    )));
                }
            }

            match std::fs::metadata(&abs) {
                Ok(stat) if stat.is_dir() => {
                    // A folder is not a deliverable, and silently presenting one
                    // would give the reader a row that opens nothing.
                    missing.push(format!("{} is a folder", abs.display()));
                }
                Ok(stat) => files.push(json!({
                    "path": abs.display().to_string(),
                    "name": file_name(&abs),
                    "bytes": stat.len(),
                })),
                Err(_) => missing.push(format!("{} does not exist", abs.display())),
            }
        }

        if files.is_empty() {
            return Err(Error::Other(format!(
                "Nothing could be presented: {}. Present files that exist.",
                missing.join("; ")
            )));
        }

        let names: Vec<&str> = files
            .iter()
            .filter_map(|file| file.get("name").and_then(Value::as_str))
            .collect();
        let paths: Vec<&str> = files
            .iter()
            .filter_map(|file| file.get("path").and_then(Value::as_str))
            .collect();

        let mut output = format!(
            "Presented {} file{}: {}.",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            paths.join(", ")
        );
        // Said in the result rather than thrown, so presenting four files does
        // not fail because the fifth was a typo.
        if !missing.is_empty() {
            output.push_str(&format!(" Not shown: {}.", missing.join("; ")));
        }

        let mut metadata = json!({ "presented": files });
        let note = args
            .get("note")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|note| !note.is_empty());
        if let Some(note) = note {
            metadata["note"] = json!(note);
        }

        Ok(ToolOutcome {
            title: Some(names.join(", ")),
            output,
            metadata: Some(metadata),
            images: Vec::new(),
        })
    }
}

/// The tool, ready for the registry.
pub fn present_tool() -> Arc<dyn Tool> {
    Arc::new(PresentTool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::tool::Decision;
    use inertia_mock::{MockGate, Policy};

    /// A folder with two files and a subfolder, the way the Electron tests
    /// laid it out.
    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(
            dir.path().join("page.tsx"),
            "export default function Page() {}\n",
        )
        .expect("file");
        std::fs::write(dir.path().join("hero.png"), vec![0u8; 64]).expect("file");
        std::fs::create_dir_all(dir.path().join("components")).expect("dir");
        dir
    }

    fn ctx(root: &Path, gate: MockGate) -> ToolContext {
        ToolContext {
            root: root.to_path_buf(),
            session: SessionId::from_existing("t1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(gate),
        }
    }

    fn joined(root: &Path, name: &str) -> String {
        root.join(name).display().to_string()
    }

    fn presented_names(out: &ToolOutcome) -> Vec<String> {
        out.metadata
            .as_ref()
            .and_then(|m| m.get("presented"))
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|f| f.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn reports_each_file_with_the_name_and_size_the_panel_draws() {
        let dir = tree();
        let out = PresentTool
            .execute(
                json!({ "paths": [joined(dir.path(), "page.tsx")] }),
                &ctx(dir.path(), MockGate::allow_all()),
            )
            .await
            .expect("the tool ran");

        assert_eq!(
            out.metadata.as_ref().expect("metadata")["presented"],
            json!([{ "path": joined(dir.path(), "page.tsx"), "name": "page.tsx", "bytes": 34 }])
        );
        assert_eq!(out.title.as_deref(), Some("page.tsx"));
    }

    #[tokio::test]
    async fn keeps_the_order_it_was_given_because_that_is_the_order_worth_reading() {
        let dir = tree();
        let out = PresentTool
            .execute(
                json!({ "paths": [joined(dir.path(), "hero.png"), joined(dir.path(), "page.tsx")] }),
                &ctx(dir.path(), MockGate::allow_all()),
            )
            .await
            .expect("the tool ran");
        assert_eq!(presented_names(&out), vec!["hero.png", "page.tsx"]);
    }

    #[tokio::test]
    async fn carries_the_note_through_and_leaves_it_out_when_there_is_none() {
        let dir = tree();
        let with_note = PresentTool
            .execute(
                json!({ "paths": [joined(dir.path(), "page.tsx")], "note": "The landing page." }),
                &ctx(dir.path(), MockGate::allow_all()),
            )
            .await
            .expect("the tool ran");
        assert_eq!(
            with_note.metadata.as_ref().expect("metadata")["note"],
            json!("The landing page.")
        );

        let without = PresentTool
            .execute(
                json!({ "paths": [joined(dir.path(), "page.tsx")] }),
                &ctx(dir.path(), MockGate::allow_all()),
            )
            .await
            .expect("the tool ran");
        assert!(without.metadata.as_ref().expect("metadata").get("note").is_none());
    }

    #[tokio::test]
    async fn resolves_a_relative_path_against_the_turns_folder() {
        let dir = tree();
        let out = PresentTool
            .execute(
                json!({ "paths": ["page.tsx"] }),
                &ctx(dir.path(), MockGate::allow_all()),
            )
            .await
            .expect("the tool ran");
        assert_eq!(
            out.metadata.as_ref().expect("metadata")["presented"][0]["path"],
            json!(joined(dir.path(), "page.tsx"))
        );
    }

    #[tokio::test]
    async fn shows_the_files_that_exist_rather_than_failing_over_the_one_that_does_not() {
        let dir = tree();
        let out = PresentTool
            .execute(
                json!({ "paths": [joined(dir.path(), "page.tsx"), joined(dir.path(), "gone.tsx")] }),
                &ctx(dir.path(), MockGate::allow_all()),
            )
            .await
            .expect("the tool ran");
        assert_eq!(presented_names(&out), vec!["page.tsx"]);
        assert!(out.output.contains("Not shown"), "{}", out.output);
        assert!(out.output.contains("gone.tsx"), "{}", out.output);
    }

    #[tokio::test]
    async fn refuses_a_folder_which_would_be_a_row_that_opens_nothing() {
        let dir = tree();
        let err = PresentTool
            .execute(
                json!({ "paths": [joined(dir.path(), "components")] }),
                &ctx(dir.path(), MockGate::allow_all()),
            )
            .await
            .expect_err("a folder is not a deliverable");
        assert!(err.to_string().contains("folder"), "{err}");
    }

    #[tokio::test]
    async fn fails_when_none_of_them_exist_rather_than_presenting_an_empty_shelf() {
        let dir = tree();
        let err = PresentTool
            .execute(
                json!({ "paths": [joined(dir.path(), "nope.tsx")] }),
                &ctx(dir.path(), MockGate::allow_all()),
            )
            .await
            .expect_err("nothing to show");
        assert!(err.to_string().contains("does not exist"), "{err}");
    }

    #[tokio::test]
    async fn asks_for_at_least_one_file() {
        let dir = tree();
        for args in [json!({ "paths": [] }), json!({})] {
            let err = PresentTool
                .execute(args, &ctx(dir.path(), MockGate::allow_all()))
                .await
                .expect_err("nothing named");
            assert!(err.to_string().contains("at least one"), "{err}");
        }
    }

    #[tokio::test]
    async fn refuses_a_file_listing_dressed_up_as_a_deliverable() {
        let dir = tree();
        let many: Vec<String> = (0..13).map(|_| joined(dir.path(), "page.tsx")).collect();
        let err = PresentTool
            .execute(json!({ "paths": many }), &ctx(dir.path(), MockGate::allow_all()))
            .await
            .expect_err("too many");
        assert!(err.to_string().contains("at most 12"), "{err}");
    }

    #[tokio::test]
    async fn a_file_outside_the_working_folder_is_asked_about_once_per_directory() {
        // Presenting is pointing, and pointing outside the folder the person
        // chose is still their decision to make.
        let dir = tree();
        let elsewhere = tempfile::tempdir().expect("a temp dir");
        std::fs::write(elsewhere.path().join("report.pdf"), b"%PDF").expect("file");

        let gate = MockGate::new(Policy::Scripted(vec![Decision::Allow]));
        let context = ctx(dir.path(), gate);
        let out = PresentTool
            .execute(
                json!({ "paths": [joined(elsewhere.path(), "report.pdf")] }),
                &context,
            )
            .await
            .expect("allowed by the person");
        assert_eq!(presented_names(&out), vec!["report.pdf"]);

        // And a refusal fails the call rather than quietly showing the file.
        let denied = ctx(dir.path(), MockGate::deny_all());
        let err = PresentTool
            .execute(
                json!({ "paths": [joined(elsewhere.path(), "report.pdf")] }),
                &denied,
            )
            .await
            .expect_err("refused");
        assert!(err.to_string().contains("outside the working folder"), "{err}");
    }

    #[test]
    fn the_permission_is_the_read_key_over_every_path() {
        let request = PresentTool.permission(&json!({ "paths": ["a.txt", "b.txt"] }));
        assert_eq!(request.key, "read");
        assert_eq!(request.target, "a.txt, b.txt");
        assert_eq!(request.always.as_deref(), Some("*"));
    }

    #[test]
    fn the_running_label_is_the_file_names() {
        assert_eq!(
            PresentTool.render(&json!({ "paths": ["/x/page.tsx", "/x/hero.png"] })),
            Some("page.tsx, hero.png".to_string())
        );
        assert_eq!(PresentTool.render(&json!({})), Some("Present".to_string()));
    }
}

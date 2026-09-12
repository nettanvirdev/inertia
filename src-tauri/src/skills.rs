//! Skills: advertising them in the prompt, and loading one on request.
//!
//! A skill is a folder: `skills/<slug>/SKILL.md` plus whatever else it needs -
//! scripts, reference documents, templates, example files. The system prompt
//! advertises only the name and the description, because a skill body can run
//! to thousands of words and paying for all of them on every turn to use one of
//! them occasionally is the wrong trade.
//!
//! So this module is both halves of that bargain. [`advertised`] is the short
//! list the prompt prints; [`SkillTool`] is what the model calls when it decides
//! a skill is relevant, and the instructions arrive in the conversation at the
//! moment they are needed, along with a listing of the folder, so the model can
//! go on to read the reference file or run the script the instructions mention.
//!
//! Loading a skill grants nothing. An `allowed-tools:` in a skill's frontmatter
//! is passed on to the model as the skill's own advice and never widens what
//! the agent may do: permission belongs to the agent, and a file in the
//! workspace folder is not allowed to hand itself more of it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use inertia_store::{collections, fsx, Collection, Layout};
use serde_json::{json, Value};

/// Enough to show what is in the folder, few enough to not become the reply.
const MAX_FILES: usize = 40;

/// Deeper than this and a listing stops describing the skill and starts
/// describing a vendored dependency somebody dropped beside it.
const MAX_LISTING_DEPTH: usize = 3;

/// The description is advertising copy for the skill, and it is paid for on
/// every single turn whether the skill is used or not. The form asks for one
/// line; this is what happens when a file on disk holds a thousand of them.
const MAX_DESCRIPTION: usize = 300;

/// What a skill with no description is listed as.
///
/// A skill with no description is still listed. Hiding it would leave the user
/// with a skill that exists, is enabled, and is never once reached for, and no
/// way to see that from either side.
const NO_DESCRIPTION: &str =
    "No description was written for this skill, so load it only if the user names it.";

/* -- the record, read the way the tool and the prompt both need it -------- */

fn text<'a>(record: &'a Value, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The name the prompt prints and the model asks for. Falls back to the folder
/// name, which the codec already does; the fallback here is for a record that
/// arrived some other way.
fn label(record: &Value) -> &str {
    let name = text(record, "name");
    if name.trim().is_empty() {
        text(record, "id")
    } else {
        name
    }
}

fn is_missing(record: &Value) -> bool {
    record.get("missing").and_then(Value::as_bool) == Some(true)
}

/// Absent means on: the codec writes `enabled: true` for a fresh skill, but a
/// hand-made folder never has the key.
fn is_enabled(record: &Value) -> bool {
    record.get("enabled").and_then(Value::as_bool) != Some(false)
}

/* -- advertising ---------------------------------------------------------- */

/// One line of the prompt's `<available_skills>` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertised {
    /// The folder name. Unique by construction.
    pub id: String,
    /// What the model should ask for. The written name, or the folder name
    /// when two skills share a written name.
    pub name: String,
    /// One line, whitespace collapsed, cut at [`MAX_DESCRIPTION`], with the
    /// stand-in sentence when nothing was written. Ready to print once it has
    /// been escaped for the element it sits in.
    pub description: String,
}

/// The description as one line, the way the prompt prints it.
fn summarise(description: &str) -> String {
    let line = description.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return NO_DESCRIPTION.to_string();
    }
    if line.chars().count() > MAX_DESCRIPTION {
        let cut: String = line.chars().take(MAX_DESCRIPTION).collect();
        return format!("{}...", cut.trim_end());
    }
    line
}

/// The skills the prompt should advertise, in folder order.
///
/// Disabled skills stay out, the way the Electron session filtered them before
/// the prompt saw the list, and so does a folder with no SKILL.md in it: a
/// skill the model can be offered but never load is a wasted call and a
/// confusing answer.
///
/// A name shared by two skills is worse than a bad name: the tool resolves it
/// to whichever it finds first, so the model would ask for one and get the
/// other with nothing to show that it had. Folder ids are unique by
/// construction, so a collision falls back to those.
pub fn advertised(layout: &Layout) -> Vec<Advertised> {
    let all = collections::list(layout, Collection::Skills);
    let usable: Vec<&Value> = all
        .iter()
        .filter(|record| is_enabled(record) && !is_missing(record))
        .filter(|record| !label(record).trim().is_empty())
        .collect();

    let shared = |name: &str| {
        usable
            .iter()
            .filter(|record| label(record).eq_ignore_ascii_case(name))
            .count()
            > 1
    };

    usable
        .iter()
        .map(|record| {
            let name = label(record);
            Advertised {
                id: text(record, "id").to_string(),
                name: if shared(name) {
                    text(record, "id").to_string()
                } else {
                    name.to_string()
                },
                description: summarise(text(record, "description")),
            }
        })
        .collect()
}

/* -- the tool ------------------------------------------------------------- */

/// A quote in a skill's name must not end the attribute it sits in.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The tools a skill says it wants, written either way round because both
/// spellings appear in skill files in the wild.
fn allowed_tools(record: &Value) -> Vec<String> {
    let extra = record.get("extra");
    let raw = extra
        .and_then(|extra| extra.get("allowed-tools"))
        .or_else(|| extra.and_then(|extra| extra.get("allowedTools")));

    let scalar = |value: &Value| -> String {
        match value {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    };

    let list: Vec<String> = match raw {
        Some(Value::Array(items)) => items.iter().map(scalar).collect(),
        Some(other) => scalar(other).split(',').map(str::to_string).collect(),
        None => Vec::new(),
    };
    list.into_iter()
        .map(|one| one.trim().to_string())
        .filter(|one| !one.is_empty())
        .collect()
}

/// Everything beside SKILL.md, relative to the skill's own folder, with
/// forward slashes whatever the platform - these are paths the model writes
/// back into instructions and tool calls, not paths it opens itself.
fn siblings(dir: &Path, prefix: &str, depth: usize) -> Vec<String> {
    if depth > MAX_LISTING_DEPTH {
        return Vec::new();
    }
    let mut out = Vec::new();
    for name in fsx::list_dir(
        dir,
        fsx::ListOptions {
            only_dirs: true,
            ..Default::default()
        },
    ) {
        out.extend(siblings(
            &dir.join(&name),
            &format!("{prefix}{name}/"),
            depth + 1,
        ));
    }
    for name in fsx::list_dir(dir, fsx::ListOptions::default()) {
        if prefix.is_empty() && name == "SKILL.md" {
            continue;
        }
        out.push(format!("{prefix}{name}"));
    }
    out
}

/// A required string argument, or the sentence saying it is missing.
fn required(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::Other(format!("`{key}` is required.")))
}

fn optional(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Debug)]
pub struct SkillTool {
    layout: Layout,
}

impl SkillTool {
    /// The skill the model named, or the sentence telling it why not.
    ///
    /// The id is tried first because it is unique; a name is whatever the user
    /// typed and two skills may well share one. The refusals are `Ok`-shaped
    /// results in Electron too: the model reads them and picks again.
    fn find(&self, wanted: &str) -> Result<Value> {
        let all = collections::list(&self.layout, Collection::Skills);
        let needle = wanted.trim().to_lowercase();

        let by_id = all
            .iter()
            .find(|record| text(record, "id").to_lowercase() == needle);
        let by_name: Vec<&Value> = all
            .iter()
            .filter(|record| text(record, "name").to_lowercase() == needle)
            .collect();

        let Some(skill) = by_id.or_else(|| by_name.first().copied()) else {
            let names: Vec<&str> = all.iter().map(label).collect();
            return Err(Error::Other(if names.is_empty() {
                format!("There is no skill called {wanted}, and no skills are installed.")
            } else {
                format!(
                    "There is no skill called {wanted}. Available skills: {}.",
                    names.join(", ")
                )
            }));
        };

        if by_id.is_none() && by_name.len() > 1 {
            // Picking the first would load one skill while the model believed
            // it had asked for the other, and nothing in the output would give
            // that away.
            let ids: Vec<&str> = by_name.iter().map(|record| text(record, "id")).collect();
            return Err(Error::Other(format!(
                "More than one skill is called {wanted}. Ask for one of these folder names instead: {}.",
                ids.join(", ")
            )));
        }
        if is_missing(skill) {
            return Err(Error::Other(format!(
                "The folder skills/{} has no SKILL.md in it, so there is nothing to load.",
                text(skill, "id")
            )));
        }
        if !is_enabled(skill) {
            return Err(Error::Other(format!(
                "The skill {} is turned off. The user can enable it in Skills.",
                text(skill, "name")
            )));
        }
        Ok(skill.clone())
    }

    /// The skill's own folder, confined to the workspace. The id came from a
    /// directory listing so it cannot escape, but the check is what makes that
    /// a fact about this function rather than about the listing.
    fn folder(&self, id: &str) -> Result<PathBuf> {
        fsx::resolve_inside(
            self.layout.root(),
            Path::new(Collection::Skills.dir()).join(id),
        )
        .map_err(|_| {
            Error::Other(format!(
                "The skill folder skills/{id} resolves outside the workspace, so it cannot be loaded."
            ))
        })
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn id(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        "Load a skill: a set of instructions the user wrote for a particular kind of work.\n\
         \n\
         The skills available to you are listed in your system prompt with a description each.\n\
         When a task matches one of those descriptions, load it BEFORE you start work rather\n\
         than after - a skill exists because there is something about this task that is not\n\
         obvious, and finding that out afterwards means doing the work twice.\n\
         \n\
         The instructions arrive in the conversation, along with a listing of the skill's own\n\
         folder. Paths mentioned in the instructions are relative to that folder, and you can\n\
         read or run those files with the ordinary tools."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name of the skill to load, exactly as it appears in your system prompt"
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        let name = optional(args, "name").unwrap_or_default();
        // Remembered per skill rather than as `*`: agreeing to load the
        // deployment skill is not agreeing to load every skill that ever gets
        // added.
        PermissionRequest::new("skill", name.clone()).with_always(name)
    }

    fn render(&self, args: &Value) -> Option<String> {
        optional(args, "name")
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let wanted = required(&args, "name")?;
        let skill = self.find(&wanted)?;
        let id = text(&skill, "id").to_string();
        let name = text(&skill, "name").to_string();

        let dir = self.folder(&id)?;
        let found = siblings(&dir, "", 0);
        let files: Vec<String> = found.iter().take(MAX_FILES).cloned().collect();

        let instructions = text(&skill, "instructions").trim();
        let mut parts: Vec<String> = vec![
            format!("<skill name=\"{}\">", escape(&name)),
            if instructions.is_empty() {
                "(This skill has no instructions.)".to_string()
            } else {
                instructions.to_string()
            },
            String::new(),
            format!("The folder for this skill is {}", dir.display()),
            "Any relative path in the instructions above is relative to that folder.".to_string(),
        ];

        // A skill may name the tools it expects to be carried out with. It is
        // a statement to the model and not a grant: what an agent may actually
        // do is decided by its permission rules, which do not change because a
        // skill was loaded, and a skill cannot widen them by asking.
        let declared = allowed_tools(&skill);
        if !declared.is_empty() {
            parts.push(String::new());
            parts.push(format!(
                "This skill says it should be carried out with these tools: {}.",
                declared.join(", ")
            ));
            parts.push(
                "That is the skill's own advice. Your permission rules are unchanged by loading it."
                    .to_string(),
            );
        }

        if !files.is_empty() {
            parts.push(String::new());
            parts.push("<skill_files>".to_string());
            parts.extend(files.iter().map(|file| format!("  {file}")));
            // Said out loud rather than silently cut: a model that cannot see
            // the file it was told to run needs to know the listing was
            // shortened, not that the file is absent.
            if found.len() > files.len() {
                parts.push(format!(
                    "  ...and {} more, not listed",
                    found.len() - files.len()
                ));
            }
            parts.push("</skill_files>".to_string());
        }
        parts.push("</skill>".to_string());

        Ok(ToolOutcome {
            title: Some(format!("Loaded {name}")),
            output: parts.join("\n"),
            metadata: Some(json!({
                "name": name,
                "id": id,
                "dir": dir.display().to_string(),
                "files": files,
            })),
            images: Vec::new(),
        })
    }
}

/// The tool, for a turn that has a workspace.
pub fn skill_tool(layout: Layout) -> Arc<dyn Tool> {
    Arc::new(SkillTool { layout })
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_mock::MockGate;

    /// A workspace with two skills in it, one turned off.
    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        write_skill(
            &layout,
            "deploy",
            &["name: Deploy", "description: Ship a release"],
            "Run the pipeline.",
        );
        write_skill(
            &layout,
            "archive",
            &["name: Archive", "description: Old way", "enabled: false"],
            "Do not.",
        );
        (dir, layout)
    }

    /// Written by hand rather than through `put`, because the interesting
    /// folders are the ones a person made in a text editor.
    fn write_skill(layout: &Layout, id: &str, frontmatter: &[&str], body: &str) -> PathBuf {
        let dir = layout.collection_dir(Collection::Skills).join(id);
        std::fs::create_dir_all(&dir).expect("the folder");
        let mut lines = vec!["---"];
        lines.extend_from_slice(frontmatter);
        lines.extend(["---", "", body, ""]);
        std::fs::write(dir.join("SKILL.md"), lines.join("\n")).expect("the file");
        dir
    }

    fn ctx(layout: &Layout) -> ToolContext {
        ToolContext {
            root: layout.root().to_path_buf(),
            session: SessionId::from_existing("t1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(MockGate::allow_all()),
        }
    }

    async fn load(layout: &Layout, name: &str) -> Result<ToolOutcome> {
        skill_tool(layout.clone())
            .execute(json!({ "name": name }), &ctx(layout))
            .await
    }

    fn message(result: Result<ToolOutcome>) -> String {
        match result {
            Err(error) => error.to_string(),
            Ok(outcome) => panic!("expected a refusal, got: {}", outcome.output),
        }
    }

    // -- advertising -------------------------------------------------------

    #[test]
    fn a_disabled_skill_is_not_advertised() {
        let (_dir, layout) = workspace();
        let list = advertised(&layout);
        assert_eq!(
            list,
            vec![Advertised {
                id: "deploy".into(),
                name: "Deploy".into(),
                description: "Ship a release".into(),
            }]
        );
    }

    #[test]
    fn a_folder_without_a_skill_file_is_not_advertised() {
        let (_dir, layout) = workspace();
        std::fs::create_dir_all(layout.collection_dir(Collection::Skills).join("half-made"))
            .expect("the folder");
        let names: Vec<String> = advertised(&layout).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["Deploy"]);
    }

    /// The tool resolves a shared name to whichever it finds first, so the
    /// prompt has to offer something unique instead.
    #[test]
    fn two_skills_with_one_name_are_advertised_by_folder() {
        let (_dir, layout) = workspace();
        write_skill(&layout, "deploy-api", &["name: Deploy", "description: API"], "A.");
        let names: Vec<String> = advertised(&layout).into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["deploy", "deploy-api"]);
    }

    #[test]
    fn a_description_is_one_line_and_bounded() {
        let (_dir, layout) = workspace();
        let long = format!("first   line \t second {}", "x".repeat(400));
        write_skill(
            &layout,
            "wordy",
            &["name: Wordy", &format!("description: \"{long}\"")],
            "B.",
        );
        write_skill(&layout, "mute", &["name: Mute"], "C.");

        let list = advertised(&layout);
        let wordy = list.iter().find(|s| s.id == "wordy").expect("wordy");
        assert!(wordy.description.starts_with("first line second x"));
        assert!(wordy.description.ends_with("..."));
        assert_eq!(wordy.description.chars().count(), MAX_DESCRIPTION + 3);

        let mute = list.iter().find(|s| s.id == "mute").expect("mute");
        assert_eq!(mute.description, NO_DESCRIPTION);
    }

    // -- loading -----------------------------------------------------------

    #[tokio::test]
    async fn a_known_skill_returns_its_instructions_and_folder() {
        let (_dir, layout) = workspace();
        let out = load(&layout, "Deploy").await.expect("the skill loaded");

        assert!(out.output.starts_with("<skill name=\"Deploy\">\nRun the pipeline."), "{}", out.output);
        let dir = layout.collection_dir(Collection::Skills).join("deploy");
        assert!(out.output.contains(&format!("The folder for this skill is {}", dir.display())));
        assert_eq!(out.title.as_deref(), Some("Loaded Deploy"));

        let metadata = out.metadata.expect("metadata");
        assert_eq!(metadata["id"], json!("deploy"));
        assert_eq!(metadata["name"], json!("Deploy"));
        assert_eq!(metadata["dir"], json!(dir.display().to_string()));
        assert_eq!(metadata["files"], json!([]));
    }

    #[tokio::test]
    async fn the_folder_name_and_the_written_name_both_resolve_in_any_case() {
        let (_dir, layout) = workspace();
        for name in ["deploy", "DEPLOY", "Deploy", "ship a release"] {
            let result = load(&layout, name).await;
            if name == "ship a release" {
                // The description is not a name.
                assert!(result.is_err(), "{name} should not resolve");
            } else {
                assert!(result.is_ok(), "{name} should resolve");
            }
        }
    }

    #[tokio::test]
    async fn an_unknown_skill_is_answered_with_the_names_that_exist() {
        let (_dir, layout) = workspace();
        let text = message(load(&layout, "nonsense").await);
        // Every installed skill, the way Electron lists them - including the
        // disabled one, which the model may then be told is turned off.
        assert_eq!(
            text,
            "There is no skill called nonsense. Available skills: Archive, Deploy."
        );
    }

    #[tokio::test]
    async fn no_skills_at_all_is_said_plainly() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        let text = message(load(&layout, "anything").await);
        assert!(text.contains("no skills are installed"), "{text}");
    }

    #[tokio::test]
    async fn a_disabled_skill_is_turned_off_not_missing() {
        let (_dir, layout) = workspace();
        let text = message(load(&layout, "archive").await);
        assert_eq!(text, "The skill Archive is turned off. The user can enable it in Skills.");
    }

    #[tokio::test]
    async fn a_folder_with_no_skill_file_is_empty_not_disabled() {
        let (_dir, layout) = workspace();
        std::fs::create_dir_all(layout.collection_dir(Collection::Skills).join("half-made"))
            .expect("the folder");
        let text = message(load(&layout, "half-made").await);
        assert!(text.contains("has no SKILL.md"), "{text}");
    }

    #[tokio::test]
    async fn two_skills_with_one_name_are_refused_rather_than_guessed() {
        let (_dir, layout) = workspace();
        // Two folders, one written name, and the name is nobody's folder id -
        // which is what makes it genuinely ambiguous. A name that also spells a
        // folder is not ambiguous at all: the id wins, as the next test says.
        write_skill(&layout, "ship-web", &["name: Ship", "description: d"], "Web.");
        write_skill(&layout, "ship-api", &["name: Ship", "description: d"], "Api.");
        let text = message(load(&layout, "Ship").await);
        assert!(text.contains("ship-web") && text.contains("ship-api"), "{text}");
        assert!(text.starts_with("More than one skill is called Ship."), "{text}");
    }

    #[tokio::test]
    async fn the_folder_name_wins_when_it_is_also_another_skills_written_name() {
        let (_dir, layout) = workspace();
        write_skill(&layout, "other", &["name: deploy", "description: d"], "By name.");
        let out = load(&layout, "deploy").await.expect("the skill loaded");
        assert!(out.output.contains("Run the pipeline."), "{}", out.output);
    }

    // -- the folder --------------------------------------------------------

    #[tokio::test]
    async fn the_folder_is_listed_without_skill_md_and_with_forward_slashes() {
        let (_dir, layout) = workspace();
        let dir = layout.collection_dir(Collection::Skills).join("deploy");
        std::fs::write(dir.join("checklist.md"), "1.").expect("file");
        std::fs::create_dir_all(dir.join("scripts")).expect("dir");
        std::fs::write(dir.join("scripts/check.py"), "print(1)").expect("file");
        // Neighbours are not this skill's business.
        std::fs::write(
            layout.collection_dir(Collection::Skills).join("loose.txt"),
            "not mine",
        )
        .expect("file");

        let out = load(&layout, "deploy").await.expect("the skill loaded");
        assert!(out.output.contains("<skill_files>\n  scripts/check.py\n  checklist.md\n</skill_files>"), "{}", out.output);
        assert!(!out.output.contains("  SKILL.md"));
        assert!(!out.output.contains("loose.txt"));
        assert_eq!(
            out.metadata.expect("metadata")["files"],
            json!(["scripts/check.py", "checklist.md"])
        );
    }

    #[tokio::test]
    async fn a_long_listing_says_how_many_it_left_out() {
        let (_dir, layout) = workspace();
        let dir = layout.collection_dir(Collection::Skills).join("deploy");
        for n in 0..45 {
            std::fs::write(dir.join(format!("file-{n:02}.txt")), "x").expect("file");
        }
        let out = load(&layout, "deploy").await.expect("the skill loaded");
        assert!(out.output.contains("  ...and 5 more, not listed"), "{}", out.output);
        assert_eq!(
            out.metadata.expect("metadata")["files"].as_array().map(Vec::len),
            Some(MAX_FILES)
        );
    }

    // -- what a skill is allowed to do -------------------------------------

    #[tokio::test]
    async fn the_tools_a_skill_names_are_passed_on_as_advice() {
        let (_dir, layout) = workspace();
        write_skill(
            &layout,
            "picky",
            &["name: Picky", "description: d", "allowed-tools: read, shell"],
            "P.",
        );
        let out = load(&layout, "picky").await.expect("the skill loaded");
        assert!(out.output.contains("carried out with these tools: read, shell."), "{}", out.output);
        assert!(out.output.contains("permission rules are unchanged"));

        // The list spelling, and the camel-case key, both count.
        write_skill(
            &layout,
            "listed",
            &["name: Listed", "description: d", "allowedTools: [glob, grep]"],
            "L.",
        );
        let out = load(&layout, "listed").await.expect("the skill loaded");
        assert!(out.output.contains("these tools: glob, grep."), "{}", out.output);
    }

    #[tokio::test]
    async fn a_quote_in_a_name_cannot_end_the_element() {
        let (_dir, layout) = workspace();
        write_skill(
            &layout,
            "odd",
            &["name: 'The \"big\" <one>'", "description: d"],
            "O.",
        );
        let out = load(&layout, "odd").await.expect("the skill loaded");
        assert!(
            out.output.starts_with("<skill name=\"The &quot;big&quot; &lt;one&gt;\">"),
            "{}",
            out.output
        );
    }

    #[tokio::test]
    async fn a_skill_named_after_a_tool_is_still_just_a_skill() {
        // Skill names and tool ids are separate namespaces: a skill is only
        // ever reached through this tool, by name.
        let (_dir, layout) = workspace();
        write_skill(&layout, "shell", &["name: shell", "description: d"], "Not the tool.");
        let tool = skill_tool(layout.clone());
        assert_eq!(tool.id(), "skill");
        let out = load(&layout, "shell").await.expect("the skill loaded");
        assert!(out.output.contains("Not the tool."));
    }

    #[test]
    fn permission_is_asked_per_skill_not_for_skills_in_general() {
        let (_dir, layout) = workspace();
        let request = skill_tool(layout).permission(&json!({ "name": "Deploy" }));
        assert_eq!(request.key, "skill");
        assert_eq!(request.target, "Deploy");
        assert_eq!(request.always.as_deref(), Some("Deploy"));
    }
}

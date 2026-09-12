//! Reading the user's hook files into a flat list of handlers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Every event, in the order they happen in a turn.
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "Notification",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "StopFailure",
];

/// Events whose `matcher` means something, and what it is matched against.
pub fn matched_by(event: &str) -> Option<&'static str> {
    Some(match event {
        "PreToolUse" | "PermissionRequest" | "PostToolUse" => "tool",
        "SessionStart" => "source",
        "PreCompact" | "PostCompact" => "trigger",
        "SubagentStart" | "SubagentStop" => "agent",
        "Notification" => "kind",
        _ => return None,
    })
}

/// Events a handler may block. Elsewhere `decision: block` is only a message.
pub fn is_blockable(event: &str) -> bool {
    matches!(
        event,
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "Stop" | "SubagentStop"
    )
}

const DEFAULT_COMMAND_TIMEOUT_S: u64 = 60;
const DEFAULT_PROMPT_TIMEOUT_S: u64 = 30;
const MAX_TIMEOUT_S: u64 = 600;

/// Where the workspace file lives, and the per-repository one beside it.
pub const PROJECT_FILE: &str = ".inertia/hooks.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Command,
    Prompt,
}

/// One handler, normalised, with where it came from attached.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Handler {
    pub id: String,
    pub event: String,
    pub matcher: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub prompt: Option<String>,
    pub timeout_ms: u64,
    /// Honoured at every level - a whole file, one group, one handler - so a
    /// person can switch a hook off to test something without deleting the
    /// thing they wrote.
    pub enabled: bool,
    pub name: Option<String>,
    pub source: String,
}

fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_string()
}

fn normalise(
    raw: &Value,
    event: &str,
    matcher: Option<&Value>,
    source: &str,
    group_index: usize,
    index: usize,
) -> Option<Handler> {
    if !raw.is_object() {
        return None;
    }

    let declared = raw.get("type").and_then(Value::as_str);
    let kind = match declared {
        Some("prompt") => Kind::Prompt,
        // A handler carrying a command is a command handler whether or not it
        // said so, which is how most people write them.
        Some("command") => Kind::Command,
        _ if raw.get("command").is_some() => Kind::Command,
        _ => return None,
    };

    let command = text(raw.get("command"));
    let prompt = text(raw.get("prompt"));
    match kind {
        Kind::Command if command.trim().is_empty() => return None,
        Kind::Prompt if prompt.trim().is_empty() => return None,
        _ => {}
    }

    let seconds = raw
        .get("timeout")
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(|s| (s as u64).min(MAX_TIMEOUT_S));

    let default = match kind {
        Kind::Prompt => DEFAULT_PROMPT_TIMEOUT_S,
        Kind::Command => DEFAULT_COMMAND_TIMEOUT_S,
    };

    Some(Handler {
        id: format!("{source}:{event}:{group_index}:{index}"),
        event: event.to_string(),
        matcher: matcher
            .filter(|m| !m.is_null())
            .map(|m| text(Some(m)))
            .unwrap_or_default(),
        kind,
        command: (kind == Kind::Command).then_some(command),
        args: raw
            .get("args")
            .and_then(Value::as_array)
            .filter(|_| kind == Kind::Command)
            .map(|items| items.iter().map(|a| text(Some(a))).collect()),
        prompt: (kind == Kind::Prompt).then_some(prompt),
        timeout_ms: seconds.unwrap_or(default) * 1000,
        enabled: raw.get("enabled") != Some(&Value::Bool(false)),
        name: raw
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .map(str::to_string),
        source: source.to_string(),
    })
}

/// One file's worth of hooks, flattened.
#[derive(Debug, Default)]
pub struct Parsed {
    pub handlers: Vec<Handler>,
    pub warnings: Vec<String>,
    pub enabled: bool,
}

/// Read one document, tolerating both shapes: the settings shape with events
/// at the top, and the plugin shape wrapped in `{ hooks: { ... } }`.
pub fn parse_config(doc: Option<&Value>, source: &str) -> Parsed {
    let mut out = Parsed {
        enabled: true,
        ..Default::default()
    };
    let Some(doc) = doc.filter(|d| d.is_object()) else {
        return out;
    };
    if doc.get("enabled") == Some(&Value::Bool(false)) {
        out.enabled = false;
    }

    let events = doc
        .get("hooks")
        .filter(|h| h.is_object())
        .unwrap_or(doc)
        .as_object();
    let Some(events) = events else { return out };

    for (event, groups) in events {
        if matches!(
            event.as_str(),
            "enabled" | "hooks" | "description" | "$schema"
        ) {
            continue;
        }
        if !EVENTS.contains(&event.as_str()) {
            out.warnings.push(format!(
                "{source}: \"{event}\" is not a hook event. Events are {}.",
                EVENTS.join(", ")
            ));
            continue;
        }
        let Some(groups) = groups.as_array() else {
            out.warnings.push(format!(
                "{source}: {event} must be an array of {{ matcher, hooks }} groups."
            ));
            continue;
        };

        for (group_index, group) in groups.iter().enumerate() {
            if !group.is_object() {
                continue;
            }
            // A group is normally `{ matcher, hooks: [...] }`, but a lone
            // handler written where a group belongs is common enough in files
            // people actually have that rejecting it would be pedantry.
            let list: Vec<Value> = match group.get("hooks").and_then(Value::as_array) {
                Some(items) => items.clone(),
                None if group.get("command").is_some() || group.get("type").is_some() => {
                    vec![group.clone()]
                }
                None => Vec::new(),
            };

            if group.get("matcher").is_some() && matched_by(event).is_none() {
                out.warnings.push(format!(
                    "{source}: {event} has a matcher, but that event is not matched against anything."
                ));
            }
            if group.get("enabled") == Some(&Value::Bool(false)) {
                continue;
            }

            for (index, raw) in list.iter().enumerate() {
                match normalise(raw, event, group.get("matcher"), source, group_index, index) {
                    Some(handler) => out.handlers.push(handler),
                    None => out.warnings.push(format!(
                        "{source}: {event} handler {group_index}.{index} needs a command or a prompt."
                    )),
                }
            }
        }
    }
    out
}

/// Where one file's handlers came from, for the settings pane.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub source: String,
    pub present: bool,
    pub enabled: bool,
    pub handlers: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// Every handler in force for one turn, and the warnings that came with them.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Loaded {
    pub handlers: Vec<Handler>,
    /// Switched off rather than absent. The pane lists these too, greyed, so
    /// somebody wondering why their hook did not fire can see that it is there.
    pub disabled: Vec<Handler>,
    pub warnings: Vec<String>,
    pub sources: Vec<Source>,
}

/// Read every hook file that applies, in the order they layer.
///
/// Read fresh each turn rather than once at launch. A person editing a hook
/// wants the next tool call to use it, and a turn costs seconds, so a file read
/// per turn is free against what it buys.
///
/// Three places a hook can come from: the workspace (`hooks/hooks.json`, the
/// person's own, for every agent), the project (`<cwd>/.inertia/hooks.json`,
/// shipped with a repository) and the agent record itself.
pub fn load(
    layout: Option<&inertia_store::Layout>,
    cwd: Option<&Path>,
    agent: Option<(&str, &Value)>,
) -> Loaded {
    let mut all: Vec<Handler> = Vec::new();
    let mut out = Loaded::default();

    /// Fold one file's parse into the answer: note where it came from, keep
    /// its warnings, and take its handlers only if the file is switched on.
    fn take(
        out: &mut Loaded,
        all: &mut Vec<Handler>,
        parsed: Parsed,
        source: &str,
        present: bool,
        path: Option<PathBuf>,
    ) {
        out.sources.push(Source {
            source: source.to_string(),
            present,
            enabled: parsed.enabled,
            handlers: parsed.handlers.len(),
            path,
        });
        out.warnings.extend(parsed.warnings);
        if parsed.enabled {
            all.extend(parsed.handlers);
        }
    }

    if let Some(layout) = layout {
        let doc = inertia_store::collections::read_document(
            layout,
            inertia_store::layout::Document::Hooks,
            Value::Null,
        );
        let present = !doc.is_null();
        take(
            &mut out,
            &mut all,
            parse_config(Some(&doc), "workspace"),
            "workspace",
            present,
            None,
        );
    }

    if let Some(cwd) = cwd {
        let file = cwd.join(PROJECT_FILE);
        let (doc, error) = match std::fs::read_to_string(&file) {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(doc) => (Some(doc), None),
                Err(e) => (None, Some(e.to_string())),
            },
            // A project with no hooks file is the normal case, not a problem.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, None),
            Err(e) => (None, Some(e.to_string())),
        };
        if let Some(error) = error {
            out.warnings.push(format!("{}: {error}", file.display()));
        }
        let present = doc.is_some();
        take(
            &mut out,
            &mut all,
            parse_config(doc.as_ref(), "project"),
            "project",
            present,
            Some(file),
        );
    }

    if let Some((name, hooks)) = agent.filter(|(_, hooks)| hooks.is_object()) {
        take(
            &mut out,
            &mut all,
            parse_config(Some(hooks), &format!("agent:{name}")),
            "agent",
            true,
            None,
        );
    }

    let (enabled, disabled): (Vec<_>, Vec<_>) = all.into_iter().partition(|h| h.enabled);
    out.handlers = enabled;
    out.disabled = disabled;
    out
}

/// Does this handler apply to this event?
///
/// The matcher is a regular expression tried against the whole subject - a
/// tool name, a subagent's name - the way Claude Code does it, and `*` or an
/// empty matcher means everything. A pattern that will not compile is treated
/// as a literal, because a hook that silently never fires is worse than one
/// that fires on the wrong thing and gets noticed.
pub fn matches(handler: &Handler, event: &str, subject: Option<&str>) -> bool {
    if handler.event != event {
        return false;
    }
    let pattern = handler.matcher.trim();
    if pattern.is_empty() || pattern == "*" {
        return true;
    }
    if matched_by(event).is_none() {
        return true;
    }
    let value = subject.unwrap_or_default();

    // Anchored: `shell` means the shell tool, not every tool with shell in its
    // name. Anyone who wants the family writes `shell.*`.
    match regex::Regex::new(&format!("^(?:{pattern})$")) {
        Ok(re) => re.is_match(value),
        Err(_) => pattern.split('|').any(|p| p.trim() == value),
    }
}

pub fn select<'a>(
    handlers: &'a [Handler],
    event: &str,
    subject: Option<&str>,
) -> Vec<&'a Handler> {
    handlers
        .iter()
        .filter(|handler| matches(handler, event, subject))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_settings_shape_and_the_plugin_shape_both_parse() {
        let flat = json!({ "PreToolUse": [{ "hooks": [{ "command": "echo hi" }] }] });
        let wrapped = json!({ "hooks": flat });
        assert_eq!(parse_config(Some(&flat), "t").handlers.len(), 1);
        assert_eq!(parse_config(Some(&wrapped), "t").handlers.len(), 1);
    }

    #[test]
    fn a_lone_handler_where_a_group_belongs_still_loads() {
        let doc = json!({ "Stop": [{ "command": "echo done" }] });
        let parsed = parse_config(Some(&doc), "t");
        assert_eq!(parsed.handlers.len(), 1);
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn an_unknown_event_is_named_rather_than_ignored() {
        let doc = json!({ "WhenIFeelLikeIt": [] });
        let parsed = parse_config(Some(&doc), "t");
        assert!(parsed.handlers.is_empty());
        assert!(parsed.warnings[0].contains("is not a hook event"));
        // And the message says what the events are, so the fix does not need
        // the documentation.
        assert!(parsed.warnings[0].contains("PreToolUse"));
    }

    #[test]
    fn a_matcher_on_an_unmatched_event_is_a_warning_not_a_failure() {
        let doc = json!({ "Stop": [{ "matcher": "shell", "hooks": [{ "command": "x" }] }] });
        let parsed = parse_config(Some(&doc), "t");
        assert_eq!(parsed.handlers.len(), 1);
        assert!(parsed.warnings[0].contains("not matched against anything"));
    }

    #[test]
    fn a_handler_with_neither_command_nor_prompt_is_reported() {
        let doc = json!({ "Stop": [{ "hooks": [{ "name": "empty", "type": "prompt" }] }] });
        let parsed = parse_config(Some(&doc), "t");
        assert!(parsed.handlers.is_empty());
        assert!(parsed.warnings[0].contains("needs a command or a prompt"));
    }

    #[test]
    fn disabling_a_group_removes_its_handlers_without_a_warning() {
        let doc = json!({
            "Stop": [{ "enabled": false, "hooks": [{ "command": "x" }] }],
        });
        let parsed = parse_config(Some(&doc), "t");
        assert!(parsed.handlers.is_empty());
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn a_disabled_file_keeps_its_handlers_out_of_the_turn() {
        let doc = json!({ "enabled": false, "Stop": [{ "hooks": [{ "command": "x" }] }] });
        let parsed = parse_config(Some(&doc), "t");
        assert!(!parsed.enabled);
        // Still parsed, so the pane can say how many are in the file it is not
        // running.
        assert_eq!(parsed.handlers.len(), 1);
    }

    #[test]
    fn timeouts_default_by_kind_and_are_capped() {
        let doc = json!({
            "Stop": [{ "hooks": [
                { "command": "x" },
                { "type": "prompt", "prompt": "p" },
                { "command": "y", "timeout": 99999 },
                { "command": "z", "timeout": -4 },
            ] }],
        });
        let ms: Vec<u64> = parse_config(Some(&doc), "t")
            .handlers
            .iter()
            .map(|h| h.timeout_ms)
            .collect();
        assert_eq!(ms, vec![60_000, 30_000, 600_000, 60_000]);
    }

    #[test]
    fn a_matcher_is_anchored() {
        let doc = json!({ "PreToolUse": [{ "matcher": "shell", "hooks": [{ "command": "x" }] }] });
        let handler = &parse_config(Some(&doc), "t").handlers[0];
        assert!(matches(handler, "PreToolUse", Some("shell")));
        assert!(!matches(handler, "PreToolUse", Some("shell_script")));
        assert!(!matches(handler, "PostToolUse", Some("shell")));
    }

    #[test]
    fn a_matcher_that_will_not_compile_is_read_literally() {
        let doc =
            json!({ "PreToolUse": [{ "matcher": "edit|write|(", "hooks": [{ "command": "x" }] }] });
        let handler = &parse_config(Some(&doc), "t").handlers[0];
        assert!(matches(handler, "PreToolUse", Some("edit")));
        assert!(matches(handler, "PreToolUse", Some("write")));
        assert!(!matches(handler, "PreToolUse", Some("read")));
    }

    #[test]
    fn a_star_or_an_empty_matcher_takes_everything() {
        let doc = json!({
            "PreToolUse": [
                { "matcher": "*", "hooks": [{ "command": "a" }] },
                { "hooks": [{ "command": "b" }] },
            ],
        });
        let handlers = parse_config(Some(&doc), "t").handlers;
        assert_eq!(select(&handlers, "PreToolUse", Some("anything")).len(), 2);
    }

    #[test]
    fn the_project_file_layers_over_the_workspace_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".inertia")).unwrap();
        std::fs::write(
            dir.path().join(PROJECT_FILE),
            r#"{ "Stop": [{ "hooks": [{ "command": "repo" }] }] }"#,
        )
        .unwrap();

        let loaded = load(None, Some(dir.path()), None);
        assert_eq!(loaded.handlers.len(), 1);
        assert_eq!(loaded.handlers[0].source, "project");
        assert!(loaded.sources.iter().any(|s| s.present));
    }

    #[test]
    fn a_project_with_no_hooks_file_is_not_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(None, Some(dir.path()), None);
        assert!(loaded.warnings.is_empty());
        assert!(!loaded.sources[0].present);
    }

    #[test]
    fn unreadable_json_says_which_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".inertia")).unwrap();
        std::fs::write(dir.path().join(PROJECT_FILE), "{ not json").unwrap();
        let loaded = load(None, Some(dir.path()), None);
        assert!(loaded.warnings[0].contains("hooks.json"));
    }

    #[test]
    fn a_switched_off_handler_is_listed_separately_rather_than_dropped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".inertia")).unwrap();
        std::fs::write(
            dir.path().join(PROJECT_FILE),
            r#"{ "Stop": [{ "hooks": [{ "command": "off", "enabled": false }] }] }"#,
        )
        .unwrap();
        let loaded = load(None, Some(dir.path()), None);
        assert!(loaded.handlers.is_empty());
        assert_eq!(loaded.disabled.len(), 1);
    }

    #[test]
    fn an_agent_brings_its_own_standing_orders() {
        let hooks = json!({ "Stop": [{ "hooks": [{ "command": "agent-only" }] }] });
        let loaded = load(None, None, Some(("reviewer", &hooks)));
        assert_eq!(loaded.handlers.len(), 1);
        assert_eq!(loaded.handlers[0].source, "agent:reviewer");
    }
}

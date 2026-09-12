//! What a handler decided, in one shape whatever it was.

use serde_json::Value;

/// The decision a `PreToolUse` or `PermissionRequest` handler can return.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Decision {
    /// Deliberately ordered: the most cautious answer wins a merge, so `deny`
    /// from one handler is not undone by `allow` from another.
    Allow,
    Ask,
    Deny,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}

/// One handler's verdict, or the merge of several.
///
/// - `block`        the event may not proceed (only honoured for blockable events)
/// - `reason`       why, written for the model
/// - `decision`     PreToolUse / PermissionRequest: allow, deny or ask
/// - `updated_input` PreToolUse: replacement arguments
/// - `context`      text to hand the model, accumulated across handlers
/// - `message`      text for the person, shown in the transcript
/// - `stop`         `continue: false` - end the turn altogether
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub block: bool,
    pub reason: Option<String>,
    pub decision: Option<Decision>,
    pub updated_input: Option<Value>,
    pub context: Vec<String>,
    pub message: Option<String>,
    pub stop: bool,
    pub stop_reason: Option<String>,
}

fn trimmed(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Read Claude Code's output contract, in every spelling it has had.
///
/// Every spelling on purpose: this format is the one people already have
/// scripts for, and a hook written against Claude Code's documentation - or
/// against the older shape of it - has to run here unchanged, or the
/// compatibility is only nominal.
pub fn parse_output(
    json: Option<&Value>,
    event: &str,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Outcome {
    let mut out = Outcome::default();

    if exit_code == Some(2) {
        // The blocking exit. What was written to stderr is the reason.
        out.block = true;
        out.reason = Some(match (stderr.trim(), stdout.trim()) {
            ("", "") => "Blocked by a hook.".to_string(),
            ("", text) => text.to_string(),
            (text, _) => text.to_string(),
        });
        if event == "PreToolUse" || event == "PermissionRequest" {
            out.decision = Some(Decision::Deny);
        }
        return out;
    }

    let Some(json) = json.filter(|j| j.is_object()) else {
        // Plain text on stdout is context for the model on the events where
        // context makes sense, and otherwise a note for the person.
        let text = stdout.trim();
        if !text.is_empty() {
            if matches!(
                event,
                "UserPromptSubmit" | "SessionStart" | "PostToolUse" | "SubagentStart"
            ) {
                out.context.push(text.to_string());
            } else {
                out.message = Some(text.to_string());
            }
        }
        return out;
    };

    if json.get("continue") == Some(&Value::Bool(false)) {
        out.stop = true;
        out.stop_reason = trimmed(json.get("stopReason"));
    }
    out.message = trimmed(json.get("systemMessage"));

    let empty = Value::Object(Default::default());
    let specific = json
        .get("hookSpecificOutput")
        .filter(|s| s.is_object())
        .unwrap_or(&empty);

    let decision = json.get("decision").and_then(Value::as_str);
    if decision == Some("block") || decision == Some("deny") {
        out.block = true;
        out.reason = trimmed(json.get("reason"))
            .or_else(|| trimmed(specific.get("permissionDecisionReason")))
            .or_else(|| Some("Blocked by a hook.".into()));
    }

    let permission = specific
        .get("permissionDecision")
        .and_then(Value::as_str)
        .or(match decision {
            Some("approve") | Some("allow") => Some("allow"),
            _ => None,
        });
    match permission {
        Some("allow") => out.decision = Some(Decision::Allow),
        Some("ask") => out.decision = Some(Decision::Ask),
        Some("deny") => {
            out.decision = Some(Decision::Deny);
            out.block = true;
            out.reason = trimmed(specific.get("permissionDecisionReason"))
                .or_else(|| trimmed(json.get("reason")))
                .or_else(|| Some("Denied by a hook.".into()));
        }
        _ => {}
    }

    // PermissionRequest's own spelling:
    // `{ decision: { behavior: "allow" | "deny", message } }`.
    if let Some(behaviour) = specific
        .get("decision")
        .filter(|d| d.is_object())
        .and_then(|d| d.get("behavior"))
        .and_then(Value::as_str)
    {
        if behaviour == "allow" {
            out.decision = Some(Decision::Allow);
        } else {
            out.decision = Some(Decision::Deny);
            out.block = true;
            out.reason = specific
                .get("decision")
                .and_then(|d| trimmed(d.get("message")))
                .or_else(|| Some("Denied by a hook.".into()));
        }
    }

    if let Some(input) = specific.get("updatedInput").filter(|i| i.is_object()) {
        out.updated_input = Some(input.clone());
    }

    if let Some(extra) =
        trimmed(specific.get("additionalContext")).or_else(|| trimmed(json.get("additionalContext")))
    {
        out.context.push(extra);
    }

    out
}

/// JSON on stdout, if that is what it is.
///
/// A trailing log line does not spoil it: a hook that prints a note and then
/// its verdict is doing something reasonable, and refusing to read it would
/// make every `echo` in a script a silent failure.
pub fn extract_json(stdout: &str) -> Option<Value> {
    let text = stdout.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str(text) {
        return Some(value);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exit_two_blocks_with_stderr_as_the_reason() {
        let out = parse_output(None, "PreToolUse", Some(2), "", "No force pushes here.");
        assert!(out.block);
        assert_eq!(out.reason.as_deref(), Some("No force pushes here."));
        assert_eq!(out.decision, Some(Decision::Deny));
    }

    #[test]
    fn exit_two_with_nothing_written_still_says_something() {
        let out = parse_output(None, "Stop", Some(2), "", "  ");
        assert_eq!(out.reason.as_deref(), Some("Blocked by a hook."));
    }

    #[test]
    fn plain_text_is_context_where_context_makes_sense() {
        let out = parse_output(None, "UserPromptSubmit", Some(0), "today is friday", "");
        assert_eq!(out.context, vec!["today is friday"]);
        assert!(out.message.is_none());
    }

    #[test]
    fn plain_text_elsewhere_is_a_note_for_the_person() {
        let out = parse_output(None, "Notification", Some(0), "done", "");
        assert_eq!(out.message.as_deref(), Some("done"));
        assert!(out.context.is_empty());
    }

    #[test]
    fn the_permission_spelling_is_read() {
        let json = json!({
            "hookSpecificOutput": {
                "permissionDecision": "deny",
                "permissionDecisionReason": "migrations are off limits",
            },
        });
        let out = parse_output(Some(&json), "PreToolUse", Some(0), "", "");
        assert_eq!(out.decision, Some(Decision::Deny));
        assert!(out.block);
        assert_eq!(out.reason.as_deref(), Some("migrations are off limits"));
    }

    #[test]
    fn permission_requests_own_spelling_is_read_too() {
        let json = json!({
            "hookSpecificOutput": { "decision": { "behavior": "deny", "message": "nope" } },
        });
        let out = parse_output(Some(&json), "PermissionRequest", Some(0), "", "");
        assert_eq!(out.decision, Some(Decision::Deny));
        assert_eq!(out.reason.as_deref(), Some("nope"));
    }

    #[test]
    fn approve_is_a_spelling_of_allow() {
        let json = json!({ "decision": "approve" });
        let out = parse_output(Some(&json), "PreToolUse", Some(0), "", "");
        assert_eq!(out.decision, Some(Decision::Allow));
        assert!(!out.block);
    }

    #[test]
    fn continue_false_stops_the_turn() {
        let json = json!({ "continue": false, "stopReason": "the tests are red" });
        let out = parse_output(Some(&json), "Stop", Some(0), "", "");
        assert!(out.stop);
        assert_eq!(out.stop_reason.as_deref(), Some("the tests are red"));
    }

    #[test]
    fn updated_input_replaces_the_arguments() {
        let json = json!({ "hookSpecificOutput": { "updatedInput": { "command": "ls" } } });
        let out = parse_output(Some(&json), "PreToolUse", Some(0), "", "");
        assert_eq!(out.updated_input, Some(json!({ "command": "ls" })));
    }

    #[test]
    fn json_after_a_log_line_is_still_found() {
        let found = extract_json("starting up\n{\"decision\":\"block\"}\n").unwrap();
        assert_eq!(found["decision"], "block");
    }

    #[test]
    fn nothing_that_is_not_json_comes_back_as_nothing() {
        assert!(extract_json("").is_none());
        assert!(extract_json("all good").is_none());
        assert!(extract_json("} {").is_none());
    }

    #[test]
    fn deny_outranks_ask_outranks_allow() {
        assert!(Decision::Deny > Decision::Ask);
        assert!(Decision::Ask > Decision::Allow);
    }
}

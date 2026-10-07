//! Running one handler: a program, or a question put to the model.

use crate::config::Handler;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// How much of a handler's stdout and stderr is kept.
const MAX_OUTPUT_CHARS: usize = 20_000;

/// What happened when a handler ran, before anyone read its verdict.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub status: Status,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Failed,
    Timeout,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
        }
    }
}

fn clip(text: &str) -> String {
    if text.chars().count() > MAX_OUTPUT_CHARS {
        let kept: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
        format!("{kept}\n... (truncated)")
    } else {
        text.to_string()
    }
}

/// A question put to the model that is already running the turn.
///
/// A trait rather than a closure because the app supplies it from the turn's
/// own provider and model, and the crate must not know which. Nothing here can
/// name Anthropic, and nothing here can name Tauri.
#[async_trait::async_trait]
pub trait Ask: Send + Sync {
    async fn ask(&self, system: &str, prompt: &str) -> Result<String, String>;
}

fn millis(started: std::time::Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

/// Run one command handler.
///
/// Exec form (`command` plus `args`) spawns directly, so a path with a space in
/// it needs no quoting. String form goes through the platform shell, the same
/// one the shell tool uses, so a hook written for this machine's terminal works
/// here too.
pub async fn run_command(
    handler: &Handler,
    input: &Value,
    cwd: Option<&Path>,
    env: &HashMap<String, String>,
) -> RunResult {
    let started = std::time::Instant::now();
    let command = handler.command.clone().unwrap_or_default();

    let mut spawner = match &handler.args {
        Some(args) => {
            let mut cmd = tokio::process::Command::new(&command);
            cmd.args(args);
            cmd
        }
        None => shell_command(&command),
    };

    if let Some(cwd) = cwd {
        spawner.current_dir(cwd);
    }
    spawner
        .envs(env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // The hook is the user's own program and has no console of its own; on
    // Windows a spawned child otherwise flashes one up over their work.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        spawner.creation_flags(CREATE_NO_WINDOW);
    }
    // A backstop only. The tree kill below is what actually clears a timed-out
    // hook; dropping the handle reaches the shell and not what the shell ran.
    spawner.kill_on_drop(true);

    let mut child = match spawner.spawn() {
        Ok(child) => child,
        Err(error) => {
            return RunResult {
                status: Status::Failed,
                error: Some(format!("Could not start: {error}")),
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: millis(started),
            }
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let payload = serde_json::to_vec(input).unwrap_or_default();
        // A child that closed stdin at once is allowed to; it may not want the
        // event at all.
        let _ = stdin.write_all(&payload).await;
        let _ = stdin.shutdown().await;
    }

    // The pipes are read alongside the wait rather than after it, and the wait
    // is on the child itself rather than on `wait_with_output`. Both matter on
    // a timeout: `wait_with_output` consumes the child, so by the time the
    // timeout elapsed there was no live process left to kill the tree of, and
    // the orphaned grandchild went on holding the inherited stdout pipe. A
    // hook cut off at 300ms was still keeping a reader thread - and the app's
    // own shutdown - waiting the full thirty seconds it was supposed to be
    // stopped at.
    let reading = tokio::spawn(drain(child.stdout.take(), child.stderr.take()));
    let pid = child.id();

    let waited =
        tokio::time::timeout(Duration::from_millis(handler.timeout_ms), child.wait()).await;

    if waited.is_err() {
        kill_tree(pid).await;
        // And the handle too, for the platform where the tree kill is refused.
        let _ = child.kill().await;
    }
    let (stdout, stderr) = reading.await.unwrap_or_default();
    let duration_ms = millis(started);

    match waited {
        Err(_) => RunResult {
            status: Status::Timeout,
            error: Some(format!(
                "Timed out after {}s.",
                handler.timeout_ms as f64 / 1000.0
            )),
            exit_code: None,
            // Kept rather than discarded: a hook that printed its reasoning
            // and then hung has said something worth reading.
            stdout: clip(&stdout),
            stderr: clip(&stderr),
            duration_ms,
        },
        Ok(Err(error)) => RunResult {
            status: Status::Failed,
            error: Some(error.to_string()),
            exit_code: None,
            stdout: clip(&stdout),
            stderr: clip(&stderr),
            duration_ms,
        },
        Ok(Ok(status)) => {
            let code = status.code();
            // Exit 2 is the blocking exit and is a verdict, not a failure. A
            // hook that blocks correctly must not also be reported as broken.
            let ok = matches!(code, Some(0) | Some(2));
            RunResult {
                status: if ok { Status::Ok } else { Status::Failed },
                error: (!ok).then(|| match code {
                    Some(code) => format!("Exited with code {code}."),
                    None => "Stopped by a signal.".to_string(),
                }),
                exit_code: code,
                stdout: clip(&stdout),
                stderr: clip(&stderr),
                duration_ms,
            }
        }
    }
}

/// Read both pipes to the end, together.
///
/// Together because a hook that fills one pipe while nobody reads the other
/// deadlocks against the operating system's buffer, which is the classic way
/// to make a working script hang only on the machine with the chattiest
/// output.
async fn drain(
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
) -> (String, String) {
    use tokio::io::AsyncReadExt as _;

    async fn read<P: tokio::io::AsyncRead + Unpin>(pipe: Option<P>) -> String {
        let Some(mut pipe) = pipe else {
            return String::new();
        };
        let mut bytes = Vec::new();
        // A read that fails has still given us whatever arrived before it did,
        // and a hook's partial output is worth more than an empty string.
        let _ = pipe.read_to_end(&mut bytes).await;
        String::from_utf8_lossy(&bytes).into_owned()
    }

    futures::future::join(read(stdout), read(stderr)).await
}

/// Kill a hook and everything it started.
///
/// Best effort by construction: the process may already be gone, and a hook
/// that will not die is not worth failing a turn over. Nothing here waits on
/// the result for that reason.
async fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };

    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("taskkill");
        cmd.args(["/pid", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.status().await;
    }

    #[cfg(not(windows))]
    {
        // Negative pid is the process group, which `sh -c` and its children
        // share. Falling back to the single process keeps this useful where
        // the group call is refused.
        let group = tokio::process::Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
        if !group.map(|s| s.success()).unwrap_or(false) {
            let _ = tokio::process::Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
        }
    }
}

/// `cmd /C` on Windows and `sh -c` elsewhere - the same choice the shell tool
/// makes, because a hook is written by the same person in the same syntax.
fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

const PROMPT_SYSTEM: &str = concat!(
    "You are evaluating a lifecycle hook for a coding agent. You will be shown the ",
    "hook's instruction and the event as JSON. Decide, then answer with ONE JSON ",
    r#"object and nothing else: {"decision":"approve"|"block","reason":"<why, one or "#,
    r#"two sentences, written for the agent>"}. Use "block" only when the instruction "#,
    "clearly says this should not proceed. When in doubt, approve."
);

/// Run one prompt handler: put the question to the model.
///
/// The model is told the event as JSON and asked for a JSON verdict, which is
/// the one shape every model can be relied on to produce when asked for
/// nothing else. Anything that is not a verdict is read as approval with the
/// text kept as a note, because a hook that blocks the agent every time the
/// model is chatty is a hook nobody keeps.
pub async fn run_prompt(handler: &Handler, input: &Value, ask: Option<&dyn Ask>) -> RunResult {
    let started = std::time::Instant::now();
    let Some(ask) = ask else {
        return RunResult {
            status: Status::Failed,
            error: Some("No model is available to evaluate a prompt hook here.".into()),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: 0,
        };
    };

    let body = [
        "Hook instruction:".to_string(),
        expand(handler.prompt.as_deref().unwrap_or_default(), input),
        String::new(),
        "Event:".to_string(),
        {
            let json = serde_json::to_string_pretty(input).unwrap_or_default();
            json.chars().take(40_000).collect()
        },
    ]
    .join("\n");

    match tokio::time::timeout(
        Duration::from_millis(handler.timeout_ms),
        ask.ask(PROMPT_SYSTEM, &body),
    )
    .await
    {
        Err(_) => RunResult {
            status: Status::Timeout,
            error: Some(format!(
                "Timed out after {}s.",
                handler.timeout_ms as f64 / 1000.0
            )),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: millis(started),
        },
        Ok(Err(error)) => RunResult {
            status: Status::Failed,
            error: Some(error),
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms: millis(started),
        },
        Ok(Ok(text)) => RunResult {
            status: Status::Ok,
            error: None,
            exit_code: Some(0),
            stdout: clip(&text),
            stderr: String::new(),
            duration_ms: millis(started),
        },
    }
}

/// The `$VARIABLES` a prompt hook may refer to, filled from the event.
fn expand(prompt: &str, input: &Value) -> String {
    let json = |key: &str| {
        serde_json::to_string(input.get(key).unwrap_or(&Value::Null)).unwrap_or("null".into())
    };
    prompt
        .replace("$TOOL_INPUT", &json("tool_input"))
        .replace("$TOOL_RESULT", &json("tool_response"))
        .replace("$TOOL_RESPONSE", &json("tool_response"))
        .replace("$USER_PROMPT", &json("prompt"))
        .replace(
            "$TRANSCRIPT_PATH",
            input
                .get("transcript_path")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_config;
    use serde_json::json;

    fn stop_hook(raw: &Value) -> Handler {
        let doc = json!({ "Stop": [{ "hooks": [raw] }] });
        parse_config(Some(&doc), "t").handlers.remove(0)
    }

    #[tokio::test]
    async fn a_command_gets_the_event_on_stdin_and_answers_on_stdout() {
        // `sort` is everywhere and does the one thing needed here: prove the
        // bytes we wrote reached the child.
        let hook = stop_hook(&json!({ "command": "sort" }));
        let result = run_command(&hook, &json!({ "a": 1 }), None, &HashMap::new()).await;
        assert_eq!(result.status, Status::Ok);
        assert!(result.stdout.contains("\"a\""), "{}", result.stdout);
    }

    #[tokio::test]
    async fn exit_two_is_a_verdict_and_not_a_failure() {
        // `exit 2` in both shells, which is the point: the blocking exit is
        // written the same way whichever machine the hook came from.
        let hook = stop_hook(&json!({ "command": "exit 2" }));
        let result = run_command(&hook, &json!({}), None, &HashMap::new()).await;
        assert_eq!(result.status, Status::Ok);
        assert_eq!(result.exit_code, Some(2));
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn a_non_zero_exit_that_is_not_two_is_a_failure_with_its_code() {
        let hook = stop_hook(&json!({ "command": "exit 7" }));
        let result = run_command(&hook, &json!({}), None, &HashMap::new()).await;
        assert_eq!(result.status, Status::Failed);
        assert!(result.error.unwrap().contains('7'));
    }

    #[tokio::test]
    async fn a_command_that_does_not_exist_is_reported_rather_than_hanging() {
        let mut hook = stop_hook(&json!({ "command": "definitely-not-a-program-xyz" }));
        // Exec form, so there is no shell to turn this into an exit code.
        hook.args = Some(vec![]);
        let result = run_command(&hook, &json!({}), None, &HashMap::new()).await;
        assert_eq!(result.status, Status::Failed);
        assert!(result.error.unwrap().contains("Could not start"));
    }

    #[tokio::test]
    async fn a_hook_that_never_finishes_times_out() {
        let sleep = if cfg!(windows) {
            "ping -n 30 127.0.0.1 > nul"
        } else {
            "sleep 30"
        };
        let mut hook = stop_hook(&json!({ "command": sleep }));
        hook.timeout_ms = 300;
        let result = run_command(&hook, &json!({}), None, &HashMap::new()).await;
        assert_eq!(result.status, Status::Timeout);
    }

    #[tokio::test]
    async fn the_environment_reaches_the_hook() {
        let read = if cfg!(windows) {
            "echo %INERTIA_HOOK_EVENT%"
        } else {
            "echo $INERTIA_HOOK_EVENT"
        };
        let hook = stop_hook(&json!({ "command": read }));
        let env = HashMap::from([("INERTIA_HOOK_EVENT".to_string(), "Stop".to_string())]);
        let result = run_command(&hook, &json!({}), None, &env).await;
        assert!(result.stdout.contains("Stop"), "{}", result.stdout);
    }

    #[test]
    fn output_past_the_cap_is_truncated_rather_than_kept() {
        let long = "x".repeat(MAX_OUTPUT_CHARS + 50);
        assert!(clip(&long).ends_with("(truncated)"));
        assert_eq!(clip("short"), "short");
    }

    #[test]
    fn a_prompt_can_name_the_tool_input() {
        let filled = expand(
            "Check $TOOL_INPUT and $USER_PROMPT",
            &json!({ "tool_input": { "command": "rm -rf /" }, "prompt": "tidy up" }),
        );
        assert!(filled.contains(r#"{"command":"rm -rf /"}"#));
        assert!(filled.contains(r#""tidy up""#));
    }

    #[test]
    fn an_absent_variable_becomes_null_rather_than_the_literal_name() {
        assert_eq!(expand("$TOOL_RESULT", &json!({})), "null");
    }

    struct Fixed(&'static str);

    #[async_trait::async_trait]
    impl Ask for Fixed {
        async fn ask(&self, _system: &str, _prompt: &str) -> Result<String, String> {
            Ok(self.0.to_string())
        }
    }

    #[tokio::test]
    async fn a_prompt_hook_asks_the_model() {
        let hook = stop_hook(&json!({ "type": "prompt", "prompt": "is this fine?" }));
        let result = run_prompt(&hook, &json!({}), Some(&Fixed(r#"{"decision":"block"}"#))).await;
        assert_eq!(result.status, Status::Ok);
        assert!(result.stdout.contains("block"));
    }

    #[tokio::test]
    async fn a_prompt_hook_with_no_model_says_so_rather_than_blocking() {
        let hook = stop_hook(&json!({ "type": "prompt", "prompt": "is this fine?" }));
        let result = run_prompt(&hook, &json!({}), None).await;
        assert_eq!(result.status, Status::Failed);
        assert!(result.error.unwrap().contains("No model"));
    }
}

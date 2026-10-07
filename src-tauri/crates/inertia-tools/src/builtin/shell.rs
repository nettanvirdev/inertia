//! `shell`.
//!
//! The one tool that can do anything, which is exactly why almost all of the
//! work here is about asking the right question before it does. Two things
//! make that possible: `command.rs`, which reads enough of the command line
//! to name what is about to happen, and the permission request below, which
//! asks about `git push origin main` but remembers `git push *`.
//!
//! The description is assembled at first use from the actual platform and the
//! actual shell, the way opencode does it. That is not cosmetic. A model told
//! it is on Windows PowerShell stops writing `head -n 20` and `&&` chains, and
//! the difference in how many turns a session wastes on shell syntax errors is
//! not small.
//!
//! A non-zero exit is **not** an error. A failing test run or a compiler error
//! is the answer to the question the agent asked, and the exit code plus the
//! output is exactly what it needs to act. Reporting it as a tool failure
//! would hide the output behind an error envelope.

// Declared from here rather than from `mod.rs` so the parser ships with the
// tool that needs it; `#[path]` keeps the file beside its siblings.
#[path = "command.rs"]
pub mod command;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use super::background::{hard_kill, Background};
use super::fence;
use crate::truncate::DEFAULT_LIMIT;

// ── which shell ────────────────────────────────────────────────────────────

/// The shell every command runs through.
#[derive(Debug)]
struct Shell {
    program: String,
    /// What the description calls it.
    name: String,
    /// PowerShell 7 or a POSIX shell: `&&` works. Windows PowerShell 5.1: it
    /// does not.
    modern: bool,
    args: Vec<&'static str>,
}

/// Is `name` runnable from PATH. Used once, to prefer pwsh.
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path).any(|dir| {
        extensions
            .iter()
            .any(|ext| dir.join(format!("{name}{ext}")).is_file())
    })
}

/// PowerShell 7 if the user has it, Windows PowerShell 5.1 otherwise.
///
/// The difference matters enough to detect rather than assume: 5.1 has no
/// `&&`, no ternary and no null-coalescing, and a model that thinks it has
/// them will produce a parser error on its first chained command.
fn pick_shell() -> Shell {
    if cfg!(windows) {
        let modern = on_path("pwsh");
        return Shell {
            program: if modern { "pwsh" } else { "powershell.exe" }.to_string(),
            name: if modern {
                "PowerShell 7 (pwsh)"
            } else {
                "Windows PowerShell 5.1"
            }
            .to_string(),
            modern,
            // -NonInteractive so a cmdlet that wants confirmation errors
            // instead of waiting forever; -NoProfile so the user's profile
            // cannot change what a command means between one machine and the
            // next.
            //
            // -ExecutionPolicy Bypass is not a loosening of the user's
            // machine. It applies to this one child process and nothing else,
            // and without it the default Restricted policy refuses to run any
            // `.ps1` - which on Windows includes `npm` and `npx`, because
            // those are shipped as PowerShell shims. Measured on a real
            // session: every `npm install` came back "npm.ps1 cannot be
            // loaded because running scripts is disabled on this system", the
            // agent spent four turns and ninety seconds inventing `cmd /c`
            // wrappers and fighting the quoting, and the build never started.
            // A command this app was explicitly asked to run has already been
            // consented to through the permission card in front of it.
            args: vec![
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
            ],
        };
    }
    let program = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/bash".to_string());
    let name = Path::new(&program)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| program.clone());
    Shell {
        program,
        name,
        modern: true,
        args: vec!["-c"],
    }
}

fn shell() -> &'static Shell {
    static SHELL: OnceLock<Shell> = OnceLock::new();
    SHELL.get_or_init(pick_shell)
}

/// What `shell` actually runs commands with, for the prompt to name.
///
/// Exported from here rather than restated in the prompt builder, because the
/// only thing worse than not telling a model which shell it has is telling it
/// the wrong one: a model that believes it is writing bash on a machine
/// running PowerShell spends its first two calls on syntax errors, and every
/// one of those is a round trip the person waits through.
pub fn shell_name() -> &'static str {
    &shell().name
}

/// PowerShell's `-Command` reports its own success, not the command's.
///
/// It exits 0 or 1 whatever the program inside it returned, so a test runner
/// that signals different failures with different codes comes back as 1 and
/// the model is told something false. The real code is in `$LASTEXITCODE`, so
/// it is handed back explicitly.
///
/// Guarded on `$LASTEXITCODE` actually being a non-zero number because it is
/// only set by native executables: a command that was pure cmdlets leaves it
/// null or stale, and in that case falling through to PowerShell's own exit
/// code is the honest answer. The trailing line is separated by a newline so
/// it cannot be swallowed by a `#` comment at the end of the command.
fn wrap(command: &str) -> String {
    if !cfg!(windows) {
        return command.to_string();
    }
    format!(
        "{command}\nif ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}"
    )
}

fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

// ── limits ─────────────────────────────────────────────────────────────────

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;

/// How long a killed process gets to actually go away, and its pipes to
/// close, before the output is read as it stands.
const GRACE: Duration = Duration::from_secs(3);

// ── the description the model reads ────────────────────────────────────────

fn description() -> &'static str {
    static DESCRIPTION: OnceLock<String> = OnceLock::new();
    DESCRIPTION.get_or_init(|| {
        let sh = shell();
        let notes = if cfg!(windows) {
            let chains = if sh.modern {
                "- `&&` and `||` work in PowerShell 7."
            } else {
                "- `&&` and `||` DO NOT EXIST in Windows PowerShell 5.1 and are a parser error. Use `;` to chain, or `if ($?) { ... }` to run something only when the previous command succeeded."
            };
            format!(
                "Shell: {}. It is PowerShell, not cmd.exe and not bash.\n\
                 {chains}\n\
                 - Unix commands do not exist here. Use: `Get-Content f -TotalCount 20` for head, `Get-Content f -Tail 20` for tail, `(Get-Command x).Source` for which, `New-Item -ItemType File f` for touch, `Remove-Item -Recurse -Force d` for rm -rf, `New-Item -ItemType Directory -Force d` for mkdir -p, `2>$null` for 2>/dev/null.\n\
                 - Environment variables are `$env:NAME`, not `%NAME%` and not `$NAME`.\n\
                 - Quote paths containing spaces, and call executables with spaces in their path through the call operator: `& \"C:\\Program Files\\app\\app.exe\"`.",
                sh.name
            )
        } else {
            format!(
                "Shell: {} at {}. `&&`, `||`, pipes and redirection all work as usual.",
                sh.name, sh.program
            )
        };

        format!(
            "Run a command in the terminal on the user's machine.\n\
             \n\
             Platform: {}.\n\
             {notes}\n\
             \n\
             Use this for terminal operations: git, npm, docker, build tools, package managers, running tests.\n\
             \n\
             DO NOT use this to read, write, search for or find files. Use the dedicated tools instead - `read` to read a file, `write` to create one, `edit` to change one, `glob` to find files by name, `grep` to search their contents. They are faster, they do not depend on which utilities happen to be installed, and their output is built for you to read.\n\
             \n\
             The working directory persists between calls. Shell state does not: environment variables, `cd`, activated virtualenvs and shell functions are all gone by the next call, because each call is a fresh process. Use the `workdir` parameter instead of a `cd` command.\n\
             \n\
             Interactive commands will hang until the timeout and then be killed. Stdin is closed, so there is nobody to answer a prompt. Pass the non-interactive flag (`-y`, `--yes`, `--no-input`, `-NonInteractive`) rather than hoping.\n\
             \n\
             Output: stdout and stderr are merged in the order they arrived. Anything over {DEFAULT_LIMIT} characters is cut from the middle, keeping the start and the end, and the notice says how much was dropped. Default timeout {DEFAULT_TIMEOUT_MS} ms, maximum {MAX_TIMEOUT_MS} ms. Use `background: true` for a dev server or a watcher: it returns immediately with a pid, and its output is captured so you can read it later with `shell_logs`.",
            platform_name()
        )
    })
}

// ── the tool ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ShellTool {
    background: Arc<Background>,
}

impl ShellTool {
    pub fn new(background: Arc<Background>) -> Self {
        Self { background }
    }
}

fn dangers_json(warnings: &[command::Danger]) -> Value {
    Value::Array(
        warnings
            .iter()
            .map(|d| json!({ "pattern": d.pattern, "why": d.why }))
            .collect(),
    )
}

/// Where the command runs.
///
/// A missing folder is checked here, before the program is blamed: a spawn
/// failure for a working directory that does not exist reads as "could not
/// run powershell.exe", and a model told that concludes the machine has no
/// shell and stops trying. That happened - a project folder was deleted, ten
/// helpers were asked to run a command, and all ten reported back that the
/// computer had no shell.
fn resolve_workdir(workdir: Option<&str>, root: &Path) -> Result<PathBuf> {
    match workdir.map(str::trim).filter(|w| !w.is_empty()) {
        None => {
            if !root.is_dir() {
                return Err(Error::Other(format!(
                    "The working folder {} does not exist, so nothing could be run there. \
                     The folder may have been deleted or renamed. Point the agent at a folder \
                     that exists, or pass `workdir` to run somewhere else.",
                    root.display()
                )));
            }
            Ok(root.to_path_buf())
        }
        Some(relative) => {
            let candidate = Path::new(relative);
            let resolved = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                root.join(candidate)
            };
            match std::fs::metadata(&resolved) {
                Err(_) => Err(Error::Other(format!(
                    "The directory {} does not exist.",
                    resolved.display()
                ))),
                Ok(meta) if !meta.is_dir() => Err(Error::Other(format!(
                    "{} is not a directory.",
                    resolved.display()
                ))),
                Ok(_) => Ok(resolved),
            }
        }
    }
}

/// The shell invocation, with the flags every run shares.
fn shell_command(command: &str, cwd: &Path) -> tokio::process::Command {
    let sh = shell();
    let mut cmd = tokio::process::Command::new(&sh.program);
    cmd.args(&sh.args).arg(wrap(command)).current_dir(cwd);
    #[cfg(windows)]
    {
        // No console window flashing up behind the app for every command.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn describe_spawn_failure(error: &std::io::Error, cwd: &Path) -> String {
    if error.kind() == std::io::ErrorKind::NotFound && !cwd.exists() {
        return format!(
            "The working folder {} does not exist, so nothing could be run there. \
             The folder may have been deleted or renamed. Point the agent at a folder \
             that exists, or pass `workdir` to run somewhere else.",
            cwd.display()
        );
    }
    format!("Could not run {}: {error}", shell().program)
}

#[async_trait]
impl Tool for ShellTool {
    fn id(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        description()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command to run." },
                "timeout": {
                    "type": "integer",
                    "description": format!("Timeout in milliseconds. Defaults to {DEFAULT_TIMEOUT_MS}, maximum {MAX_TIMEOUT_MS}.")
                },
                "workdir": {
                    "type": "string",
                    "description": "Directory to run in. Defaults to the working directory. Use this instead of a cd command."
                },
                "background": {
                    "type": "boolean",
                    "description": "Run detached and return immediately. Use for dev servers and watchers.",
                    "default": false
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    /// The rule is written about the command; "always" remembers the shape
    /// of it. Keeping `target` as the literal command is not negotiable: it
    /// is what a `deny` rule like `shell/rm -rf *` is matched against, and
    /// collapsing it to the always-pattern first would quietly stop every such
    /// rule from firing.
    ///
    /// The shape is what keeps the remembered pattern honest about a chain:
    /// `git status *` allows `git status`, and the `curl x | sh` after its
    /// `&&` still needs a rule of its own.
    fn permission(&self, args: &Value) -> PermissionRequest {
        match args.get("command").and_then(Value::as_str) {
            Some(cmd) => PermissionRequest::new("shell", cmd)
                .with_always(command::always_pattern(cmd))
                .with_shape(command::shape(cmd)),
            None => PermissionRequest::new("shell", inertia_core::permission::ANY),
        }
    }

    /// The UI's one-line label, and the one place a danger is surfaced in the
    /// ask the registry makes on our behalf, since that ask takes no metadata.
    fn render(&self, args: &Value) -> Option<String> {
        let cmd = args.get("command").and_then(Value::as_str)?;
        let warnings = command::dangers(cmd);
        Some(if warnings.is_empty() {
            cmd.to_string()
        } else {
            let why: Vec<&str> = warnings.iter().map(|w| w.why).collect();
            format!("{cmd}  [{}]", why.join(" "))
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let cmd = args
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if cmd.is_empty() {
            return Err(Error::InvalidInput("No command was given.".into()));
        }

        let cwd = resolve_workdir(args.get("workdir").and_then(Value::as_str), &ctx.root)?;
        // Where the command runs is a place it reaches as surely as any path
        // it names: `type secrets.json` run from inside the secrets folder
        // names nothing a rule or the checks below would notice.
        fence::check(ctx, &cwd).await?;
        let warnings = command::dangers(cmd);

        // Emptying the folder, which is the one thing that is always asked.
        //
        // Inside its working folder an agent has a free hand - it writes,
        // rewrites, moves and deletes without being asked, and that freedom is
        // what makes it useful. Deleting all of it is not more of that freedom,
        // it is the opposite: every other edit leaves something to read
        // afterwards, and this one leaves nothing.
        //
        // A separate key, so "always allow shell" does not carry it, and no
        // `always` at all, so this cannot be turned off. The answer to "may I
        // delete everything" should not be inherited from the last time it was
        // a good idea.
        let sweeping = command::mass_delete(cmd);
        if let Some(first) = sweeping.first() {
            let request = PermissionRequest::new("delete_everything", cmd);
            if !ctx.permissions.ask(&request).await?.is_allowed() {
                return Err(Error::Denied(format!(
                    "`{cmd}` was not allowed: {}",
                    first.why
                )));
            }
        }

        // A different question, not a repeat of the same one: running a
        // command in the project and reaching into somebody's home directory
        // are separate decisions, and folding them together means only the
        // first gets made.
        for dir in command::external_paths(cmd, &cwd) {
            // A trailing separator would make the pattern for a drive root
            // read `D:\/*`, which is a rule nobody would recognise in the
            // settings list.
            let shown = dir.to_string_lossy();
            let pattern = format!("{}/*", shown.trim_end_matches(['\\', '/']));
            let request =
                PermissionRequest::new("external_directory", &pattern).with_always(&pattern);
            if !ctx.permissions.ask(&request).await?.is_allowed() {
                return Err(Error::Denied(format!(
                    "`{cmd}` touches {shown}, outside the working folder, and that was not allowed."
                )));
            }
        }

        if args
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return self.run_detached(cmd, &cwd);
        }
        self.run_and_wait(cmd, &cwd, &args, &warnings).await
    }
}

impl ShellTool {
    /// Start it, and keep the last of what it says.
    ///
    /// The pipes are held and a bounded tail is kept, rather than spawning
    /// with the streams ignored and handing back a bare pid. A pid with no way
    /// to learn anything ever printed made the single most common real
    /// workflow - start a dev server, hit it, read the error - impossible.
    fn run_detached(&self, cmd: &str, cwd: &Path) -> Result<ToolOutcome> {
        let mut spawner = shell_command(cmd, cwd);
        spawner
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // The app can quit and the dev server keeps running; `kill_all`
            // on quit is what stops it, not a dropped handle.
            .kill_on_drop(false);
        #[cfg(unix)]
        {
            // Its own process group, so it is not taken down with the app
            // and so a kill can be aimed at it alone.
            spawner.process_group(0);
        }

        let child = spawner
            .spawn()
            .map_err(|e| Error::Other(describe_spawn_failure(&e, cwd)))?;
        let pid = self.background.adopt(child, cmd);

        Ok(ToolOutcome {
            title: Some(cmd.to_string()),
            output: match pid {
                None => "The process did not start.".to_string(),
                Some(pid) => format!(
                    "Started in the background as pid {pid}. Output is being captured - read it with shell_logs, list what is running with shell_list, and stop it with shell_kill."
                ),
            },
            metadata: Some(json!({
                "command": cmd,
                "exitCode": Value::Null,
                "durationMs": 0,
                "truncated": false,
                "outputPath": Value::Null,
                "background": true,
                "pid": pid,
            })),
            images: Vec::new(),
        })
    }

    async fn run_and_wait(
        &self,
        cmd: &str,
        cwd: &Path,
        args: &Value,
        warnings: &[command::Danger],
    ) -> Result<ToolOutcome> {
        let timeout_ms = args
            .get("timeout")
            .and_then(Value::as_u64)
            .filter(|t| *t > 0)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .min(MAX_TIMEOUT_MS);
        let timeout = Duration::from_millis(timeout_ms);
        let started = Instant::now();

        let mut spawner = shell_command(cmd, cwd);
        spawner
            // stdin is closed, always. A command that stops to ask something
            // would otherwise sit there until the timeout with nobody on the
            // other end to answer it. Closing it turns a silent hang into an
            // immediate EOF, which most tools handle by failing with a message
            // the model can act on.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("TERM", "dumb")
            .env("NO_COLOR", "1")
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .kill_on_drop(true);

        let mut child = spawner
            .spawn()
            .map_err(|e| Error::Other(describe_spawn_failure(&e, cwd)))?;
        let pid = child.id();

        // One buffer for both streams, appended in arrival order. A build's
        // errors interleaved with its progress is the thing worth reading;
        // two separate blocks would put the failure a hundred lines away from
        // what caused it.
        let buffer = Arc::new(Mutex::new(String::new()));
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(tokio::spawn(absorb(Arc::clone(&buffer), out)));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(tokio::spawn(absorb(Arc::clone(&buffer), err)));
        }

        let (status, timed_out) = {
            let finished = async {
                for reader in readers.iter_mut() {
                    let _ = reader.await;
                }
                child.wait().await.ok()
            };
            tokio::pin!(finished);
            match tokio::time::timeout(timeout, &mut finished).await {
                Ok(status) => (status, false),
                Err(_) => {
                    if let Some(pid) = pid {
                        hard_kill(pid);
                    }
                    // Whatever it printed before it was killed is still worth
                    // reading, so give the pipes a moment to close.
                    let status = tokio::time::timeout(GRACE, &mut finished)
                        .await
                        .ok()
                        .flatten();
                    (status, true)
                }
            }
        };
        for reader in readers {
            reader.abort();
        }

        let duration_ms = started.elapsed().as_millis() as u64;
        let code = status.and_then(|s| s.code());
        let captured = buffer.lock().trim().to_string();
        let mut text = if captured.is_empty() {
            "(no output)".to_string()
        } else {
            captured
        };

        if timed_out {
            text = format!(
                "The command was killed after exceeding its {timeout_ms} ms timeout. If it needs longer and is not waiting for input, retry with a larger timeout.\n\n{text}"
            );
        } else if let Some(code) = code.filter(|c| *c != 0) {
            // Prepended rather than thrown: the output of a failed build is
            // the part the model needs, and an error would reduce it to a
            // sentence.
            text = format!("Exit code: {code}\n\n{text}");
        }

        let mut metadata = json!({
            "command": cmd,
            "exitCode": if timed_out { Value::Null } else { json!(code) },
            "durationMs": duration_ms,
            "truncated": false,
            "outputPath": Value::Null,
            "background": false,
            "pid": pid,
            "timedOut": timed_out,
        });
        if !warnings.is_empty() {
            metadata["dangers"] = dangers_json(warnings);
        }

        Ok(ToolOutcome {
            title: Some(cmd.to_string()),
            output: text,
            metadata: Some(metadata),
            images: Vec::new(),
        })
    }
}

async fn absorb<R: tokio::io::AsyncRead + Unpin>(buffer: Arc<Mutex<String>>, mut reader: R) {
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buffer
                .lock()
                .push_str(&String::from_utf8_lossy(&chunk[..n])),
        }
    }
}

/// The shell tool, sharing the background table with `background_tools`.
pub fn shell_tools(background: Arc<Background>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(ShellTool::new(background))]
}

#[cfg(test)]
mod tests {
    use super::super::background::background_tools;
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_core::tool::PermissionGate;
    use inertia_mock::MockGate;

    fn ctx(root: &Path, gate: Arc<MockGate>) -> ToolContext {
        ToolContext {
            root: root.to_path_buf(),
            session: SessionId::from_existing("t1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: gate as Arc<dyn PermissionGate>,
        }
    }

    fn tools() -> (ShellTool, Arc<Background>) {
        let bg = Arc::new(Background::new());
        (ShellTool::new(Arc::clone(&bg)), bg)
    }

    /// Commands in the syntax of whichever shell this host runs.
    fn say(text: &str) -> String {
        if cfg!(windows) {
            format!("Write-Output \"{text}\"")
        } else {
            format!("echo \"{text}\"")
        }
    }

    fn exit_with(code: i32) -> String {
        if cfg!(windows) {
            // Native exit code, so `$LASTEXITCODE` is what carries it.
            format!("cmd /c \"exit {code}\"")
        } else {
            format!("sh -c 'exit {code}'")
        }
    }

    fn sleep_for(seconds: u32) -> String {
        if cfg!(windows) {
            format!("Start-Sleep -Seconds {seconds}")
        } else {
            format!("sleep {seconds}")
        }
    }

    fn counting() -> &'static str {
        if cfg!(windows) {
            "1..3 | ForEach-Object { Write-Output \"line $_\" }"
        } else {
            "for i in 1 2 3; do echo line $i; done"
        }
    }

    fn forever() -> &'static str {
        if cfg!(windows) {
            "while ($true) { Start-Sleep -Seconds 1 }"
        } else {
            "while true; do sleep 1; done"
        }
    }

    async fn settle(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    #[test]
    fn the_permission_target_is_the_command_verbatim_and_always_is_its_shape() {
        let (tool, _) = tools();
        let request = tool.permission(&json!({ "command": "git push origin main" }));
        assert_eq!(request.key, "shell");
        assert_eq!(request.target, "git push origin main");
        assert_eq!(request.always.as_deref(), Some("git push *"));
    }

    #[test]
    fn the_label_carries_the_danger() {
        let (tool, _) = tools();
        assert!(tool
            .render(&json!({ "command": "rm -rf /" }))
            .unwrap()
            .contains("filesystem"));
        assert_eq!(
            tool.render(&json!({ "command": "npm test" })).as_deref(),
            Some("npm test")
        );
    }

    #[test]
    fn the_description_names_the_platform_and_the_shell() {
        let (tool, _) = tools();
        assert!(tool.description().contains(platform_name()));
        if cfg!(windows) {
            assert!(tool.description().contains("PowerShell"));
            assert!(shell_name().contains("PowerShell"));
            assert!(shell().program.contains("pwsh") || shell().program == "powershell.exe");
        }
    }

    #[tokio::test]
    async fn it_returns_what_the_command_printed() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _) = tools();
        let out = tool
            .execute(
                json!({ "command": say("hello from inertia") }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .unwrap();
        assert!(out.output.contains("hello from inertia"), "{}", out.output);
        let meta = out.metadata.unwrap();
        assert_eq!(meta["exitCode"], 0);
        assert_eq!(meta["background"], false);
        assert_eq!(meta["timedOut"], false);
    }

    /// PowerShell's own exit code is 1 for anything. The one the program
    /// returned is the one the model reads.
    #[tokio::test]
    async fn a_non_zero_exit_code_is_where_the_model_will_read_it() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _) = tools();
        let out = tool
            .execute(
                json!({ "command": exit_with(3) }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .unwrap();
        assert!(out.output.contains("Exit code: 3"), "{}", out.output);
        assert_eq!(out.metadata.unwrap()["exitCode"], 3);
    }

    #[tokio::test]
    async fn a_command_that_outstays_its_timeout_is_killed_and_said_so() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _) = tools();
        let out = tool
            .execute(
                json!({ "command": sleep_for(20), "timeout": 500 }),
                &ctx(dir.path(), Arc::new(MockGate::allow_all())),
            )
            .await
            .unwrap();
        assert!(
            out.output.contains("exceeding its 500 ms timeout"),
            "{}",
            out.output
        );
        let meta = out.metadata.unwrap();
        assert_eq!(meta["timedOut"], true);
        assert!(meta["durationMs"].as_u64().unwrap() < 10_000);
    }

    #[tokio::test]
    async fn a_missing_working_folder_is_blamed_not_the_shell() {
        let gone = std::env::temp_dir().join("inertia-not-a-folder-9f3c1a");
        let (tool, _) = tools();
        let err = tool
            .execute(
                json!({ "command": say("1") }),
                &ctx(&gone, Arc::new(MockGate::allow_all())),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains(&gone.display().to_string()), "{err}");
        assert!(err.contains("deleted or renamed"), "{err}");
        assert!(!err.contains("Could not run"), "{err}");
    }

    #[tokio::test]
    async fn workdir_is_honoured_and_a_missing_one_is_named() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let (tool, _) = tools();
        let gate = Arc::new(MockGate::allow_all());
        let pwd = if cfg!(windows) {
            "(Get-Location).Path"
        } else {
            "pwd"
        };
        let out = tool
            .execute(
                json!({ "command": pwd, "workdir": "sub" }),
                &ctx(dir.path(), gate.clone()),
            )
            .await
            .unwrap();
        assert!(out.output.to_lowercase().contains("sub"), "{}", out.output);

        let err = tool
            .execute(
                json!({ "command": pwd, "workdir": "nope" }),
                &ctx(dir.path(), gate),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not exist"), "{err}");
        assert!(err.contains("nope"), "{err}");
    }

    /// The second questions, asked in addition to the registry's own: one
    /// about emptying a folder, one about each directory outside the project.
    #[tokio::test]
    async fn sweeping_and_reaching_outside_each_ask_their_own_question() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _) = tools();
        let gate = Arc::new(MockGate::deny_all());
        let err = tool
            .execute(
                json!({ "command": "rm -rf src" }),
                &ctx(dir.path(), gate.clone()),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Denied(_)), "{err}");
        let asked = gate.asked();
        assert_eq!(asked[0].key, "delete_everything");
        assert_eq!(asked[0].target, "rm -rf src");
        assert_eq!(asked[0].always, None);

        let gate = Arc::new(MockGate::deny_all());
        let err = tool
            .execute(
                json!({ "command": "cat ../elsewhere/notes.txt" }),
                &ctx(dir.path(), gate.clone()),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Denied(_)), "{err}");
        let asked = gate.asked();
        assert_eq!(asked[0].key, "external_directory");
        assert!(
            asked[0].target.ends_with("elsewhere/*"),
            "{}",
            asked[0].target
        );
        assert_eq!(asked[0].always.as_deref(), Some(asked[0].target.as_str()));
    }

    #[tokio::test]
    async fn a_background_command_is_listed_read_and_finishes_readable() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, bg) = tools();
        let gate = Arc::new(MockGate::allow_all());
        let context = ctx(dir.path(), gate);
        let started = tool
            .execute(
                json!({ "command": counting(), "background": true }),
                &context,
            )
            .await
            .unwrap();
        let meta = started.metadata.unwrap();
        assert_eq!(meta["background"], true);
        let pid = meta["pid"].as_u64().expect("a pid");
        assert!(started.output.contains(&format!("pid {pid}")));

        settle(2500).await;

        let [list, logs, kill, _write]: [Arc<dyn Tool>; 4] =
            background_tools(bg.clone()).try_into().ok().unwrap();

        let listed = list.execute(json!({}), &context).await.unwrap();
        assert!(
            listed.output.contains(&pid.to_string()),
            "{}",
            listed.output
        );
        assert!(listed.metadata.unwrap()["pids"]
            .as_array()
            .unwrap()
            .contains(&json!(pid)));

        let read = logs.execute(json!({ "pid": pid }), &context).await.unwrap();
        assert!(read.output.contains("line 1"), "{}", read.output);
        assert!(read.output.contains("line 3"), "{}", read.output);
        let meta = read.metadata.unwrap();
        // Finished, and still here: the moment a process dies is the moment
        // its output matters most.
        assert_eq!(meta["running"], false);
        assert_eq!(meta["exitCode"], 0);

        // Stopping a finished one forgets it; a second stop has nothing.
        kill.execute(json!({ "pid": pid }), &context).await.unwrap();
        let err = kill
            .execute(json!({ "pid": pid }), &context)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("shell_list"), "{err}");
    }

    #[tokio::test]
    async fn a_background_process_can_be_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, bg) = tools();
        let context = ctx(dir.path(), Arc::new(MockGate::allow_all()));
        let started = tool
            .execute(
                json!({ "command": forever(), "background": true }),
                &context,
            )
            .await
            .unwrap();
        let pid = started.metadata.unwrap()["pid"].as_u64().unwrap();
        settle(500).await;
        assert!(bg.list().iter().any(|e| e.pid == pid as u32 && e.running));

        let kill = background_tools(bg.clone()).remove(2);
        let out = kill.execute(json!({ "pid": pid }), &context).await.unwrap();
        assert_eq!(out.metadata.unwrap()["stopped"], true);
        assert!(!bg.list().iter().any(|e| e.pid == pid as u32));
    }

    #[tokio::test]
    async fn typing_into_a_background_process_gets_an_answer() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, bg) = tools();
        let context = ctx(dir.path(), Arc::new(MockGate::allow_all()));
        let asks = if cfg!(windows) {
            "Write-Output 'Which port?'; $x = [Console]::In.ReadLine(); Write-Output \"using $x\""
        } else {
            "echo 'Which port?'; read x; echo \"using $x\""
        };
        let started = tool
            .execute(json!({ "command": asks, "background": true }), &context)
            .await
            .unwrap();
        let pid = started.metadata.unwrap()["pid"].as_u64().unwrap();
        settle(1500).await;

        let mut set = background_tools(bg.clone());
        let write = set.remove(3);
        let logs = set.remove(1);
        let typed = write
            .execute(json!({ "pid": pid, "input": "3001" }), &context)
            .await
            .unwrap();
        assert!(typed.output.contains("Sent"));
        settle(2000).await;

        let after = logs.execute(json!({ "pid": pid }), &context).await.unwrap();
        assert!(after.output.contains("Which port?"), "{}", after.output);
        assert!(after.output.contains("using 3001"), "{}", after.output);

        // It has exited by now, so a further write is refused rather than lost.
        let err = write
            .execute(json!({ "pid": pid, "input": "more" }), &context)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("No running background process"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn unknown_pids_point_at_shell_list() {
        let dir = tempfile::tempdir().unwrap();
        let (_, bg) = tools();
        let context = ctx(dir.path(), Arc::new(MockGate::allow_all()));
        let set = background_tools(bg);
        for tool in &set[1..] {
            let err = tool
                .execute(json!({ "pid": 99_999_999, "input": "x" }), &context)
                .await
                .unwrap_err()
                .to_string();
            assert!(err.contains("shell_list"), "{}: {err}", tool.id());
        }
        let listed = set[0].execute(json!({}), &context).await.unwrap();
        assert_eq!(listed.metadata.unwrap()["count"], 0);
        assert!(listed.output.to_lowercase().contains("nothing"));
    }
}

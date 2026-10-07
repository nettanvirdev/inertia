//! Reading, listing, typing into and stopping what `shell` left running.
//!
//! `shell` with `background: true` captures a bounded tail of everything a
//! detached process prints, and tells the model to come back for it. These
//! are the four tools that promise made necessary: for a while the message
//! named `shell_logs`, `shell_list` and `shell_kill` and none of them existed,
//! which is a worse failure than the silence it replaced - a model following
//! an instruction into a tool that is not there burns the turn and has no way
//! to find out why.
//!
//! The table of processes is [`Background`], shared by `Arc` between the shell
//! tool that starts them and the four tools here that read them. It is one
//! table for the whole app rather than one per conversation because a dev
//! server outlives the turn that started it, and the whole point of
//! backgrounding is that the agent can start one, do other work, and stop it
//! later.
//!
//! All four share `shell`'s permission key. Reading what a process you
//! started has printed is not a separate power from having started it, and
//! making a person write a second rule to see the output of a command they
//! already allowed would only teach them to allow everything.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin};

/// How much of a background process's output is kept, per process.
///
/// A tail, not a log. 64k is comfortably more than the last screen of a dev
/// server's output, which is what anybody actually reads, and small enough
/// that a dozen watchers running all day cost under a megabyte between them.
const BACKGROUND_TAIL_CHARS: usize = 64 * 1024;

/// How long a finished process stays readable, and how many are kept.
const FINISHED_TTL_MS: u64 = 15 * 60 * 1000;
const MAX_FINISHED: usize = 20;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One process the table knows about.
struct Entry {
    pid: u32,
    command: String,
    started_at: u64,
    /// The tail, and how much was dropped to keep it a tail. Reporting the
    /// dropped count matters: "here is the recent output" and "here is all
    /// the output" are different claims and the agent has to know which it is
    /// holding.
    tail: String,
    dropped: usize,
    exit_code: Option<i32>,
    exited_at: Option<u64>,
    /// Kept open, unlike a foreground command's. A background process is the
    /// one that may legitimately stop and ask - a dev server offering to use
    /// another port, a REPL, a migration wanting a yes - and `shell_write` is
    /// how the agent answers it. `None` once closed with `end`.
    stdin: Option<Arc<tokio::sync::Mutex<ChildStdin>>>,
}

/// A row of `shell_list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub pid: u32,
    pub command: String,
    pub started_at: u64,
    pub exit_code: Option<i32>,
    pub exited_at: Option<u64>,
    pub running: bool,
}

/// What `shell_logs` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logs {
    pub pid: u32,
    pub command: String,
    pub started_at: u64,
    pub exit_code: Option<i32>,
    pub exited_at: Option<u64>,
    pub running: bool,
    pub output: String,
    pub dropped: usize,
}

/// Backgrounded children, by pid.
#[derive(Default)]
pub struct Background {
    entries: Mutex<HashMap<u32, Entry>>,
}

impl std::fmt::Debug for Background {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Background")
            .field("processes", &self.entries.lock().len())
            .finish()
    }
}

impl Background {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take ownership of a freshly spawned child: keep its tail, remember its
    /// exit, hold its stdin. Returns the pid the model is handed.
    ///
    /// The streams must be consumed whether or not anyone reads them. A piped
    /// child whose output nobody drains blocks once the OS buffer fills, so
    /// the reader tasks are what keep the process alive, not a convenience.
    pub fn adopt(self: &Arc<Self>, mut child: Child, command: &str) -> Option<u32> {
        let pid = child.id()?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child
            .stdin
            .take()
            .map(|s| Arc::new(tokio::sync::Mutex::new(s)));

        self.entries.lock().insert(
            pid,
            Entry {
                pid,
                command: command.to_string(),
                started_at: now_ms(),
                tail: String::new(),
                dropped: 0,
                exit_code: None,
                exited_at: None,
                stdin,
            },
        );

        if let Some(out) = stdout {
            tokio::spawn(drain(Arc::clone(self), pid, out));
        }
        if let Some(err) = stderr {
            tokio::spawn(drain(Arc::clone(self), pid, err));
        }

        // A finished process is kept, not forgotten. Deleting it on exit
        // would throw away the output at exactly the moment it became
        // interesting: a dev server that died on startup is the case where the
        // agent most needs to read what it printed.
        let table = Arc::clone(self);
        tokio::spawn(async move {
            let status = child.wait().await;
            let code = status.ok().and_then(|s| s.code());
            table.mark_exited(pid, code);
        });

        Some(pid)
    }

    fn absorb(&self, pid: u32, chunk: &str) {
        let mut entries = self.entries.lock();
        let Some(entry) = entries.get_mut(&pid) else {
            return;
        };
        entry.tail.push_str(chunk);
        if entry.tail.len() > BACKGROUND_TAIL_CHARS {
            let over = entry.tail.len() - BACKGROUND_TAIL_CHARS;
            // Resume at a line boundary where one is close by, so the first
            // line of a tail is a line rather than the back half of one.
            let mut cut = match entry.tail[floor_char_boundary(&entry.tail, over)..].find('\n') {
                Some(i) if i < 400 => floor_char_boundary(&entry.tail, over) + i + 1,
                _ => floor_char_boundary(&entry.tail, over),
            };
            cut = cut.min(entry.tail.len());
            entry.dropped += cut;
            entry.tail.drain(..cut);
        }
    }

    fn mark_exited(&self, pid: u32, code: Option<i32>) {
        {
            let mut entries = self.entries.lock();
            if let Some(entry) = entries.get_mut(&pid) {
                entry.exit_code = code;
                entry.exited_at = Some(now_ms());
                // Nobody is listening on the other end any more.
                entry.stdin = None;
            }
        }
        self.reap_finished();
    }

    /// Forget the processes that exited long enough ago to be of no interest.
    ///
    /// Their output is kept after exit deliberately, so something has to
    /// eventually let go, or a long session accumulates the tail of every
    /// command it ever backgrounded.
    fn reap_finished(&self) {
        let mut entries = self.entries.lock();
        let mut finished: Vec<(u32, u64)> = entries
            .values()
            .filter_map(|e| e.exited_at.map(|at| (e.pid, at)))
            .collect();
        finished.sort_by_key(|(_, at)| *at);

        let now = now_ms();
        let excess = finished.len().saturating_sub(MAX_FINISHED);
        for (index, (pid, at)) in finished.into_iter().enumerate() {
            if index < excess || now.saturating_sub(at) > FINISHED_TTL_MS {
                entries.remove(&pid);
            }
        }
    }

    pub fn list(&self) -> Vec<Listed> {
        self.reap_finished();
        let entries = self.entries.lock();
        let mut out: Vec<Listed> = entries
            .values()
            .map(|e| Listed {
                pid: e.pid,
                command: e.command.clone(),
                started_at: e.started_at,
                exit_code: e.exit_code,
                exited_at: e.exited_at,
                running: e.exited_at.is_none(),
            })
            .collect();
        // Oldest first, so the list reads as a history.
        out.sort_by_key(|e| (e.started_at, e.pid));
        out
    }

    /// The captured tail for one process, or `None` when the pid is unknown.
    pub fn logs(&self, pid: u32) -> Option<Logs> {
        let entries = self.entries.lock();
        let e = entries.get(&pid)?;
        Some(Logs {
            pid: e.pid,
            command: e.command.clone(),
            started_at: e.started_at,
            exit_code: e.exit_code,
            exited_at: e.exited_at,
            running: e.exited_at.is_none(),
            output: e.tail.clone(),
            dropped: e.dropped,
        })
    }

    /// Stop a process and forget it. `false` when the pid is unknown.
    pub fn kill(&self, pid: u32) -> bool {
        let removed = self.entries.lock().remove(&pid);
        match removed {
            Some(entry) => {
                if entry.exited_at.is_none() {
                    hard_kill(pid);
                }
                true
            }
            None => false,
        }
    }

    /// Kill everything this table started. For quit.
    ///
    /// A detached child does not die with its parent, so without this a dev
    /// server an agent started keeps holding its port after the app window is
    /// gone - and the user has no way to find it, because the thing that knew
    /// its pid has exited.
    pub fn kill_all(&self) -> usize {
        let pids: Vec<u32> = self.entries.lock().keys().copied().collect();
        for pid in &pids {
            self.kill(*pid);
        }
        pids.len()
    }

    /// Type into a background process.
    ///
    /// `Ok(false)` when the pid is unknown or the process has exited, and
    /// `Err` when the pipe is already closed, which is what happens after
    /// `end` - the model is told so it does not keep typing into nothing.
    pub async fn write(
        &self,
        pid: u32,
        text: &str,
        end: bool,
    ) -> std::result::Result<bool, String> {
        let stdin = {
            let entries = self.entries.lock();
            let Some(entry) = entries.get(&pid) else {
                return Ok(false);
            };
            if entry.exited_at.is_some() {
                return Ok(false);
            }
            match &entry.stdin {
                Some(stdin) => Arc::clone(stdin),
                None => return Err("Its input has already been closed.".to_string()),
            }
        };

        {
            let mut handle = stdin.lock().await;
            if !text.is_empty() {
                handle
                    .write_all(text.as_bytes())
                    .await
                    .map_err(|e| format!("Its input could not be written to: {e}"))?;
                handle.flush().await.map_err(|e| e.to_string())?;
            }
            if end {
                let _ = handle.shutdown().await;
            }
        }

        if end {
            // Dropping the last handle is what actually closes the pipe; the
            // entry keeps `None` so the next write is refused rather than lost.
            if let Some(entry) = self.entries.lock().get_mut(&pid) {
                entry.stdin = None;
            }
        }
        Ok(true)
    }
}

async fn drain<R: tokio::io::AsyncRead + Unpin>(table: Arc<Background>, pid: u32, mut reader: R) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => table.absorb(pid, &String::from_utf8_lossy(&buf[..n])),
        }
    }
}

fn floor_char_boundary(s: &str, mut index: usize) -> usize {
    index = index.min(s.len());
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Kill a process and everything it spawned.
///
/// Killing the shell on Windows leaves the actual command running, which is
/// how a killed `npm run dev` keeps holding port 3000. `taskkill /T` is the
/// only thing that takes the tree. Fire-and-forget: the process being already
/// gone is the outcome we wanted anyway.
pub fn hard_kill(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill")
            .args(["/pid", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

// ── the tools ──────────────────────────────────────────────────────────────

fn pid_of(args: &Value) -> Result<u32> {
    args.get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .ok_or_else(|| Error::InvalidInput("`pid` must be a whole number.".into()))
}

fn pid_label(args: &Value) -> String {
    match args.get("pid") {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.clone(),
        _ => "?".to_string(),
    }
}

/// How a process reads in a list: pid, state, and what it was asked to do.
fn describe(entry: &Listed) -> String {
    let state = if entry.running {
        "running".to_string()
    } else {
        format!(
            "exited {}",
            entry
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into())
        )
    };
    let age = now_ms().saturating_sub(entry.started_at) / 1000;
    format!(
        "{}  [{}]  {}s ago  {}",
        entry.pid, state, age, entry.command
    )
}

#[derive(Debug)]
pub struct ShellListTool(Arc<Background>);

#[async_trait]
impl Tool for ShellListTool {
    fn id(&self) -> &str {
        "shell_list"
    }

    fn description(&self) -> &str {
        "List the background processes this session started with `shell` and `background: true`.\n\
         \n\
         Shows each one's pid, whether it is still running or the code it exited with, how long ago it started, and the command. A process that has exited stays listed for a while so you can still read what it printed - that is usually the moment you most want it."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        PermissionRequest::new("shell", "shell_list").with_always("shell_list")
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some("list background processes".to_string())
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let entries = self.0.list();
        if entries.is_empty() {
            return Ok(ToolOutcome {
                title: Some("no background processes".into()),
                output: "Nothing is running in the background, and nothing recent has finished."
                    .into(),
                metadata: Some(json!({ "count": 0 })),
                images: Vec::new(),
            });
        }
        Ok(ToolOutcome {
            title: Some(format!(
                "{} background process{}",
                entries.len(),
                if entries.len() == 1 { "" } else { "es" }
            )),
            output: entries.iter().map(describe).collect::<Vec<_>>().join("\n"),
            metadata: Some(json!({
                "count": entries.len(),
                "pids": entries.iter().map(|e| e.pid).collect::<Vec<_>>(),
            })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
pub struct ShellLogsTool(Arc<Background>);

#[async_trait]
impl Tool for ShellLogsTool {
    fn id(&self) -> &str {
        "shell_logs"
    }

    fn description(&self) -> &str {
        "Read what a background process has printed.\n\
         \n\
         Takes the pid that `shell` returned. Output is a bounded tail rather than the whole history, so a watcher that has been running for hours costs the same as one that started a second ago; if earlier output was dropped to keep the tail, the result says how much. stdout and stderr are merged in the order they arrived.\n\
         \n\
         Use `shell_list` if you do not have the pid."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pid": { "type": "integer", "description": "The pid returned when the process was started." }
            },
            "required": ["pid"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        PermissionRequest::new("shell", format!("shell_logs {}", pid_label(args)))
            .with_always("shell_logs")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("read output of pid {}", pid_label(args)))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let pid = pid_of(&args)?;
        let Some(found) = self.0.logs(pid) else {
            // Naming the alternative matters: the pid may simply have been
            // reaped, and a model told only "unknown pid" tends to start the
            // process again.
            return Err(Error::Other(format!(
                "No background process with pid {pid}. It may have finished long enough ago to be forgotten. Run shell_list to see what is still known."
            )));
        };

        let state = if found.running {
            "still running".to_string()
        } else {
            format!(
                "exited with code {}",
                found
                    .exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".into())
            )
        };
        let header = format!("pid {} ({state}): {}", found.pid, found.command);
        let dropped = if found.dropped > 0 {
            format!(
                "\n[{} earlier characters dropped to keep this a tail]\n",
                found.dropped
            )
        } else {
            String::new()
        };
        let body = if found.output.is_empty() {
            "(nothing printed yet)"
        } else {
            found.output.as_str()
        };

        Ok(ToolOutcome {
            title: Some(header.clone()),
            output: format!("{header}\n{dropped}{body}"),
            metadata: Some(json!({
                "pid": found.pid,
                "running": found.running,
                "exitCode": found.exit_code,
                "dropped": found.dropped,
                "truncated": false,
                "outputPath": Value::Null,
            })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
pub struct ShellKillTool(Arc<Background>);

#[async_trait]
impl Tool for ShellKillTool {
    fn id(&self) -> &str {
        "shell_kill"
    }

    fn description(&self) -> &str {
        "Stop a background process, and everything it spawned.\n\
         \n\
         Takes the pid that `shell` returned. On Windows this takes the whole process tree, because killing the shell alone leaves the actual command holding its port."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pid": { "type": "integer", "description": "The pid to stop." }
            },
            "required": ["pid"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        PermissionRequest::new("shell", format!("shell_kill {}", pid_label(args)))
            .with_always("shell_kill")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("stop pid {}", pid_label(args)))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let pid = pid_of(&args)?;
        if !self.0.kill(pid) {
            return Err(Error::Other(format!(
                "No background process with pid {pid} to stop. Run shell_list to see what is still running."
            )));
        }
        Ok(ToolOutcome {
            title: Some(format!("stopped pid {pid}")),
            output: format!("Stopped pid {pid} and any processes it started."),
            metadata: Some(json!({ "pid": pid, "stopped": true })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
pub struct ShellWriteTool(Arc<Background>);

#[async_trait]
impl Tool for ShellWriteTool {
    fn id(&self) -> &str {
        "shell_write"
    }

    fn description(&self) -> &str {
        "Type into a background process.\n\
         \n\
         For a process that stopped to ask something - a dev server offering another port,\n\
         an installer wanting a yes, a REPL - send the answer here. A newline is added\n\
         unless you say otherwise, because almost every prompt waits for one. Then read\n\
         `shell_logs` to see what it did with the answer. Pass `end: true` to close its\n\
         input, which is how a process reading stdin to the end is told there is no more."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pid": { "type": "integer", "description": "The pid returned when the process was started." },
                "input": { "type": "string", "description": "What to type. May be empty when only closing the input." },
                "newline": { "type": "boolean", "description": "Add a newline after the input. Default true." },
                "end": { "type": "boolean", "description": "Close the process's input afterwards." }
            },
            "required": ["pid"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        PermissionRequest::new("shell", format!("shell_write {}", pid_label(args)))
            .with_always("shell_write")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("type into pid {}", pid_label(args)))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let pid = pid_of(&args)?;
        let input = args.get("input").and_then(Value::as_str);
        let newline = args.get("newline").and_then(Value::as_bool);
        let end = args.get("end").and_then(Value::as_bool).unwrap_or(false);

        // No input and no explicit newline means "only close it": nothing is
        // typed. Otherwise the newline is added unless refused.
        let text = if input.is_none() && newline != Some(true) {
            String::new()
        } else {
            format!(
                "{}{}",
                input.unwrap_or_default(),
                if newline == Some(false) { "" } else { "\n" }
            )
        };

        let sent =
            self.0.write(pid, &text, end).await.map_err(|message| {
                Error::Other(format!("Could not write to pid {pid}: {message}"))
            })?;
        if !sent {
            return Err(Error::Other(format!(
                "No running background process with pid {pid}. It may have exited; read shell_logs {pid} to see how, or shell_list to see what is running."
            )));
        }
        Ok(ToolOutcome {
            title: Some(format!("typed into pid {pid}")),
            output: format!(
                "Sent{}. Read shell_logs {pid} to see the response.",
                if end { " and closed its input" } else { "" }
            ),
            metadata: Some(json!({ "pid": pid, "ended": end })),
            images: Vec::new(),
        })
    }
}

/// The four, sharing the table with the shell tool that fills it.
pub fn background_tools(bg: Arc<Background>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ShellListTool(Arc::clone(&bg))),
        Arc::new(ShellLogsTool(Arc::clone(&bg))),
        Arc::new(ShellKillTool(Arc::clone(&bg))),
        Arc::new(ShellWriteTool(bg)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tools_exist_under_exactly_the_ids_shell_promises() {
        // A model following an instruction into a tool that is not there burns
        // the turn and has no way to find out why.
        let tools = background_tools(Arc::new(Background::new()));
        let ids: Vec<&str> = tools.iter().map(|t| t.id()).collect();
        assert_eq!(
            ids,
            vec!["shell_list", "shell_logs", "shell_kill", "shell_write"]
        );
        // Reading what a process you were allowed to start has printed is not
        // a separate power. A second key would only teach people to allow
        // everything.
        for tool in &tools {
            assert_eq!(tool.permission(&json!({ "pid": 1 })).key, "shell");
        }
    }

    #[test]
    fn the_tail_stays_a_tail_and_says_how_much_went() {
        let bg = Arc::new(Background::new());
        bg.entries.lock().insert(
            7,
            Entry {
                pid: 7,
                command: "x".into(),
                started_at: now_ms(),
                tail: String::new(),
                dropped: 0,
                exit_code: None,
                exited_at: None,
                stdin: None,
            },
        );
        for i in 0..2000 {
            bg.absorb(7, &format!("line {i:05} {}\n", "x".repeat(60)));
        }
        let logs = bg.logs(7).unwrap();
        assert!(logs.output.len() <= BACKGROUND_TAIL_CHARS);
        assert!(logs.dropped > 0);
        // The tail begins at a line, not in the middle of one.
        assert!(logs.output.starts_with("line "), "{}", &logs.output[..20]);
        assert!(logs.output.ends_with("x\n"));
    }
}

//! A shell that stays where you left it.
//!
//! The app already runs commands - the `shell` tool spawns one per call, which
//! is right for an agent: each call is independent, cancellable, and carries
//! its own working directory. It is wrong for a person. A person types `cd
//! apps/web`, then `npm install`, and expects the second to happen where the
//! first left them; they set an environment variable and expect it to still be
//! set. That needs one long-lived process, and this is it.
//!
//! One shell per terminal tab, and tabs are named after the conversation they
//! sit beside: `chat:<thread>:sh:<n>`. Not one per window and not one for the
//! app - two conversations are usually two folders, and a `cd` in one must not
//! move the other. The id is the conversation's, so the pane, the tool that
//! reads it and the agent all mean the same shell without any of them passing
//! one around.
//!
//! ## Why there is one backend here and the Electron app had two
//!
//! The Electron app carried a piped-shell fallback for the machine where
//! `node-pty` would not load, because a native module is a compiled binary
//! matched to a platform and an ABI and the one thing it will eventually do is
//! fail to load. `portable-pty` is an ordinary Rust dependency compiled into
//! this binary: there is no load step to fail, so there is no second backend
//! and nothing for the pane to fall back to. `capability()` still exists and
//! still answers, because the pane asks before it decides which of its two
//! shapes to draw, and it now always answers yes.
//!
//! An individual `openpty` can still fail - a policy that refuses ConPTY, a
//! shell that is not there - and that is an error from `open`, reported to the
//! person in the pane rather than silently downgraded.
//!
//! ## What is kept, and why twice
//!
//! Every byte the shell writes goes to two places. The [`screen::Screen`] is a
//! grid, and it is what `terminal_read` reads: ConPTY repaints by absolute
//! position, so the bytes alone do not contain the words (see `screen.rs`). A
//! bounded tail of the raw bytes is kept beside it for the window, which has a
//! real emulator of its own and wants the stream so a reopened pane looks
//! exactly as it did.
//!
//! ## What is deliberately not here
//!
//! Nothing writes into the person's shell on behalf of an agent. `run` exists
//! for the two callers that hold a whole command - the pane's own command box
//! and `set_cwd` - and `terminal_read` is read-only. Two writers on one stdin
//! interleave, and the person loses the one surface in the app that was
//! unambiguously theirs.

// Tests say what they mean with `expect`: a fixture that cannot be built is a
// broken test, and a `?` there would hide which line gave up.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod ansi;
pub mod screen;
pub mod shell;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use serde_json::{json, Value};

use screen::Screen;

/// How much raw output one shell keeps for the window to replay.
///
/// Enough to scroll back through a build; small enough that a command left
/// printing overnight cannot grow the process without limit. Trimmed from the
/// front, because a terminal is read from the bottom.
pub const SCROLLBACK_CHARS: usize = 400_000;

/// The size a shell is told it has before the pane has measured itself.
const DEFAULT_COLS: usize = 100;
const DEFAULT_ROWS: usize = 30;

/// How the window hears about output it did not ask for.
///
/// Output is pushed rather than polled - a build writes hundreds of lines and a
/// pane that asked for them on a timer would be a pane that stutters - so this
/// crate needs a way to speak to a window it knows nothing about. The app
/// implements it over Tauri's event bus; a test implements it with a vector.
pub trait Events: Send + Sync {
    fn emit(&self, payload: Value);
}

/// Where events go, once somebody has claimed them.
///
/// Silent until a window attaches: a terminal opened before the window is up -
/// or by a test - writes normally and announces to nobody.
#[derive(Default)]
struct Sink(Mutex<Option<Arc<dyn Events>>>);

impl Sink {
    fn emit(&self, payload: Value) {
        // Cloned out from under the lock first. An emitter that re-entered this
        // crate while the lock was held would deadlock on the next event.
        let listener = self.0.lock().clone();
        if let Some(listener) = listener {
            listener.emit(payload);
        }
    }
}

impl std::fmt::Debug for Sink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sink").finish_non_exhaustive()
    }
}

/// What the pane is handed when it opens or reopens a tab.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub cwd: String,
    /// The raw byte stream, for the pane's emulator to replay.
    pub scrollback: String,
    pub busy: bool,
    /// Always zero. The pane reads it, and a pty has no queue: a keystroke goes
    /// straight through.
    pub queued: usize,
    pub pty: bool,
    pub cols: usize,
    pub rows: usize,
    pub mirror: bool,
    pub title: String,
}

/// One terminal, as text rather than as a screen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub id: String,
    pub cwd: String,
    pub busy: bool,
    pub running: Option<String>,
    pub pty: bool,
    pub mirror: bool,
    pub title: String,
    pub text: String,
    /// Whether this is the tab the person is looking at. Only meaningful in a
    /// [`Terminals::read_thread`] result, where something decides.
    pub front: bool,
}

/// One row of [`Terminals::list`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listed {
    pub id: String,
    pub cwd: String,
    pub busy: bool,
    pub pty: bool,
}

/// Whether this build has a real terminal, for the pane to say so.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub pty: bool,
    pub reason: Option<String>,
}

/// How a tab is opened.
#[derive(Debug, Clone, Default)]
pub struct OpenOptions {
    pub cwd: Option<String>,
    pub cols: Option<usize>,
    pub rows: Option<usize>,
}

/// The mutable half of a session, behind one lock.
///
/// One lock rather than several because every field is read together: a
/// snapshot is the folder, the screen and what is running, and three locks
/// would be three chances to report a screen from after a resize with the size
/// from before it.
#[derive(Debug)]
struct Inner {
    cwd: String,
    cols: usize,
    rows: usize,
    screen: Screen,
    replay: String,
    /// What is running, for a mirrored process. `None` on a real shell: a pty
    /// has a prompt and no way to know whether the thing in front of it is a
    /// build or an idle cursor, and guessing would make `terminal_read` report
    /// an idle terminal as busy for ever.
    running: Option<String>,
    mirror: bool,
    title: String,
    alive: bool,
}

struct Session {
    id: String,
    opened_at: u128,
    inner: Mutex<Inner>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    /// What the pane's Stop button calls on a mirrored process. Without it the
    /// button would be there and do nothing, which is worse than not being
    /// there.
    on_stop: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Session {
    fn blank(id: &str, cwd: String, cols: usize, rows: usize, convert_eol: bool) -> Self {
        Self {
            id: id.to_string(),
            opened_at: now(),
            inner: Mutex::new(Inner {
                cwd,
                cols,
                rows,
                screen: Screen::new(cols, rows, convert_eol),
                replay: String::new(),
                running: None,
                mirror: false,
                title: String::new(),
                alive: true,
            }),
            writer: Mutex::new(None),
            master: Mutex::new(None),
            child: Mutex::new(None),
            on_stop: Mutex::new(None),
        }
    }

    fn snapshot(&self) -> Snapshot {
        let inner = self.inner.lock();
        Snapshot {
            id: self.id.clone(),
            cwd: inner.cwd.clone(),
            scrollback: inner.replay.clone(),
            busy: inner.running.is_some(),
            queued: 0,
            pty: !inner.mirror && inner.alive,
            cols: inner.cols,
            rows: inner.rows,
            mirror: inner.mirror,
            title: inner.title.clone(),
        }
    }
}

fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0)
}

fn sane(value: Option<usize>, fallback: usize) -> usize {
    match value {
        Some(number) if number > 0 && number < 1000 => number,
        _ => fallback,
    }
}

/// Everything the shell said, to the screen, to the replay, and to the window.
///
/// And anything it ASKED, straight back into the pty. A terminal that does not
/// answer a Device Status Report is a terminal ConPTY stops talking to; see
/// `Screen::replies`.
fn push(session: &Session, sink: &Sink, text: &str) {
    let answer = {
        let mut inner = session.inner.lock();
        inner.screen.write(text);
        inner.replay.push_str(text);
        if inner.replay.chars().count() > SCROLLBACK_CHARS {
            let skip = inner.replay.chars().count() - SCROLLBACK_CHARS;
            inner.replay = inner.replay.chars().skip(skip).collect();
        }
        inner.screen.take_replies()
    };
    if !answer.is_empty() {
        if let Some(writer) = session.writer.lock().as_mut() {
            // Fire and forget: a pty that has just closed cannot be answered,
            // and the reader thread is what reports that.
            let _ = writer.write_all(answer.as_bytes());
            let _ = writer.flush();
        }
    }
    sink.emit(json!({ "type": "data", "id": session.id, "text": text }));
}

/// The last `limit` characters, which is what a terminal is read from.
fn tail(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    text.chars().skip(count - limit).collect()
}

/// A tab that shows something the app is running, rather than a shell.
///
/// `chat:<thread>:job:<pid>`. It must never become a shell: spawning PowerShell
/// under that id would put a live prompt where a person expects a finished log.
pub fn is_mirror_id(id: &str) -> bool {
    id.starts_with("chat:") && id.contains(":job:")
}

/// The conversation a tab belongs to, or `None` for an id that is not one.
pub fn thread_of(id: &str) -> Option<&str> {
    let rest = id.strip_prefix("chat:")?;
    // From the right: a conversation id is opaque and the marker is not.
    let cut = rest.rfind(":sh:").or_else(|| rest.rfind(":job:"))?;
    Some(&rest[..cut])
}

/// Every terminal this process has open.
#[derive(Debug, Default)]
pub struct Terminals {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    sink: Arc<Sink>,
}

impl Terminals {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim the events. The window does this once, when it comes up.
    pub fn attach(&self, events: Arc<dyn Events>) {
        *self.sink.0.lock() = Some(events);
    }

    /// Whether this build has a real terminal. It does; see the module doc.
    pub fn capability(&self) -> Capability {
        Capability {
            pty: true,
            reason: None,
        }
    }

    fn get(&self, id: &str) -> Result<Arc<Session>, String> {
        self.sessions
            .lock()
            .get(id)
            .cloned()
            .ok_or_else(|| format!("No terminal is open with id {id}."))
    }

    /// Open a tab, or hand back the one already under this id.
    pub fn open(&self, id: &str, options: &OpenOptions) -> Result<Snapshot, String> {
        if let Some(existing) = self.sessions.lock().get(id) {
            return Ok(existing.snapshot());
        }

        let cwd = options
            .cwd
            .clone()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_default()
            });
        let cols = sane(options.cols, DEFAULT_COLS);
        let rows = sane(options.rows, DEFAULT_ROWS);

        // A tab whose process died with the app, or a window reopened onto a
        // job that has been reaped.
        if is_mirror_id(id) {
            let session = Arc::new(Session::blank(id, cwd, cols, rows, true));
            session.inner.lock().mirror = true;
            self.sessions
                .lock()
                .insert(id.to_string(), Arc::clone(&session));
            push(&session, &self.sink, "[this process is no longer running]\r\n");
            return Ok(session.snapshot());
        }

        let session = Arc::new(Session::blank(id, cwd.clone(), cols, rows, false));
        self.start(&session, &cwd, cols, rows)?;
        self.sessions
            .lock()
            .insert(id.to_string(), Arc::clone(&session));
        Ok(session.snapshot())
    }

    /// Spawn the shell and the thread that reads it.
    fn start(&self, session: &Arc<Session>, cwd: &str, cols: usize, rows: usize) -> Result<(), String> {
        let shell = shell::shell_for_pty();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("The terminal could not be opened: {error}"))?;

        let mut command = CommandBuilder::new(&shell.program);
        for arg in &shell.args {
            command.arg(arg);
        }
        if std::path::Path::new(cwd).is_dir() {
            command.cwd(cwd);
        }
        // The opposite of what the `shell` tool asks for, and deliberately so:
        // this IS a terminal, so a program that draws colour and progress bars
        // should. `TERM_PROGRAM` is said out loud so a script can behave
        // differently inside the app if it wants to.
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "Inertia");

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("The shell {} could not start: {error}", shell.program))?;
        // The slave end belongs to the child now. Holding ours open would mean
        // the read below never sees end-of-file when the shell exits, and the
        // tab would sit there looking alive for ever.
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("The terminal could not be read: {error}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| format!("The terminal could not be written to: {error}"))?;

        *session.writer.lock() = Some(writer);
        *session.child.lock() = Some(child);
        *session.master.lock() = Some(pair.master);

        let watched = Arc::clone(session);
        let sink = Arc::clone(&self.sink);
        std::thread::Builder::new()
            .name(format!("terminal {}", session.id))
            .spawn(move || read_loop(&watched, &sink, reader))
            .map_err(|error| format!("The terminal could not be started: {error}"))?;

        // Anything the shell could not be started with, typed in once it is up.
        // POSIX only - a single line, because a shell echoes what is typed at
        // it and nine lines of setup across the top of a new tab reads as a
        // broken app. After a moment, so it lands once the person's profile has
        // finished printing rather than in the middle of it.
        if let Some(line) = shell.typed {
            let typing = Arc::clone(session);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(700));
                if let Some(writer) = typing.writer.lock().as_mut() {
                    // A shell that will not take it is a shell whose folder we
                    // do not know. Everything else about it still works.
                    let _ = writer.write_all(line.as_bytes());
                    let _ = writer.flush();
                }
            });
        }

        Ok(())
    }

    /// Keystrokes, straight through.
    ///
    /// This is what makes Ctrl+C reach the program rather than the shell, what
    /// makes the arrow keys walk the history, and what lets `vim` see the
    /// letters.
    pub fn write(&self, id: &str, data: &str) -> Result<usize, String> {
        let session = self.get(id)?;
        if data.is_empty() {
            return Ok(0);
        }
        // A mirror has no stdin of ours to write to. The process's own belongs
        // to whoever started it, and a second writer on it interleaves.
        if session.inner.lock().mirror {
            return Ok(0);
        }
        let mut writer = session.writer.lock();
        let Some(writer) = writer.as_mut() else {
            return Ok(0);
        };
        writer
            .write_all(data.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|error| format!("The terminal could not be written to: {error}"))?;
        Ok(data.len())
    }

    pub fn resize(&self, id: &str, cols: Option<usize>, rows: Option<usize>) -> Result<(usize, usize), String> {
        let session = self.get(id)?;
        let (cols, rows) = {
            let mut inner = session.inner.lock();
            inner.cols = sane(cols, inner.cols);
            inner.rows = sane(rows, inner.rows);
            let (cols, rows) = (inner.cols, inner.rows);
            inner.screen.resize(cols, rows);
            (cols, rows)
        };
        if let Some(master) = session.master.lock().as_ref() {
            // A pty that has just exited. The exit handler is what reports
            // that; a failed resize is not news.
            let _ = master.resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        Ok((cols, rows))
    }

    /// Run a whole command, by typing it in.
    ///
    /// The person sees it in their own scrollback exactly as if they had typed
    /// it, which is the difference between a shell that behaves the way they
    /// expect and one that teleports.
    pub fn run(&self, id: &str, command: &str) -> Result<Snapshot, String> {
        let session = self.get(id)?;
        let text = command.trim();
        if text.is_empty() || session.inner.lock().mirror {
            return Ok(session.snapshot());
        }
        self.write(id, &format!("{text}\r"))?;
        Ok(session.snapshot())
    }

    /// Stop what is running.
    ///
    /// On a pty this is exactly what a terminal does: Ctrl+C is a byte, the pty
    /// turns it into an interrupt, and it goes to the foreground process - so
    /// the build stops and the shell survives, which is what everybody expects.
    pub fn interrupt(&self, id: &str) -> Result<Snapshot, String> {
        let session = self.get(id)?;
        if session.inner.lock().mirror {
            // The stop button on a mirrored process. This is the whole reason a
            // mirror carries a callback: a person watching their dev server in
            // this pane must be able to stop it here rather than being told to
            // ask the agent.
            if let Some(stop) = session.on_stop.lock().as_ref() {
                stop();
            }
            return Ok(session.snapshot());
        }
        // ETX, built rather than typed: a raw control character in source is
        // invisible, survives no copy-paste, and is exactly the sort of byte an
        // editor silently normalises away.
        self.write(id, &char::from(3u8).to_string())?;
        Ok(session.snapshot())
    }

    /// The tail of what this shell has printed, as text rather than as a screen.
    ///
    /// A tab that is not open is an empty answer rather than an error: the
    /// agent asking about a terminal nobody opened has learned something true.
    pub fn read(&self, id: &str, chars: Option<usize>) -> Reading {
        let limit = chars.unwrap_or(8000).clamp(200, SCROLLBACK_CHARS);
        let Ok(session) = self.get(id) else {
            return Reading {
                id: id.to_string(),
                cwd: String::new(),
                busy: false,
                running: None,
                pty: false,
                mirror: false,
                title: String::new(),
                text: String::new(),
                front: false,
            };
        };
        let inner = session.inner.lock();
        Reading {
            id: session.id.clone(),
            cwd: inner.cwd.clone(),
            busy: inner.running.is_some(),
            running: inner.running.clone(),
            pty: !inner.mirror && inner.alive,
            mirror: inner.mirror,
            title: inner.title.clone(),
            // From the grid, not from the bytes. See `screen.rs`.
            text: tail(&inner.screen.text(), limit),
            front: false,
        }
    }

    /// Move the shell, by typing the move.
    ///
    /// Not by respawning it somewhere else: a `cd` the person can see in their
    /// own scrollback is a shell that behaves the way they expect.
    pub fn set_cwd(&self, id: &str, cwd: &str) -> Result<Snapshot, String> {
        let quoted = serde_json::to_string(cwd).unwrap_or_else(|_| format!("\"{cwd}\""));
        self.run(id, &format!("cd {quoted}"))
    }

    pub fn close(&self, id: &str) -> bool {
        let Some(session) = self.sessions.lock().remove(id) else {
            return false;
        };
        session.inner.lock().alive = false;
        *session.on_stop.lock() = None;
        kill(session);
        true
    }

    /// Everything, for the app shutting down.
    pub fn close_all(&self) {
        let ids: Vec<String> = self.sessions.lock().keys().cloned().collect();
        for id in ids {
            self.close(&id);
        }
    }

    pub fn list(&self) -> Vec<Listed> {
        let sessions: Vec<Arc<Session>> = self.sessions.lock().values().cloned().collect();
        sessions
            .iter()
            .map(|session| {
                let inner = session.inner.lock();
                Listed {
                    id: session.id.clone(),
                    cwd: inner.cwd.clone(),
                    busy: inner.running.is_some(),
                    pty: !inner.mirror && inner.alive,
                }
            })
            .collect()
    }

    /// The terminal tab this conversation is showing, or `None`.
    ///
    /// A session is "front" from the moment it is opened, which is the only
    /// signal the pane gives - and is right, because opening a terminal tab is
    /// exactly the act of bringing it forward.
    pub fn front_of(&self, thread: &str) -> Option<String> {
        let prefix = format!("chat:{thread}:sh:");
        let sessions: Vec<Arc<Session>> = self.sessions.lock().values().cloned().collect();
        sessions
            .iter()
            .filter(|session| session.id.starts_with(&prefix))
            .max_by_key(|session| session.opened_at)
            .map(|session| session.id.clone())
    }

    /// Read EVERY terminal in a conversation, front tab first.
    ///
    /// Not the front one. A person watching a build in one tab and a server in
    /// the next has two terminals, and an agent that can only see one of them
    /// will confidently explain a failure using the wrong half of it.
    ///
    /// The budget is split between the tabs rather than spent on the first, and
    /// never cut so fine that a tab returns a line and a half: a tab that
    /// cannot say anything useful is worse than one summarised as busy.
    pub fn read_thread(&self, thread: &str, chars: Option<usize>) -> Vec<Reading> {
        let shells = format!("chat:{thread}:sh:");
        let jobs = format!("chat:{thread}:job:");
        let front = self.front_of(thread);

        let mut found: Vec<Arc<Session>> = self
            .sessions
            .lock()
            .values()
            .filter(|session| session.id.starts_with(&shells) || session.id.starts_with(&jobs))
            .cloned()
            .collect();
        if found.is_empty() {
            return Vec::new();
        }
        found.sort_by_key(|session| {
            let is_front = front.as_deref() == Some(session.id.as_str());
            (!is_front, session.opened_at)
        });

        let each = (chars.unwrap_or(8000) / found.len()).max(1200);
        found
            .iter()
            .map(|session| {
                let mut reading = self.read(&session.id, Some(each));
                reading.front = front.as_deref() == Some(session.id.as_str());
                reading
            })
            .collect()
    }

    /// Ask the window to bring this terminal forward.
    ///
    /// A request rather than a command: the window decides what showing it
    /// means - see `features/chat/pane-reveal.js`, where opening the dock, the
    /// panel and the tab are three different owners.
    pub fn reveal(&self, id: &str) {
        self.sink.emit(json!({
            "type": "reveal",
            "id": id,
            "kind": "sh",
            "threadId": thread_of(id),
        }));
    }

    /// Make (or update) a tab that mirrors something the app is running.
    ///
    /// `shell` with `background: true` starts a dev server and hands back a
    /// pid. Everything it prints is readable by the agent and by nobody else,
    /// so the person watching their app fail to boot has to ask the agent what
    /// their own server said. That is exactly backwards. A mirror is a session
    /// with a screen and no shell behind it: bytes are fed in from wherever the
    /// process actually lives, and it appears in the pane as a tab like any
    /// other. Read-only, because there is nothing to type at.
    pub fn mirror(
        &self,
        id: &str,
        cwd: &str,
        title: &str,
        on_stop: Option<Box<dyn Fn() + Send + Sync>>,
    ) -> Snapshot {
        if let Some(existing) = self.sessions.lock().get(id) {
            {
                let mut inner = existing.inner.lock();
                if !title.is_empty() {
                    inner.title = title.to_string();
                }
            }
            if on_stop.is_some() {
                *existing.on_stop.lock() = on_stop;
            }
            return existing.snapshot();
        }

        let session = Arc::new(Session::blank(
            id,
            cwd.to_string(),
            DEFAULT_COLS,
            DEFAULT_ROWS,
            true,
        ));
        {
            let mut inner = session.inner.lock();
            inner.mirror = true;
            inner.title = title.to_string();
            // A mirrored process is running until it says otherwise, and saying
            // so is what makes `terminal_read` report a dev server as up rather
            // than as a tab with nothing in it.
            inner.running = Some(if title.is_empty() {
                "a background process".to_string()
            } else {
                title.to_string()
            });
        }
        *session.on_stop.lock() = on_stop;
        self.sessions
            .lock()
            .insert(id.to_string(), Arc::clone(&session));
        session.snapshot()
    }

    /// Bytes from whatever this tab is mirroring. Ignored for a real shell.
    ///
    /// `done` is the last of them: the process has stopped, so the tab is a
    /// record rather than a thing to stop, and nothing here should still claim
    /// to be running.
    pub fn feed(&self, id: &str, text: &str, done: bool) -> usize {
        let Ok(session) = self.get(id) else {
            return 0;
        };
        if !session.inner.lock().mirror {
            return 0;
        }
        if !text.is_empty() {
            push(&session, &self.sink, text);
        }
        if done {
            {
                let mut inner = session.inner.lock();
                inner.running = None;
            }
            *session.on_stop.lock() = None;
            // Said out loud, or the pane goes on offering a stop button for
            // something that has already stopped - and pressing it does
            // nothing, which is the worst kind of button.
            self.sink
                .emit(json!({ "type": "exit", "id": id, "code": Value::Null }));
        }
        text.len()
    }
}

/// Everything the shell says, until it stops saying anything.
fn read_loop(session: &Arc<Session>, sink: &Arc<Sink>, mut reader: Box<dyn Read + Send>) {
    let mut buffer = [0u8; 8192];
    let mut carry: Vec<u8> = Vec::new();

    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            // A closed pty reads as an error on Windows rather than as
            // end-of-file. Either way the shell is gone.
            Err(_) => break,
        };
        carry.extend_from_slice(&buffer[..read]);
        let text = take_utf8(&mut carry);
        if text.is_empty() {
            continue;
        }

        // Read before the bytes are handed on, not after: the window gets the
        // raw stream including the escape sequence, and xterm.js ignores the
        // one it does not recognise. Stripping it here to keep the pane clean
        // would mean parsing every byte of every build twice.
        if let Some(where_it_is) = ansi::read_cwd(&text) {
            let changed = {
                let mut inner = session.inner.lock();
                if inner.cwd == where_it_is {
                    false
                } else {
                    inner.cwd = where_it_is.clone();
                    true
                }
            };
            if changed {
                sink.emit(json!({ "type": "cwd", "id": session.id, "cwd": where_it_is }));
            }
        }

        push(session, sink, &text);
    }

    let code = session
        .child
        .lock()
        .as_mut()
        .and_then(|child| child.wait().ok())
        .map(|status| status.exit_code())
        .unwrap_or(0);
    session.inner.lock().alive = false;
    push(session, sink, &format!("\r\n[the shell exited with {code}]\r\n"));
    sink.emit(json!({ "type": "exit", "id": session.id, "code": code }));
}

/// As much of the buffer as is whole characters, leaving the rest for next time.
///
/// A read lands in the middle of a multi-byte character as often as not, and
/// `from_utf8_lossy` on the whole buffer would turn every one of those into a
/// replacement character that never goes away.
fn take_utf8(carry: &mut Vec<u8>) -> String {
    let mut out = String::new();
    loop {
        match std::str::from_utf8(carry) {
            Ok(text) => {
                out.push_str(text);
                carry.clear();
                return out;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                out.push_str(&String::from_utf8_lossy(&carry[..valid]));
                match error.error_len() {
                    // Genuinely invalid, rather than the front half of a
                    // character. Dropped and the scan continues, or one bad
                    // byte from a program printing binary would block every
                    // later read behind it for ever.
                    Some(len) => {
                        carry.drain(0..valid + len);
                    }
                    // The front half of a character whose tail is in the next
                    // read. Kept, or it becomes a replacement character that
                    // never goes away.
                    None => {
                        carry.drain(0..valid);
                        return out;
                    }
                }
            }
        }
    }
}

/// Kill a shell and everything it started.
///
/// The ORDER is the whole trick: killing the shell orphans its children, and an
/// orphan is no longer in the tree `taskkill /T` walks. So the tree goes first
/// and the pty is killed once that has finished. A `npm run dev` under a closed
/// tab is otherwise a port still held by a process with nothing left to show it.
///
/// On its own thread, because `taskkill` is a process to wait for and closing a
/// tab must return to the window immediately.
fn kill(session: Arc<Session>) {
    std::thread::spawn(move || {
        let pid = session
            .child
            .lock()
            .as_ref()
            .and_then(|child| child.process_id());
        if let Some(pid) = pid {
            kill_tree(pid);
        }
        if let Some(child) = session.child.lock().as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // Dropping the master closes our end, which is what releases the reader
        // thread if the shell somehow outlived the kill.
        session.master.lock().take();
        session.writer.lock().take();
    });
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    // Windows has no process group to signal, so this is `taskkill /T`, which
    // is what every other part of this app that stops a tree already uses. A
    // failure to kill what is already dead is the outcome we wanted.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/pid", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(windows))]
fn kill_tree(pid: u32) {
    // The shell is a process group leader on a pty, so the negative pid signals
    // everything it started.
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window that writes its events down.
    #[derive(Debug, Default)]
    struct Heard(Mutex<Vec<Value>>);

    impl Events for Heard {
        fn emit(&self, payload: Value) {
            self.0.lock().push(payload);
        }
    }

    impl Heard {
        fn types(&self) -> Vec<String> {
            self.0
                .lock()
                .iter()
                .filter_map(|event| event.get("type")?.as_str().map(str::to_string))
                .collect()
        }
    }

    fn marker(terminals: &Terminals, id: &str, wanted: &str) -> Option<String> {
        // ConPTY is not instant and the shell has a profile to load. Polled
        // rather than slept on a fixed number, because a fixed number is either
        // slow or flaky and usually both.
        for _ in 0..200 {
            let text = terminals.read(id, Some(8000)).text;
            if text.contains(wanted) {
                return Some(text);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        None
    }

    /// The whole thing, against a real shell on this machine.
    ///
    /// Skipped rather than failed where a pty cannot be opened at all - a
    /// build machine with no ConPTY, a container with no /dev/ptmx - because
    /// that is an environment saying it has no terminal, not this code being
    /// wrong.
    #[test]
    fn a_command_typed_into_a_shell_comes_back_as_its_output() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let terminals = Terminals::new();
        let heard = Arc::new(Heard::default());
        terminals.attach(heard.clone());

        let id = "chat:t1:sh:1";
        let opened = terminals.open(
            id,
            &OpenOptions {
                cwd: Some(dir.path().display().to_string()),
                cols: Some(100),
                rows: Some(30),
            },
        );
        let Ok(snapshot) = opened else {
            eprintln!("no pty here, skipping: {opened:?}");
            return;
        };
        assert!(snapshot.pty);
        assert!(!snapshot.mirror);

        // Typed in, the way the person would. The shell echoes it, so the
        // marker has to be something the echo cannot be mistaken for.
        terminals
            .run(id, "echo INERTIA_MARKER_OK")
            .expect("the command was typed in");

        let text = marker(&terminals, id, "INERTIA_MARKER_OK")
            .unwrap_or_else(|| panic!("the shell never printed the marker"));
        // Twice: once echoed by the shell, once as the output. Finding it at
        // all is the test - a grid-less stripper finds neither, which is the
        // bug `screen.rs` exists for.
        assert!(text.matches("INERTIA_MARKER_OK").count() >= 1, "{text}");

        // The window heard the bytes as they arrived rather than being asked to
        // poll for them.
        assert!(heard.types().iter().any(|kind| kind == "data"));

        assert!(terminals.close(id));
        assert!(!terminals.close(id));
    }

    /// The folder is not something a pty knows, so the shell is asked to say
    /// it from its own prompt (see `shell.rs`). Proved by MOVING: the folder a
    /// tab opens in is one this crate already knew, so a test that only checked
    /// that number would pass with the reporting removed entirely.
    #[test]
    fn the_shell_reports_the_folder_when_the_person_moves_it() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::create_dir(dir.path().join("moved")).expect("a folder to move into");
        let terminals = Terminals::new();

        let id = "chat:t2:sh:1";
        let opened = terminals.open(
            id,
            &OpenOptions {
                cwd: Some(dir.path().display().to_string()),
                cols: Some(100),
                rows: Some(30),
            },
        );
        if opened.is_err() {
            eprintln!("no pty here, skipping: {opened:?}");
            return;
        }

        // Waited for, or the `cd` is typed at a shell that has not drawn its
        // first prompt and the keystrokes land nowhere.
        assert!(
            marker(&terminals, id, ">").is_some(),
            "the shell never drew a prompt"
        );
        terminals.run(id, "cd moved").expect("the move was typed in");

        let mut reported = None;
        for _ in 0..200 {
            let cwd = terminals.read(id, None).cwd;
            if cwd.ends_with("moved") {
                reported = Some(cwd);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        terminals.close(id);
        assert!(reported.is_some(), "the shell never reported the move");
    }

    #[test]
    fn a_terminal_nobody_opened_is_an_empty_answer_rather_than_an_error() {
        let terminals = Terminals::new();
        let reading = terminals.read("chat:t9:sh:1", None);
        assert_eq!(reading.text, "");
        assert!(!reading.busy);
        assert!(terminals.read_thread("t9", None).is_empty());
        assert_eq!(terminals.front_of("t9"), None);
    }

    /// A job id names a thing the app was running. Spawning PowerShell under it
    /// would put a live prompt where a person expects a finished log.
    #[test]
    fn a_job_tab_reopened_after_the_process_died_is_a_record_not_a_shell() {
        let terminals = Terminals::new();
        let id = "chat:t3:job:8123";
        let snapshot = terminals
            .open(id, &OpenOptions::default())
            .expect("a mirror needs no shell");
        assert!(snapshot.mirror);
        assert!(!snapshot.pty);
        assert!(terminals.read(id, None).text.contains("no longer running"));
        // And nothing can be typed at it.
        assert_eq!(terminals.write(id, "rm -rf /"), Ok(0));
    }

    #[test]
    fn a_mirrored_process_is_a_tab_that_reports_itself_as_running() {
        let terminals = Terminals::new();
        let heard = Arc::new(Heard::default());
        terminals.attach(heard.clone());

        let id = "chat:t4:job:4242";
        let snapshot = terminals.mirror(id, "D:\\work", "npm run dev", None);
        assert!(snapshot.mirror);

        terminals.feed(id, "ready on :3000\n", false);
        let reading = terminals.read(id, None);
        assert!(reading.busy);
        assert_eq!(reading.running.as_deref(), Some("npm run dev"));
        assert!(reading.text.contains("ready on :3000"));

        terminals.feed(id, "stopped\n", true);
        assert!(!terminals.read(id, None).busy);
        assert!(heard.types().iter().any(|kind| kind == "exit"));
    }

    /// Every tab, not the front one, and the front one first.
    #[test]
    fn reading_a_conversation_returns_every_tab_with_the_front_one_first() {
        let terminals = Terminals::new();
        terminals.mirror("chat:t5:job:1", "/a", "build", None);
        std::thread::sleep(std::time::Duration::from_millis(5));
        terminals.mirror("chat:t5:job:2", "/b", "server", None);
        // A different conversation's tabs must not appear.
        terminals.mirror("chat:other:job:3", "/c", "elsewhere", None);
        terminals.feed("chat:t5:job:1", "first\n", false);
        terminals.feed("chat:t5:job:2", "second\n", false);

        let tabs = terminals.read_thread("t5", Some(8000));
        assert_eq!(tabs.len(), 2);
        let ids: Vec<&str> = tabs.iter().map(|tab| tab.id.as_str()).collect();
        assert_eq!(ids, vec!["chat:t5:job:1", "chat:t5:job:2"]);
        assert!(tabs[0].text.contains("first"));
        assert!(tabs[1].text.contains("second"));
    }

    #[test]
    fn reveal_asks_the_window_for_the_tab_and_names_the_conversation() {
        let terminals = Terminals::new();
        let heard = Arc::new(Heard::default());
        terminals.attach(heard.clone());
        terminals.reveal("chat:t6:sh:2");

        let events = heard.0.lock();
        let event = events.last().expect("an event");
        assert_eq!(event["type"], json!("reveal"));
        assert_eq!(event["id"], json!("chat:t6:sh:2"));
        assert_eq!(event["kind"], json!("sh"));
        assert_eq!(event["threadId"], json!("t6"));
    }

    #[test]
    fn a_conversation_id_is_read_off_the_tab_from_the_right() {
        assert_eq!(thread_of("chat:t1:sh:1"), Some("t1"));
        assert_eq!(thread_of("chat:a:b:job:99"), Some("a:b"));
        assert_eq!(thread_of("something-else"), None);
        assert!(is_mirror_id("chat:t1:job:7"));
        assert!(!is_mirror_id("chat:t1:sh:7"));
    }

    #[test]
    fn a_character_split_across_two_reads_is_not_lost() {
        let mut carry = Vec::new();
        carry.extend_from_slice(&[b'h', b'i', 0xc3]);
        assert_eq!(take_utf8(&mut carry), "hi");
        carry.push(0xa9);
        assert_eq!(take_utf8(&mut carry), "\u{e9}");
        // And a genuinely invalid byte does not block everything behind it.
        carry.extend_from_slice(&[0xff, b'o', b'k']);
        assert_eq!(take_utf8(&mut carry), "ok");
    }
}


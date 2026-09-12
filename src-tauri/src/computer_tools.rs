//! The agent's own machine.
//!
//! `shell` runs a command where Inertia is running - the user's laptop, in the
//! project folder they pointed at. These run somewhere else entirely: the
//! computer assigned to this agent, which is a real sandbox with its own
//! filesystem, its own network, and a screen.
//!
//! That distinction is why these are separate tools rather than a flag on
//! `shell`, and it is a distinction the model has to be able to reason about.
//! "Install this package" means one thing on the user's machine and another
//! inside a container that gets thrown away, and a model that cannot tell them
//! apart will eventually install something on the wrong one.
//!
//! The machine is not a parameter. It is whichever computer the agent has been
//! assigned in the app - handed to [`computer_tools`] when the turn is built -
//! so the model cannot reach a teammate's machine by naming one, and an agent
//! with no computer is not offered these tools at all.
//!
//! ## Why several tools rather than one with an `action`
//!
//! There was one, with thirteen actions and a schema carrying every argument
//! any of them might need. A model reads that as a menu and picks badly: it
//! would call `screenshot` with a `command`, or reach for `run` and pipe curl
//! into a file because the shape of the tool suggested a shell was the main
//! event.
//!
//! Split, each tool has a name that says what it is for and a schema with only
//! its own arguments. They still share one permission key, so the user sees a
//! single "computer" rule rather than nine - the split is for the model, and
//! the user should not pay for it.
//!
//! ## Why observing is part of acting
//!
//! `computer_act` performs a batch and returns the screen afterwards. Acting
//! and looking as separate calls doubles the round trips and, worse, invites
//! the model to act twice before looking once - clicking the second thing
//! against a picture taken before the first had happened.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use inertia_computers::desktop::{self, Action, Observation};
use inertia_computers::{ExecRequest, ExecResult, Provider};
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use serde_json::{json, Map, Value};

const DEFAULT_TIMEOUT_S: u64 = 120;
const MAX_TIMEOUT_S: u64 = 900;
const MAX_OUTPUT: usize = 60_000;

/// Shared by every tool here, so the user writes one rule and not nine.
const PERMISSION_KEY: &str = "computer";

/// The last frame each conversation was shown, so an unchanged screen can be
/// named instead of sent again.
///
/// Keyed by session and machine, because two agents on one machine are looking
/// at the same screen but have their own conversations, and a frame one of them
/// has seen tells you nothing about what the other has.
///
/// Process-wide rather than per-turn on purpose: the point is to recognise a
/// picture this conversation was shown three turns ago, and a table rebuilt
/// with the tools would forget it every time.
static FRAMES: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Forgets every remembered frame. For teardown, and for a machine released.
pub fn forget_frames() {
    if let Ok(mut frames) = FRAMES.lock() {
        frames.clear();
    }
}

/// The machine these tools drive, and how to reach it.
///
/// Built once per turn from the record the coordinator read, rather than looked
/// up per call. The record is the app's own memory of what the machine is
/// doing, which is what the Computers screen polls and repairs; asking the
/// provider again on every tool call would be a container round trip to learn
/// something that was true a second ago.
#[derive(Clone)]
struct Machine {
    provider: Arc<dyn Provider>,
    id: String,
    name: String,
    handle: String,
    provider_id: String,
    workdir: String,
    status: String,
}

impl std::fmt::Debug for Machine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Machine")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl Machine {
    fn read(provider: Arc<dyn Provider>, record: &Value) -> Self {
        let text = |key: &str| {
            record
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let id = text("id");
        let name = {
            let named = text("name");
            if named.is_empty() {
                id.clone()
            } else {
                named
            }
        };
        let provider_id = {
            let named = text("provider");
            if named.is_empty() {
                provider.id().to_string()
            } else {
                named
            }
        };
        // Not a hardcoded /workspace: that folder exists in our container and
        // nowhere else, and a default that points at a missing directory reads
        // as the tool being broken rather than the path being wrong.
        let workdir = {
            let recorded = text("workdir");
            if !recorded.is_empty() {
                recorded
            } else if provider_id == "daytona" {
                "/home/daytona".to_string()
            } else {
                "/workspace".to_string()
            }
        };

        Self {
            provider,
            id,
            name,
            handle: text("handle"),
            provider_id,
            workdir,
            status: text("status"),
        }
    }

    /// The machine has to be up before anything is asked of it.
    fn ready(&self) -> Result<()> {
        if self.status == "running" {
            return Ok(());
        }
        let doing = if self.status.is_empty() {
            "not running"
        } else {
            &self.status
        };
        Err(Error::Other(format!(
            "Your computer \"{}\" is {doing}. Ask the user to start it.",
            self.name
        )))
    }

    async fn exec(&self, command: String, timeout: Duration) -> Result<ExecResult> {
        self.ready()?;
        self.provider
            .exec(
                &self.handle,
                &ExecRequest {
                    command,
                    timeout: Some(timeout),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| Error::Other(e.to_string()))
    }

    /// Runs one of the desktop commands and turns its exit code into a
    /// sentence.
    ///
    /// Exit 3 is the agreed "this machine cannot do that", and it is the one
    /// case worth raising rather than returning: a model handed "no display" as
    /// ordinary output will try clicking anyway.
    async fn drive(&self, command: String, timeout_ms: u64) -> Result<ExecResult> {
        let result = self
            .exec(command, Duration::from_millis(timeout_ms))
            .await?;
        if result.code == desktop::NO_DESKTOP_CODE {
            let said = said(&result);
            return Err(Error::Other(if said.is_empty() {
                format!("Your computer \"{}\" has no desktop.", self.name)
            } else {
                said
            }));
        }
        Ok(result)
    }
}

/// What a command said, both streams together.
fn said(result: &ExecResult) -> String {
    [result.stdout.as_str(), result.stderr.as_str()]
        .iter()
        .filter(|part| !part.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Cuts a string to at most `limit` bytes without splitting a character.
///
/// Byte-counted because the cap exists to bound what reaches the model, and
/// landing mid-character would produce a string that is not valid UTF-8 at the
/// provider boundary.
fn cap(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// An observation, in the shape a tool result takes.
///
/// The metadata always goes back as text. The picture only goes back when it
/// differs from the last one this conversation was shown - a model watching a
/// page load does not need six identical screenshots of a spinner, and sending
/// them is how a conversation becomes mostly pictures of nothing happening.
fn observation_result(
    seen: &Observation,
    ctx: &ToolContext,
    machine: &Machine,
    note: &str,
    clicked: Option<(i64, i64)>,
) -> ToolOutcome {
    let key = format!("{}:{}", ctx.session.as_str(), machine.id);
    let id = desktop::frame_id(seen.image.as_deref());
    let unchanged = match (&id, FRAMES.lock().ok()) {
        (Some(id), Some(mut frames)) => {
            let same = frames.get(&key) == Some(id);
            frames.insert(key, id.clone());
            same
        }
        _ => false,
    };

    let mut facts = vec![format!("{}x{}", seen.width, seen.height)];
    if let Some(title) = &seen.window_title {
        facts.push(format!("active window: {title}"));
    }
    if let Some((x, y)) = seen.cursor {
        facts.push(format!("pointer at {x},{y} (the red ring)"));
    }

    let mut lines = vec![
        format!("{note}{}", if unchanged { " (screen unchanged)" } else { "" }),
        facts.join(" | "),
    ];
    if !unchanged {
        // Said every time rather than once in the prompt, because it is read
        // beside the picture it describes and a summarised conversation keeps
        // the picture and loses the prompt's paragraph about it.
        if seen.image.is_some() {
            lines.push(
                "The picture has coordinate rulers along its top and left edges: read x off the \
                 top, y off the left."
                    .to_string(),
            );
        }
        if let (Some((x, y)), true) = (clicked, seen.closeup.is_some()) {
            lines.push(format!(
                "Then a close-up of {x},{y} at twice the size, crosshair on the exact point you \
                 clicked. If the crosshair is not on the thing you meant, you missed: adjust the \
                 coordinates before typing anything."
            ));
        }
    }
    if seen.image.is_none() {
        lines.push(
            "The screen could not be photographed. This machine may have no display.".to_string(),
        );
    }

    let mut images = Vec::new();
    let mut metadata = Map::new();
    metadata.insert("machine".into(), json!(machine.id));
    metadata.insert("frameId".into(), json!(id));
    metadata.insert("unchanged".into(), json!(unchanged));
    metadata.insert("width".into(), json!(seen.width));
    metadata.insert("height".into(), json!(seen.height));
    if let Some((x, y)) = seen.cursor {
        metadata.insert("cursor".into(), json!({ "x": x, "y": y }));
    }
    if let Some((x, y)) = clicked {
        metadata.insert("clicked".into(), json!({ "x": x, "y": y }));
    }
    if let Some(window) = &seen.window_id {
        let mut active = json!({ "id": window });
        if let Some(title) = &seen.window_title {
            active["title"] = json!(title);
        }
        metadata.insert("activeWindow".into(), active);
    }
    if let Some(image) = &seen.image {
        let url = format!("data:image/png;base64,{image}");
        if !unchanged {
            images.push(url.clone());
        }
        metadata.insert("screenshot".into(), json!(url));
    }
    if let (Some(closeup), Some(_)) = (&seen.closeup, clicked) {
        let url = format!("data:image/png;base64,{closeup}");
        if !unchanged {
            images.push(url.clone());
        }
        metadata.insert("closeup".into(), json!(url));
    }

    ToolOutcome {
        title: Some(machine.name.clone()),
        output: lines.join("\n"),
        metadata: Some(Value::Object(metadata)),
        images,
    }
}

/// The permission every tool here asks for.
fn permission(target: &str, always: &str) -> PermissionRequest {
    PermissionRequest::new(PERMISSION_KEY, target).with_always(always)
}

fn text_arg(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/* -- looking -------------------------------------------------------------- */

#[derive(Debug)]
struct ObserveTool(Machine);

#[async_trait]
impl Tool for ObserveTool {
    fn id(&self) -> &str {
        "computer_observe"
    }

    fn description(&self) -> &str {
        "Look at the screen of your own computer. Returns the size, the active window, the cursor \
         position and a picture. Observe before any click that uses coordinates, and again \
         whenever something else may have changed the screen. If the screen has not changed since \
         you last looked you are told so instead of being shown the same picture again."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [], "additionalProperties": false })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        permission("observe", "observe")
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some("look at the screen".into())
    }

    async fn execute(&self, _args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let result = self.0.drive(desktop::observe_annotated(), 45_000).await?;
        let seen = desktop::parse_observation(&result.stdout);
        Ok(observation_result(&seen, ctx, &self.0, "observed", None))
    }
}

/* -- acting --------------------------------------------------------------- */

#[derive(Debug)]
struct ActTool {
    machine: Machine,
    description: String,
}

#[async_trait]
impl Tool for ActTool {
    fn id(&self) -> &str {
        "computer_act"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "actions": {
                    "type": "array",
                    "description": "The actions to perform, in order",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": {
                                "type": "string",
                                "enum": desktop::KINDS,
                                "description": "What kind of action"
                            },
                            "x": { "type": "integer", "description": "For click, move, down and up: x in screen pixels" },
                            "y": { "type": "integer", "description": "For click, move, down and up: y in screen pixels" },
                            "button": { "type": "string", "enum": ["left", "right"], "description": "For click: which button. Default left" },
                            "double": { "type": "boolean", "description": "For click: double-click" },
                            "text": { "type": "string", "description": "For type: the text to enter" },
                            "key": { "type": "string", "description": "For key: the key, such as Return, Tab, Escape, a" },
                            "modifiers": {
                                "type": "array",
                                "items": { "type": "string", "description": "A modifier: ctrl, alt, shift, super" }
                            },
                            "direction": { "type": "string", "enum": ["up", "down"], "description": "For scroll: which way" },
                            "amount": { "type": "integer", "description": "For scroll: notches, 1 to 20. Default 3" },
                            "ms": { "type": "integer", "description": "For wait: milliseconds, up to 30000" }
                        },
                        "required": ["kind"],
                        "additionalProperties": false
                    }
                },
                "settle_ms": { "type": "integer", "description": "How long to let the screen settle before looking. Default 350" },
                "observe": { "type": "boolean", "description": "Return the screen afterwards. Default true" }
            },
            // Not required at the schema. A call with the fields of one action
            // at the top level and no list is read by `normalize` below; one
            // with nothing at all is refused by the compiler, with the example.
            "required": [],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        permission("act", "act")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let list = args.get("actions").and_then(Value::as_array);
        let count = list.map_or(0, Vec::len);
        let first = list
            .and_then(|items| items.first())
            .and_then(|item| item.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("act")
            .to_string();
        Some(if count > 1 {
            format!("{first} and {} more", count - 1)
        } else {
            first
        })
    }

    /// The call as the model sent it, put into the shape the schema describes.
    ///
    /// The schema is right and models still miss it: one action instead of a
    /// list, the kind as a string with the coordinates beside the list, the
    /// fields of one action at the top level with no list at all.
    /// `lift_actions` reads all of those; this only has to hand it the whole
    /// call so the stray top-level fields are there to be read, and to accept
    /// `settleMs` beside `settle_ms`.
    fn normalize(&self, args: Value) -> Value {
        if !args.is_object() {
            return args;
        }
        let Ok(lifted) = desktop::lift_actions(args.get("actions"), &args) else {
            // Unrepairable is left exactly as it came, for the validator and
            // then the compiler to report properly.
            return args;
        };

        let mut out = json!({ "actions": lifted });
        let wait = args
            .get("settle_ms")
            .or_else(|| args.get("settleMs"))
            .filter(|value| !value.is_null());
        if let Some(wait) = wait {
            out["settle_ms"] = wait.clone();
        }
        if let Some(observe) = args.get("observe").filter(|value| !value.is_null()) {
            out["observe"] = observe.clone();
        }
        out
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        // A malformed batch is the model's mistake to fix, and it can only fix
        // it if it is told which part was wrong.
        let actions: Vec<Action> =
            desktop::parse_actions(args.get("actions"), &args).map_err(Error::Other)?;

        let wants_screen = args.get("observe") != Some(&Value::Bool(false));
        let command = desktop::batch(
            &actions,
            args.get("settle_ms").and_then(Value::as_i64),
            wants_screen,
        );
        let result = self.machine.drive(command, 120_000).await?;

        let note = format!(
            "did {} action{}",
            actions.len(),
            if actions.len() == 1 { "" } else { "s" }
        );
        if !wants_screen {
            return Ok(ToolOutcome {
                title: Some(self.machine.name.clone()),
                output: format!(
                    "{note}. You did not ask for the screen; observe when you need to see it."
                ),
                metadata: Some(json!({
                    "machine": self.machine.id,
                    "completed": actions.len(),
                })),
                images: Vec::new(),
            });
        }

        let seen = desktop::parse_observation(&result.stdout);
        Ok(observation_result(
            &seen,
            ctx,
            &self.machine,
            &note,
            desktop::last_press(&actions),
        ))
    }
}

/* -- opening -------------------------------------------------------------- */

#[derive(Debug)]
struct OpenTool(Machine);

#[async_trait]
impl Tool for OpenTool {
    fn id(&self) -> &str {
        "computer_open"
    }

    fn description(&self) -> &str {
        "Open a file from your computer's workspace, or an http(s) URL, in whatever application \
         handles it, and return the resulting screen. NOT the same as `browser_navigate`. This \
         opens a page inside YOUR SANDBOX - a separate machine with its own network, its own \
         cookies and its own filesystem, which the person cannot see except through the pictures \
         you take. Use it when the point is to be somewhere that is not the user's machine: \
         signing into an account of your own, running something untrusted, or working while they \
         use their computer for something else. It cannot reach the user's localhost and it does \
         not have their logins."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "A path on the machine, or an http(s) URL" }
            },
            "required": ["target"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        permission(&text_arg(args, "target"), "open")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("open {}", cap(&text_arg(args, "target"), 80)))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let target = text_arg(&args, "target").trim().to_string();
        if target.is_empty() {
            return Err(Error::Other("open needs a path or a URL.".into()));
        }
        let scheme = target.to_lowercase();
        if scheme.starts_with("javascript:")
            || scheme.starts_with("data:")
            || scheme.starts_with("file:")
        {
            return Err(Error::Other(format!(
                "Refusing to open \"{target}\". Use an http(s) URL or a path."
            )));
        }

        self.0.drive(desktop::open(&target), 90_000).await?;
        let looked = self.0.drive(desktop::observe_annotated(), 45_000).await?;
        let seen = desktop::parse_observation(&looked.stdout);
        Ok(observation_result(
            &seen,
            ctx,
            &self.0,
            &format!("opened {target}"),
            None,
        ))
    }
}

#[derive(Debug)]
struct LaunchTool(Machine);

#[async_trait]
impl Tool for LaunchTool {
    fn id(&self) -> &str {
        "computer_launch"
    }

    fn description(&self) -> &str {
        "Start an application on your computer and return the resulting screen. Use \"browser\" \
         for the web browser; anything else is taken as a command name on the machine."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "application": { "type": "string", "description": "The application, or \"browser\"" },
                "uri": { "type": "string", "description": "Optional argument to open with it" }
            },
            "required": ["application"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        permission(&text_arg(args, "application"), "launch")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("launch {}", text_arg(args, "application")))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
        let application = text_arg(&args, "application");
        let uri = args
            .get("uri")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty());

        self.0
            .drive(desktop::launch(&application, uri), 90_000)
            .await?;
        let looked = self.0.drive(desktop::observe_annotated(), 45_000).await?;
        let seen = desktop::parse_observation(&looked.stdout);
        Ok(observation_result(
            &seen,
            ctx,
            &self.0,
            &format!("launched {application}"),
            None,
        ))
    }
}

/* -- reading a page without eyes ------------------------------------------ */

/// The one way page text lies.
///
/// There is no debugging port on that browser, so the page is read by selecting
/// all and copying. Select-all belongs to whatever has the keyboard, and if
/// that is a text box - which it is, every time, immediately after typing into
/// a form - the clipboard comes back holding the box's contents and nothing
/// else. It was watched happening: an agent filling a signup form read the page
/// three times and got the password it had just typed, each time, and spent its
/// reasoning wondering why the page had been redacted.
///
/// The copy cannot be made to ignore the focused field from out here, so the
/// result says so instead. A page is many lines; a form field is one short one,
/// which is a good enough tell to warn on and a bad one to act on silently.
const FIELD_SUSPECT_CHARS: usize = 120;

const FIELD_WARNING: &str = "\n\n[This is what the browser copied, and it is short and unbroken - \
     which is what a text box's contents look like, not a page. The copy takes whatever holds the \
     keyboard, so if you have just typed into a field this is that field. Click a blank part of \
     the page and read again to get the page itself.]";

/// Whether what came back looks like one form field rather than a page.
pub fn looks_like_a_field(text: &str) -> bool {
    let body = text.trim();
    !body.is_empty() && body.chars().count() < FIELD_SUSPECT_CHARS && !body.contains('\n')
}

#[derive(Debug)]
struct PageTextTool(Machine);

#[async_trait]
impl Tool for PageTextTool {
    fn id(&self) -> &str {
        "computer_page_text"
    }

    fn description(&self) -> &str {
        "Read the text of the page the browser is showing, as text rather than as a picture. Much \
         cheaper than a screenshot and usually enough when you need to know what a page says \
         rather than what it looks like. Use a screenshot when the layout matters or you need to \
         click."
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {}, "required": [], "additionalProperties": false })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        permission("page_text", "page_text")
    }

    fn render(&self, _args: &Value) -> Option<String> {
        Some("read the page".into())
    }

    async fn execute(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let result = self.0.drive(desktop::page_text(), 45_000).await?;
        let text = said(&result);
        if text.trim().is_empty() {
            return Err(Error::Other(
                "The page came back empty. It may still be loading, or nothing is open. Open \
                 something first, or observe to see what is there."
                    .into(),
            ));
        }
        let suspect = looks_like_a_field(&text);
        Ok(ToolOutcome {
            title: Some(format!("page text on {}", self.0.name)),
            output: format!(
                "{}{}",
                cap(&text, MAX_OUTPUT),
                if suspect { FIELD_WARNING } else { "" }
            ),
            metadata: Some(json!({
                "machine": self.0.id,
                "bytes": text.len(),
                "fromField": suspect,
            })),
            images: Vec::new(),
        })
    }
}

/* -- the shell and the filesystem ----------------------------------------- */

#[derive(Debug)]
struct RunTool(Machine);

#[async_trait]
impl Tool for RunTool {
    fn id(&self) -> &str {
        "computer_run"
    }

    fn description(&self) -> &str {
        "Run a shell command on your own computer - a Linux machine with its own filesystem and \
         network, separate from the one Inertia is running on. Use it for anything that should \
         not touch the user's machine: installing packages, running untrusted code, long builds, \
         scratch work. Use the shell tool instead when the work is on the user's own files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to run" },
                "cwd": { "type": "string", "description": "The directory to run in. Defaults to /workspace" },
                "timeout": { "type": "integer", "description": format!("Seconds to wait. Default {DEFAULT_TIMEOUT_S}") }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, args: &Value) -> PermissionRequest {
        permission(&text_arg(args, "command"), "*")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(cap(&text_arg(args, "command"), 120))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        self.0.ready()?;
        let command = text_arg(&args, "command").trim().to_string();
        if command.is_empty() {
            return Err(Error::Other("run needs a command.".into()));
        }

        let seconds = args
            .get("timeout")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_TIMEOUT_S)
            .min(MAX_TIMEOUT_S);

        // The machine's own working folder when the model does not name one,
        // which is what the schema has always promised. Passing `None` through
        // left the choice to the provider, and the two do not agree: Docker
        // takes the image WORKDIR, Daytona the sandbox user's home. So a model
        // that read the description and omitted `cwd` landed somewhere else on
        // Daytona than the one place its files are.
        let cwd = args
            .get("cwd")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map_or_else(|| self.0.workdir.clone(), str::to_string);
        let cwd = Some(cwd).filter(|path| !path.is_empty());

        let result = self
            .0
            .provider
            .exec(
                &self.0.handle,
                &ExecRequest {
                    command: command.clone(),
                    cwd,
                    timeout: Some(Duration::from_secs(seconds)),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        // A non-zero exit is a result, not a failure of the tool: a model told
        // the tests failed goes and looks at why; one handed an exception
        // cannot tell that from an unreachable machine.
        let body = said(&result);
        let status = if result.timed_out {
            format!("Timed out after {seconds}s.")
        } else if result.code == 0 {
            String::new()
        } else {
            format!("Exit code {}.", result.code)
        };

        let output = [
            if body.is_empty() { "(no output)" } else { &body },
            &status,
        ]
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n");

        Ok(ToolOutcome {
            title: Some(format!("{} on {}", cap(&command, 60), self.0.name)),
            output: cap(&output, MAX_OUTPUT),
            metadata: Some(json!({
                "machine": self.0.id,
                "provider": self.0.provider_id,
                "exitCode": result.code,
                "timedOut": result.timed_out,
                "durationMs": result.duration_ms,
            })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct ListTool(Machine);

#[async_trait]
impl Tool for ListTool {
    fn id(&self) -> &str {
        "computer_list"
    }

    fn description(&self) -> &str {
        "List a directory on your own computer. Defaults to where your work lives."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The directory. Defaults to your working folder." }
            },
            "required": [],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        permission("list", "list")
    }

    fn render(&self, args: &Value) -> Option<String> {
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        Some(format!("list {path}"))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        self.0.ready()?;
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map_or_else(|| self.0.workdir.clone(), str::to_string);

        let rows = self
            .0
            .provider
            .list_dir(&self.0.handle, &path)
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        let mut lines = vec![path.clone()];
        if rows.is_empty() {
            lines.push("(empty)".into());
        }
        for row in &rows {
            lines.push(if row.kind == "dir" {
                format!("{}/", row.name)
            } else {
                format!(
                    "{}  {}b",
                    row.name,
                    row.size.map_or_else(|| "?".to_string(), |size| size.to_string())
                )
            });
        }

        Ok(ToolOutcome {
            title: Some(format!("{path} on {}", self.0.name)),
            output: cap(&lines.join("\n"), MAX_OUTPUT),
            metadata: Some(json!({
                "machine": self.0.id,
                "path": path,
                "entries": rows.len(),
            })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct ReadTool(Machine);

#[async_trait]
impl Tool for ReadTool {
    fn id(&self) -> &str {
        "computer_read"
    }

    fn description(&self) -> &str {
        "Read a text file from your own computer. Open a picture or a PDF with computer_open \
         instead."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "The file to read" } },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        permission("read", "read")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("read {}", text_arg(args, "path")))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        self.0.ready()?;
        let path = text_arg(&args, "path");
        if path.trim().is_empty() {
            return Err(Error::Other("read needs a path.".into()));
        }
        let text = self
            .0
            .provider
            .read_file(&self.0.handle, &path)
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        Ok(ToolOutcome {
            title: Some(format!("{path} on {}", self.0.name)),
            output: cap(&text, MAX_OUTPUT),
            metadata: Some(json!({
                "machine": self.0.id,
                "path": path,
                "bytes": text.len(),
            })),
            images: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct WriteTool(Machine);

#[async_trait]
impl Tool for WriteTool {
    fn id(&self) -> &str {
        "computer_write"
    }

    fn description(&self) -> &str {
        "Create or overwrite a text file on your own computer."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The file to write" },
                "content": { "type": "string", "description": "Its new contents" }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        permission("write", "write")
    }

    fn render(&self, args: &Value) -> Option<String> {
        Some(format!("write {}", text_arg(args, "path")))
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        self.0.ready()?;
        let path = text_arg(&args, "path");
        if path.trim().is_empty() {
            return Err(Error::Other("write needs a path.".into()));
        }
        let content = text_arg(&args, "content");
        self.0
            .provider
            .write_file(&self.0.handle, &path, &content)
            .await
            .map_err(|e| Error::Other(e.to_string()))?;

        let bytes = content.len();
        Ok(ToolOutcome {
            title: Some(format!("{path} on {}", self.0.name)),
            output: format!("Wrote {bytes} bytes to {path}."),
            metadata: Some(json!({
                "machine": self.0.id,
                "path": path,
                "bytes": bytes,
            })),
            images: Vec::new(),
        })
    }
}

/// Every tool that drives the agent's machine.
///
/// Built per turn and handed the machine, rather than looking one up per call:
/// `ToolContext` carries the session but not the seat, and a tool that had to
/// guess which agent called it would be one call away from driving a
/// teammate's computer.
///
/// `machine` is the stored record, as JSON. Not a typed mirror: records on disk
/// carry fields this code has never heard of, and a struct that round-trips
/// them silently drops the ones it does not know.
pub fn computer_tools(provider: Arc<dyn Provider>, machine: Value) -> Vec<Arc<dyn Tool>> {
    let machine = Machine::read(provider, &machine);

    let act_description = format!(
        "Perform up to {max} ordered actions on your computer's screen and return the screen \
         afterwards, with a close-up of where the last click landed.\n\n\
         Actions: click (x, y, optional button left|right and double), move (x, y), down (x, y), \
         up (x, y), type (text - pasted, so it is safe for URLs and passwords), key (key, \
         optional modifiers like [\"ctrl\"]), scroll (direction up|down, optional amount 1-20), \
         wait (ms).\n\n\
         The call is a list of action objects: {example}\n\n\
         Batch only what you can predict. Opening a menu and clicking an item you have not seen \
         yet is two calls, not one - stop before any outcome you need to look at. Coordinates \
         come from the rulers on the picture you were last shown, so observe first and never \
         guess where something is. After a click, check the close-up: the crosshair must be on \
         the control you meant before you type into it.",
        max = desktop::MAX_ACTIONS,
        example = desktop::EXAMPLE,
    );

    vec![
        Arc::new(ObserveTool(machine.clone())),
        Arc::new(ActTool {
            machine: machine.clone(),
            description: act_description,
        }),
        Arc::new(OpenTool(machine.clone())),
        Arc::new(LaunchTool(machine.clone())),
        Arc::new(PageTextTool(machine.clone())),
        Arc::new(RunTool(machine.clone())),
        Arc::new(ListTool(machine.clone())),
        Arc::new(ReadTool(machine.clone())),
        Arc::new(WriteTool(machine)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_computers::{
        Created, DirEntry, Health, Readiness, Screen, Snapshot, Spec, Stats, Status,
    };
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_mock::MockGate;

    /// A machine that never existed, answering from a script.
    ///
    /// The commands are recorded because half of what these tools do is
    /// compose a shell line, and a fake that only returned output would let a
    /// wrong command pass every test.
    #[derive(Debug, Default)]
    struct FakeProvider {
        stdout: Mutex<Vec<String>>,
        code: i32,
        commands: Mutex<Vec<String>>,
        files: Mutex<HashMap<String, String>>,
        entries: Mutex<Vec<DirEntry>>,
    }

    impl FakeProvider {
        fn answering(lines: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                stdout: Mutex::new(lines.iter().map(|line| (*line).to_string()).collect()),
                ..Default::default()
            })
        }

        fn failing(code: i32, said: &str) -> Arc<Self> {
            Arc::new(Self {
                stdout: Mutex::new(vec![said.to_string()]),
                code,
                ..Default::default()
            })
        }

        fn commands(&self) -> Vec<String> {
            self.commands.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Provider for FakeProvider {
        fn id(&self) -> &'static str {
            "docker"
        }
        fn label(&self) -> &'static str {
            "Fake"
        }
        fn blurb(&self) -> &'static str {
            "A machine that never existed."
        }
        async fn available(&self) -> Readiness {
            Readiness::ready()
        }
        async fn create(&self, _spec: &Spec) -> inertia_computers::Result<Created> {
            unimplemented!("the tools never provision")
        }
        async fn start(&self, _handle: &str) -> inertia_computers::Result<()> {
            Ok(())
        }
        async fn stop(&self, _handle: &str) -> inertia_computers::Result<()> {
            Ok(())
        }
        async fn pause(&self, _handle: &str) -> inertia_computers::Result<()> {
            Ok(())
        }
        async fn resume(&self, _handle: &str) -> inertia_computers::Result<()> {
            Ok(())
        }
        async fn remove(&self, _handle: &str, _name: Option<&str>) -> inertia_computers::Result<()> {
            Ok(())
        }
        async fn status(&self, _handle: &str) -> Health {
            Health {
                status: Status::Running,
                started_at: None,
            }
        }
        async fn stats(&self, _handle: &str) -> Stats {
            Stats::default()
        }

        async fn exec(
            &self,
            _handle: &str,
            request: &ExecRequest,
        ) -> inertia_computers::Result<ExecResult> {
            self.commands.lock().unwrap().push(request.command.clone());
            let mut queued = self.stdout.lock().unwrap();
            let said = if queued.is_empty() {
                String::new()
            } else {
                queued.remove(0)
            };
            Ok(ExecResult {
                code: self.code,
                stdout: if self.code == 0 { said.clone() } else { String::new() },
                stderr: if self.code == 0 { String::new() } else { said },
                duration_ms: 12,
                timed_out: false,
            })
        }

        async fn list_dir(
            &self,
            _handle: &str,
            _path: &str,
        ) -> inertia_computers::Result<Vec<DirEntry>> {
            Ok(self.entries.lock().unwrap().clone())
        }
        async fn read_file(&self, _handle: &str, path: &str) -> inertia_computers::Result<String> {
            self.files
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| {
                    inertia_computers::ComputerError::Failed(format!("File not found: {path}"))
                })
        }
        async fn read_file_base64(
            &self,
            _handle: &str,
            _path: &str,
        ) -> inertia_computers::Result<String> {
            Ok(String::new())
        }
        async fn write_file(
            &self,
            _handle: &str,
            path: &str,
            content: &str,
        ) -> inertia_computers::Result<()> {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), content.to_string());
            Ok(())
        }
        async fn snapshot(
            &self,
            _handle: &str,
            _name: &str,
        ) -> inertia_computers::Result<Snapshot> {
            unimplemented!("the tools never snapshot")
        }
        async fn snapshots(&self, _handle: &str) -> inertia_computers::Result<Vec<Snapshot>> {
            Ok(Vec::new())
        }
        async fn restore(
            &self,
            _handle: &str,
            _snapshot_id: &str,
            _name: &str,
        ) -> inertia_computers::Result<Created> {
            unimplemented!("the tools never restore")
        }
        async fn screenshot(&self, _handle: &str) -> inertia_computers::Result<Option<String>> {
            Ok(None)
        }
        async fn screen(&self, _handle: &str) -> inertia_computers::Result<Option<Screen>> {
            Ok(None)
        }
    }

    fn record(status: &str) -> Value {
        json!({
            "id": "box",
            "name": "Sandbox",
            "handle": "c0ffee",
            "provider": "docker",
            "status": status,
            "workdir": "/workspace",
        })
    }

    /// A context built by hand rather than from `AppState`: constructing the
    /// app's state inside a test links Tauri's window chrome into the harness
    /// and the whole binary fails to start on Windows.
    fn context(session: &str) -> ToolContext {
        ToolContext {
            root: std::env::temp_dir(),
            session: SessionId::from_existing(session),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(MockGate::allow_all()),
        }
    }

    fn tool(tools: &[Arc<dyn Tool>], id: &str) -> Arc<dyn Tool> {
        tools
            .iter()
            .find(|tool| tool.id() == id)
            .expect("the family must carry that tool")
            .clone()
    }

    /// A screen with one pixel in it, as the machine would print it.
    fn observed(image: &str) -> String {
        format!("WIDTH=1600\nHEIGHT=900\nX=10\nY=20\nWINDOW=42\nCOMPLETED=1\nIMAGE={image}\n")
    }

    #[test]
    fn the_family_is_the_same_nine_tools_the_renderer_draws() {
        let tools = computer_tools(FakeProvider::answering(&[]), record("running"));
        let mut ids: Vec<&str> = tools.iter().map(|tool| tool.id()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "computer_act",
                "computer_launch",
                "computer_list",
                "computer_observe",
                "computer_open",
                "computer_page_text",
                "computer_read",
                "computer_run",
                "computer_write",
            ]
        );
    }

    /// One rule, not nine. The split is for the model and the user should not
    /// pay for it.
    #[test]
    fn every_tool_asks_under_the_one_shared_key() {
        let tools = computer_tools(FakeProvider::answering(&[]), record("running"));
        for tool in &tools {
            let asked = tool.permission(&json!({ "command": "ls", "target": "x", "application": "browser" }));
            assert_eq!(asked.key, "computer", "{} asked elsewhere", tool.id());
            assert!(asked.always.is_some(), "{} cannot be allowed always", tool.id());
        }
    }

    /// The `always` strings are what a stored rule is written against, so they
    /// have to be the ones the other shell wrote.
    #[test]
    fn the_remembered_targets_match_the_shell_this_was_ported_from() {
        let tools = computer_tools(FakeProvider::answering(&[]), record("running"));
        let always = |id: &str, args: Value| tool(&tools, id).permission(&args).always.unwrap();
        assert_eq!(always("computer_observe", json!({})), "observe");
        assert_eq!(always("computer_act", json!({})), "act");
        assert_eq!(always("computer_open", json!({ "target": "https://x" })), "open");
        assert_eq!(always("computer_launch", json!({ "application": "browser" })), "launch");
        assert_eq!(always("computer_page_text", json!({})), "page_text");
        // `run` is the one that generalises to everything: approving one
        // command should not mean approving only that exact string forever.
        assert_eq!(always("computer_run", json!({ "command": "ls" })), "*");
        assert_eq!(always("computer_list", json!({})), "list");
        assert_eq!(always("computer_read", json!({ "path": "a" })), "read");
        assert_eq!(always("computer_write", json!({ "path": "a" })), "write");

        // The target is the thing a person would write a rule about.
        assert_eq!(
            tool(&tools, "computer_run").permission(&json!({ "command": "npm test" })).target,
            "npm test"
        );
        assert_eq!(
            tool(&tools, "computer_open").permission(&json!({ "target": "https://x" })).target,
            "https://x"
        );
    }

    /// A machine that is not up is a sentence the model can act on, not a
    /// command that fails inside a container that is not there.
    #[tokio::test]
    async fn a_stopped_machine_says_so_and_says_what_to_do() {
        let tools = computer_tools(FakeProvider::answering(&[]), record("stopped"));
        for id in ["computer_observe", "computer_run", "computer_list", "computer_read"] {
            let failure = tool(&tools, id)
                .execute(json!({ "command": "ls", "path": "a" }), &context("s1"))
                .await
                .unwrap_err()
                .to_string();
            assert!(failure.contains("Sandbox"), "{id}: {failure}");
            assert!(failure.contains("stopped"), "{id}: {failure}");
            assert!(failure.contains("start it"), "{id}: {failure}");
        }
    }

    #[tokio::test]
    async fn observing_sends_the_picture_and_reads_the_rulers_out_loud() {
        let provider = FakeProvider::answering(&[&observed("iVBORw0Kone")]);
        let tools = computer_tools(provider.clone(), record("running"));
        let outcome = tool(&tools, "computer_observe")
            .execute(json!({}), &context("s-observe"))
            .await
            .unwrap();

        assert_eq!(outcome.images.len(), 1);
        assert!(outcome.images[0].starts_with("data:image/png;base64,iVBORw0K"));
        assert!(outcome.output.contains("1600x900"));
        assert!(outcome.output.contains("pointer at 10,20"));
        assert!(outcome.output.contains("coordinate rulers"));
        assert_eq!(outcome.metadata.unwrap()["machine"], json!("box"));
        // It asked for the annotated screen, which is what carries the rulers.
        assert!(provider.commands()[0].contains("convert -"));
    }

    /// The whole point of remembering a frame: a model watching a page load
    /// does not need six identical pictures of a spinner.
    #[tokio::test]
    async fn the_same_screen_twice_is_named_rather_than_sent_again() {
        let provider = FakeProvider::answering(&[
            &observed("iVBORw0Ksame"),
            &observed("iVBORw0Ksame"),
            &observed("iVBORw0Kother"),
        ]);
        let tools = computer_tools(provider, record("running"));
        let observe = tool(&tools, "computer_observe");
        let ctx = context("s-unchanged");

        let first = observe.execute(json!({}), &ctx).await.unwrap();
        assert_eq!(first.images.len(), 1);
        assert!(!first.output.contains("unchanged"));

        let again = observe.execute(json!({}), &ctx).await.unwrap();
        assert!(again.images.is_empty());
        assert!(again.output.contains("(screen unchanged)"));
        // The metadata still carries the picture, because the card in the UI
        // draws it even when the model was spared it.
        assert!(again.metadata.unwrap()["screenshot"].is_string());

        let moved = observe.execute(json!({}), &ctx).await.unwrap();
        assert_eq!(moved.images.len(), 1);
    }

    /// Two agents on one machine are looking at the same screen, and a frame
    /// one of them has seen tells you nothing about what the other has.
    #[tokio::test]
    async fn another_conversation_is_still_shown_the_picture() {
        let provider = FakeProvider::answering(&[
            &observed("iVBORw0Kshared"),
            &observed("iVBORw0Kshared"),
        ]);
        let tools = computer_tools(provider, record("running"));
        let observe = tool(&tools, "computer_observe");

        let mine = observe.execute(json!({}), &context("s-mine")).await.unwrap();
        let theirs = observe.execute(json!({}), &context("s-theirs")).await.unwrap();
        assert_eq!(mine.images.len(), 1);
        assert_eq!(theirs.images.len(), 1);
    }

    /// Exit 3 is the machine saying it cannot do that, and it is raised rather
    /// than returned: a model handed "no display" as ordinary output will try
    /// clicking anyway.
    #[tokio::test]
    async fn a_machine_with_no_desktop_refuses_rather_than_returning_nothing() {
        let provider = FakeProvider::failing(3, "This machine has no display running.");
        let tools = computer_tools(provider, record("running"));
        let failure = tool(&tools, "computer_observe")
            .execute(json!({}), &context("s-nodesktop"))
            .await
            .unwrap_err()
            .to_string();
        assert!(failure.contains("no display running"), "{failure}");
    }

    #[tokio::test]
    async fn acting_runs_the_batch_and_returns_the_screen_with_a_closeup() {
        let provider = FakeProvider::answering(&[&format!(
            "COMPLETED=2\nCLOSEUP=iVBORw0Kzoom\n{}",
            observed("iVBORw0Kafter")
        )]);
        let tools = computer_tools(provider.clone(), record("running"));
        let act = tool(&tools, "computer_act");
        let args = act.normalize(json!({
            "actions": [
                { "kind": "click", "x": 640, "y": 360 },
                { "kind": "type", "text": "hello" }
            ]
        }));
        let outcome = act.execute(args, &context("s-act")).await.unwrap();

        assert!(outcome.output.starts_with("did 2 actions"));
        assert!(outcome.output.contains("close-up of 640,360"));
        // The screen and the close-up both, in that order.
        assert_eq!(outcome.images.len(), 2);
        let metadata = outcome.metadata.unwrap();
        assert_eq!(metadata["clicked"], json!({ "x": 640, "y": 360 }));

        let command = &provider.commands()[0];
        assert!(command.contains("xdotool mousemove"));
        assert!(command.contains("xclip -selection clipboard -i"));
        assert!(command.contains("CLOSEUP="));
    }

    /// Measured: a model sends the fields of one action at the top level and no
    /// list at all. Refusing it cost a whole turn, so `normalize` reads it.
    #[tokio::test]
    async fn a_call_written_in_the_wrong_shape_is_repaired_rather_than_refused() {
        let provider = FakeProvider::answering(&[&observed("iVBORw0Kx")]);
        let tools = computer_tools(provider.clone(), record("running"));
        let act = tool(&tools, "computer_act");

        let args = act.normalize(json!({ "kind": "click", "x": 12, "y": 34 }));
        assert_eq!(args["actions"][0]["kind"], json!("click"));

        let outcome = act.execute(args, &context("s-shape")).await.unwrap();
        assert!(outcome.output.starts_with("did 1 action"));
        assert!(provider.commands()[0].contains("mousemove --sync 12 34"));
    }

    /// `settleMs` beside `settle_ms` is the other spelling a model reaches for.
    #[test]
    fn the_other_spelling_of_the_settle_is_accepted() {
        let tools = computer_tools(FakeProvider::answering(&[]), record("running"));
        let args = tool(&tools, "computer_act")
            .normalize(json!({ "actions": [{ "kind": "wait", "ms": 10 }], "settleMs": 900 }));
        assert_eq!(args["settle_ms"], json!(900));
    }

    #[tokio::test]
    async fn not_asking_for_the_screen_returns_a_count_and_no_picture() {
        let provider = FakeProvider::answering(&["COMPLETED=1\n"]);
        let tools = computer_tools(provider.clone(), record("running"));
        let act = tool(&tools, "computer_act");
        let args = act.normalize(json!({
            "actions": [{ "kind": "key", "key": "Return" }],
            "observe": false
        }));
        let outcome = act.execute(args, &context("s-quiet")).await.unwrap();

        assert!(outcome.images.is_empty());
        assert!(outcome.output.contains("You did not ask for the screen"));
        assert_eq!(outcome.metadata.unwrap()["completed"], json!(1));
        assert!(!provider.commands()[0].contains("IMAGE="));
    }

    #[tokio::test]
    async fn a_malformed_batch_says_which_part_was_wrong() {
        let tools = computer_tools(FakeProvider::answering(&[]), record("running"));
        let failure = tool(&tools, "computer_act")
            .execute(json!({ "actions": [{ "kind": "teleport" }] }), &context("s-bad"))
            .await
            .unwrap_err()
            .to_string();
        assert!(failure.contains("teleport"), "{failure}");
        assert!(failure.contains("scroll"), "{failure}");
    }

    #[tokio::test]
    async fn opening_a_javascript_url_is_refused_by_name() {
        let tools = computer_tools(FakeProvider::answering(&[]), record("running"));
        let failure = tool(&tools, "computer_open")
            .execute(json!({ "target": "javascript:alert(1)" }), &context("s-js"))
            .await
            .unwrap_err()
            .to_string();
        assert!(failure.contains("Refusing to open"), "{failure}");
        assert!(failure.contains("http(s)"), "{failure}");
    }

    #[tokio::test]
    async fn opening_a_page_opens_it_and_then_looks() {
        let provider = FakeProvider::answering(&["opened", &observed("iVBORw0Kpage")]);
        let tools = computer_tools(provider.clone(), record("running"));
        let outcome = tool(&tools, "computer_open")
            .execute(json!({ "target": "https://example.com" }), &context("s-open"))
            .await
            .unwrap();

        assert!(outcome.output.starts_with("opened https://example.com"));
        let commands = provider.commands();
        assert!(commands[0].contains("xdg-open 'https://example.com'"));
        assert!(commands[1].contains("IMAGE="));
    }

    #[tokio::test]
    async fn launching_the_browser_tries_the_names_an_image_might_use() {
        let provider = FakeProvider::answering(&["launched chromium", &observed("iVBORw0Kb")]);
        let tools = computer_tools(provider.clone(), record("running"));
        let outcome = tool(&tools, "computer_launch")
            .execute(json!({ "application": "browser" }), &context("s-launch"))
            .await
            .unwrap();

        assert!(outcome.output.starts_with("launched browser"));
        assert!(provider.commands()[0].contains("'inertia-browser' 'chromium'"));
    }

    /// The warning exists because it was watched happening: an agent filling a
    /// signup form read the page three times and got the password it had just
    /// typed.
    #[tokio::test]
    async fn page_text_that_looks_like_a_form_field_says_so() {
        let provider = FakeProvider::answering(&["hunter2"]);
        let tools = computer_tools(provider, record("running"));
        let outcome = tool(&tools, "computer_page_text")
            .execute(json!({}), &context("s-page"))
            .await
            .unwrap();

        assert!(outcome.output.contains("hunter2"));
        assert!(outcome.output.contains("Click a blank part of the page"));
        assert_eq!(outcome.metadata.unwrap()["fromField"], json!(true));
    }

    #[tokio::test]
    async fn a_real_page_carries_no_warning() {
        let long = "A heading\n".repeat(40);
        let provider = FakeProvider::answering(&[&long]);
        let tools = computer_tools(provider, record("running"));
        let outcome = tool(&tools, "computer_page_text")
            .execute(json!({}), &context("s-page2"))
            .await
            .unwrap();
        assert!(!outcome.output.contains("Click a blank part"));
        assert_eq!(outcome.metadata.unwrap()["fromField"], json!(false));
    }

    #[tokio::test]
    async fn an_empty_page_says_what_to_do_next() {
        let provider = FakeProvider::answering(&["   "]);
        let tools = computer_tools(provider, record("running"));
        let failure = tool(&tools, "computer_page_text")
            .execute(json!({}), &context("s-empty"))
            .await
            .unwrap_err()
            .to_string();
        assert!(failure.contains("Open something first"), "{failure}");
    }

    /// A non-zero exit is a result, not a failure of the tool: a model told the
    /// tests failed goes and looks at why.
    #[tokio::test]
    async fn a_command_that_exited_non_zero_is_still_a_result() {
        let provider = FakeProvider::failing(1, "2 tests failed");
        let tools = computer_tools(provider.clone(), record("running"));
        let outcome = tool(&tools, "computer_run")
            .execute(json!({ "command": "npm test" }), &context("s-run"))
            .await
            .unwrap();

        assert!(outcome.output.contains("2 tests failed"));
        assert!(outcome.output.contains("Exit code 1."));
        let metadata = outcome.metadata.unwrap();
        assert_eq!(metadata["exitCode"], json!(1));
        assert_eq!(metadata["provider"], json!("docker"));
        assert_eq!(metadata["machine"], json!("box"));
        assert_eq!(provider.commands(), ["npm test"]);
    }

    #[tokio::test]
    async fn a_command_with_no_output_says_so_rather_than_answering_blank() {
        let provider = FakeProvider::answering(&[""]);
        let tools = computer_tools(provider, record("running"));
        let outcome = tool(&tools, "computer_run")
            .execute(json!({ "command": "true" }), &context("s-quiet2"))
            .await
            .unwrap();
        assert_eq!(outcome.output, "(no output)");
    }

    #[tokio::test]
    async fn a_listing_defaults_to_where_the_work_lives() {
        let provider = FakeProvider::answering(&[]);
        *provider.entries.lock().unwrap() = vec![
            DirEntry {
                name: "src".into(),
                kind: "dir".into(),
                size: None,
                modified_at: None,
            },
            DirEntry {
                name: "notes.md".into(),
                kind: "file".into(),
                size: Some(42),
                modified_at: None,
            },
        ];
        let tools = computer_tools(provider, record("running"));
        let outcome = tool(&tools, "computer_list")
            .execute(json!({}), &context("s-list"))
            .await
            .unwrap();

        assert!(outcome.output.starts_with("/workspace\n"));
        assert!(outcome.output.contains("src/"));
        assert!(outcome.output.contains("notes.md  42b"));
        assert_eq!(outcome.metadata.unwrap()["entries"], json!(2));
    }

    /// A Daytona machine's work lives somewhere else, and a default pointing at
    /// a folder that does not exist reads as the tool being broken rather than
    /// as the path being wrong.
    #[tokio::test]
    async fn a_record_with_no_workdir_falls_back_per_provider() {
        let tools = computer_tools(
            FakeProvider::answering(&[]),
            json!({ "id": "cloud", "name": "Cloud", "provider": "daytona", "status": "running" }),
        );
        let outcome = tool(&tools, "computer_list")
            .execute(json!({}), &context("s-cloud"))
            .await
            .unwrap();
        assert!(outcome.output.starts_with("/home/daytona\n"));
    }

    #[tokio::test]
    async fn files_round_trip_and_a_missing_one_names_itself() {
        let provider = FakeProvider::answering(&[]);
        let tools = computer_tools(provider, record("running"));

        let written = tool(&tools, "computer_write")
            .execute(
                json!({ "path": "/workspace/a.txt", "content": "hello" }),
                &context("s-files"),
            )
            .await
            .unwrap();
        assert!(written.output.contains("Wrote 5 bytes to /workspace/a.txt."));

        let read = tool(&tools, "computer_read")
            .execute(json!({ "path": "/workspace/a.txt" }), &context("s-files"))
            .await
            .unwrap();
        assert_eq!(read.output, "hello");

        let failure = tool(&tools, "computer_read")
            .execute(json!({ "path": "/workspace/nope.txt" }), &context("s-files"))
            .await
            .unwrap_err()
            .to_string();
        // Written for the model to act on, naming the file.
        assert!(failure.contains("File not found: /workspace/nope.txt"), "{failure}");
    }

    #[test]
    fn one_short_unbroken_line_is_the_tell_that_page_text_is_a_field() {
        assert!(looks_like_a_field("hunter2"));
        assert!(!looks_like_a_field(""));
        assert!(!looks_like_a_field("one\ntwo"));
        assert!(!looks_like_a_field(&"x".repeat(FIELD_SUSPECT_CHARS)));
    }

    #[test]
    fn capping_never_splits_a_character() {
        // Four bytes, one character: cutting at three must drop it whole.
        assert_eq!(cap("ab\u{1F600}", 4), "ab");
        assert_eq!(cap("abc", 10), "abc");
    }
}

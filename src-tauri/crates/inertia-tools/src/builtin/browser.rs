//! The browser the person is looking at, driven by the agent.
//!
//! This is the pane in the window - the same one, not a hidden second browser.
//! That is the whole design and it is worth being explicit about, because a
//! headless browser would have been easier: an agent that answers "yes, the
//! layout is fixed" about a page nobody can see is an agent whose answers
//! cannot be checked. Here the person watches the page navigate, sees the click
//! land, and can take the mouse back at any point.
//!
//! ## Why several tools rather than one with an `action`
//!
//! One tool with an action argument and a schema carrying every argument any
//! action might need reads to a model as a menu, and it picks badly:
//! `screenshot` with a `selector`, `click` with a `url`. Split, each name says
//! what it is for and each schema holds only its own arguments. They share one
//! permission key, so the person still writes one rule.
//!
//! ## Why looking is separate from acting here
//!
//! A web page can be read as structure - roles, names, and a `ref` per
//! interactive element - which is smaller, more accurate, and directly
//! actionable than a picture. So the loop is read, act by ref, read again.
//!
//! ## Why the pane arrives through a trait
//!
//! The pane is a native webview layered over the app's own window, which means
//! creating it needs an `AppHandle` and a window, both of which live in the app
//! crate and neither of which can exist in a test. Behind [`Pane`] the app
//! supplies the real thing and the tests below supply a fake that records every
//! script it was asked to run - which is what lets them drive this exact
//! `execute` rather than a parallel implementation written to be testable.
//!
//! ## What is emulated rather than native, and why it matters
//!
//! Electron could post real OS input events at a page. Tauri's webview cannot:
//! there is no `sendInputEvent`, so clicking, typing and pressing keys are
//! done by dispatching the corresponding DOM events from inside the page. That
//! covers everything built on listeners - which is the whole modern web, React
//! included - and does not cover the handful of behaviours the browser itself
//! implements off a trusted event. The two that come up are named where they
//! are worked around: a click on a `<label>` still toggles its control because
//! the script clicks the control, and Enter in a form still submits because the
//! script calls `requestSubmit`.

use std::sync::Arc;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::Result;
use serde_json::{json, Value};

/// What the app supplies so these tools can drive the pane in the window.
///
/// Deliberately four methods. Everything else a tool wants - reading the page,
/// clicking, the console log - is a script run in the page, so widening this
/// would mean the app crate held page behaviour that cannot be tested there.
#[async_trait]
pub trait Pane: Send + Sync + std::fmt::Debug {
    /// The tab this conversation is showing, or `None` when none is.
    fn front_of(&self, thread: &str) -> Option<String>;

    /// Ask the window to bring this pane forward, opening it if it is closed.
    ///
    /// A request rather than an instruction: the window decides what forward
    /// means. Without it the tools worked perfectly and invisibly, and a tool
    /// result about a window nobody can see is indistinguishable from a model
    /// making it up.
    async fn reveal(&self, id: &str) -> std::result::Result<(), String>;

    /// Make the pane if it is not there, optionally navigate, and answer the
    /// page's state: `{ id, url, title, loading, error, canGoBack,
    /// canGoForward }`.
    async fn open(&self, id: &str, url: Option<&str>) -> std::result::Result<Value, String>;

    /// Run script in the page and return what it evaluated to.
    async fn evaluate(&self, id: &str, code: &str) -> std::result::Result<Value, String>;
}

/// Every browser tool, bound to one pane.
pub fn browser_tools(pane: Arc<dyn Pane>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(NavigateTool(pane.clone())),
        Arc::new(ReadPageTool(pane.clone())),
        Arc::new(ReadTextTool(pane.clone())),
        Arc::new(ClickTool(pane.clone())),
        Arc::new(TypeTool(pane.clone())),
        Arc::new(PressTool(pane.clone())),
        Arc::new(EvaluateTool(pane.clone())),
        Arc::new(ConsoleTool(pane.clone())),
        Arc::new(NetworkTool(pane)),
    ]
}

/* -- which tab the tools drive -------------------------------------------- */

/// The tab the PERSON is looking at, in this conversation.
///
/// The pane is never a tool argument, so a model cannot reach another
/// conversation's browser by guessing an id: the conversation it is running in
/// is the only one it can address. When nothing is open yet this falls back to
/// the first tab's id, which is the one the pane adopts when the person opens
/// it - so a tool that runs before the pane is showing does not strand a page
/// in a view nobody will ever be given.
pub fn pane_for(pane: &dyn Pane, ctx: &ToolContext) -> String {
    let session: &str = ctx.session.as_ref();
    let thread = session.split('/').next().unwrap_or(session);
    let thread = if thread.is_empty() { "default" } else { thread };
    pane.front_of(thread)
        .unwrap_or_else(|| format!("chat:{thread}:web:1"))
}

/// Shared by every tool here, so the person writes one rule and not nine.
fn permission_for(args: &Value) -> PermissionRequest {
    let target = args
        .get("url")
        .and_then(Value::as_str)
        .or_else(|| args.get("ref").and_then(Value::as_str))
        .unwrap_or("the open page")
        .to_string();
    PermissionRequest::new("browser", target).with_always("*")
}

/// A failure the model can read and act on, in the shape every tool here
/// returns it.
///
/// Never `Err`: a page that would not load, a ref that has gone stale and a
/// pane the person just closed are all things the model can try differently,
/// and ending the turn over one would be worse than saying so.
fn refused(message: impl Into<String>) -> ToolOutcome {
    ToolOutcome::text(message.into()).with_title("the browser pane")
}

/// The pane, brought forward, or the sentence to hand back instead.
///
/// Every tool does this and not just the one that navigates: reading the
/// console of a page the person cannot see is the same problem as loading one.
/// They have no way to check the answer against the thing it is about.
async fn front(pane: &dyn Pane, ctx: &ToolContext) -> std::result::Result<String, String> {
    let id = pane_for(pane, ctx);
    pane.reveal(&id).await.map(|()| id)
}

/* -- argument handling ---------------------------------------------------- */

/// How much of a page a `maxChars` argument asks for.
///
/// Clamped at both ends because both ends have been wrong in practice: a model
/// asking for 200 characters gets an outline that stops mid-element and reads
/// as an empty page, and one asking for a million spends the context window it
/// was trying to save.
pub fn max_chars(args: &Value, fallback: i64) -> i64 {
    let asked = args
        .get("maxChars")
        .and_then(Value::as_f64)
        .map(|n| n as i64)
        .filter(|n| *n > 0)
        .unwrap_or(fallback);
    asked.clamp(500, 50_000)
}

/// How many log lines a `count` argument asks for.
pub fn log_count(args: &Value) -> usize {
    let asked = args
        .get("count")
        .and_then(Value::as_f64)
        .map(|n| n as i64)
        .filter(|n| *n > 0)
        .unwrap_or(50);
    asked.clamp(1, 500) as usize
}

fn flag(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// The newest `count` entries that match, oldest first.
///
/// The filter runs here rather than in the page because this is the part most
/// likely to be wrong in a way a test catches: "only errors" that also drops
/// warnings, a pattern that is case sensitive when nobody typed it that way,
/// a count applied before the filter so asking for fifty errors returns three.
pub fn filter_console(
    entries: &[Value],
    only_errors: bool,
    pattern: Option<&str>,
    count: usize,
) -> Vec<Value> {
    let needle = pattern.map(str::to_lowercase);
    let matched: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            let level = entry.get("level").and_then(Value::as_str).unwrap_or("info");
            if only_errors && level != "error" {
                return false;
            }
            match &needle {
                None => true,
                Some(needle) => entry
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(needle),
            }
        })
        .cloned()
        .collect();
    let from = matched.len().saturating_sub(count);
    matched[from..].to_vec()
}

/// The same, for requests. Separate rather than generic: "failed" here means
/// a transport error OR a status of 400 and above, and folding that into a
/// shared predicate is how one of the two stops being reported.
pub fn filter_network(
    entries: &[Value],
    only_failed: bool,
    url_pattern: Option<&str>,
    count: usize,
) -> Vec<Value> {
    let needle = url_pattern.map(str::to_lowercase);
    let matched: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            if only_failed && !entry.get("failed").and_then(Value::as_bool).unwrap_or(false) {
                return false;
            }
            match &needle {
                None => true,
                Some(needle) => entry
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(needle),
            }
        })
        .cloned()
        .collect();
    let from = matched.len().saturating_sub(count);
    matched[from..].to_vec()
}

/// One console line, written for a model.
pub fn console_line(entry: &Value) -> String {
    let level = entry.get("level").and_then(Value::as_str).unwrap_or("info");
    let message = entry.get("message").and_then(Value::as_str).unwrap_or("");
    let source = entry.get("source").and_then(Value::as_str).unwrap_or("");
    let line = entry.get("line").and_then(Value::as_f64).unwrap_or(0.0) as i64;
    if source.is_empty() {
        format!("[{level}] {message}")
    } else {
        format!("[{level}] {message} ({source}:{line})")
    }
}

/// One request, written for a model.
pub fn network_line(entry: &Value) -> String {
    let method = entry.get("method").and_then(Value::as_str).unwrap_or("GET");
    let status = match entry.get("status").and_then(Value::as_f64) {
        Some(code) => format!("{}", code as i64),
        None => "failed".to_string(),
    };
    let ms = match entry.get("ms").and_then(Value::as_f64) {
        Some(ms) => format!("{}", ms as i64),
        None => "?".to_string(),
    };
    let url = entry.get("url").and_then(Value::as_str).unwrap_or("");
    format!("{method} {status} {ms}ms {url}")
}

/* -- what the page is asked ----------------------------------------------- */

/// The map of refs lives on `window` under a name nothing else will use, and
/// is rebuilt on every read. A ref from an older read of the same page still
/// resolves as long as the element is still there, which is the common case; a
/// ref from before a navigation does not, and says so rather than clicking
/// something else that has inherited the number.
const REF_KEY: &str = "__inertiaRefs";

/// Walk the document and return the outline.
///
/// Written as a string because it runs in the page, not here. Everything
/// inside runs in the page's world and must not assume anything about it: a
/// page can and does replace `Array.prototype.map`, so the walker uses only
/// syntax and direct property access.
pub fn read_page_script(max_chars: i64, interactive_only: bool) -> String {
    format!(
        r#"(() => {{
  const refs = new Map();
  let nextRef = 0;
  const lines = [];
  let spent = 0;
  const budget = {max_chars};
  const interactiveOnly = {interactive_only};

  const INTERACTIVE = new Set(["a", "button", "input", "select", "textarea", "summary", "label", "option"]);
  const SKIP = new Set(["script", "style", "noscript", "template", "svg", "head", "meta", "link"]);

  const visible = (el) => {{
    if (!(el instanceof Element)) return false;
    const style = el.ownerDocument.defaultView.getComputedStyle(el);
    if (!style) return true;
    if (style.display === "none" || style.visibility === "hidden") return false;
    if (style.opacity === "0") return false;
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0 && el.childElementCount === 0) return false;
    return true;
  }};

  const roleOf = (el) => {{
    const explicit = el.getAttribute("role");
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    if (tag === "a") return el.hasAttribute("href") ? "link" : "generic";
    if (tag === "input") {{
      const type = (el.getAttribute("type") || "text").toLowerCase();
      if (type === "checkbox" || type === "radio") return type;
      if (type === "submit" || type === "button") return "button";
      return "textbox";
    }}
    if (tag === "textarea") return "textbox";
    if (tag === "select") return "combobox";
    if (tag === "button" || tag === "summary") return "button";
    if (/^h[1-6]$/.test(tag)) return "heading";
    if (tag === "img") return "image";
    if (tag === "li") return "listitem";
    if (tag === "table") return "table";
    if (tag === "form") return "form";
    if (tag === "nav") return "navigation";
    if (tag === "main") return "main";
    return "";
  }};

  const nameOf = (el) => {{
    const label =
      el.getAttribute("aria-label") ||
      el.getAttribute("alt") ||
      el.getAttribute("placeholder") ||
      el.getAttribute("title") ||
      "";
    if (label) return label;
    const text = (el.innerText || el.textContent || "").replace(/\s+/g, " ").trim();
    return text.length > 160 ? text.slice(0, 160) + "…" : text;
  }};

  const interactive = (el) => {{
    const tag = el.tagName.toLowerCase();
    if (INTERACTIVE.has(tag)) return true;
    if (el.hasAttribute("onclick")) return true;
    if (el.hasAttribute("contenteditable")) return true;
    const role = el.getAttribute("role");
    return role === "button" || role === "link" || role === "checkbox" || role === "tab" || role === "menuitem";
  }};

  const emit = (depth, text) => {{
    if (spent >= budget) return false;
    const line = "  ".repeat(Math.min(depth, 12)) + text;
    lines.push(line);
    spent += line.length + 1;
    return true;
  }};

  const walk = (el, depth) => {{
    if (spent >= budget) return;
    const tag = el.tagName ? el.tagName.toLowerCase() : "";
    if (SKIP.has(tag)) return;
    if (!visible(el)) return;

    const role = roleOf(el);
    const name = nameOf(el);
    const clickable = interactive(el);

    let shown = false;
    if (clickable || (!interactiveOnly && role && name)) {{
      let ref = "";
      if (clickable) {{
        ref = "ref_" + ++nextRef;
        refs.set(ref, el);
      }}
      const value = el.value !== undefined && typeof el.value === "string" && el.value ? ' value="' + el.value.slice(0, 60) + '"' : "";
      const state = el.disabled ? " disabled" : el.checked ? " checked" : "";
      const head = (role || tag) + (name ? ' "' + name + '"' : "");
      shown = emit(depth, head + value + state + (ref ? " [" + ref + "]" : ""));
      if (!shown) return;
    }}

    const next = shown ? depth + 1 : depth;
    for (const child of el.children) walk(child, next);
  }};

  if (document.body) walk(document.body, 0);
  window["{REF_KEY}"] = refs;

  const truncated = spent >= budget;
  return {{
    url: location.href,
    title: document.title || "",
    refs: nextRef,
    truncated,
    outline: lines.join("\n") + (truncated ? "\n… (page truncated)" : ""),
  }};
}})()"#
    )
}

/// The readable text of a page, article first, body as the fallback.
pub fn page_text_script(max_chars: i64) -> String {
    format!(
        r#"(() => {{
  const main = document.querySelector("article") || document.querySelector("main") || document.body;
  const text = (main ? main.innerText : "") || "";
  return {{
    url: location.href,
    title: document.title || "",
    text: text.length > {max_chars} ? text.slice(0, {max_chars}) + "\n… (text truncated)" : text,
  }};
}})()"#
    )
}

/// The preamble every acting script shares: find the element behind a ref, or
/// say why it could not be found.
///
/// A stale ref is an explicit failure rather than a fallback to a coordinate,
/// because a fallback here is how a model ends up clicking whatever has moved
/// under the old position.
fn lookup(reference: &str) -> String {
    let quoted = json!(reference).to_string();
    format!(
        r#"  const refs = window["{REF_KEY}"];
  if (!refs) return {{ ok: false, error: "The page has not been read yet. Call browser_read_page first." }};
  const el = refs.get({quoted});
  if (!el || !el.isConnected) return {{ ok: false, error: "That ref is no longer on the page. Read it again." }};
"#
    )
}

/// Click by ref, or at a point.
///
/// A full pointer sequence rather than `el.click()`, because a synthetic click
/// skips the hover, focus and pointer events half the web is built on, and a
/// button that "did nothing" is almost always one that wanted a pointerdown.
/// A `<label>` is redirected to its control: the browser does that itself for
/// a trusted click and does not for a dispatched one, so without this line
/// clicking a checkbox's label would look like it worked and change nothing.
pub fn click_script(reference: Option<&str>, x: Option<f64>, y: Option<f64>) -> String {
    let find = match reference {
        Some(reference) => lookup(reference),
        None => format!(
            r#"  const el = document.elementFromPoint({}, {});
  if (!el) return {{ ok: false, error: "Nothing is at that point in the page." }};
"#,
            x.unwrap_or(0.0),
            y.unwrap_or(0.0)
        ),
    };
    format!(
        r#"(() => {{
{find}  const target = el.tagName === "LABEL" && el.control ? el.control : el;
  target.scrollIntoView({{ block: "center", inline: "center" }});
  const rect = target.getBoundingClientRect();
  const at = {{ clientX: rect.x + rect.width / 2, clientY: rect.y + rect.height / 2, bubbles: true, cancelable: true, composed: true, view: window }};
  const pointer = {{ ...at, pointerId: 1, pointerType: "mouse", isPrimary: true }};
  if (target.focus) try {{ target.focus(); }} catch (error) {{ /* a disabled control refuses focus */ }}
  target.dispatchEvent(new PointerEvent("pointerover", pointer));
  target.dispatchEvent(new MouseEvent("mouseover", at));
  target.dispatchEvent(new PointerEvent("pointerdown", pointer));
  target.dispatchEvent(new MouseEvent("mousedown", {{ ...at, button: 0, buttons: 1 }}));
  target.dispatchEvent(new PointerEvent("pointerup", pointer));
  target.dispatchEvent(new MouseEvent("mouseup", {{ ...at, button: 0, buttons: 0 }}));
  target.dispatchEvent(new MouseEvent("click", {{ ...at, button: 0 }}));
  return {{ ok: true, clicked: (target.tagName || "").toLowerCase() }};
}})()"#
    )
}

/// Type into the page.
///
/// Appended rather than replaced, because that is what typing is, and written
/// through the prototype's own value setter so React sees a change: assigning
/// `el.value` directly updates the DOM and leaves React's shadow copy stale,
/// which shows up as a field that visibly holds text the app does not know
/// about.
pub fn type_script(value: &str, reference: Option<&str>) -> String {
    let find = match reference {
        Some(reference) => lookup(reference),
        None => r#"  const el = document.activeElement;
  if (!el || el === document.body) return { ok: false, error: "Nothing on the page has focus. Give a ref to type into." };
"#
        .to_string(),
    };
    let quoted = json!(value).to_string();
    format!(
        r#"(() => {{
{find}  const text = {quoted};
  if (el.focus) el.focus();
  if (el.isContentEditable) {{
    el.textContent = (el.textContent || "") + text;
    el.dispatchEvent(new InputEvent("input", {{ bubbles: true, data: text, inputType: "insertText" }}));
    return {{ ok: true, typed: text.length }};
  }}
  if (typeof el.value !== "string") return {{ ok: false, error: "That element cannot be typed into." }};
  const proto = Object.getPrototypeOf(el);
  const setter = Object.getOwnPropertyDescriptor(proto, "value");
  const next = el.value + text;
  if (setter && setter.set) setter.set.call(el, next);
  else el.value = next;
  el.dispatchEvent(new InputEvent("input", {{ bubbles: true, data: text, inputType: "insertText" }}));
  el.dispatchEvent(new Event("change", {{ bubbles: true }}));
  return {{ ok: true, typed: text.length }};
}})()"#
    )
}

/// One key or chord, as DOM events.
///
/// `requestSubmit` is the compromise that makes Enter mean anything. A
/// dispatched keydown is untrusted, so the browser does not run its own
/// default action for it and a form that submits on Enter for a person does
/// nothing for the agent. Calling the form's own submit path instead runs its
/// validation and fires `submit`, which is the behaviour anybody pressing
/// Enter in a text field was asking for.
pub fn press_script(keys: &str) -> String {
    let parts: Vec<&str> = keys
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let key = parts.last().copied().unwrap_or("");
    let modifiers: Vec<String> = parts[..parts.len().saturating_sub(1)]
        .iter()
        .map(|part| part.to_lowercase())
        .collect();
    let has = |name: &str| modifiers.iter().any(|m| m == name);
    let quoted = json!(key).to_string();
    let ctrl = has("control") || has("ctrl");
    let shift = has("shift");
    let alt = has("alt");
    let meta = has("meta") || has("command") || has("cmd");
    format!(
        r#"(() => {{
  const key = {quoted};
  const el = document.activeElement || document.body;
  const init = {{ key, code: key.length === 1 ? "Key" + key.toUpperCase() : key, bubbles: true, cancelable: true, composed: true,
    ctrlKey: {ctrl}, shiftKey: {shift}, altKey: {alt}, metaKey: {meta} }};
  const down = el.dispatchEvent(new KeyboardEvent("keydown", init));
  el.dispatchEvent(new KeyboardEvent("keyup", init));
  if (down && key === "Enter" && el.form && el.form.requestSubmit) el.form.requestSubmit();
  return {{ ok: true, pressed: key }};
}})()"#
    )
}

/// The console ring the page has been filling since it last navigated.
pub fn console_log_script() -> String {
    r#"(() => {
  const log = window.__inertiaConsole;
  return log && log.entries ? log.entries : [];
})()"#
        .to_string()
}

/// The requests the page has made since it last navigated.
pub fn network_log_script() -> String {
    r#"(() => {
  const log = window.__inertiaNetwork;
  return log && log.entries ? log.entries : [];
})()"#
        .to_string()
}

/* -- the tools ------------------------------------------------------------ */

/// The boilerplate every tool here repeats: the id, the source, the shared
/// permission, and a render label. Written once because nine copies of it is
/// nine places for one of them to drift.
macro_rules! browser_tool {
    ($name:ident, $id:literal, $description:expr, $parameters:expr, $render:expr, $run:expr) => {
        #[derive(Debug)]
        pub struct $name(Arc<dyn Pane>);

        #[async_trait]
        impl Tool for $name {
            fn id(&self) -> &str {
                $id
            }

            fn description(&self) -> &str {
                $description
            }

            fn parameters(&self) -> Value {
                $parameters
            }

            fn source(&self) -> ToolSource {
                ToolSource::Builtin
            }

            fn permission(&self, args: &Value) -> PermissionRequest {
                permission_for(args)
            }

            fn render(&self, args: &Value) -> Option<String> {
                let render: fn(&Value) -> Option<String> = $render;
                render(args)
            }

            async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutcome> {
                let id = match front(self.0.as_ref(), ctx).await {
                    Ok(id) => id,
                    Err(error) => return Ok(refused(error)),
                };
                let run: fn(
                    Arc<dyn Pane>,
                    String,
                    Value,
                ) -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = ToolOutcome> + Send>,
                > = $run;
                Ok(run(self.0.clone(), id, args).await)
            }
        }
    };
}

/// The page, read. Split out because five tools end by doing exactly this and
/// each one doing it slightly differently is five shapes of output for one
/// thing.
async fn read_outline(pane: &dyn Pane, id: &str, budget: i64) -> std::result::Result<Value, String> {
    pane.evaluate(id, &read_page_script(budget, false)).await
}

/// What a script that answers `{ ok, error }` said, or the error to report.
fn acted(value: &Value) -> std::result::Result<(), String> {
    if value.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return Ok(());
    }
    Err(value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("That did not work on the open page.")
        .to_string())
}

browser_tool!(
    NavigateTool,
    "browser_navigate",
    "Open a URL in the browser pane beside the conversation, and return the page's outline. \
     This is the browser IN THIS WINDOW, on the user's own machine: their localhost and dev \
     servers are reachable, their logged-in sessions are the ones that apply, and they are \
     watching the page as you drive it. Use it for anything about the work in front of you - \
     checking a change you made, reading documentation, following a link they sent. \
     NOT `computer_open`, which opens a page inside a sandboxed machine somewhere else that \
     cannot see localhost and has none of their logins. If the point is to be somewhere that is \
     not the user's machine, that is the tool; if the point is the page they are working on, \
     this is. The pane opens if it is not already showing.",
    json!({
        "type": "object",
        "properties": {
            "url": { "type": "string", "description": "The address to open, for example `http://localhost:5173` or `https://example.com/docs`." }
        },
        "required": ["url"],
        "additionalProperties": false
    }),
    |args| args.get("url").and_then(Value::as_str).map(str::to_string),
    |pane, id, args| Box::pin(async move {
        let url = args.get("url").and_then(Value::as_str).unwrap_or("");
        let state = match pane.open(&id, Some(url)).await {
            Ok(state) => state,
            Err(error) => return refused(error),
        };
        if let Some(error) = state.get("error").and_then(Value::as_str) {
            return refused(error.to_string());
        }
        let landed = state
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or(url)
            .to_string();
        // Read after the load rather than making the model ask twice. A
        // navigation whose only answer is "ok" costs a second round trip to
        // find out what arrived, every single time.
        let page = read_outline(pane.as_ref(), &id, 12_000).await.unwrap_or(Value::Null);
        let title = page
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .or_else(|| state.get("title").and_then(Value::as_str))
            .filter(|t| !t.is_empty())
            .unwrap_or("Untitled")
            .to_string();
        let outline = page.get("outline").and_then(Value::as_str).unwrap_or("");
        let output = if outline.is_empty() {
            format!("{landed}\nThe page loaded but produced no readable outline yet.")
        } else {
            format!("{landed}\n\n{outline}")
        };
        ToolOutcome {
            title: Some(format!("{title} - {landed}")),
            output,
            metadata: Some(json!({ "url": landed, "pane": id })),
            images: Vec::new(),
        }
    })
);

browser_tool!(
    ReadPageTool,
    "browser_read_page",
    "Read the open page as an outline of what is on it: roles, names, and a `ref_N` for every \
     element that can be clicked or typed into. Prefer this over a screenshot for checking text, \
     structure and whether a control is there - it is smaller, exact, and the refs it returns are \
     what `browser_click` and `browser_type` take.",
    json!({
        "type": "object",
        "properties": {
            "interactiveOnly": { "type": "boolean", "description": "Only list things that can be clicked or typed into. Smaller, when you already know what the page says." },
            "maxChars": { "type": "number", "description": "How much of the outline to return. Defaults to 20000." }
        },
        "additionalProperties": false
    }),
    |_args| Some("the open page".to_string()),
    |pane, id, args| Box::pin(async move {
        let script = read_page_script(max_chars(&args, 20_000), flag(&args, "interactiveOnly"));
        let page = match pane.evaluate(&id, &script).await {
            Ok(page) => page,
            Err(error) => return refused(error),
        };
        let url = page.get("url").and_then(Value::as_str).unwrap_or("");
        let title = page
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .unwrap_or("the open page");
        let outline = page
            .get("outline")
            .and_then(Value::as_str)
            .filter(|o| !o.trim().is_empty())
            .unwrap_or("The page is empty.");
        ToolOutcome {
            title: Some(title.to_string()),
            output: format!("{url}\n\n{outline}"),
            metadata: Some(json!({
                "url": url,
                "refs": page.get("refs").and_then(Value::as_f64).unwrap_or(0.0) as i64,
                "truncated": page.get("truncated").and_then(Value::as_bool).unwrap_or(false),
            })),
            images: Vec::new(),
        }
    })
);

browser_tool!(
    ReadTextTool,
    "browser_read_text",
    "The readable text of the open page, article or main content first. Use this when you want to \
     READ a page - documentation, an article, a changelog - rather than interact with it.",
    json!({
        "type": "object",
        "properties": {
            "maxChars": { "type": "number", "description": "How much text to return. Defaults to 20000." }
        },
        "additionalProperties": false
    }),
    |_args| Some("the open page".to_string()),
    |pane, id, args| Box::pin(async move {
        let script = page_text_script(max_chars(&args, 20_000));
        let page = match pane.evaluate(&id, &script).await {
            Ok(page) => page,
            Err(error) => return refused(error),
        };
        let title = page
            .get("title")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .unwrap_or("the open page");
        let text = page
            .get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or("The page has no readable text.");
        ToolOutcome {
            title: Some(title.to_string()),
            output: text.to_string(),
            metadata: Some(json!({ "url": page.get("url").and_then(Value::as_str).unwrap_or("") })),
            images: Vec::new(),
        }
    })
);

browser_tool!(
    ClickTool,
    "browser_click",
    "Click something on the open page. Give a `ref` from `browser_read_page` - the element itself, \
     wherever it has moved to - rather than coordinates, which go stale the moment the page scrolls. \
     The click is a full pointer sequence, so hover, focus and pointer handlers all fire.",
    json!({
        "type": "object",
        "properties": {
            "ref": { "type": "string", "description": "A `ref_N` from browser_read_page." },
            "x": { "type": "number", "description": "An x coordinate in the page, when there is no ref for what you want." },
            "y": { "type": "number", "description": "A y coordinate in the page, when there is no ref for what you want." }
        },
        "additionalProperties": false
    }),
    |args| Some(match args.get("ref").and_then(Value::as_str) {
        Some(reference) => reference.to_string(),
        None => format!(
            "{},{}",
            args.get("x").and_then(Value::as_f64).unwrap_or(0.0),
            args.get("y").and_then(Value::as_f64).unwrap_or(0.0)
        ),
    }),
    |pane, id, args| Box::pin(async move {
        let reference = text(&args, "ref");
        let x = args.get("x").and_then(Value::as_f64);
        let y = args.get("y").and_then(Value::as_f64);
        if reference.is_none() && (x.is_none() || y.is_none()) {
            return refused("Give either a ref from browser_read_page or an x and y.");
        }
        let script = click_script(reference.as_deref(), x, y);
        match pane.evaluate(&id, &script).await {
            Err(error) => return refused(error),
            Ok(value) => {
                if let Err(error) = acted(&value) {
                    return refused(error);
                }
            }
        }
        // What the click did, which is the only interesting part. Read after a
        // beat, because a click that navigates or opens a menu has not
        // finished doing it by the time the dispatch returns.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let page = read_outline(pane.as_ref(), &id, 12_000).await.unwrap_or(Value::Null);
        let url = page.get("url").and_then(Value::as_str).unwrap_or("");
        let outline = page.get("outline").and_then(Value::as_str).unwrap_or("");
        ToolOutcome {
            title: Some(reference.unwrap_or_else(|| "the open page".to_string())),
            output: format!("{url}\n\n{outline}"),
            metadata: Some(json!({ "url": url })),
            images: Vec::new(),
        }
    })
);

browser_tool!(
    TypeTool,
    "browser_type",
    "Type into the open page. With a `ref` it focuses that element first; without one it types \
     wherever the focus already is. Follow with `browser_press` for Enter or Tab.",
    json!({
        "type": "object",
        "properties": {
            "text": { "type": "string", "description": "The text to type." },
            "ref": { "type": "string", "description": "A `ref_N` from browser_read_page to focus first." }
        },
        "required": ["text"],
        "additionalProperties": false
    }),
    |args| Some(
        args.get("ref")
            .and_then(Value::as_str)
            .unwrap_or("the focused field")
            .to_string()
    ),
    |pane, id, args| Box::pin(async move {
        let value = args.get("text").and_then(Value::as_str).unwrap_or("");
        let reference = text(&args, "ref");
        let script = type_script(value, reference.as_deref());
        let result = match pane.evaluate(&id, &script).await {
            Ok(result) => result,
            Err(error) => return refused(error),
        };
        if let Err(error) = acted(&result) {
            return refused(error);
        }
        let typed = result.get("typed").and_then(Value::as_f64).unwrap_or(0.0) as i64;
        ToolOutcome::text(format!("Typed {typed} characters."))
            .with_title(reference.unwrap_or_else(|| "the focused field".to_string()))
    })
);

browser_tool!(
    PressTool,
    "browser_press",
    "Press a key or a chord in the open page: `Enter`, `Tab`, `Escape`, `Control+a`. Use it to \
     submit a form, move focus, or dismiss something.",
    json!({
        "type": "object",
        "properties": {
            "keys": { "type": "string", "description": "The key or chord, for example `Enter` or `Control+a`." }
        },
        "required": ["keys"],
        "additionalProperties": false
    }),
    |args| args.get("keys").and_then(Value::as_str).map(str::to_string),
    |pane, id, args| Box::pin(async move {
        let keys = args.get("keys").and_then(Value::as_str).unwrap_or("");
        if keys.trim().is_empty() {
            return refused("Name a key to press.");
        }
        if let Err(error) = pane.evaluate(&id, &press_script(keys)).await {
            return refused(error);
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let page = read_outline(pane.as_ref(), &id, 12_000).await.unwrap_or(Value::Null);
        let url = page.get("url").and_then(Value::as_str).unwrap_or("");
        let outline = page.get("outline").and_then(Value::as_str).unwrap_or("");
        ToolOutcome::text(format!("{url}\n\n{outline}")).with_title(keys.to_string())
    })
);

browser_tool!(
    EvaluateTool,
    "browser_evaluate",
    "Run JavaScript in the open page and return the result, for DEBUGGING and INSPECTION. \
     The last expression is returned, and a promise is awaited - wrap `await` itself in an \
     async IIFE. Use it to read computed styles, check a global, or measure an element. \
     Do NOT use it to implement changes - edit the source and reload, or the fix exists only \
     until the next refresh.",
    json!({
        "type": "object",
        "properties": {
            "code": { "type": "string", "description": "The JavaScript to evaluate in the page." }
        },
        "required": ["code"],
        "additionalProperties": false
    }),
    |_args| Some("the open page".to_string()),
    |pane, id, args| Box::pin(async move {
        let code = args.get("code").and_then(Value::as_str).unwrap_or("");
        match pane.evaluate(&id, code).await {
            Err(error) => refused(error),
            Ok(value) => {
                let text = match &value {
                    Value::Null => "undefined".to_string(),
                    Value::String(text) => text.clone(),
                    other => serde_json::to_string_pretty(other)
                        .unwrap_or_else(|_| other.to_string()),
                };
                ToolOutcome::text(text).with_title("the open page")
            }
        }
    })
);

browser_tool!(
    ConsoleTool,
    "browser_console",
    "The console output of the open page since it last navigated. The first thing to check after a \
     change that should have worked and did not.",
    json!({
        "type": "object",
        "properties": {
            "onlyErrors": { "type": "boolean", "description": "Only error-level messages." },
            "pattern": { "type": "string", "description": "Only messages containing this text." },
            "count": { "type": "number", "description": "How many of the most recent to return. Defaults to 50." }
        },
        "additionalProperties": false
    }),
    |_args| Some("the console".to_string()),
    |pane, id, args| Box::pin(async move {
        let entries = match pane.evaluate(&id, &console_log_script()).await {
            Ok(Value::Array(entries)) => entries,
            Ok(_) => Vec::new(),
            Err(error) => return refused(error),
        };
        let pattern = text(&args, "pattern");
        let kept = filter_console(
            &entries,
            flag(&args, "onlyErrors"),
            pattern.as_deref(),
            log_count(&args),
        );
        if kept.is_empty() {
            return ToolOutcome::text("The console is empty.").with_title("the console");
        }
        let lines: Vec<String> = kept.iter().map(console_line).collect();
        ToolOutcome::text(lines.join("\n")).with_title(format!("the console ({})", kept.len()))
    })
);

browser_tool!(
    NetworkTool,
    "browser_network",
    "The requests the open page has made since it last navigated, with their status and timing. \
     Use it to check that an API call went out, what it answered, and what failed.",
    json!({
        "type": "object",
        "properties": {
            "onlyFailed": { "type": "boolean", "description": "Only requests that failed or returned 400 and above." },
            "urlPattern": { "type": "string", "description": "Only requests whose URL contains this text." },
            "count": { "type": "number", "description": "How many of the most recent to return. Defaults to 50." }
        },
        "additionalProperties": false
    }),
    |_args| Some("the network log".to_string()),
    |pane, id, args| Box::pin(async move {
        let entries = match pane.evaluate(&id, &network_log_script()).await {
            Ok(Value::Array(entries)) => entries,
            Ok(_) => Vec::new(),
            Err(error) => return refused(error),
        };
        let pattern = text(&args, "urlPattern");
        let kept = filter_network(
            &entries,
            flag(&args, "onlyFailed"),
            pattern.as_deref(),
            log_count(&args),
        );
        if kept.is_empty() {
            return ToolOutcome::text("No requests have been recorded.")
                .with_title("the network log");
        }
        let lines: Vec<String> = kept.iter().map(network_line).collect();
        ToolOutcome::text(lines.join("\n")).with_title(format!("the network log ({})", kept.len()))
    })
);

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_mock::MockGate;
    use parking_lot::Mutex;

    /// A pane made of canned answers, so the tests drive the shipping
    /// `execute` rather than a rehearsal of it. Every script it was asked to
    /// run is kept, because for most of these tools the script IS the
    /// behaviour.
    #[derive(Debug, Default)]
    struct FakePane {
        front: Option<String>,
        revealed: Mutex<Vec<String>>,
        opened: Mutex<Vec<(String, Option<String>)>>,
        scripts: Mutex<Vec<String>>,
        /// Answered in order; the last one repeats once the list runs out.
        answers: Mutex<Vec<std::result::Result<Value, String>>>,
        /// One answer, not a list: nothing here opens a pane twice.
        open_answer: Mutex<Option<std::result::Result<Value, String>>>,
    }

    impl FakePane {
        fn new() -> Self {
            Self {
                open_answer: Mutex::new(Some(Ok(json!({
                    "id": "chat:t1:web:1",
                    "url": "http://localhost:5173/",
                    "title": "Dev",
                    "loading": false,
                    "error": null,
                    "canGoBack": false,
                    "canGoForward": false
                })))),
                ..Default::default()
            }
        }

        fn answering(self, answers: Vec<std::result::Result<Value, String>>) -> Self {
            *self.answers.lock() = answers;
            self
        }

        fn page(url: &str, outline: &str) -> Value {
            json!({ "url": url, "title": "Dev", "refs": 2, "truncated": false, "outline": outline })
        }
    }

    #[async_trait]
    impl Pane for FakePane {
        fn front_of(&self, _thread: &str) -> Option<String> {
            self.front.clone()
        }

        async fn reveal(&self, id: &str) -> std::result::Result<(), String> {
            self.revealed.lock().push(id.to_string());
            Ok(())
        }

        async fn open(&self, id: &str, url: Option<&str>) -> std::result::Result<Value, String> {
            self.opened
                .lock()
                .push((id.to_string(), url.map(str::to_string)));
            self.open_answer
                .lock()
                .clone()
                .unwrap_or_else(|| Ok(Value::Null))
        }

        async fn evaluate(&self, _id: &str, code: &str) -> std::result::Result<Value, String> {
            self.scripts.lock().push(code.to_string());
            let mut answers = self.answers.lock();
            if answers.len() > 1 {
                answers.remove(0)
            } else {
                answers.first().cloned().unwrap_or(Ok(Value::Null))
            }
        }
    }

    fn context(session: &str) -> ToolContext {
        ToolContext {
            root: std::path::PathBuf::from("."),
            session: SessionId::from(session),
            call_id: ToolCallId::from("call_1"),
            permissions: Arc::new(MockGate::allow_all()),
        }
    }

    fn tool(pane: Arc<dyn Pane>, id: &str) -> Arc<dyn Tool> {
        browser_tools(pane)
            .into_iter()
            .find(|tool| tool.id() == id)
            .expect("the tool is registered")
    }

    /* -- which tab, and the permission the person writes ------------------ */

    #[test]
    fn a_conversation_with_no_pane_open_gets_its_first_tab() {
        let pane = FakePane::new();
        assert_eq!(
            pane_for(&pane, &context("t1/sub")),
            "chat:t1:web:1",
            "the subagent suffix is not part of the conversation"
        );
    }

    #[test]
    fn the_tab_the_person_is_looking_at_wins() {
        let mut pane = FakePane::new();
        pane.front = Some("chat:t1:web:3".to_string());
        assert_eq!(pane_for(&pane, &context("t1")), "chat:t1:web:3");
    }

    #[test]
    fn every_tool_shares_one_permission_key() {
        let pane: Arc<dyn Pane> = Arc::new(FakePane::new());
        for tool in browser_tools(pane) {
            let asked = tool.permission(&json!({}));
            assert_eq!(asked.key, "browser", "{} asked under another key", tool.id());
            assert_eq!(asked.always.as_deref(), Some("*"));
        }
    }

    #[test]
    fn the_permission_target_is_what_a_rule_would_be_written_about() {
        let pane: Arc<dyn Pane> = Arc::new(FakePane::new());
        let navigate = tool(pane.clone(), "browser_navigate");
        assert_eq!(
            navigate
                .permission(&json!({ "url": "http://localhost:5173" }))
                .target,
            "http://localhost:5173"
        );
        assert_eq!(
            tool(pane, "browser_read_page").permission(&json!({})).target,
            "the open page"
        );
    }

    /* -- argument handling ------------------------------------------------ */

    #[test]
    fn max_chars_is_clamped_at_both_ends() {
        assert_eq!(max_chars(&json!({}), 20_000), 20_000);
        assert_eq!(max_chars(&json!({ "maxChars": 12 }), 20_000), 500);
        assert_eq!(max_chars(&json!({ "maxChars": 999_999 }), 20_000), 50_000);
        // A model answering "0" is asking for the default, not for nothing.
        assert_eq!(max_chars(&json!({ "maxChars": 0 }), 20_000), 20_000);
    }

    #[test]
    fn log_count_is_clamped_at_both_ends() {
        assert_eq!(log_count(&json!({})), 50);
        assert_eq!(log_count(&json!({ "count": -4 })), 50);
        assert_eq!(log_count(&json!({ "count": 100_000 })), 500);
        assert_eq!(log_count(&json!({ "count": 3 })), 3);
    }

    #[test]
    fn the_count_is_applied_after_the_filter() {
        let entries: Vec<Value> = (0..10)
            .map(|n| json!({ "level": if n % 2 == 0 { "error" } else { "info" }, "message": format!("line {n}") }))
            .collect();
        let kept = filter_console(&entries, true, None, 3);
        assert_eq!(kept.len(), 3, "three errors, not three lines of which one is");
        assert_eq!(
            kept[2].get("message").and_then(Value::as_str),
            Some("line 8"),
            "the newest are kept"
        );
    }

    #[test]
    fn a_console_pattern_ignores_case() {
        let entries = vec![
            json!({ "level": "warning", "message": "Hydration MISMATCH" }),
            json!({ "level": "info", "message": "ready" }),
        ];
        let kept = filter_console(&entries, false, Some("mismatch"), 50);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn only_failed_keeps_both_kinds_of_failure() {
        let entries = vec![
            json!({ "method": "GET", "url": "/a", "status": 200, "ms": 4, "failed": false }),
            json!({ "method": "GET", "url": "/b", "status": 500, "ms": 9, "failed": true }),
            json!({ "method": "POST", "url": "/c", "status": null, "ms": null, "failed": true }),
        ];
        let kept = filter_network(&entries, true, None, 50);
        assert_eq!(kept.len(), 2);
        assert_eq!(network_line(&kept[0]), "GET 500 9ms /b");
        assert_eq!(
            network_line(&kept[1]),
            "POST failed ?ms /c",
            "a request that never answered says so rather than showing a zero"
        );
    }

    #[test]
    fn a_console_line_without_a_source_does_not_print_an_empty_one() {
        assert_eq!(
            console_line(&json!({ "level": "error", "message": "boom" })),
            "[error] boom"
        );
        assert_eq!(
            console_line(&json!({ "level": "error", "message": "boom", "source": "app.js", "line": 12 })),
            "[error] boom (app.js:12)"
        );
    }

    /* -- the scripts, which for most of these tools are the behaviour ----- */

    #[test]
    fn a_press_chord_carries_its_modifiers() {
        let script = press_script("Control+a");
        assert!(script.contains(r#"const key = "a""#), "{script}");
        assert!(script.contains("ctrlKey: true"));
        assert!(script.contains("shiftKey: false"));
    }

    #[test]
    fn press_understands_the_names_people_actually_write() {
        for chord in ["Ctrl+s", "Control+s", "cmd+s", "Meta+s"] {
            let script = press_script(chord);
            assert!(
                script.contains("ctrlKey: true") || script.contains("metaKey: true"),
                "{chord} lost its modifier"
            );
        }
    }

    #[test]
    fn a_typed_value_is_escaped_rather_than_pasted_into_the_script() {
        // A page title, a quote and a newline in one string. Without the JSON
        // encoding this closes the literal and the script is a syntax error at
        // best and the model's text as code at worst.
        let script = type_script("he said \"hi\";\n</script>", None);
        assert!(!script.contains("he said \"hi\""), "{script}");
        assert!(script.contains(r#"\"hi\""#));
    }

    #[test]
    fn a_ref_is_escaped_too() {
        let script = click_script(Some("ref_1\"); alert(1); //"), None, None);
        // The quote that would have closed the argument arrives escaped, so
        // the whole thing is still one string literal and the injection is
        // just a ref that will not be found.
        assert!(script.contains(r#"refs.get("ref_1\"); alert(1); //")"#), "{script}");
    }

    #[test]
    fn a_click_without_a_ref_finds_the_element_at_the_point() {
        let script = click_script(None, Some(12.5), Some(40.0));
        assert!(script.contains("elementFromPoint(12.5, 40)"), "{script}");
    }

    #[test]
    fn a_label_click_is_redirected_to_its_control() {
        let script = click_script(Some("ref_2"), None, None);
        assert!(script.contains(r#"el.tagName === "LABEL" && el.control"#));
    }

    #[test]
    fn the_outline_budget_reaches_the_page() {
        let script = read_page_script(12_000, true);
        assert!(script.contains("const budget = 12000;"));
        assert!(script.contains("const interactiveOnly = true;"));
    }

    /* -- the tools, driven ------------------------------------------------ */

    #[tokio::test]
    async fn navigating_opens_the_tab_and_reads_what_arrived() {
        let pane = Arc::new(
            FakePane::new().answering(vec![Ok(FakePane::page(
                "http://localhost:5173/",
                "main \"Dashboard\"",
            ))]),
        );
        let outcome = tool(pane.clone(), "browser_navigate")
            .execute(json!({ "url": "http://localhost:5173" }), &context("t1"))
            .await
            .expect("the tool ran");

        assert_eq!(
            pane.opened.lock().as_slice(),
            &[(
                "chat:t1:web:1".to_string(),
                Some("http://localhost:5173".to_string())
            )]
        );
        assert_eq!(
            pane.revealed.lock().as_slice(),
            &["chat:t1:web:1".to_string()],
            "the person is shown the page the answer is about"
        );
        assert!(outcome.output.contains("main \"Dashboard\""));
        assert_eq!(
            outcome.metadata.as_ref().and_then(|m| m.get("pane")),
            Some(&json!("chat:t1:web:1"))
        );
        assert_eq!(
            outcome.title.as_deref(),
            Some("Dev - http://localhost:5173/")
        );
    }

    #[tokio::test]
    async fn a_page_that_would_not_load_is_reported_rather_than_ending_the_turn() {
        let pane = Arc::new(FakePane::new());
        *pane.open_answer.lock() = Some(Ok(json!({
            "id": "chat:t1:web:1",
            "url": "",
            "title": "",
            "loading": false,
            "error": "ERR_CONNECTION_REFUSED (-102) loading http://localhost:9999/"
        })));
        let outcome = tool(pane, "browser_navigate")
            .execute(json!({ "url": "http://localhost:9999" }), &context("t1"))
            .await
            .expect("the tool ran");
        assert!(outcome.output.contains("ERR_CONNECTION_REFUSED"));
    }

    #[tokio::test]
    async fn reading_the_page_reports_how_many_refs_it_left_behind() {
        let pane = Arc::new(
            FakePane::new().answering(vec![Ok(FakePane::page("https://x.test/", "link \"Docs\" [ref_1]"))]),
        );
        let outcome = tool(pane.clone(), "browser_read_page")
            .execute(json!({ "maxChars": 4000, "interactiveOnly": true }), &context("t1"))
            .await
            .expect("the tool ran");

        let script = pane.scripts.lock()[0].clone();
        assert!(script.contains("const budget = 4000;"));
        assert!(script.contains("const interactiveOnly = true;"));
        let metadata = outcome.metadata.expect("metadata");
        assert_eq!(metadata.get("refs"), Some(&json!(2)));
        assert_eq!(metadata.get("truncated"), Some(&json!(false)));
    }

    #[tokio::test]
    async fn clicking_needs_a_ref_or_a_point_and_says_which() {
        let pane = Arc::new(FakePane::new());
        let outcome = tool(pane.clone(), "browser_click")
            .execute(json!({ "x": 10 }), &context("t1"))
            .await
            .expect("the tool ran");
        assert_eq!(
            outcome.output,
            "Give either a ref from browser_read_page or an x and y."
        );
        assert!(
            pane.scripts.lock().is_empty(),
            "nothing was run in the page"
        );
    }

    #[tokio::test]
    async fn a_stale_ref_is_reported_in_the_page_s_own_words() {
        let pane = Arc::new(FakePane::new().answering(vec![Ok(
            json!({ "ok": false, "error": "That ref is no longer on the page. Read it again." }),
        )]));
        let outcome = tool(pane, "browser_click")
            .execute(json!({ "ref": "ref_4" }), &context("t1"))
            .await
            .expect("the tool ran");
        assert_eq!(
            outcome.output,
            "That ref is no longer on the page. Read it again."
        );
    }

    #[tokio::test]
    async fn typing_answers_with_what_it_typed() {
        let pane = Arc::new(FakePane::new().answering(vec![Ok(json!({ "ok": true, "typed": 5 }))]));
        let outcome = tool(pane.clone(), "browser_type")
            .execute(json!({ "text": "hello", "ref": "ref_2" }), &context("t1"))
            .await
            .expect("the tool ran");
        assert_eq!(outcome.output, "Typed 5 characters.");
        assert_eq!(outcome.title.as_deref(), Some("ref_2"));
    }

    #[tokio::test]
    async fn an_empty_console_says_so_rather_than_printing_nothing() {
        let pane = Arc::new(FakePane::new().answering(vec![Ok(json!([]))]));
        let outcome = tool(pane, "browser_console")
            .execute(json!({}), &context("t1"))
            .await
            .expect("the tool ran");
        assert_eq!(outcome.output, "The console is empty.");
        assert_eq!(outcome.title.as_deref(), Some("the console"));
    }

    #[tokio::test]
    async fn the_console_tool_filters_what_the_page_handed_back() {
        let pane = Arc::new(FakePane::new().answering(vec![Ok(json!([
            { "level": "info", "message": "ready" },
            { "level": "error", "message": "Uncaught TypeError: x is not a function", "source": "app.js", "line": 3 }
        ]))]));
        let outcome = tool(pane, "browser_console")
            .execute(json!({ "onlyErrors": true }), &context("t1"))
            .await
            .expect("the tool ran");
        assert_eq!(
            outcome.output,
            "[error] Uncaught TypeError: x is not a function (app.js:3)"
        );
        assert_eq!(outcome.title.as_deref(), Some("the console (1)"));
    }

    #[tokio::test]
    async fn a_pane_that_has_gone_is_a_sentence_and_not_a_dead_turn() {
        let pane = Arc::new(
            FakePane::new().answering(vec![Err("That browser pane was closed.".to_string())]),
        );
        let outcome = tool(pane, "browser_read_text")
            .execute(json!({}), &context("t1"))
            .await
            .expect("the tool ran");
        assert_eq!(outcome.output, "That browser pane was closed.");
    }

    #[tokio::test]
    async fn evaluate_prints_a_structure_rather_than_rust_s_debug() {
        let pane = Arc::new(FakePane::new().answering(vec![Ok(json!({ "width": 320 }))]));
        let outcome = tool(pane, "browser_evaluate")
            .execute(json!({ "code": "getComputedStyle(document.body)" }), &context("t1"))
            .await
            .expect("the tool ran");
        assert_eq!(outcome.output, "{\n  \"width\": 320\n}");
    }
}

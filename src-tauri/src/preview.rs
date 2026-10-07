//! A real browser, inside the window.
//!
//! The app already had a browser: the one inside a sandboxed computer, driven
//! by keystrokes and watched through screenshots. That one is deliberately
//! somewhere else - a container with its own network and its own cookies - and
//! it is the right tool for "go and do something on the internet as a machine
//! that is not mine".
//!
//! This is the other half, and it is the half a person building software wants:
//! the page they are working on, open beside the conversation, on this machine,
//! with the dev server's cookies and localhost reachable. The app already
//! contains a webview, so this costs no dependency at all - a child webview
//! parented to the main window, positioned over a hole the renderer leaves for
//! it.
//!
//! ## Why the view is not in the React tree
//!
//! It cannot be. A child webview is a native sibling of the app's own webview,
//! not a DOM node, so it is layered over the page rather than inside it. The
//! renderer measures where the pane is and tells this module; this module puts
//! the view there. The consequence to remember is that nothing in the app can
//! be drawn ON TOP of the view (a dropdown that overlaps it will be behind it),
//! which is why the pane's own chrome is drawn around the hole and never over
//! it.
//!
//! ## What the agent gets
//!
//! The same view, driven by tools. See `inertia_tools::builtin::browser`: this
//! module supplies the [`Pane`] it runs against, and nothing else here knows
//! what a tool is. That the model and the person look at the SAME browser is
//! the whole design - a tool driving a hidden second browser would answer
//! questions about a page nobody was looking at.
//!
//! ## What Tauri does not give us, and what was done instead
//!
//! Three things Electron had are not in Tauri's webview API, and each is
//! handled rather than quietly missing:
//!
//!   - **Input events.** There is no `sendInputEvent`. Clicking, typing and
//!     key presses are dispatched as DOM events from inside the page instead;
//!     see the browser tools for exactly what that does and does not cover.
//!   - **Console and network.** There are no events for either. An
//!     initialization script wraps `console`, the error handlers, `fetch`,
//!     `XMLHttpRequest` and the resource timing observer into two bounded
//!     rings on `window`, which the tools read back on demand. A fresh
//!     document runs the script again, so the log is per page, which is what
//!     "since it last navigated" meant in the first place.
//!   - **Capturing a picture of the page.** There is genuinely no API for it:
//!     `capturePage` has no counterpart, and the ways to reach one on Windows
//!     (WebView2's `CapturePreview` through `webview2-com`, or `PrintWindow`
//!     plus a PNG encoder) both mean new dependencies and a COM callback that
//!     no test in this workspace could exercise. So `browser_screenshot` is
//!     not offered, and `previewAPI.screenshot` says so in a sentence rather
//!     than being a button that fails.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use inertia_core::tool::Tool;
use inertia_tools::builtin::browser::{self, Pane as PaneSeam};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl, Wry};

/// The one channel the pane speaks to the window over. `state` payloads drive
/// the address bar and the back and forward buttons; `reveal` asks the window
/// to bring a tab forward. Both shapes are read by `src/lib/preview.js`.
const EVENT: &str = "preview:event";

/// Only the app's own webview hears them. `emit` would broadcast to every
/// webview in the process - which now includes arbitrary pages the person has
/// opened - and handing a web page this app's internal events is not a thing
/// worth doing by accident.
const WINDOW: &str = "main";

/// How long a script gets to answer before the tool is told the page did not.
const EVAL_TIMEOUT: Duration = Duration::from_secs(15);

/// How often the result slot is checked while a script is running.
const EVAL_POLL: Duration = Duration::from_millis(25);

/// How many console lines and requests each page keeps.
const LOG_LIMIT: usize = 500;

/* -- the parts that are not a browser ------------------------------------- */

/// Bounds from the renderer, made safe to hand to a webview.
///
/// The numbers come from `getBoundingClientRect` in a window that may be
/// mid-animation, mid-resize, or scrolled - so fractions, negatives and NaN all
/// arrive here routinely, and a webview positioned with any of them is drawn
/// somewhere absurd. Rounded, clamped at zero, and a zero-sized rect is left as
/// zero rather than nudged to one: zero is how the pane says "I am not on
/// screen", and a 1px view parked over the transcript is a visible bug.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    fn as_json(&self) -> Value {
        json!({ "x": self.x, "y": self.y, "width": self.width, "height": self.height })
    }
}

/// A rect the renderer reported, clamped into something drawable.
pub fn safe_bounds(rect: &Value) -> Bounds {
    let number = |key: &str| {
        rect.get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .map(f64::round)
            .unwrap_or(0.0)
    };
    Bounds {
        x: number("x").max(0.0),
        y: number("y").max(0.0),
        width: number("width").max(0.0),
        height: number("height").max(0.0),
    }
}

/// Whether a rect the renderer called visible is actually big enough to draw.
///
/// A zero-sized view is not merely pointless: on Windows it is the difference
/// between a hidden pane and a webview asked for an invalid size. The pane
/// says "not on screen" by measuring to nothing, and this is where that is
/// understood.
pub fn is_drawable(bounds: &Bounds, visible: bool) -> bool {
    visible && bounds.width > 0.0 && bounds.height > 0.0
}

/// What somebody typed in the address bar, as a URL - or `None`.
///
/// Three cases, and the third is the whole reason this is a function. A real
/// URL passes through. A bare host or path gets `https://`, because nobody
/// types the scheme and a pane that refused `example.com` would be a pane
/// nobody uses. And anything whose scheme can execute - `javascript:`, `data:`,
/// `blob:`, `file:` - is refused outright, because this address bar is reachable
/// by a model: `browser_navigate` takes a string, the string comes from a
/// conversation, and a conversation can contain a web page that asked for one.
/// `javascript:` in this box would be script running in a page's own origin
/// with whatever session it holds.
///
/// A search box would be a fourth case. There is deliberately not one: an
/// address bar that quietly turns a typo into a search sends whatever was typed
/// to a third party, and a typo in this box may be a private path.
pub fn normalize_url(input: &str) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }

    // A scheme this app will not open, named explicitly, is refused rather
    // than treated as a hostname: `javascript:alert(1)` must not become
    // `https://javascript:alert(1)`. The digit check is doing real work -
    // `localhost:5173` is a host and a port, and a scheme test that does not
    // exclude digits reads "localhost" as the scheme and refuses the single
    // most common thing anybody types into this box.
    if let Some(colon) = text.find(':') {
        let head = &text[..colon];
        let scheme_shaped = !head.is_empty()
            && head.starts_with(|c: char| c.is_ascii_alphabetic())
            && head
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-');
        let followed_by_digit = text[colon + 1..].starts_with(|c: char| c.is_ascii_digit());
        if scheme_shaped && !followed_by_digit {
            let scheme = head.to_ascii_lowercase();
            if !matches!(scheme.as_str(), "http" | "https" | "about") {
                return None;
            }
            return tauri::Url::parse(text).ok().map(|url| url.to_string());
        }
    }

    // No scheme. `localhost:5173`, `example.com/x`, `192.168.1.4:8080` - all
    // things people type, none of them URLs yet. A single word with no dot and
    // no port is not a host either, and is refused rather than guessed at.
    let host = text.split(['/', '?', '#']).next().unwrap_or(text);
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    let ported = host
        .rsplit_once(':')
        .is_some_and(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()));
    let bare = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    let loopback = matches!(
        bare.as_str(),
        "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0"
    );
    if !host.contains('.') && !loopback && !ported {
        return None;
    }
    // A dev server is http. Defaulting the whole world to https is right and
    // defaulting `localhost:5173` to it is wrong every single time: there is
    // no certificate, the load fails, and the pane shows an error for a server
    // that is running perfectly well.
    let scheme = if loopback { "http" } else { "https" };
    tauri::Url::parse(&format!("{scheme}://{text}"))
        .ok()
        .map(|url| url.to_string())
}

/// The conversation a pane belongs to.
///
/// Pane ids are `chat:<threadId>:<kind>:<serial>` - built in the window by
/// `pane-tabs.js` - and the thread is the part that matters here, because the
/// agent's tools are told which CONVERSATION they are in and have to find the
/// tab the person is actually looking at. Anything not in that shape belongs to
/// no conversation, which is the safe answer: a tool that cannot find a tab
/// opens its own rather than guessing at somebody else's.
pub fn thread_of_pane(id: &str) -> Option<String> {
    let rest = id.strip_prefix("chat:")?;
    let (thread, tail) = rest.rsplit_once(':')?;
    if !tail.chars().all(|c| c.is_ascii_digit()) || tail.is_empty() {
        return None;
    }
    let (thread, kind) = thread.rsplit_once(':')?;
    if kind != "web" && kind != "sh" {
        return None;
    }
    if thread.is_empty() {
        return None;
    }
    Some(thread.to_string())
}

/// A webview label for a pane id.
///
/// Labels are restricted to a small alphabet, and a pane id carries a thread id
/// that came from somewhere else entirely. The serial is what makes reopening a
/// closed tab safe: a label is not reliably free the instant its webview is
/// told to close, and a collision there is a pane that silently never appears.
pub fn label_for(id: &str, serial: u64) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '/') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("preview-{serial}-{safe}")
}

/// Where a pane is in its own history.
///
/// Tauri's webview offers no `canGoBack`, so this is the model kept by hand.
/// It is the browser's own: a list with a cursor in it. A new page truncates
/// everything ahead of the cursor, going back moves the cursor without
/// changing the list, and the two buttons read off the cursor's position.
///
/// Kept separate and tested because the failure mode is silent - a Back button
/// that is greyed out on a page with history behind it looks exactly like a
/// page with no history behind it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct History {
    started: bool,
    index: usize,
    len: usize,
}

impl History {
    /// A new document was committed, not by going back or forward.
    pub fn visited(&mut self) {
        if self.started {
            self.index += 1;
        } else {
            self.started = true;
            self.index = 0;
        }
        self.len = self.index + 1;
    }

    /// The pane was sent back. Answers whether there was anywhere to go, so
    /// the caller does not have to ask twice.
    pub fn went_back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.index -= 1;
        true
    }

    pub fn went_forward(&mut self) -> bool {
        if !self.can_go_forward() {
            return false;
        }
        self.index += 1;
        true
    }

    pub fn can_go_back(&self) -> bool {
        self.started && self.index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.started && self.index + 1 < self.len
    }
}

/* -- what runs in the page ------------------------------------------------ */

/// The console, the errors and the network log, captured from inside.
///
/// Injected before anything on the page runs, and again on every document, so
/// a log is always about the page it is read from. The rings are bounded
/// because a page left open overnight writes an unbounded number of lines and
/// every one of them would sit in the page until it navigated.
fn capture_script() -> String {
    format!(
        r#"(function () {{
  if (window.__inertiaConsole) return;
  var LIMIT = {LOG_LIMIT};
  function ring() {{
    return {{ entries: [], push: function (entry) {{
      this.entries.push(entry);
      if (this.entries.length > LIMIT) this.entries.splice(0, this.entries.length - LIMIT);
    }} }};
  }}
  var out = ring();
  var net = ring();
  window.__inertiaConsole = out;
  window.__inertiaNetwork = net;

  function say(value) {{
    try {{ return typeof value === "string" ? value : JSON.stringify(value); }}
    catch (error) {{ return String(value); }}
  }}
  function clip(text) {{
    text = String(text);
    return text.length <= 4000 ? text : text.slice(0, 4000) + "\n\u2026 (" + (text.length - 4000) + " more characters not shown)";
  }}
  function log(level, message, source, line) {{
    try {{ out.push({{ level: level, message: clip(message), source: source || "", line: line || 0, at: Date.now() }}); }}
    catch (error) {{ /* the log must never be why a page breaks */ }}
  }}

  var LEVELS = {{ log: "info", info: "info", debug: "verbose", warn: "warning", error: "error" }};
  Object.keys(LEVELS).forEach(function (name) {{
    var original = console[name];
    if (!original) return;
    console[name] = function () {{
      var parts = [];
      for (var i = 0; i < arguments.length; i += 1) parts.push(say(arguments[i]));
      log(LEVELS[name], parts.join(" "));
      return original.apply(console, arguments);
    }};
  }});
  window.addEventListener("error", function (event) {{
    log("error", event.message || String(event.error), event.filename, event.lineno);
  }});
  window.addEventListener("unhandledrejection", function (event) {{
    log("error", "Unhandled rejection: " + say(event.reason));
  }});

  var nextId = 0;
  function record(method, url, ms, status, failed, type) {{
    try {{
      net.push({{ id: String(++nextId), method: String(method || "GET").toUpperCase(), url: String(url),
        status: status === undefined ? null : status, type: type || null,
        ms: ms === null ? null : Math.round(ms), failed: !!failed, at: Date.now() }});
    }} catch (error) {{ /* as above */ }}
  }}

  // Everything the page loaded that is not a fetch or an XHR - scripts,
  // stylesheets, images. Those two are wrapped below instead, because timing
  // alone cannot say whether a request failed and a failed API call is the
  // thing anybody is looking for.
  try {{
    new PerformanceObserver(function (list) {{
      var entries = list.getEntries();
      for (var i = 0; i < entries.length; i += 1) {{
        var entry = entries[i];
        if (entry.initiatorType === "fetch" || entry.initiatorType === "xmlhttprequest") continue;
        var status = typeof entry.responseStatus === "number" ? entry.responseStatus : null;
        record("GET", entry.name, entry.duration, status, status !== null && status >= 400, entry.initiatorType);
      }}
    }}).observe({{ type: "resource", buffered: true }});
  }} catch (error) {{ /* an old engine simply reports less */ }}

  var fetch_ = window.fetch;
  if (fetch_) {{
    window.fetch = function (input, init) {{
      var started = performance.now();
      var url = typeof input === "string" ? input : (input && input.url) || String(input);
      var method = (init && init.method) || (input && input.method) || "GET";
      return fetch_.apply(this, arguments).then(function (response) {{
        record(method, url, performance.now() - started, response.status, response.status >= 400, "fetch");
        return response;
      }}, function (error) {{
        record(method, url, performance.now() - started, null, true, "fetch");
        throw error;
      }});
    }};
  }}

  var open_ = XMLHttpRequest.prototype.open;
  var send_ = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (method, url) {{
    this.__inertiaRequest = {{ method: method, url: url }};
    return open_.apply(this, arguments);
  }};
  XMLHttpRequest.prototype.send = function () {{
    var request = this;
    var info = this.__inertiaRequest || {{}};
    var started = performance.now();
    this.addEventListener("loadend", function () {{
      var status = request.status || null;
      record(info.method, info.url, performance.now() - started, status, !status || status >= 400, "xhr");
    }});
    return send_.apply(this, arguments);
  }};
}})();"#
    )
}

/// Start a script running and park its answer under `token`.
///
/// Two steps rather than one because the webview's callback cannot wait: it
/// hands back whatever the script evaluated to right now, and "right now" for
/// anything asynchronous is a pending promise. So the page resolves the value
/// into a slot and [`poll_script`] collects it, which is what makes
/// `browser_evaluate` on an async expression return the answer rather than
/// `{}`.
fn begin_script(token: &str, code: &str) -> String {
    let token = json!(token).to_string();
    let code = json!(code).to_string();
    format!(
        r#"(function () {{
  var store = window.__inertiaEval || (window.__inertiaEval = {{}});
  var key = {token};
  function safe(value) {{
    if (value === undefined) return null;
    // A DOM node, a window, anything cyclic: the webview serialises the slot
    // with JSON, so a value that cannot survive that would make the result
    // unreadable rather than merely ugly.
    try {{ return JSON.parse(JSON.stringify(value)); }} catch (error) {{ return String(value); }}
  }}
  function fail(error) {{
    store[key] = {{ ok: false, error: String((error && error.message) || error) }};
  }}
  try {{
    Promise.resolve((0, eval)({code})).then(function (value) {{
      store[key] = {{ ok: true, value: safe(value) }};
    }}, fail);
  }} catch (error) {{
    fail(error);
  }}
  return true;
}})();"#
    )
}

/// Collect a finished script's answer, or `null` while it is still running.
fn poll_script(token: &str) -> String {
    let token = json!(token).to_string();
    format!(
        r#"(function () {{
  var store = window.__inertiaEval;
  if (!store) return null;
  var result = store[{token}];
  if (result === undefined) return null;
  delete store[{token}];
  return result;
}})();"#
    )
}

/* -- the panes ------------------------------------------------------------ */

struct Live {
    label: String,
    webview: Webview<Wry>,
    bounds: Bounds,
    visible: bool,
    title: String,
    url: String,
    loading: bool,
    error: Option<String>,
    history: History,
    /// Set when this app asked for a traversal, so the load it causes is not
    /// counted as a new page. Without it, pressing Back would push a history
    /// entry and the Forward button would never light up.
    traversing: bool,
    /// The `seq` of the last placement applied. See [`Panes::place`].
    placed: u64,
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live")
            .field("label", &self.label)
            .field("url", &self.url)
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

/// Every browser pane in the window.
///
/// A process-wide table rather than a field of a turn or of a command, because
/// that is what a pane is: the person opens one, a tool five minutes later
/// drives it, and the turn that started the page has long since ended.
#[derive(Default)]
pub struct Panes {
    live: Mutex<HashMap<String, Live>>,
    /// The tab each conversation is looking at, by thread id.
    ///
    /// The agent's `browser_*` tools are told which conversation they are in,
    /// not which tab - the person opens and closes those, and expects the agent
    /// to be driving the one in front of them. So `place` records whichever tab
    /// is currently visible, and the tools ask here.
    front: Mutex<HashMap<String, String>>,
    serial: AtomicU64,
}

impl std::fmt::Debug for Panes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Panes")
            .field("open", &self.live.lock().len())
            .finish_non_exhaustive()
    }
}

static PANES: OnceLock<Arc<Panes>> = OnceLock::new();

impl Panes {
    /// The one table, made on first use.
    pub fn global() -> Arc<Panes> {
        PANES.get_or_init(|| Arc::new(Panes::default())).clone()
    }

    /// The tab this conversation is showing, or `None`.
    pub fn front_of(&self, thread: &str) -> Option<String> {
        let id = self.front.lock().get(thread).cloned()?;
        self.live.lock().contains_key(&id).then_some(id)
    }

    /// Any pane that is open, for the things that are true of all of them.
    ///
    /// The cookie store is one: every pane is a child webview of the same
    /// window and they share it, so signing one in signs them all in. Which
    /// one answers is therefore not a question worth asking.
    pub fn any_webview(&self) -> Option<Webview<Wry>> {
        self.live
            .lock()
            .values()
            .next()
            .map(|pane| pane.webview.clone())
    }

    fn webview(&self, id: &str) -> Option<Webview<Wry>> {
        self.live.lock().get(id).map(|pane| pane.webview.clone())
    }

    /// What the pane tells the window about itself.
    fn announce(&self, app: &AppHandle, id: &str) -> Option<Value> {
        let state = {
            let live = self.live.lock();
            let pane = live.get(id)?;
            json!({
                "type": "state",
                "id": id,
                "url": pane.url,
                "title": pane.title,
                "loading": pane.loading,
                "error": pane.error,
                "canGoBack": pane.history.can_go_back(),
                "canGoForward": pane.history.can_go_forward(),
            })
        };
        let _ = app.emit_to(WINDOW, EVENT, state.clone());
        Some(state)
    }

    /// The pane, made if it did not exist.
    fn ensure(self: &Arc<Self>, app: &AppHandle, id: &str) -> Result<(), String> {
        if self.live.lock().contains_key(id) {
            return Ok(());
        }

        let window = app
            .get_window(WINDOW)
            .ok_or_else(|| "There is no window to put a browser in.".to_string())?;
        let serial = self.serial.fetch_add(1, Ordering::Relaxed) + 1;
        let label = label_for(id, serial);

        let watcher = Arc::downgrade(self);
        let blank = tauri::Url::parse("about:blank").map_err(|error| error.to_string())?;

        let builder = WebviewBuilder::new(label.clone(), WebviewUrl::External(blank))
            // The page runs as a page. No initialization of this app's IPC
            // reaches it: a child webview gets the app's commands only if a
            // capability names it, and none does.
            .initialization_script(capture_script())
            .on_page_load({
                let app = app.clone();
                let watcher = watcher.clone();
                let id = id.to_string();
                move |_webview, payload| {
                    let Some(panes) = watcher.upgrade() else {
                        return;
                    };
                    let started = matches!(payload.event(), tauri::webview::PageLoadEvent::Started);
                    let url = payload.url().to_string();
                    // The empty page every pane is created on is not a page
                    // anybody visited. Counted, it would light up Back on the
                    // first real navigation and send the person to a blank
                    // view; recorded as the url, it would replace the pane's
                    // own "type an address" placeholder with nothing.
                    let blank = url == "about:blank";
                    {
                        let mut live = panes.live.lock();
                        let Some(pane) = live.get_mut(&id) else {
                            return;
                        };
                        if !blank {
                            pane.url = url;
                        }
                        pane.loading = started && !blank;
                        if started && !blank {
                            pane.error = None;
                            // A traversal is a move within the list, not a new
                            // entry in it - the cursor was already updated
                            // when the traversal was asked for.
                            if pane.traversing {
                                pane.traversing = false;
                            } else {
                                pane.history.visited();
                            }
                        }
                    }
                    panes.announce(&app, &id);
                }
            })
            .on_document_title_changed({
                let app = app.clone();
                let watcher = watcher.clone();
                let id = id.to_string();
                move |_webview, title| {
                    let Some(panes) = watcher.upgrade() else {
                        return;
                    };
                    if let Some(pane) = panes.live.lock().get_mut(&id) {
                        pane.title = title;
                    }
                    panes.announce(&app, &id);
                }
            })
            .on_new_window({
                let watcher = watcher.clone();
                let id = id.to_string();
                move |url, _features| {
                    // A target=_blank inside the pane opens in the pane. A
                    // second native window would be a browser the app does not
                    // manage and cannot close.
                    if let Some(panes) = watcher.upgrade() {
                        if let Some(target) = normalize_url(url.as_str()) {
                            if let Some(webview) = panes.webview(&id) {
                                if let Ok(parsed) = tauri::Url::parse(&target) {
                                    let _ = webview.navigate(parsed);
                                }
                            }
                        }
                    }
                    tauri::webview::NewWindowResponse::Deny
                }
            })
            .on_navigation(|url| {
                // The same rule as the address bar, at the last possible
                // moment: a page can hand itself a `javascript:` or `data:`
                // location, and refusing only what was typed would leave the
                // interesting half open.
                matches!(url.scheme(), "http" | "https" | "about")
            });

        // Made at the size it will be placed at in a moment, and immediately
        // hidden: a webview that appears before the renderer has measured its
        // hole flashes full-window over the transcript.
        let webview = window
            .add_child(
                builder,
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1.0, 1.0),
            )
            .map_err(|error| format!("The browser pane could not be created: {error}"))?;
        let _ = webview.hide();

        self.live.lock().insert(
            id.to_string(),
            Live {
                label,
                webview,
                bounds: Bounds::default(),
                visible: false,
                title: String::new(),
                url: String::new(),
                loading: false,
                error: None,
                history: History::default(),
                traversing: false,
                placed: 0,
            },
        );
        Ok(())
    }

    /// Where the pane is, and whether it should be drawn.
    /// Put a pane where the renderer measured its hole, or hide it.
    ///
    /// Two rules here, and both were learned from the same white rectangle
    /// left over the transcript after the browser was closed.
    ///
    /// **This never creates a pane.** Positioning something that does not
    /// exist is meaningless, and it used to call `ensure` - so the very call
    /// whose job was to hide a pane on unmount could instead conjure one up
    /// and leave it on screen with nothing left to tell it where to be.
    ///
    /// **A newer placement always wins.** These arrive as separate commands
    /// and are answered concurrently, so the order they are handled in is not
    /// the order they were sent in. The pane sends a rising `seq` and anything
    /// older than what has already been applied is dropped. Without that, the
    /// last thing said on the way out - hide - could be overtaken by a
    /// `visible: true` from the timer a moment earlier, and nothing would ever
    /// correct it: the component that would have is gone.
    pub fn place(
        self: &Arc<Self>,
        _app: &AppHandle,
        id: &str,
        rect: &Value,
        visible: bool,
        seq: u64,
    ) -> Result<Value, String> {
        let bounds = safe_bounds(rect);
        let on = is_drawable(&bounds, visible);

        let webview = {
            let mut live = self.live.lock();
            let Some(pane) = live.get_mut(id) else {
                // Not an error. A pane closes while its last placement is
                // still in flight every single time one is closed.
                return Ok(json!({ "id": id, "closed": true }));
            };
            if seq > 0 && seq < pane.placed {
                return Ok(
                    json!({ "id": id, "bounds": pane.bounds.as_json(), "visible": pane.visible, "stale": true }),
                );
            }
            pane.placed = seq;
            pane.bounds = bounds;
            pane.visible = on;
            pane.webview.clone()
        };

        if on {
            let _ = webview.set_position(LogicalPosition::new(bounds.x, bounds.y));
            let _ = webview.set_size(LogicalSize::new(bounds.width, bounds.height));
            let _ = webview.show();
        } else {
            // Deliberately not resized to zero first: an invalid size is an
            // error on Windows, and hiding is what "not on screen" means
            // anyway.
            let _ = webview.hide();
        }

        // Whichever tab is on screen is the one the agent's tools should drive.
        if let Some(thread) = thread_of_pane(id) {
            let mut front = self.front.lock();
            if on {
                front.insert(thread, id.to_string());
            } else if front.get(&thread).map(String::as_str) == Some(id) {
                front.remove(&thread);
            }
        }

        Ok(json!({ "id": id, "bounds": bounds.as_json(), "visible": on }))
    }

    /// Open the pane, and go somewhere if asked.
    pub async fn open(
        self: &Arc<Self>,
        app: &AppHandle,
        id: &str,
        url: Option<&str>,
    ) -> Result<Value, String> {
        self.ensure(app, id)?;
        if let Some(url) = url.map(str::trim).filter(|url| !url.is_empty()) {
            return self.navigate(app, id, url).await;
        }
        self.announce(app, id)
            .ok_or_else(|| "That browser pane has gone away. Open it again.".to_string())
    }

    pub async fn navigate(
        self: &Arc<Self>,
        app: &AppHandle,
        id: &str,
        url: &str,
    ) -> Result<Value, String> {
        self.ensure(app, id)?;
        let Some(target) = normalize_url(url) else {
            let shown: String = url.chars().take(120).collect();
            return Err(format!(
                "\"{shown}\" is not a web address this pane will open. Give an http or https URL."
            ));
        };
        let parsed = tauri::Url::parse(&target).map_err(|error| error.to_string())?;

        let webview = self
            .webview(id)
            .ok_or_else(|| "That browser pane has gone away. Open it again.".to_string())?;
        {
            let mut live = self.live.lock();
            if let Some(pane) = live.get_mut(id) {
                pane.error = None;
                pane.loading = true;
            }
        }
        if let Err(error) = webview.navigate(parsed) {
            // A failed load is a page state, not a crashed call: the pane
            // shows the error and stays open, which is what a browser does.
            let mut live = self.live.lock();
            if let Some(pane) = live.get_mut(id) {
                pane.loading = false;
                pane.error = Some(error.to_string());
            }
        }

        // The load is asynchronous, and the page's own events are what update
        // the url and the title. Waiting for the first of them means the tool
        // that navigated reads the page it asked for rather than the one
        // before it.
        self.settle(id).await;
        self.announce(app, id)
            .ok_or_else(|| "That browser pane was closed while the page was loading.".to_string())
    }

    /// Wait for the page to stop loading, or give up and let the caller read
    /// whatever is there.
    ///
    /// A ceiling rather than a promise: a page that streams for ever - a chat
    /// UI holding an open response, a dev server's websocket - would otherwise
    /// hold a tool call open until it timed out, and the outline of a page that
    /// has painted is worth more than a perfect wait.
    ///
    /// `loading` is set by the caller before the navigation is asked for, so
    /// this cannot return on the gap between asking and the page's first
    /// event. That gap is exactly where the tool would have read the PREVIOUS
    /// page and reported it as the new one.
    async fn settle(&self, id: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            tokio::time::sleep(Duration::from_millis(60)).await;
            let done = {
                let live = self.live.lock();
                match live.get(id) {
                    None => true,
                    Some(pane) => !pane.loading,
                }
            };
            if done || std::time::Instant::now() >= deadline {
                return;
            }
        }
    }

    /// Back, forward, reload, stop.
    pub fn history(&self, app: &AppHandle, id: &str, direction: &str) -> Result<Value, String> {
        let webview = self
            .webview(id)
            .ok_or_else(|| format!("No browser pane is open with id {id}."))?;

        let moved = {
            let mut live = self.live.lock();
            let Some(pane) = live.get_mut(id) else {
                return Err("That browser pane has gone away. Open it again.".to_string());
            };
            match direction {
                "back" => pane.history.went_back(),
                "forward" => pane.history.went_forward(),
                _ => false,
            }
        };
        if moved {
            if let Some(pane) = self.live.lock().get_mut(id) {
                pane.traversing = true;
            }
        }

        let script = match direction {
            "back" if moved => "history.back()",
            "forward" if moved => "history.forward()",
            "reload" => "location.reload()",
            "stop" => "window.stop()",
            _ => "",
        };
        if !script.is_empty() {
            let _ = webview.eval(script);
        }
        self.announce(app, id)
            .ok_or_else(|| "That browser pane has gone away. Open it again.".to_string())
    }

    /// The pane's state, or `None` when there is no such pane. Not an error:
    /// the window asks about tabs it is still mounting.
    pub fn state(&self, app: &AppHandle, id: &str) -> Option<Value> {
        self.announce(app, id)
    }

    pub fn list(&self) -> Vec<Value> {
        self.live
            .lock()
            .iter()
            .map(|(id, pane)| {
                json!({ "id": id, "url": pane.url, "title": pane.title, "visible": pane.visible })
            })
            .collect()
    }

    /// Ask the window to bring this pane forward.
    ///
    /// The agent drives a browser that the person may not have open. Without
    /// this it worked perfectly and invisibly: pages loaded, clicks landed, and
    /// the only evidence was a tool result claiming things about a window
    /// nobody could see - which is indistinguishable, from the outside, from a
    /// model making it up.
    ///
    /// A request rather than an instruction. The window decides what "forward"
    /// means: open the dock, pick the Browser tab, put this tab in front. This
    /// side has no business knowing the panel is a panel.
    pub fn reveal(self: &Arc<Self>, app: &AppHandle, id: &str) -> Result<Value, String> {
        self.ensure(app, id)?;
        let payload = json!({
            "type": "reveal",
            "id": id,
            "kind": "web",
            "threadId": thread_of_pane(id),
        });
        let _ = app.emit_to(WINDOW, EVENT, payload);
        Ok(json!({ "id": id }))
    }

    pub fn dev_tools(&self, id: &str, open: bool) -> Result<Value, String> {
        // Unconditional, because the app crate turns on Tauri's `devtools`
        // feature: without it these two methods only exist in a debug build,
        // and the button in the pane's own chrome would be dead in the app
        // people actually install.
        let webview = self
            .webview(id)
            .ok_or_else(|| format!("No browser pane is open with id {id}."))?;
        if open {
            webview.open_devtools();
        } else {
            webview.close_devtools();
        }
        Ok(json!({ "devTools": open }))
    }

    pub fn close(&self, id: &str) -> Value {
        let Some(pane) = self.live.lock().remove(id) else {
            return json!({ "closed": false });
        };
        if let Some(thread) = thread_of_pane(id) {
            let mut front = self.front.lock();
            if front.get(&thread).map(String::as_str) == Some(id) {
                front.remove(&thread);
            }
        }
        if let Err(error) = pane.webview.close() {
            tracing::warn!(target: "preview", pane = %id, label = %pane.label, "could not close pane: {error}");
        }
        json!({ "closed": true })
    }

    /// Run script in the page and return what it evaluated to.
    pub async fn evaluate(&self, id: &str, code: &str) -> Result<Value, String> {
        let webview = self
            .webview(id)
            .ok_or_else(|| "That browser pane was closed.".to_string())?;

        let token = format!("e{}", uuid::Uuid::now_v7().simple());
        raw_eval(&webview, &begin_script(&token, code)).await?;

        let deadline = std::time::Instant::now() + EVAL_TIMEOUT;
        loop {
            match raw_eval(&webview, &poll_script(&token)).await? {
                Value::Null => {}
                result => {
                    if result.get("ok").and_then(Value::as_bool) == Some(true) {
                        return Ok(result.get("value").cloned().unwrap_or(Value::Null));
                    }
                    return Err(result
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("The page could not run that.")
                        .to_string());
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err("The page did not answer within 15 seconds.".to_string());
            }
            tokio::time::sleep(EVAL_POLL).await;
        }
    }
}

/// One round trip into the page, with its answer parsed.
///
/// The callback is `Fn` and fires once, so the sender lives behind a mutex:
/// a `FnOnce` cannot be handed to a webview that has no way to promise it will
/// only call it once.
async fn raw_eval(webview: &Webview<Wry>, code: &str) -> Result<Value, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Mutex::new(Some(tx));
    webview
        .eval_with_callback(code, move |answer| {
            if let Some(tx) = tx.lock().take() {
                let _ = tx.send(answer);
            }
        })
        .map_err(|error| format!("The page could not be reached: {error}"))?;

    let answer = tokio::time::timeout(EVAL_TIMEOUT, rx)
        .await
        .map_err(|_| "The page did not answer within 15 seconds.".to_string())?
        .map_err(|_| "That browser pane was closed while the page was answering.".to_string())?;

    // `undefined` and a value that would not serialise both come back as
    // something that is not JSON. Null is the honest reading of both, and is
    // what the caller's own `ok`/`error` envelope distinguishes anyway.
    Ok(serde_json::from_str(&answer).unwrap_or(Value::Null))
}

/* -- the seam the tools run against --------------------------------------- */

/// The browser tools' view of this module.
#[derive(Debug)]
struct PaneBridge {
    app: AppHandle,
    panes: Arc<Panes>,
}

#[async_trait]
impl PaneSeam for PaneBridge {
    fn front_of(&self, thread: &str) -> Option<String> {
        self.panes.front_of(thread)
    }

    async fn reveal(&self, id: &str) -> Result<(), String> {
        self.panes.reveal(&self.app, id).map(|_| ())
    }

    async fn open(&self, id: &str, url: Option<&str>) -> Result<Value, String> {
        self.panes.open(&self.app, id, url).await
    }

    async fn evaluate(&self, id: &str, code: &str) -> Result<Value, String> {
        self.panes.evaluate(id, code).await
    }
}

/// Every `browser_*` tool, bound to this window's panes.
pub fn browser_tools(app: AppHandle) -> Vec<Arc<dyn Tool>> {
    browser::browser_tools(Arc::new(PaneBridge {
        app,
        panes: Panes::global(),
    }))
}

/* -- the window's half ---------------------------------------------------- */

/// Every command here is `async` on purpose, and it is not about waiting.
/// Creating and moving a child webview has to happen on the main thread, and
/// Tauri gets there by posting to the event loop and blocking until it
/// answers - which from a synchronous command, already on that thread, is a
/// deadlock. An async command runs on the runtime instead.
#[tauri::command]
pub async fn preview_open(
    app: AppHandle,
    id: String,
    url: Option<String>,
) -> Result<Value, String> {
    Panes::global().open(&app, &id, url.as_deref()).await
}

#[tauri::command]
pub async fn preview_navigate(app: AppHandle, id: String, url: String) -> Result<Value, String> {
    Panes::global().navigate(&app, &id, &url).await
}

#[tauri::command]
pub async fn preview_history(
    app: AppHandle,
    id: String,
    direction: String,
) -> Result<Value, String> {
    Panes::global().history(&app, &id, &direction)
}

/// Called on every resize and scroll of the pane, so it is deliberately the
/// cheapest thing here: no page work, one move.
#[tauri::command]
pub async fn preview_place(
    app: AppHandle,
    id: String,
    rect: Value,
    visible: bool,
    seq: Option<u64>,
) -> Result<Value, String> {
    Panes::global().place(&app, &id, &rect, visible, seq.unwrap_or(0))
}

#[tauri::command]
pub async fn preview_state(app: AppHandle, id: String) -> Result<Option<Value>, String> {
    Ok(Panes::global().state(&app, &id))
}

#[tauri::command]
pub async fn preview_list() -> Result<Vec<Value>, String> {
    Ok(Panes::global().list())
}

/// The console and the network log, for the window rather than for a tool.
///
/// The same rings the `browser_console` and `browser_network` tools read, and
/// the same filtering, so a person reading the log in the pane and a model
/// reading it in a turn are looking at one thing. Both live in the page, so
/// both are gone the moment it navigates - which is the definition anybody
/// reading a console after a change actually wants.
#[tauri::command]
pub async fn preview_console(id: String, options: Option<Value>) -> Result<Vec<Value>, String> {
    let options = options.unwrap_or_else(|| json!({}));
    let entries = page_log(&id, &browser::console_log_script()).await?;
    Ok(browser::filter_console(
        &entries,
        options
            .get("onlyErrors")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        options.get("pattern").and_then(Value::as_str),
        browser::log_count(&options),
    ))
}

#[tauri::command]
pub async fn preview_network(id: String, options: Option<Value>) -> Result<Vec<Value>, String> {
    let options = options.unwrap_or_else(|| json!({}));
    let entries = page_log(&id, &browser::network_log_script()).await?;
    Ok(browser::filter_network(
        &entries,
        options
            .get("onlyFailed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        options.get("urlPattern").and_then(Value::as_str),
        browser::log_count(&options),
    ))
}

/// One of the page's two rings, or an empty one for a page that has not run
/// the capture script yet.
async fn page_log(id: &str, script: &str) -> Result<Vec<Value>, String> {
    match Panes::global().evaluate(id, script).await? {
        Value::Array(entries) => Ok(entries),
        _ => Ok(Vec::new()),
    }
}

#[tauri::command]
pub async fn preview_dev_tools(id: String, open: bool) -> Result<Value, String> {
    Panes::global().dev_tools(&id, open)
}

#[tauri::command]
pub async fn preview_close(id: String) -> Result<Value, String> {
    Ok(Panes::global().close(&id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /* -- placement -------------------------------------------------------- */

    #[test]
    fn a_measured_rect_is_rounded_and_clamped() {
        let bounds = safe_bounds(&json!({ "x": 12.4, "y": -3.0, "width": 640.6, "height": 480.2 }));
        assert_eq!(
            bounds,
            Bounds {
                x: 12.0,
                y: 0.0,
                width: 641.0,
                height: 480.0
            }
        );
    }

    #[test]
    fn a_rect_that_did_not_survive_the_bridge_is_zero_rather_than_nonsense() {
        // The bug that cost the whole pane once: a DOMRect crossing a bridge
        // that does not clone host objects arrives as `{}`. Zero is the right
        // reading, and zero means "not on screen" rather than "one pixel".
        let bounds = safe_bounds(&json!({}));
        assert_eq!(bounds, Bounds::default());
        assert!(!is_drawable(&bounds, true));
    }

    #[test]
    fn infinities_and_strings_do_not_reach_the_webview() {
        let bounds =
            safe_bounds(&json!({ "x": "12", "y": null, "width": f64::MAX, "height": 8.0 }));
        assert_eq!(bounds.x, 0.0, "a string is not a measurement");
        assert_eq!(bounds.height, 8.0);
        assert!(bounds.width.is_finite());
    }

    #[test]
    fn a_visible_pane_with_no_size_is_not_drawn() {
        let flat = Bounds {
            x: 10.0,
            y: 10.0,
            width: 400.0,
            height: 0.0,
        };
        assert!(!is_drawable(&flat, true), "a collapsed dock draws nothing");
        let real = Bounds {
            width: 400.0,
            height: 300.0,
            ..Default::default()
        };
        assert!(is_drawable(&real, true));
        assert!(!is_drawable(&real, false), "a background tab stays hidden");
    }

    /* -- the address bar -------------------------------------------------- */

    #[test]
    fn a_dev_server_is_http_and_the_rest_of_the_world_is_https() {
        assert_eq!(
            normalize_url("localhost:5173").as_deref(),
            Some("http://localhost:5173/")
        );
        assert_eq!(
            normalize_url("127.0.0.1:8080/app").as_deref(),
            Some("http://127.0.0.1:8080/app")
        );
        assert_eq!(
            normalize_url("example.com/docs").as_deref(),
            Some("https://example.com/docs")
        );
    }

    #[test]
    fn a_scheme_that_can_execute_is_refused_rather_than_guessed_at() {
        // Reachable by a model: `browser_navigate` takes a string, and the
        // string can have come from a page the agent was reading.
        for hostile in [
            "javascript:alert(1)",
            "data:text/html,<script>x()</script>",
            "file:///C:/Windows/System32",
            "blob:https://example.com/x",
        ] {
            assert_eq!(
                normalize_url(hostile),
                None,
                "{hostile} was allowed through"
            );
        }
    }

    #[test]
    fn a_bare_word_is_not_turned_into_a_search() {
        assert_eq!(normalize_url("recipes"), None);
        assert_eq!(normalize_url(""), None);
        assert_eq!(normalize_url("   "), None);
    }

    #[test]
    fn a_real_url_passes_through_untouched() {
        assert_eq!(
            normalize_url("https://example.com/a?b=c#d").as_deref(),
            Some("https://example.com/a?b=c#d")
        );
        assert_eq!(
            normalize_url("http://localhost:1420/").as_deref(),
            Some("http://localhost:1420/")
        );
    }

    /* -- which conversation a pane belongs to ----------------------------- */

    #[test]
    fn a_pane_id_names_its_conversation() {
        assert_eq!(
            thread_of_pane("chat:thread_123:web:2").as_deref(),
            Some("thread_123")
        );
        assert_eq!(
            thread_of_pane("chat:a:b:c:sh:1").as_deref(),
            Some("a:b:c"),
            "a thread id with colons in it is still one thread"
        );
    }

    #[test]
    fn anything_else_belongs_to_no_conversation() {
        for odd in [
            "",
            "chat:web:1",
            "chat:t1:web:x",
            "other:t1:web:1",
            "chat::web:1",
        ] {
            assert_eq!(thread_of_pane(odd), None, "{odd} was claimed by a thread");
        }
    }

    #[test]
    fn a_label_survives_a_thread_id_from_anywhere() {
        let label = label_for("chat:thread #1 (draft):web:1", 3);
        assert!(label.starts_with("preview-3-"));
        assert!(
            label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '/')),
            "{label}"
        );
    }

    /* -- the history model ------------------------------------------------ */

    #[test]
    fn a_pane_that_has_seen_one_page_can_go_nowhere() {
        let mut history = History::default();
        assert!(!history.can_go_back());
        history.visited();
        assert!(!history.can_go_back());
        assert!(!history.can_go_forward());
    }

    #[test]
    fn going_back_lights_up_forward() {
        let mut history = History::default();
        history.visited();
        history.visited();
        assert!(history.can_go_back());
        assert!(history.went_back());
        assert!(!history.can_go_back());
        assert!(history.can_go_forward());
        assert!(history.went_forward());
        assert!(!history.can_go_forward());
    }

    #[test]
    fn a_new_page_throws_away_what_was_ahead() {
        let mut history = History::default();
        history.visited();
        history.visited();
        history.visited();
        history.went_back();
        assert!(history.can_go_forward());
        history.visited();
        assert!(
            !history.can_go_forward(),
            "the branch that was ahead is gone, which is what a browser does"
        );
        assert!(history.can_go_back());
    }

    #[test]
    fn a_traversal_with_nowhere_to_go_changes_nothing() {
        let mut history = History::default();
        history.visited();
        assert!(!history.went_back());
        assert!(!history.went_forward());
        assert_eq!(history, {
            let mut fresh = History::default();
            fresh.visited();
            fresh
        });
    }

    /* -- what is sent into the page --------------------------------------- */

    #[test]
    fn the_code_a_model_wrote_is_an_argument_and_never_source() {
        // A model asked to evaluate something containing a quote, a backtick
        // and a closing brace used to end the script it was inside.
        let script = begin_script("e1", "document.title + \"`}\"");
        assert!(
            script.contains(r#"eval)("document.title + \"`}\"")"#),
            "{script}"
        );
        assert!(script.contains(r#"var key = "e1";"#));
    }

    #[test]
    fn a_finished_answer_is_taken_rather_than_left_behind() {
        // A slot that is read and not cleared would answer the next call with
        // the previous call's result, which is the worst kind of wrong.
        let script = poll_script("e2");
        assert!(script.contains(r#"delete store["e2"]"#), "{script}");
    }

    #[test]
    fn the_capture_script_installs_itself_only_once_per_document() {
        let script = capture_script();
        assert!(script.contains("if (window.__inertiaConsole) return;"));
        assert!(script.contains("window.__inertiaNetwork = net;"));
        assert!(script.contains(&format!("var LIMIT = {LOG_LIMIT};")));
    }
}

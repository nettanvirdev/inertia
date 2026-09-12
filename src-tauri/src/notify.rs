//! Telling someone who is not looking at the app, and what the app does
//! without a window at all.
//!
//! The reason this exists is work that outlives attention. A turn used to be
//! something you watched: it ran while you sat in front of it, and the only way
//! to miss the end of it was to walk away from a machine you had deliberately
//! left running. A turn that spends four minutes in a tool loop, a routine that
//! fires at nine in the morning, a team of agents - all of them finish while
//! the person is somewhere else, and the useful moment is the one where
//! something failed, finished, or is blocked waiting for a yes.
//!
//! # Two channels, and why not one
//!
//! A toast in the window is right when the window is in front of you: quiet,
//! next to the thing it is about, and it does not steal focus. It is useless
//! when the app is behind a browser, which is exactly when a long turn is
//! running.
//!
//! An operating-system banner is right then, and wrong the rest of the time. A
//! banner for something already visible on screen is the noise that teaches
//! people to switch notifications off, and once they have, the one that
//! mattered never arrives either.
//!
//! So the window's focus decides which channel, and the renderer is told either
//! way: `src/lib/notify.js` draws every notice as a toast, including the ones
//! that also became a banner, so somebody who was away comes back to a toast
//! still on screen rather than to a banner they already dismissed and no trace
//! of it anywhere in the app.
//!
//! # What is worth saying
//!
//! Almost nothing. A turn that failed, a turn that finished while nobody was
//! watching, and a permission card nobody can see. Not a turn starting, and
//! never a cancellation - cancelling is something the person just did, and
//! telling them they did it is the definition of noise. The bar is "would this
//! change what you do next".
//!
//! # Why the seams
//!
//! Everything above is a decision, and decisions are testable. Raising a
//! banner, reading a window's focus and writing a registry key are not: they
//! need a real window, a notification daemon and a login session. Those three
//! live behind [`Desktop`] and [`LoginItem`], which the app fills in at startup
//! and the tests fill in with fakes. Constructing an `AppState` or an
//! `AppHandle` in a test here would take the whole lib-test binary down with
//! `STATUS_ENTRYPOINT_NOT_FOUND` on Windows, so nothing in this file's tests
//! goes near either.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use inertia_store::layout::Document;
use inertia_store::Layout;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;

/// The channel `src/lib/notify.js` subscribes to. Named for the Electron IPC
/// channel it replaces, because the renderer is the same renderer.
pub const NOTIFY_EVENT: &str = "notify:event";

/// How a notice looks in the window.
///
/// Only the toast has a tone: no platform we run on lets a banner say "this one
/// is bad", and a dial that does nothing is worse than no dial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Problem,
}

impl Tone {
    /// The word the renderer switches on (`variant: "danger"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Tone::Info => "info",
            Tone::Problem => "problem",
        }
    }
}

/// One thing worth telling somebody.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub tone: Tone,
    /// What the notice is about, so a future click can open it. Carried
    /// verbatim to the window; nothing in this file reads it.
    pub meta: Value,
}

impl Notice {
    /// The event the window receives.
    ///
    /// `focused` travels with it because the renderer is told about notices it
    /// did not need a banner for, and the flag is how a later reader of a
    /// notice log can tell the two apart.
    pub fn payload(&self, focused: bool) -> Value {
        json!({
            "title": self.title,
            "body": self.body,
            "tone": self.tone.as_str(),
            "meta": self.meta,
            "at": jiff::Timestamp::now().as_millisecond(),
            "focused": focused,
        })
    }
}

/// How a turn stopped.
///
/// Three cases rather than a boolean, because the interesting one is the middle
/// one: a turn the person cancelled is not a turn that failed, and it is the
/// one ending that must never produce a banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    Finished,
    Failed { error: String },
    Interrupted,
}

/// A turn that has just stopped, as the decision below needs to see it.
#[derive(Debug, Clone, Copy)]
pub struct Finished<'a> {
    /// The agent's display name, when the turn had one.
    pub agent: Option<&'a str>,
    pub ending: &'a Ending,
    /// Is the person looking at us right now? Focused, not merely visible: a
    /// window behind the browser they are reading in is not being looked at,
    /// whatever the compositor thinks.
    pub watching: bool,
    /// What the turn said, for a body that describes this turn rather than
    /// turns in general.
    pub said: &'a str,
    pub thread_id: &'a str,
    pub turn_id: &'a str,
}

/// The longest a body may be before it is cut.
///
/// A banner is one or two lines on every platform we run on; anything past that
/// is silently dropped by the OS, and a sentence that ends mid-word looks like
/// a bug in the app rather than a limit of the shelf it is sitting on.
const MAX_BODY: usize = 180;

/// The first line of something, trimmed and capped.
fn first_line(text: &str, limit: usize) -> String {
    let line = text.trim().lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() <= limit {
        return line.to_string();
    }
    // By characters, not bytes: cutting a multi-byte character in half would
    // panic on the slice, and this text is whatever a model just wrote.
    let mut out: String = line.chars().take(limit).collect();
    out.push('\u{2026}');
    out
}

/// Does this ending deserve interrupting somebody, and what does it say?
///
/// The whole point of this function is that it is the part with the bugs in it,
/// and it takes no window, no app handle and no clock - so it can be tested
/// against every ending in a millisecond.
pub fn for_finished_turn(finish: &Finished<'_>) -> Option<Notice> {
    // Nothing is announced about a turn the person is watching. The reply is on
    // screen, streaming, in front of them.
    if finish.watching {
        return None;
    }
    // Cancelling is something they just did.
    if matches!(finish.ending, Ending::Interrupted) {
        return None;
    }

    let who = finish.agent.map(str::trim).filter(|name| !name.is_empty());
    let meta = json!({ "conversationId": finish.thread_id, "turnId": finish.turn_id });

    match finish.ending {
        Ending::Failed { error } => Some(Notice {
            title: match who {
                Some(name) => format!("{name} failed"),
                None => "Your turn failed".to_string(),
            },
            body: {
                let said = first_line(error, MAX_BODY);
                if said.is_empty() {
                    "It stopped without saying why.".to_string()
                } else {
                    said
                }
            },
            tone: Tone::Problem,
            meta,
        }),
        Ending::Finished => Some(Notice {
            title: match who {
                Some(name) => format!("{name} finished"),
                None => "Your turn finished".to_string(),
            },
            body: {
                let said = first_line(finish.said, MAX_BODY);
                if said.is_empty() {
                    "It finished without saying anything.".to_string()
                } else {
                    said
                }
            },
            tone: Tone::Info,
            meta,
        }),
        // Handled above; repeated here so adding a fourth ending is a compile
        // error rather than a silent banner.
        Ending::Interrupted => None,
    }
}

/// A tool is blocked on a question nobody can see.
///
/// Worth a banner precisely because it is not going to resolve itself: the turn
/// is stopped until somebody clicks, and a person who walked away has no way to
/// know that from the outside. Silent when the window is focused, because the
/// card is already on screen with the buttons on it.
pub fn for_waiting_permission(
    agent: Option<&str>,
    key: &str,
    target: &str,
    watching: bool,
) -> Option<Notice> {
    if watching {
        return None;
    }
    let who = agent
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Inertia");
    let target = first_line(target, MAX_BODY);
    Some(Notice {
        title: format!("{who} is waiting for you"),
        body: if target.is_empty() {
            format!("It needs permission for {key}.")
        } else {
            format!("{key}: {target}")
        },
        tone: Tone::Info,
        meta: json!({ "key": key }),
    })
}

/* -- the seam between a decision and a desktop --------------------------- */

/// Where a notice can go, once something has decided it is worth sending.
///
/// A trait rather than an `AppHandle` so the tests can watch both channels
/// without a window, a compositor or a notification daemon.
pub trait Desktop: Send + Sync + std::fmt::Debug {
    /// Is the person looking at us right now?
    fn watching(&self) -> bool;
    /// Put it in the window. Always called, focused or not.
    fn toast(&self, payload: &Value);
    /// Raise an operating-system banner.
    fn banner(&self, notice: &Notice);
}

/// Which channel a notice actually went out on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Toast,
    Banner,
}

/// Say something once, through whichever channel fits.
///
/// The window is told either way - that is what makes the toast the record -
/// and the banner is added only when nobody is looking.
pub fn announce(desktop: &dyn Desktop, notice: &Notice) -> Shown {
    let focused = desktop.watching();
    desktop.toast(&notice.payload(focused));
    if focused {
        return Shown::Toast;
    }
    desktop.banner(notice);
    Shown::Banner
}

/// Raising a real banner, installed by the app at startup.
///
/// A hook rather than a direct call because the banner is a Tauri plugin: this
/// module stays compilable and testable without one, and the app decides which
/// plugin fills it in. A build that never installs one still toasts, which is
/// exactly the Electron behaviour on a desktop with no notification daemon.
type BannerFn = Box<dyn Fn(&AppHandle, &Notice) + Send + Sync + 'static>;

static BANNER: OnceLock<BannerFn> = OnceLock::new();

/// Teach the app how to raise a banner. Called once, from `setup`.
pub fn set_banner(raise: impl Fn(&AppHandle, &Notice) + Send + Sync + 'static) {
    // `set` rather than a panic on a second call: a duplicate registration is a
    // wiring mistake, not a reason to refuse to start.
    let _ = BANNER.set(Box::new(raise));
}

/// The real desktop: this app's windows, and whatever banner was installed.
#[derive(Debug, Clone)]
pub struct AppDesktop(AppHandle);

impl AppDesktop {
    pub fn new(app: &AppHandle) -> Self {
        Self(app.clone())
    }
}

impl Desktop for AppDesktop {
    fn watching(&self) -> bool {
        self.0
            .webview_windows()
            .values()
            .any(|window| window.is_focused().unwrap_or(false))
    }

    fn toast(&self, payload: &Value) {
        // A failure here means there is no window to tell, which is the same
        // situation as a window that is not listening yet. The banner is the
        // half that matters in that case and it has already gone out.
        let _ = self.0.emit(NOTIFY_EVENT, payload);
    }

    fn banner(&self, notice: &Notice) {
        if let Some(raise) = BANNER.get() {
            raise(&self.0, notice);
        }
    }
}

/// A turn stopped. Say so if it is worth saying.
///
/// Takes the app handle rather than a `Desktop` so the call site in `turn.rs`
/// is one line and knows nothing about focus.
pub fn turn_finished(
    app: &AppHandle,
    agent: Option<&str>,
    ending: &Ending,
    said: &str,
    thread_id: &str,
    turn_id: &str,
) {
    let desktop = AppDesktop::new(app);
    let notice = for_finished_turn(&Finished {
        agent,
        ending,
        watching: desktop.watching(),
        said,
        thread_id,
        turn_id,
    });
    if let Some(notice) = notice {
        announce(&desktop, &notice);
    }
}

/// A permission card is up and nobody is looking at it.
pub fn permission_waiting(app: &AppHandle, agent: Option<&str>, key: &str, target: &str) {
    let desktop = AppDesktop::new(app);
    if let Some(notice) = for_waiting_permission(agent, key, target, desktop.watching()) {
        announce(&desktop, &notice);
    }
}

/* -- what Inertia does without a window ---------------------------------- */

/// Closing the window keeps the app running.
///
/// The default because it is what a person means by closing a window with
/// agents working behind it, and because the alternative silently cancels
/// whatever was mid-turn.
pub const DEFAULT_MINIMISE_TO_TRAY: bool = true;

/// The last value read, so a window's close handler can answer without
/// touching the disk.
///
/// A `close` request is answered synchronously - by the time a read of a JSON
/// file came back the window would already be gone - so the answer has to be in
/// memory before it is asked for. Every read and every write refreshes it.
static MINIMISE: AtomicBool = AtomicBool::new(DEFAULT_MINIMISE_TO_TRAY);

/// What the close handler reads. Never blocks, never fails.
pub fn minimise_to_tray_now() -> bool {
    MINIMISE.load(Ordering::Relaxed)
}

/// Pull our one field out of whatever `settings.app` happens to contain.
///
/// The document is shared with the voice, Composio and working-directory
/// settings, so it is read defensively and written back as a patch. A stored
/// `null`, a hand-edited string, or a file somebody truncated all have to land
/// on the default rather than on "off", which is the one guess that loses work
/// at the one moment - a window closing - where it is expensive.
pub fn normalise(stored: &Value) -> bool {
    stored
        .get("minimiseToTray")
        .and_then(Value::as_bool)
        .unwrap_or(DEFAULT_MINIMISE_TO_TRAY)
}

/// Read the stored value, refreshing the cache on the way past.
pub fn minimise_to_tray(layout: &Layout) -> bool {
    let stored = inertia_store::collections::read_document(layout, Document::App, json!({}));
    let value = normalise(&stored);
    MINIMISE.store(value, Ordering::Relaxed);
    value
}

/// Merge our field into `settings.app`, leaving every other writer's alone.
pub fn save_minimise_to_tray(layout: &Layout, value: bool) -> Result<(), String> {
    let stored = inertia_store::collections::read_document(layout, Document::App, json!({}));
    let mut doc = match stored {
        Value::Object(map) => map,
        // A document that is not an object is a document nobody can patch. The
        // setting is worth more than whatever was in there instead.
        _ => serde_json::Map::new(),
    };
    doc.insert("minimiseToTray".into(), Value::Bool(value));
    inertia_store::collections::write_document(layout, Document::App, &Value::Object(doc))
        .map_err(|e| format!("Could not save the background settings: {e}"))?;
    MINIMISE.store(value, Ordering::Relaxed);
    Ok(())
}

/// Starting with the machine.
///
/// Deliberately not stored anywhere of ours: it is a registry entry on Windows
/// and a login item on macOS, both of which the user can undo without telling
/// us. Reading it back every time means the switch shows what the machine
/// actually does rather than what we last remembered asking for.
pub trait LoginItem: Send + Sync + std::fmt::Debug {
    fn supported(&self) -> bool;
    fn enabled(&self) -> bool;
    fn set(&self, on: bool) -> Result<(), String>;
}

/// What a build with no autostart plugin answers.
///
/// `supported: false` rather than a silent no-op: the pane hides the row
/// entirely, which is honest, where a switch that slides across and means
/// nothing is the worst of the three options.
#[derive(Debug, Clone, Copy)]
pub struct NoLoginItem;

impl LoginItem for NoLoginItem {
    fn supported(&self) -> bool {
        false
    }
    fn enabled(&self) -> bool {
        false
    }
    fn set(&self, _on: bool) -> Result<(), String> {
        Err("This build cannot change what starts at login.".to_string())
    }
}

static LOGIN: OnceLock<Arc<dyn LoginItem>> = OnceLock::new();

/// Teach the app how to read and write the login item. Called once, from
/// `setup`, with something backed by the autostart plugin.
pub fn set_login_item(item: Arc<dyn LoginItem>) {
    let _ = LOGIN.set(item);
}

fn login_item() -> Arc<dyn LoginItem> {
    LOGIN
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(NoLoginItem) as Arc<dyn LoginItem>)
}

/// The platform name the renderer was written against.
///
/// Node's spelling, not Rust's: `GeneralPane.jsx` compares against `"darwin"`
/// to decide whether to offer a tray row at all, and `"macos"` would quietly
/// give every Mac user a switch for a tray that does not exist there.
pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

/// The whole answer `useBackgroundSettings` reads.
///
/// `layout` is optional because there is a real moment - first run, before a
/// folder has been chosen - when there is nowhere to read from. The defaults
/// are the truth then, and a refusal would leave the pane's switches stuck on
/// "loading" with no way out.
pub fn settings(layout: Option<&Layout>, login: &dyn LoginItem) -> Value {
    let minimise = match layout {
        Some(layout) => minimise_to_tray(layout),
        None => DEFAULT_MINIMISE_TO_TRAY,
    };
    json!({
        "minimiseToTray": minimise,
        // Asked of the machine every time, never remembered.
        "launchAtLogin": login.supported() && login.enabled(),
        "launchAtLoginSupported": login.supported(),
        "platform": platform(),
    })
}

/// What the tray icon says when you hover it.
///
/// The tooltip is the only thing a hidden Inertia can say about itself, so it
/// answers the one question somebody hovering has: is it doing anything, or is
/// it just sitting there? A count rather than a spinner, because the person who
/// closed the window wants to know whether reopening it will show them
/// something new.
pub fn tray_tooltip(name: &str, running_turns: usize) -> String {
    let label = match name.trim() {
        "" => "Inertia",
        trimmed => trimmed,
    };
    match running_turns {
        0 => format!("{label} - running in the background"),
        1 => format!("{label} - 1 turn running"),
        many => format!("{label} - {many} turns running"),
    }
}

/* -- commands ------------------------------------------------------------ */

/// The workspace layout, or nothing before a folder has been chosen.
fn layout_of(state: &AppState) -> Option<Layout> {
    state.workspace().ok().map(|ws| ws.layout.clone())
}

/// Both switches, as the machine currently has them.
#[tauri::command]
pub fn background_settings(state: State<'_, AppState>) -> Value {
    settings(layout_of(&state).as_ref(), login_item().as_ref())
}

/// Closing the window hides it, or quits.
#[tauri::command]
pub fn background_set_minimise_to_tray(state: State<'_, AppState>, value: bool) -> Value {
    if let Some(layout) = layout_of(&state) {
        if let Err(e) = save_minimise_to_tray(&layout, value) {
            tracing::warn!(error = %e, "could not save minimiseToTray");
        }
    } else {
        // No folder yet, so nowhere to write. Held in memory rather than queued:
        // a first-run user who has not picked a workspace has also not opened
        // the settings sheet, and the close handler still needs an answer.
        MINIMISE.store(value, Ordering::Relaxed);
    }
    settings(layout_of(&state).as_ref(), login_item().as_ref())
}

/// Start with the machine, or do not.
///
/// The answer is re-read from the OS rather than echoed back, because setting a
/// login item can be refused by policy and a switch that slides across and then
/// silently means nothing is worse than one that takes a moment.
#[tauri::command]
pub fn background_set_launch_at_login(state: State<'_, AppState>, value: bool) -> Value {
    let login = login_item();
    if login.supported() {
        if let Err(e) = login.set(value) {
            tracing::warn!(error = %e, "could not change the login item");
        }
    }
    settings(layout_of(&state).as_ref(), login.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    /// A desktop with no desktop: it records what it was asked to show.
    #[derive(Debug, Default)]
    struct FakeDesktop {
        focused: bool,
        toasts: Mutex<Vec<Value>>,
        banners: Mutex<Vec<Notice>>,
    }

    impl FakeDesktop {
        fn new(focused: bool) -> Self {
            Self {
                focused,
                ..Default::default()
            }
        }
    }

    impl Desktop for FakeDesktop {
        fn watching(&self) -> bool {
            self.focused
        }
        fn toast(&self, payload: &Value) {
            self.toasts.lock().push(payload.clone());
        }
        fn banner(&self, notice: &Notice) {
            self.banners.lock().push(notice.clone());
        }
    }

    fn finished<'a>(ending: &'a Ending, watching: bool, said: &'a str) -> Finished<'a> {
        Finished {
            agent: Some("Doc writer"),
            ending,
            watching,
            said,
            thread_id: "t1",
            turn_id: "r1",
        }
    }

    #[test]
    fn a_turn_the_person_is_watching_says_nothing() {
        // The reply is on screen, streaming, in front of them. A banner about
        // it is the noise that gets notifications switched off for good.
        assert_eq!(
            for_finished_turn(&finished(&Ending::Finished, true, "done")),
            None
        );
        assert_eq!(
            for_finished_turn(&finished(
                &Ending::Failed {
                    error: "no key".into()
                },
                true,
                ""
            )),
            None
        );
    }

    #[test]
    fn cancelling_is_never_announced() {
        // Telling somebody they did the thing they just did.
        assert_eq!(
            for_finished_turn(&finished(&Ending::Interrupted, false, "half a sentence")),
            None
        );
    }

    #[test]
    fn a_failure_names_the_agent_and_says_what_went_wrong() {
        let ending = Ending::Failed {
            error: "The provider refused the key.".into(),
        };
        let notice = for_finished_turn(&finished(&ending, false, "")).expect("a notice");
        assert_eq!(notice.title, "Doc writer failed");
        assert_eq!(notice.body, "The provider refused the key.");
        assert_eq!(notice.tone, Tone::Problem);
        assert_eq!(notice.meta["conversationId"], "t1");
    }

    #[test]
    fn a_turn_with_no_agent_still_gets_a_sentence() {
        let ending = Ending::Finished;
        let notice = for_finished_turn(&Finished {
            agent: None,
            ending: &ending,
            watching: false,
            said: "",
            thread_id: "t1",
            turn_id: "r1",
        })
        .expect("a notice");
        assert_eq!(notice.title, "Your turn finished");
        // Never an empty banner: an OS banner with a blank body reads as a bug.
        assert_eq!(notice.body, "It finished without saying anything.");
    }

    #[test]
    fn the_body_is_one_line_and_fits_on_a_banner() {
        let ending = Ending::Finished;
        let said = format!("  \n{}\nand a second paragraph\n", "x".repeat(400));
        let notice = for_finished_turn(&finished(&ending, false, &said)).expect("a notice");
        assert_eq!(notice.body.chars().count(), MAX_BODY + 1); // the ellipsis
        assert!(!notice.body.contains('\n'));
    }

    #[test]
    fn a_body_is_cut_on_characters_rather_than_bytes() {
        // Whatever a model just wrote, including scripts where one character is
        // three bytes. Slicing by byte would panic here.
        let ending = Ending::Finished;
        let said = "\u{09AC}".repeat(400);
        let notice = for_finished_turn(&finished(&ending, false, &said)).expect("a notice");
        assert_eq!(notice.body.chars().count(), MAX_BODY + 1);
    }

    #[test]
    fn a_waiting_permission_is_only_announced_to_somebody_who_cannot_see_it() {
        assert_eq!(for_waiting_permission(Some("Dev"), "shell", "rm -rf", true), None);
        let notice =
            for_waiting_permission(Some("Dev"), "shell", "rm -rf /tmp/x", false).expect("a notice");
        assert_eq!(notice.title, "Dev is waiting for you");
        assert_eq!(notice.body, "shell: rm -rf /tmp/x");
    }

    #[test]
    fn a_waiting_permission_with_no_target_still_reads_as_a_sentence() {
        let notice = for_waiting_permission(None, "network", "  ", false).expect("a notice");
        assert_eq!(notice.title, "Inertia is waiting for you");
        assert_eq!(notice.body, "It needs permission for network.");
    }

    #[test]
    fn the_window_is_told_whether_or_not_it_is_focused() {
        // The toast is the record. Someone who was away comes back to it still
        // on screen rather than to a banner they already dismissed.
        for focused in [true, false] {
            let desktop = FakeDesktop::new(focused);
            let notice = Notice {
                title: "Your agents finished".into(),
                body: "3 of 3.".into(),
                tone: Tone::Info,
                meta: Value::Null,
            };
            let shown = announce(&desktop, &notice);

            let toasts = desktop.toasts.lock();
            assert_eq!(toasts.len(), 1);
            assert_eq!(toasts[0]["title"], "Your agents finished");
            assert_eq!(toasts[0]["focused"], focused);
            assert_eq!(toasts[0]["tone"], "info");
            assert_eq!(desktop.banners.lock().len(), usize::from(!focused));
            assert_eq!(shown, if focused { Shown::Toast } else { Shown::Banner });
        }
    }

    #[test]
    fn the_tone_reaches_the_toast_because_only_the_toast_has_one() {
        let desktop = FakeDesktop::new(true);
        announce(
            &desktop,
            &Notice {
                title: "Doc writer failed".into(),
                body: "refused".into(),
                tone: Tone::Problem,
                meta: Value::Null,
            },
        );
        assert_eq!(desktop.toasts.lock()[0]["tone"], "problem");
    }

    /* -- the settings ---------------------------------------------------- */

    #[derive(Debug)]
    struct FakeLogin {
        supported: bool,
        on: Mutex<bool>,
    }

    impl LoginItem for FakeLogin {
        fn supported(&self) -> bool {
            self.supported
        }
        fn enabled(&self) -> bool {
            *self.on.lock()
        }
        fn set(&self, on: bool) -> Result<(), String> {
            if !self.supported {
                return Err("not here".into());
            }
            *self.on.lock() = on;
            Ok(())
        }
    }

    /// The cached `minimiseToTray` is process-wide, so the tests that read it
    /// back take turns. Without this they pass alone and fail together, which
    /// is the worst kind of flake to chase.
    static SETTINGS_TURN: Mutex<()> = Mutex::new(());

    fn workspace() -> (tempfile::TempDir, Layout) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        layout.scaffold().expect("a scaffolded workspace");
        (dir, layout)
    }

    #[test]
    fn a_workspace_with_no_setting_keeps_the_app_running() {
        let _turn = SETTINGS_TURN.lock();
        let (_dir, layout) = workspace();
        // The expensive guess is "off": closing the window would cancel
        // whatever was mid-turn.
        assert!(minimise_to_tray(&layout));
    }

    #[test]
    fn the_setting_round_trips_through_the_workspace() {
        let _turn = SETTINGS_TURN.lock();
        let (_dir, layout) = workspace();
        save_minimise_to_tray(&layout, false).expect("saved");
        assert!(!minimise_to_tray(&layout));
        // And is readable by anything else holding the same folder.
        let stored =
            inertia_store::collections::read_document(&layout, Document::App, json!(null));
        assert_eq!(stored["minimiseToTray"], json!(false));

        save_minimise_to_tray(&layout, true).expect("saved");
        assert!(minimise_to_tray(&layout));
    }

    #[test]
    fn saving_leaves_every_other_writer_in_settings_app_alone() {
        // `settings.app` is shared with voice, Composio and the working
        // directory. A whole-document write here would delete all three.
        let _turn = SETTINGS_TURN.lock();
        let (_dir, layout) = workspace();
        inertia_store::collections::write_document(
            &layout,
            Document::App,
            &json!({ "voice": { "voiceId": "abc" }, "workingDirectory": "D:\\work" }),
        )
        .expect("a document with other people's settings in it");

        save_minimise_to_tray(&layout, false).expect("saved");

        let stored = inertia_store::collections::read_document(&layout, Document::App, json!({}));
        assert_eq!(stored["voice"]["voiceId"], "abc");
        assert_eq!(stored["workingDirectory"], "D:\\work");
        assert_eq!(stored["minimiseToTray"], json!(false));
    }

    #[test]
    fn a_document_somebody_truncated_reads_as_a_fresh_install() {
        assert!(normalise(&json!(null)));
        assert!(normalise(&json!("nonsense")));
        assert!(normalise(&json!({ "minimiseToTray": "yes" })));
        assert!(!normalise(&json!({ "minimiseToTray": false })));
    }

    #[test]
    fn the_close_handler_can_answer_without_reading_the_disk() {
        let _turn = SETTINGS_TURN.lock();
        let (_dir, layout) = workspace();
        save_minimise_to_tray(&layout, false).expect("saved");
        assert!(!minimise_to_tray_now());
        save_minimise_to_tray(&layout, true).expect("saved");
        assert!(minimise_to_tray_now());
    }

    #[test]
    fn the_pane_gets_every_field_it_reads() {
        let _turn = SETTINGS_TURN.lock();
        let (_dir, layout) = workspace();
        let login = FakeLogin {
            supported: true,
            on: Mutex::new(true),
        };
        let answer = settings(Some(&layout), &login);
        assert_eq!(answer["minimiseToTray"], json!(true));
        assert_eq!(answer["launchAtLogin"], json!(true));
        assert_eq!(answer["launchAtLoginSupported"], json!(true));
        // Node's spelling: the pane compares it against "darwin".
        assert!(matches!(
            answer["platform"].as_str(),
            Some("win32" | "darwin" | "linux")
        ));
    }

    #[test]
    fn a_machine_that_cannot_do_login_items_says_so_rather_than_lying() {
        let login = FakeLogin {
            supported: false,
            on: Mutex::new(true),
        };
        // No workspace at all: first run, before a folder is chosen.
        let answer = settings(None, &login);
        assert_eq!(answer["launchAtLoginSupported"], json!(false));
        // Not `true` from the stale flag above: unsupported means off.
        assert_eq!(answer["launchAtLogin"], json!(false));
        assert_eq!(answer["minimiseToTray"], json!(DEFAULT_MINIMISE_TO_TRAY));
    }

    #[test]
    fn the_tooltip_answers_the_one_question_somebody_hovering_has() {
        assert_eq!(
            tray_tooltip("Inertia", 0),
            "Inertia - running in the background"
        );
        assert_eq!(tray_tooltip("Inertia", 1), "Inertia - 1 turn running");
        assert_eq!(tray_tooltip("Inertia", 4), "Inertia - 4 turns running");
        assert_eq!(tray_tooltip("   ", 1), "Inertia - 1 turn running");
    }
}

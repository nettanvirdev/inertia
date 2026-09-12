//! Routines, actually running.
//!
//! A routine is a Markdown playbook plus a schedule. The renderer for it is
//! complete - a list, a detail pane, a dialog that writes cron - and until this
//! file there was nothing behind any of it: `window.routineAPI` was absent, so
//! a routine set to run every weekday at nine did nothing at nine, or ever.
//!
//! ## Why this lives in the app process
//!
//! Because the window is not the app. A schedule that only fires while a chat
//! screen is mounted is not a schedule, and the whole point of a routine is
//! that it happens whether or not anyone is looking.
//!
//! ## The folder is still the truth
//!
//! Routines are read from the workspace collection on every tick rather than
//! cached. It is one small directory read every half minute, and it means a
//! routine created, edited, enabled or deleted in the window - or by an agent
//! writing the file directly, which is a thing that happens here - is picked up
//! without anything having to tell the scheduler about it.
//!
//! ## One at a time
//!
//! Runs are serialised. Two routines that both want the same working folder at
//! nine in the morning would otherwise fight over it. Late is fine;
//! interleaved is not.
//!
//! ## A paused agent gets no work
//!
//! Routines belong to agents, and pausing an agent has to mean the whole agent
//! rather than only the chat window. The roster is read on every tick and a
//! paused agent's routines are passed over. Their plans are left alone, so the
//! pause postpones runs rather than cancelling them.
//!
//! ## The two seams, and why they are traits
//!
//! [`Runner`] starts a turn and reports what it did; [`Emitter`] tells the
//! window. Both are traits because the alternative is a scheduler that can only
//! be exercised by running a model inside a Tauri app - which is to say, not
//! exercised. The tests below drive the real `tick` against a temp workspace
//! with a scripted runner, so the part that decides *when* is tested without
//! anything that decides *what*.
//!
//! ## Local time, deliberately
//!
//! "Every weekday at 9" means nine in the morning where the person is, and a
//! scheduler that fires at 09:00 UTC is wrong for almost everybody. So cron
//! fields are matched against local time, which also means a routine can be
//! skipped or repeated across a daylight-saving boundary. That is the same
//! behaviour cron itself has, and the alternative - firing at the wrong hour
//! for half the year - is worse than an edge case twice a year.

use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use inertia_core::tool::{PermissionRequest, Tool, ToolContext, ToolOutcome, ToolSource};
use inertia_core::{Error, Result};
use inertia_store::{Collection, Layout};
use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};

// ── the clock ───────────────────────────────────────────────────────────────

/// How often to look. A minute is the finest a cron field can express, so
/// looking more often would only burn a directory read.
pub const TICK: Duration = Duration::from_secs(30);

/// How long a single run may take before it is stopped. A routine that hangs
/// must not block every other routine forever.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How often to ask whether the turn has ended.
const POLL: Duration = Duration::from_secs(1);

/// Enough to see a pattern, few enough that the record stays a record.
const MAX_HISTORY: usize = 20;

/// The channel the window listens on.
pub const ROUTINE_EVENT: &str = "routine:event";

/// How far ahead to look before giving up. A schedule with no next run inside
/// a year is a schedule with no next run: `0 0 30 2 *` is the classic.
const SEARCH_LIMIT_MINUTES: u32 = 366 * 24 * 60;

// ── reading a schedule ──────────────────────────────────────────────────────

/*
 * The cron parser is written out rather than pulled in.
 *
 * A dependency for this would be reasonable. Writing it is also reasonable, and
 * it is what this codebase does elsewhere: the whole surface used here is five
 * fields, four operators and a search forward in minutes. What a library would
 * add is the parts nobody in this app writes - seconds, years, `L`, `W`, `#` -
 * and a supply-chain entry for something that has to be trusted to fire the
 * user's work on time.
 */

#[derive(Debug, Clone, Copy)]
struct Field {
    name: &'static str,
    min: i64,
    max: i64,
}

const FIELDS: [Field; 5] = [
    Field { name: "minute", min: 0, max: 59 },
    Field { name: "hour", min: 0, max: 23 },
    Field { name: "day", min: 1, max: 31 },
    Field { name: "month", min: 1, max: 12 },
    Field { name: "weekday", min: 0, max: 6 },
];

/// Names, because `0 9 * * mon-fri` is what people write.
fn named(field: &Field, token: &str) -> Option<i64> {
    let key: String = token.trim().to_lowercase().chars().take(3).collect();
    let found = match field.name {
        "weekday" => match key.as_str() {
            "sun" => Some(0),
            "mon" => Some(1),
            "tue" => Some(2),
            "wed" => Some(3),
            "thu" => Some(4),
            "fri" => Some(5),
            "sat" => Some(6),
            _ => None,
        },
        "month" => match key.as_str() {
            "jan" => Some(1),
            "feb" => Some(2),
            "mar" => Some(3),
            "apr" => Some(4),
            "may" => Some(5),
            "jun" => Some(6),
            "jul" => Some(7),
            "aug" => Some(8),
            "sep" => Some(9),
            "oct" => Some(10),
            "nov" => Some(11),
            "dec" => Some(12),
            _ => None,
        },
        _ => None,
    };
    found.or_else(|| number(token))
}

/// A token as a whole number, or nothing.
///
/// Blank counts as zero, which is what the other shell's `Number("")` does, so
/// a half-typed `9-` fails as "out of range" there and here rather than as two
/// different sentences.
fn number(token: &str) -> Option<i64> {
    let text = token.trim();
    if text.is_empty() {
        return Some(0);
    }
    text.parse::<i64>().ok()
}

/// One cron field, as the set of values that match it.
///
/// A set rather than a predicate so an unparseable field fails here, once, at
/// the moment someone types it - rather than as a routine that quietly never
/// fires and gives no one anything to look at.
fn parse_field(source: &str, field: &Field) -> std::result::Result<BTreeSet<i64>, String> {
    let mut values = BTreeSet::new();

    for part in source.split(',') {
        let (range, step_text) = match part.split_once('/') {
            Some((range, step)) => (range, Some(step)),
            None => (part, None),
        };
        let step = match step_text {
            None => 1,
            Some(text) => match number(text) {
                Some(step) if step >= 1 => step,
                _ => return Err(format!("Bad step in \"{part}\"")),
            },
        };

        let (from, to) = if range == "*" {
            (Some(field.min), Some(field.max))
        } else if let Some((a, b)) = range.split_once('-') {
            (named(field, a), named(field, b))
        } else {
            let from = named(field, range);
            let to = if step_text.is_none() { from } else { Some(field.max) };
            (from, to)
        };

        let (Some(mut from), Some(mut to)) = (from, to) else {
            return Err(format!("Bad {} in \"{part}\"", field.name));
        };
        // Sunday is both 0 and 7 in every cron anyone has used.
        if field.name == "weekday" {
            if from == 7 {
                from = 0;
            }
            if to == 7 {
                to = 0;
            }
        }
        if from < field.min || to > field.max || to < from {
            return Err(format!("{} out of range in \"{part}\"", field.name));
        }

        let mut value = from;
        while value <= to {
            values.insert(value);
            value += step;
        }
    }

    if values.is_empty() {
        return Err(format!("Nothing matches \"{source}\""));
    }
    Ok(values)
}

/// A cron expression, as five sets. Fails with a readable reason.
pub fn parse_cron(expression: &str) -> std::result::Result<Vec<BTreeSet<i64>>, String> {
    let parts: Vec<&str> = expression.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!(
            "A schedule needs five fields - minute hour day month weekday - and \"{expression}\" has {}.",
            parts.len()
        ));
    }
    FIELDS
        .iter()
        .zip(parts.iter())
        .map(|(field, part)| parse_field(part, field))
        .collect()
}

/// An instant, as ISO 8601. The one thing a once-only schedule carries.
///
/// A time with no zone on it is read as local, which is what the window's own
/// `Date.parse` does with the same string - a routine written as
/// `2026-09-05T09:30` means half past nine where the person is.
pub fn parse_moment(expression: &str) -> std::result::Result<Timestamp, String> {
    let text = expression.trim();
    if let Ok(stamp) = text.parse::<Timestamp>() {
        return Ok(stamp);
    }
    if let Ok(civil) = text.parse::<jiff::civil::DateTime>() {
        if let Ok(zoned) = civil.to_zoned(TimeZone::system()) {
            return Ok(zoned.timestamp());
        }
    }
    Err(format!(
        "\"{expression}\" is not a time. Write it as ISO 8601, like 2026-09-05T09:30:00Z."
    ))
}

/// An ISO-8601 duration, in milliseconds.
///
/// Only the part of the grammar this app can produce: the interval picker
/// writes PT15M, PT1H, P1D and the like. Months and years are not accepted at
/// all, because "every month" as a fixed number of milliseconds is a lie and
/// the cron kind is the honest way to say it.
pub fn parse_duration(expression: &str) -> std::result::Result<i64, String> {
    let bad = || format!("\"{expression}\" is not an interval. Write it like PT30M, PT2H or P1D.");
    let text = expression.trim().to_uppercase();
    let Some(body) = text.strip_prefix('P') else {
        return Err(bad());
    };

    let (date, time) = match body.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (body, None),
    };

    // Parsed by hand rather than with a regex so each unit is checked against
    // the ones this grammar allows, in the half it is allowed in: `M` before
    // the `T` is months, which is exactly what must not be guessed at, and `M`
    // after it is minutes.
    let mut parts = [0i64; 5]; // weeks, days, hours, minutes, seconds
    let mut seen = false;

    let mut take = |section: &str, units: &[(char, usize)]| -> std::result::Result<(), String> {
        let mut digits = String::new();
        for ch in section.chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
                continue;
            }
            let Some((_, slot)) = units.iter().find(|(unit, _)| *unit == ch) else {
                return Err(bad());
            };
            let Ok(value) = digits.parse::<i64>() else {
                return Err(bad());
            };
            parts[*slot] = value;
            digits.clear();
            seen = true;
        }
        // Trailing digits with no unit after them: `P1`.
        if digits.is_empty() {
            Ok(())
        } else {
            Err(bad())
        }
    };

    take(date, &[('W', 0), ('D', 1)])?;
    if let Some(time) = time {
        take(time, &[('H', 2), ('M', 3), ('S', 4)])?;
    }
    if !seen {
        return Err(bad());
    }

    let ms = ((parts[0] * 7 + parts[1]) * 24 * 3600 + parts[2] * 3600 + parts[3] * 60 + parts[4])
        * 1000;
    if ms <= 0 {
        return Err("An interval has to be longer than nothing.".to_string());
    }
    // A routine that runs every ten seconds is a runaway bill, not a schedule.
    if ms < 60_000 {
        return Err("The shortest interval is one minute.".to_string());
    }
    Ok(ms)
}

/// The first minute at or after `from` that this cron matches.
///
/// Searched a minute at a time. That is not clever, and it does not need to be:
/// the loop runs at most a few thousand times for any schedule a person writes,
/// once every tick, and the arithmetic that would replace it is where every
/// subtle scheduler bug lives.
///
/// The day-of-month and weekday fields are OR-ed when both are restricted,
/// which is cron's own rule and surprises everyone exactly once: `0 0 1 * mon`
/// means the first of the month AND every Monday, not Mondays that fall on the
/// first.
pub fn next_cron(
    expression: &str,
    from: &Zoned,
) -> std::result::Result<Option<Zoned>, String> {
    let sets = parse_cron(expression)?;
    let written: Vec<&str> = expression.split_whitespace().collect();
    let restricted_day = written.get(2).copied() != Some("*");
    let restricted_weekday = written.get(4).copied() != Some("*");

    // Start at the top of the next minute: a schedule is never "now", or a tick
    // that lands on the matching minute would fire the same run repeatedly.
    let mut cursor = from
        .with()
        .second(0)
        .subsec_nanosecond(0)
        .build()
        .map_err(|error| error.to_string())?
        .checked_add(1.minute())
        .map_err(|error| error.to_string())?;

    for _ in 0..SEARCH_LIMIT_MINUTES {
        let day_matches = sets[2].contains(&i64::from(cursor.day()));
        let weekday_matches = sets[4].contains(&sunday_zero(cursor.weekday()));
        let date_matches = if restricted_day && restricted_weekday {
            day_matches || weekday_matches
        } else {
            day_matches && weekday_matches
        };

        if sets[0].contains(&i64::from(cursor.minute()))
            && sets[1].contains(&i64::from(cursor.hour()))
            && sets[3].contains(&i64::from(cursor.month()))
            && date_matches
        {
            return Ok(Some(cursor));
        }
        cursor = cursor
            .checked_add(1.minute())
            .map_err(|error| error.to_string())?;
    }
    Ok(None)
}

/// Sunday is 0, as every cron field writes it.
fn sunday_zero(weekday: Weekday) -> i64 {
    i64::from(weekday.to_sunday_zero_offset())
}

/// When a routine is next due, and why it is not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Next {
    /// An ISO 8601 instant, or nothing.
    pub at: Option<String>,
    /// Why there is no next run, when the reason is worth showing.
    pub reason: Option<String>,
}

impl Next {
    fn nothing() -> Self {
        Self::default()
    }

    fn at(stamp: Timestamp) -> Self {
        Self {
            at: Some(iso(stamp)),
            reason: None,
        }
    }

    fn because(reason: String) -> Self {
        Self {
            at: None,
            reason: Some(reason),
        }
    }

    /// The shape `lib/routines.js` unwraps: `{ at, reason }`, both nullable.
    pub fn to_json(&self) -> Value {
        json!({ "at": self.at, "reason": self.reason })
    }
}

/// When this routine should next run.
///
/// Nothing means "not on a clock" - a manual or triggered routine, a disabled
/// one, or a schedule that cannot be read. The caller treats all four the same
/// way, which is why the reason comes back separately rather than as an error:
/// one bad expression should not stop the other routines from being scheduled.
pub fn next_run(routine: &Value, now: Timestamp) -> Next {
    if enabled_is_false(routine) {
        return Next::nothing();
    }

    let kind = schedule_text(routine, "kind").unwrap_or_else(|| "manual".to_string());
    let expression = schedule_text(routine, "expression").unwrap_or_default();

    match kind.as_str() {
        "manual" | "trigger" => Next::nothing(),

        // A time, and one run at it. Ran already - the last run is at or after
        // the time - means nothing is next; not yet means the time, even if it
        // is in the past, so a run that was due while the app was closed still
        // happens once rather than never.
        "once" => match parse_moment(&expression) {
            Err(reason) => Next::because(reason),
            Ok(at) => match last_run_at(routine) {
                Some(last) if last >= at => Next::nothing(),
                _ => Next::at(at),
            },
        },

        "cron" => {
            let from = now.to_zoned(TimeZone::system());
            match next_cron(&expression, &from) {
                Err(reason) => Next::because(reason),
                Ok(Some(at)) => Next::at(at.timestamp()),
                Ok(None) => Next::because("That schedule never comes round.".to_string()),
            }
        }

        "interval" => match parse_duration(&expression) {
            Err(reason) => Next::because(reason),
            Ok(every) => {
                // Measured from the last run, not from now, or an app that is
                // restarted every hour would never reach the end of a two-hour
                // interval.
                let base = last_run_at(routine)
                    .map(|last| last.as_millisecond())
                    .unwrap_or_else(|| now.as_millisecond());
                let at = (base + every).max(now.as_millisecond() + 1000);
                match Timestamp::from_millisecond(at) {
                    Ok(at) => Next::at(at),
                    Err(error) => Next::because(error.to_string()),
                }
            }
        },

        other => Next::because(format!("Unknown schedule kind \"{other}\".")),
    }
}

/// Is this routine due?
///
/// Compared against the stored `nextRunAt` when there is one, so a routine that
/// was due while the app was closed runs once on the next launch rather than
/// being silently skipped. A missed schedule that stays missed is the failure
/// people notice; a run that happens twenty minutes late is one they forgive.
pub fn is_due(routine: &Value, now: Timestamp) -> bool {
    if enabled_is_false(routine) {
        return false;
    }
    let kind = schedule_text(routine, "kind").unwrap_or_else(|| "manual".to_string());
    if kind == "manual" || kind == "trigger" {
        return false;
    }
    // Already going. Two copies of one routine racing each other is worse than
    // a late run, and this is the only thing standing between us and that.
    if last_run_text(routine, "status").as_deref() == Some("running") {
        return false;
    }

    match schedule_text(routine, "nextRunAt").and_then(|text| parse_moment(&text).ok()) {
        Some(planned) => planned <= now,
        // No plan recorded yet - a routine someone just enabled. It is not due;
        // the scheduler will write a nextRunAt for it on this same tick.
        None => false,
    }
}

/// A schedule in words, for a UI that should not have to parse cron.
pub fn describe(routine: &Value) -> String {
    let kind = schedule_text(routine, "kind").unwrap_or_else(|| "manual".to_string());
    let expression = schedule_text(routine, "expression").unwrap_or_default();

    match kind.as_str() {
        "manual" => "Runs when you ask".to_string(),
        "trigger" => format!(
            "Runs on {}",
            if expression.is_empty() { "a trigger" } else { expression.as_str() }
        ),
        "once" => match parse_moment(&expression) {
            Err(reason) => reason,
            Ok(at) => {
                let ran = matches!(last_run_at(routine), Some(last) if last >= at);
                format!(
                    "{} {}",
                    if ran { "Ran once, at" } else { "Once, at" },
                    local(at)
                )
            }
        },
        "interval" => match parse_duration(&expression) {
            Err(reason) => reason,
            Ok(ms) => {
                let minutes = (ms as f64 / 60_000.0).round() as i64;
                if minutes % 1440 == 0 {
                    format!("Every {} day(s)", minutes / 1440)
                } else if minutes % 60 == 0 {
                    format!("Every {} hour(s)", minutes / 60)
                } else {
                    format!("Every {minutes} minute(s)")
                }
            }
        },
        _ => match parse_cron(&expression) {
            Err(reason) => reason,
            Ok(_) => format!("On the schedule {expression}"),
        },
    }
}

// ── small readers over a record ─────────────────────────────────────────────

/*
 * Records are `serde_json::Value` and are read field by field, never mirrored
 * into a struct. A typed mirror of a routine would drop whatever the window
 * added to the record since - and a routine round-tripped through one would
 * lose it on the first patch the scheduler writes.
 */

fn text_of(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|found| !found.is_empty())
        .map(str::to_string)
}

fn schedule_text(routine: &Value, key: &str) -> Option<String> {
    routine.get("schedule").and_then(|s| text_of(s, key))
}

fn last_run_text(routine: &Value, key: &str) -> Option<String> {
    routine.get("lastRun").and_then(|run| text_of(run, key))
}

fn last_run_at(routine: &Value) -> Option<Timestamp> {
    last_run_text(routine, "at").and_then(|text| parse_moment(&text).ok())
}

/// `enabled: false` and nothing else. A record with no `enabled` field at all
/// is enabled, which is what the window assumes when it writes one.
fn enabled_is_false(routine: &Value) -> bool {
    routine.get("enabled").and_then(Value::as_bool) == Some(false)
}

fn id_of(routine: &Value) -> String {
    text_of(routine, "id").unwrap_or_default()
}

/// The record's own timestamp format: UTC, milliseconds, `Z` - byte for byte
/// what `new Date().toISOString()` writes, so a folder's timestamps stay
/// comparable as plain strings no matter which side wrote them.
fn iso(stamp: Timestamp) -> String {
    stamp.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The same instant where the person is, for a sentence they will read.
fn local(stamp: Timestamp) -> String {
    stamp
        .to_zoned(TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string()
}

fn now() -> Timestamp {
    Timestamp::now()
}

// ── the seams ───────────────────────────────────────────────────────────────

/// What the scheduler needs to start a turn.
///
/// A trait rather than a call into `commands::agent_send`, because the thing
/// worth testing here is which routine fires and when - and a test that had to
/// reach a model to find out would not be run.
#[async_trait]
pub trait Runner: Send + Sync + std::fmt::Debug {
    /// Start the turn. `Err` is a run that never began.
    async fn start(&self, run: &Run) -> std::result::Result<Started, String>;

    /// How that turn ended, or `None` while it is still going.
    fn outcome(&self, turn_id: &str, thread_id: &str) -> Option<Outcome>;

    /// Stop a turn that has outstayed its welcome.
    fn cancel(&self, turn_id: &str);
}

/// Everything a turn started by a routine is told.
#[derive(Debug, Clone)]
pub struct Run {
    /// The conversation the run writes into, which is `routine-<id>`: the
    /// routine it came from is carried by that rather than repeated here.
    pub thread_id: String,
    pub agent_id: Option<String>,
    /// The playbook, as the user's message.
    pub text: String,
    /// The mode the routine's turn holds. `autonomous` unless the record says.
    pub mode: String,
    /// How much it asks. `auto` unless the record says, because unattended
    /// "ask" means "refuse".
    pub approval: String,
}

/// A turn that started.
#[derive(Debug, Clone)]
pub struct Started {
    pub turn_id: String,
    /// The id of the reply the turn is streaming into, when the path that
    /// started it mints one. A window that hears it can attach to the run and
    /// watch the tool calls land; without it the run is still recorded, and the
    /// folder watcher redraws the row when it ends.
    pub message_id: Option<String>,
}

/// How a run ended, in the three words the routines screen colours by.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// `success`, `warning` or `error`.
    pub status: String,
    pub summary: String,
}

/// Where a routine event goes.
pub trait Emitter: Send + Sync + std::fmt::Debug {
    fn emit(&self, payload: Value);
}

/// Which workspace is open, asked on every tick rather than held.
///
/// The folder can be chosen after launch and closed again while the app runs,
/// which is exactly why the supervisor next door asks the same question on
/// every pass instead of capturing an answer at startup.
pub trait Workspaces: Send + Sync + std::fmt::Debug {
    fn layout(&self) -> Option<Layout>;
}

/// One workspace, for a test that already holds the folder. The app resolves
/// the workspace on every pass instead, so nothing outside a test builds this.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct OneWorkspace(pub Layout);

#[cfg(test)]
impl Workspaces for OneWorkspace {
    fn layout(&self) -> Option<Layout> {
        Some(self.0.clone())
    }
}

// ── the scheduler ───────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Scheduler {
    workspaces: Arc<dyn Workspaces>,
    runner: Arc<dyn Runner>,
    events: Arc<dyn Emitter>,
    /// The routines this process is running right now.
    ///
    /// The record on disk says "running" too, but the record can say that about
    /// a run the last process started and never finished. This set is the one
    /// that cannot lie.
    in_flight: Mutex<HashSet<String>>,
    busy: AtomicBool,
    settled: AtomicBool,
    poll: Duration,
    timeout: Duration,
}

impl Scheduler {
    pub fn new(
        workspaces: Arc<dyn Workspaces>,
        runner: Arc<dyn Runner>,
        events: Arc<dyn Emitter>,
    ) -> Self {
        Self {
            workspaces,
            runner,
            events,
            in_flight: Mutex::new(HashSet::new()),
            busy: AtomicBool::new(false),
            settled: AtomicBool::new(false),
            poll: POLL,
            timeout: RUN_TIMEOUT,
        }
    }

    /// Shorter waits, for a test that must not take fifteen minutes to find out
    /// that a hung run is stopped.
    #[cfg(test)]
    pub fn with_waits(mut self, poll: Duration, timeout: Duration) -> Self {
        self.poll = poll;
        self.timeout = timeout;
        self
    }

    /// One pass over the routines.
    ///
    /// Two jobs, in this order. First every enabled routine that has no plan
    /// gets one, which is what makes a routine created a moment ago
    /// schedulable without the window having to compute anything. Then anything
    /// due is run.
    pub async fn tick(&self) -> Value {
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return json!({ "ran": [], "skipped": "busy" });
        }
        let report = self.pass().await;
        self.busy.store(false, Ordering::SeqCst);
        report
    }

    async fn pass(&self) -> Value {
        // No workspace yet is the normal state at launch, not a problem.
        let Some(layout) = self.workspaces.layout() else {
            return json!({ "ran": [], "skipped": "no workspace" });
        };

        if !self.settled.swap(true, Ordering::SeqCst) {
            self.settle_stale(&layout);
        }

        let mut routines = inertia_store::collections::list(&layout, Collection::Routines);
        let at = now();

        for routine in routines.iter_mut() {
            if enabled_is_false(routine) {
                continue;
            }
            if schedule_text(routine, "nextRunAt")
                .and_then(|text| parse_moment(&text).ok())
                .is_some()
            {
                continue;
            }
            let Some(planned) = next_run(routine, at).at else {
                continue;
            };
            let schedule = with_field(routine.get("schedule"), "nextRunAt", json!(planned));
            self.patch(&layout, &id_of(routine), json!({ "schedule": schedule.clone() }));
            if let Value::Object(map) = routine {
                map.insert("schedule".into(), schedule);
            }
        }

        // A routine belongs to an agent, and a paused agent is not to start work
        // of any kind. The plan written above is left where it is rather than
        // pushed forward: a routine that came due during a pause runs shortly
        // after the agent is resumed, the same as one that came due while the
        // app was closed. Skipping it silently forever would make pausing an
        // agent quietly delete its schedule.
        let paused = paused_agents(&layout);
        let due: Vec<Value> = routines
            .into_iter()
            .filter(|routine| {
                is_due(routine, at)
                    && !text_of(routine, "agentId")
                        .map(|agent| paused.contains(&agent))
                        .unwrap_or(false)
            })
            .collect();

        let mut ran = Vec::new();
        for routine in due {
            let id = id_of(&routine);
            let run = self.run_once(&layout, &routine, false).await;
            ran.push(json!({ "id": id, "run": run }));
        }
        json!({ "ran": ran })
    }

    /// Run one now, whatever its schedule says.
    ///
    /// The button in the routines screen. A manual run does not disturb the
    /// clock: the next scheduled run is recomputed from it for an interval
    /// routine, because "every two hours" should mean two hours from the last
    /// time it actually ran, and left alone for a cron one, because "weekdays
    /// at nine" does not move because someone pressed a button.
    pub async fn run_now(&self, routine_id: &str) -> std::result::Result<Value, String> {
        let Some(layout) = self.workspaces.layout() else {
            return Err("Choose a workspace folder first.".to_string());
        };
        let routine = inertia_store::collections::get(&layout, Collection::Routines, routine_id)
            .ok()
            .flatten()
            .ok_or_else(|| "That routine no longer exists.".to_string())?;

        // Running in this process, not merely recorded as running: a record left
        // by a process that died mid-run must not block the button forever.
        if self.in_flight.lock().contains(routine_id) {
            return Err("That routine is already running.".to_string());
        }

        // Pressing Run now on a paused agent's routine is refused with the
        // reason rather than obeyed. The button is not a way round the pause,
        // and refusing here - before a run record is written - keeps the history
        // from filling with failures that only say what the person could have
        // been told first.
        if let Some(agent_id) = text_of(&routine, "agentId") {
            if let Ok(agent) = crate::agents::resolve(&layout, &agent_id) {
                if agent.is_paused() {
                    return Err(format!(
                        "{} is paused, so it will not start new work. \
                         Open it under Agents and press Resume to let it run again.",
                        agent.name
                    ));
                }
            }
        }

        Ok(self.run_once(&layout, &routine, true).await)
    }

    /// Fire a routine.
    ///
    /// The playbook is sent as the user's message, which is the honest
    /// translation: a routine is a thing someone would otherwise have typed, on
    /// a schedule. The turn runs against the routine's own agent, with that
    /// agent's tools, permissions and working folder, because a routine that
    /// ran with different powers to its agent would be a second permission
    /// model nobody asked for.
    ///
    /// Note what this does NOT do: it does not ask. There may be nobody there,
    /// so a routine whose playbook needs a decision stops and says so in its
    /// summary rather than hanging until someone happens to look.
    async fn run_once(&self, layout: &Layout, routine: &Value, manual: bool) -> Value {
        let id = id_of(routine);
        let started_at = iso(now());
        let began = std::time::Instant::now();
        self.in_flight.lock().insert(id.clone());

        self.patch(
            layout,
            &id,
            json!({
                "lastRun": {
                    "at": started_at,
                    "status": "running",
                    "durationMs": 0,
                    "summary": if manual { "Started by hand." } else { "Started on schedule." },
                }
            }),
        );

        let thread_id = thread_id_for(&id);
        // Written before the turn starts, so a window that hears "started" can
        // open the conversation and find the run it is about to watch.
        let thread = self.open_thread(layout, routine, &thread_id, &started_at);

        let request = Run {
            thread_id: thread_id.clone(),
            agent_id: text_of(routine, "agentId"),
            text: playbook(routine),
            mode: text_of(routine, "mode").unwrap_or_else(|| "autonomous".to_string()),
            approval: text_of(routine, "approval").unwrap_or_else(|| "auto".to_string()),
        };

        let (status, summary) = match self.runner.start(&request).await {
            Err(reason) => {
                // Only sent when the turn never started; one that started
                // announces itself below, and a window told twice would attach
                // twice.
                self.events.emit(json!({
                    "type": "routine:started",
                    "routineId": id,
                    "at": started_at,
                }));
                ("error".to_string(), reason)
            }
            Ok(started) => {
                self.events.emit(json!({
                    "type": "routine:started",
                    "routineId": id,
                    "at": started_at,
                    "turnId": started.turn_id,
                    "threadId": thread_id,
                    "messageId": started.message_id,
                    "agentId": text_of(routine, "agentId"),
                    "thread": thread,
                    "messages": [],
                }));
                let finished = self.wait_for(&started.turn_id, &thread_id).await;
                (finished.status, finished.summary)
            }
        };

        self.in_flight.lock().remove(&id);

        let run = json!({
            "at": started_at,
            "status": status,
            "durationMs": began.elapsed().as_millis() as u64,
            "summary": summary,
            "threadId": thread_id,
        });

        // Gone while it ran. Writing the record back would resurrect a routine
        // the user deleted, which is worse than losing one run's history.
        let Ok(Some(current)) = inertia_store::collections::get(layout, Collection::Routines, &id)
        else {
            return run;
        };

        let mut history = vec![run.clone()];
        if let Some(Value::Array(previous)) = current.get("runHistory") {
            history.extend(previous.iter().take(MAX_HISTORY - 1).cloned());
        }

        let mut settled = current.clone();
        if let Value::Object(map) = &mut settled {
            map.insert("lastRun".into(), run.clone());
        }
        let schedule = with_field(
            current.get("schedule"),
            "nextRunAt",
            json!(next_run(&settled, now()).at),
        );

        self.patch(
            layout,
            &id,
            json!({ "lastRun": run, "runHistory": history, "schedule": schedule }),
        );
        self.events.emit(json!({
            "type": "routine:finished",
            "routineId": id,
            "run": run,
            // The schedule, because the run just moved it. Persisted a line
            // above and never told to anybody, so the row kept pointing at the
            // run that had already happened - "next run 2 hours ago" - and the
            // "Next up" tile with it, until the app restarted.
            "schedule": schedule,
        }));
        run
    }

    /// Wait for a turn to end, and read what happened out of it.
    ///
    /// Polled rather than subscribed because the turn's events go to the
    /// window, and a routine may be running with no window at all.
    async fn wait_for(&self, turn_id: &str, thread_id: &str) -> Outcome {
        let deadline = std::time::Instant::now() + self.timeout;
        while std::time::Instant::now() < deadline {
            if let Some(outcome) = self.runner.outcome(turn_id, thread_id) {
                return outcome;
            }
            tokio::time::sleep(self.poll).await;
        }

        // Stopped, not merely abandoned. Giving up on waiting while the turn
        // went on running leaves the tools working and the reply streaming into
        // a record nobody will read again - and the next run refuses to start
        // because the routine is, truthfully, still running.
        self.runner.cancel(turn_id);
        Outcome {
            status: "warning".to_string(),
            summary: format!("Stopped after {} minutes.", self.timeout.as_secs() / 60),
        }
    }

    /// The conversation a routine's runs are written into.
    ///
    /// Created before the turn and never overwritten wholesale: the window owns
    /// what else is on a thread, and a routine that ran twice must not lose the
    /// first run's messages to the second run's record.
    fn open_thread(
        &self,
        layout: &Layout,
        routine: &Value,
        thread_id: &str,
        started_at: &str,
    ) -> Value {
        let existing = inertia_store::collections::get(layout, Collection::Threads, thread_id)
            .ok()
            .flatten();
        let mut thread = match existing {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        };

        thread.insert("id".into(), json!(thread_id));
        thread.insert("routineId".into(), json!(id_of(routine)));
        thread.insert("agentId".into(), json!(text_of(routine, "agentId")));
        thread.insert(
            "title".into(),
            json!(format!(
                "Routine: {}",
                text_of(routine, "name").unwrap_or_else(|| id_of(routine))
            )),
        );
        thread.insert("draft".into(), json!(false));
        thread.insert("updatedAt".into(), json!(started_at));
        thread.insert(
            "preview".into(),
            json!(clip(
                &text_of(routine, "markdown")
                    .or_else(|| text_of(routine, "description"))
                    .unwrap_or_default(),
                120
            )),
        );

        let record = Value::Object(thread);
        match inertia_store::collections::put(layout, Collection::Threads, record.clone()) {
            Ok(written) => written,
            // A thread that cannot be written is not worth taking the run down
            // for: the run itself is recorded on the routine either way.
            Err(error) => {
                tracing::warn!(thread = %thread_id, error = %error, "could not open the routine's conversation");
                record
            }
        }
    }

    /// Routines that say they are running and are not.
    ///
    /// Written by a process that stopped before its run ended - the app was quit
    /// or crashed mid-routine. Nothing is going to finish those, so a record
    /// that still says "running" on the way in is the same lie as a reply stuck
    /// on "streaming": Run now refuses, the row spins forever, and the only way
    /// out was the JSON file. Settled once, on the first tick, to the truth.
    fn settle_stale(&self, layout: &Layout) -> usize {
        let mut settled = 0;
        for routine in inertia_store::collections::list(layout, Collection::Routines) {
            let id = id_of(&routine);
            if last_run_text(&routine, "status").as_deref() != Some("running")
                || self.in_flight.lock().contains(&id)
            {
                continue;
            }
            let at = last_run_text(&routine, "at").unwrap_or_else(|| iso(now()));
            let run = json!({
                "at": at,
                "status": "warning",
                "durationMs": 0,
                "summary": "Interrupted: the app closed while this was running.",
            });

            let mut history: Vec<Value> = match routine.get("runHistory") {
                Some(Value::Array(previous)) => previous.clone(),
                _ => Vec::new(),
            };
            // The interrupted run's own row, rewritten rather than doubled.
            let same = history
                .first()
                .map(|first| {
                    text_of(first, "at") == text_of(&run, "at")
                        && text_of(first, "status").as_deref() == Some("running")
                })
                .unwrap_or(false);
            if same {
                history.remove(0);
            }
            history.insert(0, run.clone());
            history.truncate(MAX_HISTORY);

            self.patch(layout, &id, json!({ "lastRun": run, "runHistory": history }));
            settled += 1;
        }
        settled
    }

    /// A change to one routine, where failing is not worth a stack trace.
    ///
    /// A routine deleted mid-run, or a folder that went away. Neither is worth
    /// taking the scheduler down for.
    fn patch(&self, layout: &Layout, id: &str, changes: Value) {
        if let Err(error) = inertia_store::collections::patch(layout, Collection::Routines, id, changes)
        {
            tracing::debug!(routine = %id, error = %error, "the routine could not be updated");
        }
    }
}

/// The conversation a routine's runs are written into.
pub fn thread_id_for(routine_id: &str) -> String {
    format!("routine-{routine_id}")
}

/// The message a routine sends.
///
/// The playbook verbatim, with a few lines of context in front of it. The
/// context is not decoration: a model that does not know it is running
/// unattended will ask a clarifying question and wait, and there is nobody
/// there to answer it.
pub fn playbook(routine: &Value) -> String {
    let name = text_of(routine, "name").unwrap_or_else(|| id_of(routine));
    let approval = text_of(routine, "approval").unwrap_or_else(|| "auto".to_string());
    let body = text_of(routine, "markdown")
        .or_else(|| text_of(routine, "description"))
        .unwrap_or_else(|| "(This routine has no playbook.)".to_string());

    [
        format!("You are running the routine \"{name}\" on a schedule, unattended."),
        "Nobody is watching, so do not ask questions - make a reasonable choice, say".to_string(),
        "what you chose, and finish. End with one short paragraph summarising what".to_string(),
        "you did and anything a person should look at.".to_string(),
        // Told which it is, so a held-back routine plans around the refusals
        // instead of discovering them one call at a time.
        match approval.as_str() {
            "auto" => "Tool calls go through without anyone approving them, so you are the check: prefer the reversible version of an action and say plainly what you did.".to_string(),
            "edits" => "File changes in the working folder go through; a command or anything outside it that a rule would ask about is refused, since nobody can approve it. Plan around that.".to_string(),
            _ => "This routine is set to ask, and nobody can answer: any tool call a rule would ask about is refused. Do what can be done without one and say what could not.".to_string(),
        },
        String::new(),
        body,
    ]
    .join("\n")
}

/// The agents that are not to be given work.
///
/// Read on the tick alongside the routines, for the same reason the routines
/// are: pausing an agent in the window writes one file, and a scheduler holding
/// a cached roster would keep firing that agent's routines until something
/// happened to reload it.
fn paused_agents(layout: &Layout) -> HashSet<String> {
    crate::agents::list(layout)
        .into_iter()
        .filter(crate::agents::Agent::is_paused)
        .map(|agent| agent.id)
        .collect()
}

/// One field changed on a schedule object, with everything else kept.
///
/// The schedule carries fields this side has no opinion about - `humanLabel`,
/// whatever the dialog adds next - and a `schedule` written from scratch would
/// delete them on the first run.
fn with_field(current: Option<&Value>, key: &str, value: Value) -> Value {
    let mut map = match current {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    map.insert(key.into(), value);
    Value::Object(map)
}

fn clip(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

// ── the app's own wiring ────────────────────────────────────────────────────

/// The workspace, as the app knows it.
#[derive(Debug)]
struct AppWorkspaces(tauri::AppHandle);

impl Workspaces for AppWorkspaces {
    fn layout(&self) -> Option<Layout> {
        use tauri::Manager;
        let state = self.0.try_state::<crate::state::AppState>()?;
        state.workspace().ok().map(|w| w.layout.clone())
    }
}

/// The window, when there is one.
#[derive(Debug)]
struct Window(tauri::AppHandle);

impl Emitter for Window {
    fn emit(&self, payload: Value) {
        use tauri::Emitter as _;
        // A tick that lands while the window is closing has nobody to tell.
        let _ = self.0.emit(ROUTINE_EVENT, payload);
    }
}

/// Starting a routine's turn the way the composer starts one.
#[derive(Debug)]
struct AppRunner(tauri::AppHandle);

#[async_trait]
impl Runner for AppRunner {
    async fn start(&self, run: &Run) -> std::result::Result<Started, String> {
        use tauri::Manager;
        let started = crate::commands::agent_send(
            self.0.clone(),
            self.0.state::<crate::state::AppState>(),
            run.thread_id.clone(),
            run.text.clone(),
            None,
            run.agent_id.clone(),
            // The routine's own pills. Written on its record, and until now
            // dropped on the floor: a routine set to Plan ran with full tools.
            Some(run.mode.clone()),
            Some(run.approval.clone()),
        )
        .await?;
        Ok(Started {
            turn_id: started.turn_id,
            // That path mints the reply's id as it writes it, so there is
            // nothing to hand a window that wants to watch. See the note on
            // `Started::message_id`.
            message_id: None,
        })
    }

    fn outcome(&self, turn_id: &str, thread_id: &str) -> Option<Outcome> {
        use tauri::Manager;
        let state = self.0.try_state::<crate::state::AppState>()?;
        if state.running_ids().iter().any(|id| id == turn_id) {
            return None;
        }

        // The turn is over, so the transcript on disk is what it did. Read the
        // reply rather than asking the loop, because the loop has already
        // forgotten: the record is the one view of a turn that does not depend
        // on anyone having watched it.
        let workspace = state.workspace().ok()?;
        let reply = workspace
            .conversations
            .read_messages(thread_id, &[])
            .into_iter()
            .rev()
            .find(|message| message.role != "user");

        Some(match reply {
            None => Outcome {
                status: "error".to_string(),
                summary: "The turn left nothing behind.".to_string(),
            },
            Some(message) => Outcome {
                status: if message.error.is_some() {
                    "error".to_string()
                } else if message.stopped {
                    "warning".to_string()
                } else {
                    "success".to_string()
                },
                summary: message
                    .error
                    .clone()
                    .unwrap_or_else(|| summarise(&message.content)),
            },
        })
    }

    fn cancel(&self, turn_id: &str) {
        use tauri::Manager;
        if let Some(state) = self.0.try_state::<crate::state::AppState>() {
            state.stop(turn_id);
        }
    }
}

/// The last thing the agent said, trimmed to something a row can hold.
///
/// The agent was told to finish with a summary paragraph, so the last paragraph
/// of the reply is the summary - and when it did not, the last paragraph is
/// still the most useful sentence available.
fn summarise(reply: &str) -> String {
    let text = reply.trim();
    if text.is_empty() {
        return "Finished with nothing to report.".to_string();
    }
    let last = text
        .split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .last()
        .unwrap_or(text);
    if last.chars().count() > 280 {
        format!("{}...", last.chars().take(277).collect::<String>())
    } else {
        last.to_string()
    }
}

/// Starts keeping time. Runs for the life of the app.
///
/// A tick loop rather than a timer per routine, for the reasons the MCP
/// supervisor next door settled on: the workspace may be chosen after launch,
/// the folder is edited behind the scheduler's back, and a laptop that sleeps
/// through nine o'clock must still run the nine o'clock routine when it wakes.
/// The first pass happens immediately, so a routine that came due while the app
/// was closed runs shortly after launch rather than at the next round minute.
pub fn start(app: &tauri::AppHandle) {
    use tauri::Manager;

    let scheduler = Arc::new(Scheduler::new(
        Arc::new(AppWorkspaces(app.clone())),
        Arc::new(AppRunner(app.clone())),
        Arc::new(Window(app.clone())),
    ));
    // Managed so the commands below reach the same scheduler the loop is
    // ticking: Run now must not start a second copy of a routine this process
    // is already running, and that is only true if there is one `in_flight`.
    app.manage(scheduler.clone());

    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            scheduler.tick().await;
        }
    });
}

fn scheduler(app: &tauri::AppHandle) -> std::result::Result<Arc<Scheduler>, String> {
    use tauri::Manager;
    app.try_state::<Arc<Scheduler>>()
        .map(|state| state.inner().clone())
        .ok_or_else(|| "The scheduler is not running.".to_string())
}

// ── commands ────────────────────────────────────────────────────────────────

/// Run one now, whatever its schedule says. Resolves when the turn is done.
#[tauri::command]
pub async fn routine_run(app: tauri::AppHandle, id: String) -> std::result::Result<Value, String> {
    scheduler(&app)?.run_now(&id).await
}

/// When each routine is next due, computed rather than read.
///
/// The stored `nextRunAt` is what the scheduler fires on, but a screen showing
/// a schedule the user is in the middle of editing should show what they are
/// about to get - not what was planned before they started typing. So one
/// routine handed in is answered about itself, and no argument is answered
/// about the whole folder.
#[tauri::command]
pub fn routine_next(
    app: tauri::AppHandle,
    routine: Option<Value>,
) -> std::result::Result<Value, String> {
    let at = now();
    if let Some(routine @ Value::Object(_)) = routine {
        return Ok(next_run(&routine, at).to_json());
    }

    use tauri::Manager;
    let Some(state) = app.try_state::<crate::state::AppState>() else {
        return Ok(json!({}));
    };
    let Ok(workspace) = state.workspace() else {
        return Ok(json!({}));
    };
    let mut all = Map::new();
    for routine in inertia_store::collections::list(&workspace.layout, Collection::Routines) {
        all.insert(id_of(&routine), next_run(&routine, at).to_json());
    }
    Ok(Value::Object(all))
}

/// What a schedule says, in words. The screen should not parse cron.
#[tauri::command]
pub fn routine_describe(routine: Option<Value>) -> String {
    describe(&routine.unwrap_or(Value::Null))
}

/// A pass on demand, so a test - or an impatient user - does not wait.
#[tauri::command]
pub async fn routine_tick(app: tauri::AppHandle) -> std::result::Result<Value, String> {
    Ok(scheduler(&app)?.tick().await)
}

// ── the `later` tool ────────────────────────────────────────────────────────

/*
 * Come back to this later.
 *
 * "Check whether the deploy finished in twenty minutes." "Look at the inbox
 * again at nine." A turn cannot wait that long - a conversation left open on a
 * timer is a window somebody has to keep open - but a routine can, and a
 * routine that runs once at a time is exactly a note to self. So this writes
 * one: the brief, the agent, the time, and a schedule of kind `once`. The
 * scheduler above picks it up on its next tick whether or not the window is
 * still open, runs it as its own conversation under Routines, and it never
 * fires again.
 */

/// Between now and a year out: a typo in the minutes should not book 2037.
const MAX_AHEAD_MS: i64 = 366 * 24 * 60 * 60 * 1000;

/// The tool, for a turn that has a workspace and an agent.
///
/// Built per turn rather than in the builtins crate for the reason the group
/// tools are: `ToolContext` carries the session but not the seat, and a tool
/// that had to guess which agent called it would schedule the future run as
/// somebody else.
pub struct LaterTool {
    layout: Layout,
    agent_id: Option<String>,
    /// The agent's display name, for the sentence the model reads back.
    agent: Option<String>,
    /// The conversation's own approval dial, carried onto the routine.
    approval: Option<String>,
    /// Told that a routine was written.
    ///
    /// This tool's own output tells the person the routine "will run in its own
    /// conversation under Routines" - and it did not appear there, because
    /// nothing announced the write. The Routines screen reads the folder once,
    /// at launch, and is told about changes afterwards. `inertia_save` does
    /// announce for the same collection; this one never did.
    emit: crate::inertia_tools::Emitter,
}

// By hand: an emitter is a closure and has no `Debug`.
impl std::fmt::Debug for LaterTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaterTool")
            .field("agent_id", &self.agent_id)
            .finish_non_exhaustive()
    }
}

pub fn later_tool(
    layout: Layout,
    agent_id: Option<String>,
    agent: Option<String>,
    approval: Option<String>,
    emit: crate::inertia_tools::Emitter,
) -> Arc<dyn Tool> {
    Arc::new(LaterTool {
        layout,
        agent_id,
        agent,
        approval,
        emit,
    })
}

/// When the future run happens, from whichever of the two arguments was given.
fn when(args: &Value, at_ms: i64) -> std::result::Result<Timestamp, String> {
    if let Some(text) = args
        .get("at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let at = parse_moment(text)?;
        if at.as_millisecond() <= at_ms {
            return Err(format!("{text} has already passed. Give a time in the future."));
        }
        if at.as_millisecond() - at_ms > MAX_AHEAD_MS {
            return Err("That is more than a year away. Give a nearer time.".to_string());
        }
        return Ok(at);
    }

    let minutes = args.get("in_minutes").and_then(Value::as_f64);
    let Some(minutes) = minutes.filter(|minutes| minutes.is_finite() && *minutes >= 1.0) else {
        return Err(
            "Say when: `in_minutes` (a number, at least 1) or `at` (an ISO 8601 time).".to_string(),
        );
    };
    let ahead = (minutes.round() as i64) * 60_000;
    if ahead > MAX_AHEAD_MS {
        return Err("That is more than a year away. Give a nearer time.".to_string());
    }
    Timestamp::from_millisecond(at_ms + ahead).map_err(|error| error.to_string())
}

#[async_trait]
impl Tool for LaterTool {
    fn id(&self) -> &str {
        "later"
    }

    fn description(&self) -> &str {
        "Come back to something later, after this turn has ended. Writes a one-off routine\n\
         that runs the brief you give it at the time you give it, in a conversation of its\n\
         own under Routines, whether or not this conversation is still open.\n\
         \n\
         Use it for work that has to wait on the world - a deploy finishing, a reply\n\
         arriving, a build that takes an hour - rather than sitting in `wait` for it. The\n\
         brief is all the future run will see, so write it as instructions to someone who\n\
         has not read this conversation: what to check, what to do about each outcome, and\n\
         what to tell the person. Say in your reply that you have scheduled it and when."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "brief": {
                    "type": "string",
                    "description": "The full instructions for the future run. It sees nothing else."
                },
                "in_minutes": {
                    "type": "number",
                    "description": "How many minutes from now. Use this or `at`."
                },
                "at": {
                    "type": "string",
                    "description": "An ISO 8601 time to run at, like 2026-09-05T09:30:00Z. Use this or `in_minutes`."
                },
                "name": {
                    "type": "string",
                    "description": "A short name for the Routines list. Defaults to the first words of the brief."
                }
            },
            "required": ["brief"],
            "additionalProperties": false
        })
    }

    fn source(&self) -> ToolSource {
        ToolSource::Builtin
    }

    fn permission(&self, _args: &Value) -> PermissionRequest {
        /*
         * The key the Permissions screen actually writes.
         *
         * A deliberate divergence from the other shell, which asked under
         * `inertia_save` - a tool id, not a key. No settings row writes that,
         * so the rule a person sets under "Set up Inertia" governed every
         * `inertia_*` tool and quietly did not govern this one, and there was
         * no row anywhere that did. `routine` stays the target, so a rule
         * written about scheduling in particular still means what it said.
         */
        PermissionRequest::new(crate::inertia_tools::PERMISSION_KEY, "routine")
    }

    fn render(&self, args: &Value) -> Option<String> {
        match args.get("in_minutes").and_then(Value::as_f64) {
            Some(minutes) => Some(format!("In {minutes} min")),
            None => args
                .get("at")
                .and_then(Value::as_str)
                .map(|at| format!("At {at}")),
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutcome> {
        let brief = args
            .get("brief")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if brief.is_empty() {
            return Err(Error::Other(
                "The brief is empty. Say what the future run should do.".into(),
            ));
        }
        let Some(agent_id) = self.agent_id.clone() else {
            return Err(Error::Other(
                "This turn has no agent to run the routine as.".into(),
            ));
        };

        let at = when(&args, now().as_millisecond()).map_err(Error::Other)?;
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "Later: {}",
                    brief.split_whitespace().take(6).collect::<Vec<_>>().join(" ")
                )
            });

        let record = inertia_store::collections::put(
            &self.layout,
            Collection::Routines,
            json!({
                "name": name,
                "agentId": agent_id,
                "description": format!("Scheduled from a conversation, to run once at {}.", local(at)),
                "markdown": brief,
                "icon": "Clock",
                "enabled": true,
                "tags": ["later"],
                // The dials of the conversation that wrote it. Unattended,
                // "ask" is "refuse", so a routine scheduled from a chat set to
                // ask will refuse the same calls the chat would have asked
                // about - which is what the person chose.
                "mode": "autonomous",
                "approval": self.approval.clone().unwrap_or_else(|| "auto".to_string()),
                "schedule": {
                    "kind": "once",
                    "expression": iso(at),
                    "humanLabel": format!("Once, at {}", local(at)),
                    "nextRunAt": iso(at),
                },
            }),
        )
        .map_err(|error| Error::Other(format!("The routine could not be written: {error}")))?;

        let id = id_of(&record);
        (self.emit)(json!({ "collection": "routines", "id": id, "op": "put" }));
        let written = text_of(&record, "name").unwrap_or_default();
        let as_who = self.agent.clone().unwrap_or(agent_id);

        Ok(ToolOutcome {
            title: Some(format!("Scheduled for {}", local(at))),
            output: [
                format!(
                    "Scheduled \"{written}\" (routine {id}) to run once at {}, as {as_who}.",
                    local(at)
                ),
                "It will run in its own conversation under Routines, whether or not this one is open."
                    .to_string(),
                "Tell the person it is scheduled and when. If they change their mind, the routine can be"
                    .to_string(),
                "deleted from the Routines screen or with inertia_remove.".to_string(),
            ]
            .join("\n"),
            metadata: Some(json!({ "routineId": id, "at": iso(at) })),
            images: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inertia_core::id::{SessionId, ToolCallId};
    use inertia_mock::MockGate;

    /*
     * A scheduler is the one piece of an app nobody can test by using it: being
     * wrong about "the first Monday of the month" costs a month to notice. So
     * the arithmetic is pure and every rule that surprised somebody once is
     * pinned here, including cron's own surprises - the same vectors the other
     * shell pins, because the two must agree about when a routine fires.
     *
     * Local-time constructors on purpose: the scheduler matches local time, so
     * a test written in UTC would be testing something that does not ship.
     */

    fn at(y: i16, m: i8, d: i8, h: i8, min: i8) -> Zoned {
        jiff::civil::datetime(y, m, d, h, min, 0, 0)
            .to_zoned(TimeZone::system())
            .expect("a local time")
    }

    fn stamp(y: i16, m: i8, d: i8, h: i8, min: i8) -> Timestamp {
        at(y, m, d, h, min).timestamp()
    }

    fn routine(schedule: Value) -> Value {
        json!({ "id": "r1", "name": "R", "enabled": true, "schedule": schedule })
    }

    // -- reading a cron expression -----------------------------------------

    #[test]
    fn a_schedule_takes_the_five_fields_and_nothing_else() {
        assert!(parse_cron("0 9 * * *").is_ok());
        assert!(parse_cron("0 9 * *").unwrap_err().contains("five fields"));
        assert!(parse_cron("0 9 * * * *").unwrap_err().contains("five fields"));
    }

    #[test]
    fn it_reads_lists_ranges_steps_and_names() {
        let values = |expression: &str, field: usize| -> Vec<i64> {
            parse_cron(expression).expect("a schedule")[field]
                .iter()
                .copied()
                .collect()
        };
        assert_eq!(values("0,30 * * * *", 0), vec![0, 30]);
        assert_eq!(values("0 9-11 * * *", 1), vec![9, 10, 11]);
        assert_eq!(values("*/20 * * * *", 0), vec![0, 20, 40]);
        assert_eq!(values("0 0 * * mon-fri", 4), vec![1, 2, 3, 4, 5]);
        assert_eq!(values("0 0 1 jan *", 3), vec![1]);
    }

    #[test]
    fn seven_is_sunday_because_every_cron_anyone_has_used_takes_it() {
        let sunday: Vec<i64> = parse_cron("0 0 * * 7").expect("a schedule")[4]
            .iter()
            .copied()
            .collect();
        assert_eq!(sunday, vec![0]);
    }

    #[test]
    fn a_field_it_cannot_read_is_refused_rather_than_matching_nothing_forever() {
        // The failure this prevents is the quiet one: a routine that never
        // fires and gives nobody anything to look at.
        assert!(parse_cron("0 99 * * *").unwrap_err().contains("hour"));
        assert!(parse_cron("0 9 * * xyz").unwrap_err().contains("weekday"));
        assert!(parse_cron("*/0 * * * *")
            .unwrap_err()
            .to_lowercase()
            .contains("step"));
    }

    // -- the next time a cron comes round ----------------------------------

    #[test]
    fn a_weekday_schedule_skips_the_weekend() {
        // Saturday morning; the next weekday nine is Monday.
        let next = next_cron("0 9 * * 1-5", &at(2026, 9, 5, 10, 0))
            .expect("a schedule")
            .expect("a next run");
        assert_eq!(next.weekday(), Weekday::Monday);
        assert_eq!(next.hour(), 9);
    }

    #[test]
    fn the_next_run_is_never_now_so_a_tick_on_the_matching_minute_fires_once() {
        let now = at(2026, 9, 3, 9, 0);
        let next = next_cron("0 9 * * *", &now)
            .expect("a schedule")
            .expect("a next run");
        assert!(next.timestamp() > now.timestamp());
    }

    #[test]
    fn day_of_month_is_ored_with_weekday_which_is_crons_own_rule() {
        // `0 0 1 * mon` means the 1st AND every Monday - not Mondays that fall
        // on the 1st. It surprises everyone exactly once.
        let next = next_cron("0 0 1 * mon", &at(2026, 9, 3, 10, 0))
            .expect("a schedule")
            .expect("a next run");
        // 7 September 2026 is a Monday, and it comes before 1 October.
        assert_eq!(next.day(), 7);
    }

    #[test]
    fn a_schedule_that_never_comes_round_gives_up() {
        assert_eq!(
            next_cron("0 0 30 2 *", &at(2026, 9, 3, 0, 0)).expect("a schedule"),
            None
        );
    }

    // -- reading an interval -----------------------------------------------

    #[test]
    fn it_takes_the_durations_the_picker_writes() {
        assert_eq!(parse_duration("PT30M"), Ok(1_800_000));
        assert_eq!(parse_duration("PT2H"), Ok(7_200_000));
        assert_eq!(parse_duration("P1D"), Ok(86_400_000));
        assert_eq!(parse_duration("P1W"), Ok(604_800_000));
    }

    #[test]
    fn anything_shorter_than_a_minute_is_refused() {
        // Not pedantry: the tick is every thirty seconds, and a routine that
        // wants to run every ten is a bill rather than a schedule.
        assert!(parse_duration("PT10S").unwrap_err().contains("one minute"));
    }

    #[test]
    fn months_and_years_are_refused_rather_than_guessed_at() {
        assert!(parse_duration("P1M").unwrap_err().contains("not an interval"));
        assert!(parse_duration("every hour")
            .unwrap_err()
            .contains("not an interval"));
    }

    // -- when a routine is next due ----------------------------------------

    #[test]
    fn a_routine_that_is_not_on_a_clock_is_never_next() {
        assert_eq!(next_run(&routine(json!({ "kind": "manual" })), now()).at, None);
        assert_eq!(
            next_run(
                &routine(json!({ "kind": "trigger", "expression": "push" })),
                now()
            )
            .at,
            None
        );
    }

    #[test]
    fn a_disabled_routine_is_never_next_whatever_its_schedule_says() {
        let mut record = routine(json!({ "kind": "cron", "expression": "0 9 * * *" }));
        record["enabled"] = json!(false);
        assert_eq!(next_run(&record, now()).at, None);
    }

    #[test]
    fn an_interval_is_measured_from_the_last_run_not_from_now() {
        // An app restarted every hour would otherwise never reach the end of a
        // two-hour interval.
        let mut record = routine(json!({ "kind": "interval", "expression": "PT2H" }));
        record["lastRun"] = json!({ "at": iso(stamp(2026, 9, 3, 11, 0)) });
        let next = next_run(&record, stamp(2026, 9, 3, 12, 0))
            .at
            .expect("a next run");
        let next = parse_moment(&next).expect("a time").to_zoned(TimeZone::system());
        assert_eq!(next.hour(), 13);
    }

    #[test]
    fn a_schedule_it_cannot_read_comes_back_as_a_reason_rather_than_an_error() {
        // One bad expression must not stop the other routines being scheduled.
        let next = next_run(
            &routine(json!({ "kind": "cron", "expression": "nonsense" })),
            now(),
        );
        assert_eq!(next.at, None);
        assert!(next.reason.expect("a reason").contains("five fields"));
    }

    // -- deciding to run ---------------------------------------------------

    fn planned(next_run_at: Timestamp) -> Value {
        routine(json!({
            "kind": "cron",
            "expression": "0 9 * * *",
            "nextRunAt": iso(next_run_at),
        }))
    }

    #[test]
    fn a_routine_that_came_due_while_the_app_was_closed_runs() {
        // A missed schedule that stays missed is the failure people notice.
        // Late is the one they forgive.
        assert!(is_due(
            &planned(stamp(2026, 9, 3, 9, 0)),
            stamp(2026, 9, 3, 11, 30)
        ));
    }

    #[test]
    fn one_that_is_not_due_yet_does_not_run() {
        assert!(!is_due(
            &planned(stamp(2026, 9, 3, 9, 0)),
            stamp(2026, 9, 3, 8, 59)
        ));
    }

    #[test]
    fn a_second_copy_of_a_running_routine_is_not_started() {
        let mut record = planned(stamp(2026, 9, 3, 9, 0));
        record["lastRun"] = json!({ "status": "running" });
        assert!(!is_due(&record, stamp(2026, 9, 3, 11, 0)));
    }

    #[test]
    fn one_with_no_plan_recorded_yet_does_not_run() {
        // The tick writes it a plan first; running on the same pass would fire
        // a routine the moment it was created.
        let record = routine(json!({ "kind": "cron", "expression": "0 9 * * *" }));
        assert!(!is_due(&record, stamp(2026, 9, 3, 11, 0)));
    }

    #[test]
    fn a_manual_routine_never_runs_on_its_own() {
        let record = routine(json!({ "kind": "manual", "nextRunAt": "2020-01-01T00:00:00Z" }));
        assert!(!is_due(&record, now()));
    }

    // -- saying what a schedule means --------------------------------------

    #[test]
    fn a_schedule_is_described_in_words_a_person_would_use() {
        assert!(describe(&routine(json!({ "kind": "manual" }))).contains("when you ask"));
        assert!(
            describe(&routine(json!({ "kind": "interval", "expression": "PT2H" })))
                .contains("2 hour")
        );
        assert!(
            describe(&routine(json!({ "kind": "interval", "expression": "P1D" }))).contains("1 day")
        );
    }

    #[test]
    fn a_broken_schedule_says_what_is_wrong_rather_than_pretending_it_is_fine() {
        assert!(describe(&routine(json!({ "kind": "cron", "expression": "0 99 * * *" })))
            .contains("hour"));
    }

    // -- a schedule that runs once -----------------------------------------

    #[test]
    fn a_once_schedule_is_due_at_its_time_and_stays_due_once_it_has_passed_unrun() {
        let moment = "2026-09-05T09:30:00.000Z";
        let before = parse_moment("2026-09-05T09:00:00.000Z").expect("a time");
        let after = parse_moment("2026-09-05T10:00:00.000Z").expect("a time");
        let record = json!({
            "enabled": true,
            "schedule": { "kind": "once", "expression": moment },
        });

        assert_eq!(next_run(&record, before).at.as_deref(), Some(moment));
        // Due while the app was closed: it still runs, once, late. Never is the
        // failure people notice.
        assert_eq!(next_run(&record, after).at.as_deref(), Some(moment));
    }

    #[test]
    fn a_once_schedule_has_nothing_next_once_it_has_run() {
        let after = parse_moment("2026-09-05T10:00:00.000Z").expect("a time");
        let record = json!({
            "enabled": true,
            "schedule": { "kind": "once", "expression": "2026-09-05T09:30:00.000Z" },
            "lastRun": { "at": "2026-09-05T09:30:04.000Z", "status": "success" },
        });
        assert_eq!(next_run(&record, after), Next::nothing());
        assert!(describe(&record).starts_with("Ran once, at "));
    }

    #[test]
    fn a_time_it_cannot_read_is_refused_in_words_that_say_what_a_time_looks_like() {
        let before = parse_moment("2026-09-05T09:00:00.000Z").expect("a time");
        let record = json!({
            "enabled": true,
            "schedule": { "kind": "once", "expression": "tomorrow-ish" },
        });
        let next = next_run(&record, before);
        assert_eq!(next.at, None);
        assert!(next.reason.expect("a reason").contains("ISO 8601"));
    }

    // -- the scheduler itself ----------------------------------------------

    /// A turn that ends the moment it starts, so a tick can be tested without
    /// a model, a provider or a window.
    #[derive(Debug, Default)]
    struct ScriptedRunner {
        started: Mutex<Vec<Run>>,
        /// What every turn reports. `None` means "still running", which is how
        /// the timeout path is exercised.
        outcome: Option<Outcome>,
        cancelled: Mutex<Vec<String>>,
    }

    impl ScriptedRunner {
        fn finishing() -> Self {
            Self {
                outcome: Some(Outcome {
                    status: "success".into(),
                    summary: "Nothing to report.".into(),
                }),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl Runner for ScriptedRunner {
        async fn start(&self, run: &Run) -> std::result::Result<Started, String> {
            self.started.lock().push(run.clone());
            Ok(Started {
                turn_id: format!("turn-{}", run.thread_id),
                message_id: Some("msg-1".into()),
            })
        }

        fn outcome(&self, _turn_id: &str, _thread_id: &str) -> Option<Outcome> {
            self.outcome.clone()
        }

        fn cancel(&self, turn_id: &str) {
            self.cancelled.lock().push(turn_id.to_string());
        }
    }

    #[derive(Debug, Default)]
    struct Recorder(Mutex<Vec<Value>>);

    impl Emitter for Recorder {
        fn emit(&self, payload: Value) {
            self.0.lock().push(payload);
        }
    }

    struct Bench {
        _dir: tempfile::TempDir,
        layout: Layout,
        scheduler: Scheduler,
        runner: Arc<ScriptedRunner>,
        events: Arc<Recorder>,
    }

    fn bench(runner: ScriptedRunner) -> Bench {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        let runner = Arc::new(runner);
        let events = Arc::new(Recorder::default());
        let scheduler = Scheduler::new(
            Arc::new(OneWorkspace(layout.clone())),
            runner.clone(),
            events.clone(),
        )
        .with_waits(Duration::from_millis(5), Duration::from_millis(60));
        Bench {
            _dir: dir,
            layout,
            scheduler,
            runner,
            events,
        }
    }

    fn write(layout: &Layout, collection: Collection, record: Value) -> Value {
        inertia_store::collections::put(layout, collection, record).expect("the record")
    }

    fn read(layout: &Layout, id: &str) -> Value {
        inertia_store::collections::get(layout, Collection::Routines, id)
            .expect("the folder")
            .expect("the routine")
    }

    #[tokio::test]
    async fn a_due_routine_is_picked_up_by_one_tick_and_marked_as_having_run() {
        let bench = bench(ScriptedRunner::finishing());
        write(
            &bench.layout,
            Collection::Routines,
            json!({
                "id": "morning",
                "name": "Morning sweep",
                "agentId": "agent-1",
                "enabled": true,
                "markdown": "Look at the inbox.",
                "schedule": {
                    "kind": "cron",
                    "expression": "0 9 * * *",
                    "humanLabel": "Weekdays at nine",
                    // Due two hours ago: the app was closed at nine.
                    "nextRunAt": iso(Timestamp::from_millisecond(
                        now().as_millisecond() - 2 * 3600 * 1000
                    ).expect("a time")),
                },
            }),
        );

        bench.scheduler.tick().await;

        let after = read(&bench.layout, "morning");
        assert_eq!(last_run_text(&after, "status").as_deref(), Some("success"));
        assert_eq!(
            last_run_text(&after, "summary").as_deref(),
            Some("Nothing to report.")
        );
        assert_eq!(
            after["runHistory"].as_array().map(Vec::len),
            Some(1),
            "the run is in the history: {after}"
        );
        // The plan moved on rather than staying in the past, or the routine
        // would fire again on the very next tick.
        let planned = schedule_text(&after, "nextRunAt").expect("a plan");
        assert!(
            parse_moment(&planned).expect("a time") > now(),
            "next run should be in the future, was {planned}"
        );
        // And the schedule kept everything else it carried.
        assert_eq!(
            schedule_text(&after, "humanLabel").as_deref(),
            Some("Weekdays at nine")
        );

        // The turn was started as the routine's own agent, with the playbook as
        // the message and its own conversation.
        let started = bench.runner.started.lock().clone();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].agent_id.as_deref(), Some("agent-1"));
        assert_eq!(started[0].thread_id, "routine-morning");
        assert!(started[0].text.contains("Look at the inbox."));
        assert!(started[0].text.contains("unattended"));

        // The conversation exists and says whose it is.
        let thread = inertia_store::collections::get(
            &bench.layout,
            Collection::Threads,
            "routine-morning",
        )
        .expect("the folder")
        .expect("the thread");
        assert_eq!(text_of(&thread, "title").as_deref(), Some("Routine: Morning sweep"));
        assert_eq!(text_of(&thread, "routineId").as_deref(), Some("morning"));

        let events = bench.events.0.lock().clone();
        let kinds: Vec<&str> = events
            .iter()
            .filter_map(|event| event.get("type").and_then(Value::as_str))
            .collect();
        assert_eq!(kinds, vec!["routine:started", "routine:finished"]);
        assert_eq!(events[0]["turnId"], json!("turn-routine-morning"));
        assert_eq!(events[1]["run"]["status"], json!("success"));
    }

    #[tokio::test]
    async fn a_routine_with_no_plan_gets_one_and_is_not_run_on_that_same_pass() {
        let bench = bench(ScriptedRunner::finishing());
        write(
            &bench.layout,
            Collection::Routines,
            json!({
                "id": "hourly",
                "name": "Hourly",
                "enabled": true,
                "schedule": { "kind": "interval", "expression": "PT1H" },
            }),
        );

        bench.scheduler.tick().await;

        let after = read(&bench.layout, "hourly");
        assert!(schedule_text(&after, "nextRunAt").is_some());
        assert!(after.get("lastRun").is_none(), "it should not have run: {after}");
        assert!(bench.runner.started.lock().is_empty());
    }

    #[tokio::test]
    async fn a_paused_agents_routine_is_passed_over_and_keeps_its_plan() {
        let bench = bench(ScriptedRunner::finishing());
        write(
            &bench.layout,
            Collection::Agents,
            json!({ "id": "agent-1", "name": "Atlas", "status": "paused" }),
        );
        let due = iso(
            Timestamp::from_millisecond(now().as_millisecond() - 60_000).expect("a time"),
        );
        write(
            &bench.layout,
            Collection::Routines,
            json!({
                "id": "swept",
                "name": "Swept",
                "agentId": "agent-1",
                "enabled": true,
                "schedule": { "kind": "cron", "expression": "0 9 * * *", "nextRunAt": due },
            }),
        );

        bench.scheduler.tick().await;

        assert!(bench.runner.started.lock().is_empty());
        // The plan is left where it is: the pause postpones the run rather than
        // quietly deleting the schedule.
        let after = read(&bench.layout, "swept");
        assert_eq!(schedule_text(&after, "nextRunAt").as_deref(), Some(due.as_str()));
    }

    #[tokio::test]
    async fn run_now_on_a_paused_agents_routine_is_refused_with_the_reason() {
        let bench = bench(ScriptedRunner::finishing());
        write(
            &bench.layout,
            Collection::Agents,
            json!({ "id": "agent-1", "name": "Atlas", "status": "paused" }),
        );
        write(
            &bench.layout,
            Collection::Routines,
            json!({ "id": "held", "name": "Held", "agentId": "agent-1", "enabled": true,
                    "schedule": { "kind": "manual" } }),
        );

        let refusal = bench.scheduler.run_now("held").await.unwrap_err();
        assert!(refusal.starts_with("Atlas is paused"), "{refusal}");
        assert!(bench.runner.started.lock().is_empty());
        // Nothing was written, so the history does not fill with failures
        // nobody performed.
        assert!(read(&bench.layout, "held").get("lastRun").is_none());
    }

    #[tokio::test]
    async fn run_now_runs_a_manual_routine_and_answers_with_the_run() {
        let bench = bench(ScriptedRunner::finishing());
        write(
            &bench.layout,
            Collection::Routines,
            json!({ "id": "byhand", "name": "By hand", "enabled": true,
                    "markdown": "Tidy up.", "schedule": { "kind": "manual" } }),
        );

        let run = bench.scheduler.run_now("byhand").await.expect("a run");
        assert_eq!(run["status"], json!("success"));
        assert_eq!(run["threadId"], json!("routine-byhand"));
        assert_eq!(
            read(&bench.layout, "byhand")["lastRun"]["summary"],
            json!("Nothing to report.")
        );
    }

    #[tokio::test]
    async fn a_run_that_never_ends_is_stopped_rather_than_left_going() {
        let bench = bench(ScriptedRunner::default());
        write(
            &bench.layout,
            Collection::Routines,
            json!({ "id": "hangs", "name": "Hangs", "enabled": true,
                    "schedule": { "kind": "manual" } }),
        );

        let run = bench.scheduler.run_now("hangs").await.expect("a run");
        assert_eq!(run["status"], json!("warning"));
        assert_eq!(bench.runner.cancelled.lock().clone(), vec!["turn-routine-hangs"]);
        // And the routine is no longer in flight, so it can be run again.
        assert!(bench.scheduler.run_now("hangs").await.is_ok());
    }

    #[tokio::test]
    async fn a_routine_left_running_by_a_crashed_process_is_settled_on_the_first_tick() {
        let bench = bench(ScriptedRunner::finishing());
        write(
            &bench.layout,
            Collection::Routines,
            json!({
                "id": "stuck",
                "name": "Stuck",
                "enabled": true,
                "schedule": { "kind": "manual" },
                "lastRun": { "at": "2026-09-03T09:00:00.000Z", "status": "running",
                             "durationMs": 0, "summary": "Started on schedule." },
                "runHistory": [{ "at": "2026-09-03T09:00:00.000Z", "status": "running" }],
            }),
        );

        bench.scheduler.tick().await;

        let after = read(&bench.layout, "stuck");
        assert_eq!(last_run_text(&after, "status").as_deref(), Some("warning"));
        assert!(last_run_text(&after, "summary")
            .expect("a summary")
            .contains("the app closed"));
        // The interrupted row was rewritten, not doubled.
        assert_eq!(after["runHistory"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn with_no_workspace_open_a_tick_does_nothing_and_says_so() {
        #[derive(Debug)]
        struct Nothing;
        impl Workspaces for Nothing {
            fn layout(&self) -> Option<Layout> {
                None
            }
        }
        let scheduler = Scheduler::new(
            Arc::new(Nothing),
            Arc::new(ScriptedRunner::finishing()),
            Arc::new(Recorder::default()),
        );
        assert_eq!(scheduler.tick().await["skipped"], json!("no workspace"));
    }

    // -- the `later` tool --------------------------------------------------

    fn ctx(layout: &Layout) -> ToolContext {
        ToolContext {
            root: layout.root().to_path_buf(),
            session: SessionId::from_existing("thread-1"),
            call_id: ToolCallId::from_existing("tc_1"),
            permissions: Arc::new(MockGate::allow_all()),
        }
    }

    async fn schedule_later(layout: &Layout, args: Value, approval: &str) -> Result<ToolOutcome> {
        later_tool(
            layout.clone(),
            Some("agent-1".into()),
            Some("Atlas".into()),
            Some(approval.into()),
            std::sync::Arc::new(|_| {}),
        )
        .execute(args, &ctx(layout))
        .await
    }

    #[tokio::test]
    async fn later_writes_a_routine_that_runs_once_as_this_agent_at_the_time_asked_for() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        let before = now().as_millisecond();

        let outcome = schedule_later(
            &layout,
            json!({ "brief": "Check whether the deploy finished and tell the user.", "in_minutes": 20 }),
            "auto",
        )
        .await
        .expect("it was scheduled");
        assert!(outcome.output.contains("as Atlas"), "{}", outcome.output);

        let all = inertia_store::collections::list(&layout, Collection::Routines);
        assert_eq!(all.len(), 1);
        let record = &all[0];
        assert_eq!(text_of(record, "agentId").as_deref(), Some("agent-1"));
        assert!(text_of(record, "markdown")
            .expect("a brief")
            .contains("deploy finished"));
        assert_eq!(schedule_text(record, "kind").as_deref(), Some("once"));
        assert_eq!(text_of(record, "mode").as_deref(), Some("autonomous"));
        assert_eq!(text_of(record, "approval").as_deref(), Some("auto"));
        assert!(text_of(record, "name")
            .expect("a name")
            .starts_with("Later: Check whether the deploy"));

        let expression = schedule_text(record, "expression").expect("a time");
        let at = parse_moment(&expression).expect("a time").as_millisecond();
        assert!(at - before >= 19 * 60_000, "{at} vs {before}");
        assert!(at - before <= 21 * 60_000, "{at} vs {before}");

        // The scheduler agrees it is next due then, and not again afterwards.
        assert_eq!(
            next_run(record, Timestamp::from_millisecond(before).expect("a time")).at,
            Some(expression.clone())
        );
        let mut ran = record.clone();
        ran["lastRun"] = json!({ "at": iso(Timestamp::from_millisecond(at + 1000).expect("a time")) });
        assert_eq!(
            next_run(&ran, Timestamp::from_millisecond(at + 2000).expect("a time")).at,
            None
        );
    }

    #[tokio::test]
    async fn later_takes_an_absolute_time_and_refuses_one_that_has_passed() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());

        let soon = iso(Timestamp::from_millisecond(now().as_millisecond() + 5 * 60_000)
            .expect("a time"));
        let ok = schedule_later(&layout, json!({ "brief": "Look at the inbox.", "at": soon }), "auto")
            .await
            .expect("it was scheduled");
        assert_eq!(ok.metadata.expect("metadata")["at"], json!(soon));

        let gone = schedule_later(
            &layout,
            json!({ "brief": "Look at the inbox.", "at": "2020-01-01T00:00:00Z" }),
            "auto",
        )
        .await;
        assert!(message(gone).contains("already passed"));
    }

    #[tokio::test]
    async fn later_insists_on_knowing_when() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        let refusal = message(schedule_later(&layout, json!({ "brief": "Do the thing." }), "auto").await);
        assert!(refusal.contains("in_minutes"), "{refusal}");
        assert!(refusal.contains("`at`"), "{refusal}");
    }

    #[tokio::test]
    async fn later_carries_the_conversations_approval_so_a_held_back_chat_schedules_a_held_back_run()
    {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        schedule_later(&layout, json!({ "brief": "Tidy the folder.", "in_minutes": 1 }), "ask")
            .await
            .expect("it was scheduled");
        let all = inertia_store::collections::list(&layout, Collection::Routines);
        assert_eq!(text_of(&all[0], "approval").as_deref(), Some("ask"));
        // And the playbook tells the future run what that means for it.
        assert!(playbook(&all[0]).contains("nobody can answer"));
    }

    #[tokio::test]
    async fn later_refuses_a_turn_with_no_agent_to_run_the_routine_as() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = Layout::new(dir.path());
        let refusal = message(
            later_tool(layout.clone(), None, None, None, std::sync::Arc::new(|_| {}))
                .execute(json!({ "brief": "Do it.", "in_minutes": 5 }), &ctx(&layout))
                .await,
        );
        assert!(refusal.contains("no agent"), "{refusal}");
    }

    fn message(result: Result<ToolOutcome>) -> String {
        match result {
            Err(error) => error.to_string(),
            Ok(outcome) => panic!("expected a refusal, got: {}", outcome.output),
        }
    }
}

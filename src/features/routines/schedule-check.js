import * as React from "react";
import { isSchedulerAvailable, routines as scheduler } from "@/lib/routines";

/**
 * Reading a schedule back to the person writing it.
 *
 * The cron field used to be free text. Whatever was typed was stored verbatim,
 * the row underneath said "Not a readable 5-field cron expression - it will
 * still be stored as written", and a routine with a typo in it simply never
 * fired. Nothing ever said so afterwards, because a routine that never runs
 * produces no failures to look at.
 *
 * The parser that decides whether a schedule fires is in the Rust backend, in
 * `src-tauri/src/routines.rs`, and it is the only opinion that matters - a second
 * one written here would eventually disagree with it, and the one the user
 * could see would be the wrong one. So this asks that parser the same two
 * questions the scheduler asks: what does this mean, and when does it next come
 * round. An expression with no next run is not a schedule, whatever the reason.
 *
 * Debounced because it crosses the process boundary on every keystroke
 * otherwise, and because "0 9 * * 1-" is a state everyone passes through on
 * the way to typing "0 9 * * 1-5" and does not need to be told off for.
 */

/** Long enough to finish typing a field, short enough to feel like a reaction. */
const DEBOUNCE = 300;

/** Nothing has been asked yet, so nothing is refused. A schedule is guilty only
 *  once the parser has actually said so. */
const UNCHECKED = {
  checking: false,
  checked: false,
  valid: true,
  reason: null,
  words: null,
  nextRunAt: null,
};

/**
 * What the two answers from the parser mean together.
 *
 * Pure, and separate from the hook, because this is the rule that decides
 * whether the Create button works - and a rule that can only be exercised by
 * rendering a dialog is a rule that does not get tested.
 */
export function readSchedule(kind, next, words) {
  // Neither of these is on a clock, so there is nothing to parse and nothing
  // that can be wrong. A trigger is a name from a fixed list.
  if (kind === "manual" || kind === "trigger") {
    return { checking: false, checked: true, valid: true, reason: null, words, nextRunAt: null };
  }

  const at = next?.at ?? null;
  if (at) {
    return { checking: false, checked: true, valid: true, reason: null, words, nextRunAt: at };
  }
  return {
    checking: false,
    checked: true,
    valid: false,
    // The parser's own sentence names the field and quotes the part it could
    // not read, which is more use than anything that could be said here.
    reason: next?.reason || "That schedule cannot be read.",
    words: null,
    nextRunAt: null,
  };
}

/**
 * The schedule being edited, checked as it is typed.
 *
 * Returns `valid` false only when the parser has been asked and said no. A
 * browser preview has no scheduler behind it, so there is nobody to ask and
 * nothing is blocked - the desktop app is where a routine can actually fire,
 * and refusing a save in a preview would be refusing on a guess.
 */
export function useScheduleCheck(kind, expression) {
  const [state, setState] = React.useState(UNCHECKED);

  React.useEffect(() => {
    if (!isSchedulerAvailable()) {
      setState(UNCHECKED);
      return undefined;
    }

    let alive = true;
    setState((prev) => ({ ...prev, checking: true }));
    // `enabled` matters: the parser answers "no next run" for a disabled
    // routine, and a routine being edited would look unparseable for a reason
    // that has nothing to do with what was typed.
    const routine = { enabled: true, schedule: { kind, expression } };

    const timer = window.setTimeout(() => {
      Promise.all([scheduler.describe(routine), scheduler.next(routine)])
        .then(([words, next]) => {
          if (alive) setState(readSchedule(kind, next, words));
        })
        .catch(() => {
          // The bridge failed, which says nothing about the expression. A
          // schedule is not refused because the answer never arrived.
          if (alive) setState(UNCHECKED);
        });
    }, DEBOUNCE);

    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [kind, expression]);

  return state;
}

/**
 * When it next fires, in the reader's own clock.
 *
 * Local rather than UTC, because the scheduler matches cron fields against
 * local time - showing the moment in any other zone would be showing a number
 * the person then has to convert before they can tell whether it is what they
 * meant.
 */
export function nextRunLabel(iso) {
  if (!iso) return null;
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return null;
  return at.toLocaleString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

const pad = (v) => String(v).padStart(2, "0");

/**
 * A deliberately small cron reader - it covers the shapes this product writes
 * and returns nothing when it cannot read an expression, rather than guessing.
 *
 * It is a nicety on top of the parser, not a second opinion: it never decides
 * whether a schedule is valid, only whether it can put one into a sentence.
 * When it cannot, the parser's own wording is shown instead.
 *
 * The times are local. This used to say UTC, which was simply wrong - the
 * scheduler matches cron fields against local time on purpose, so a person
 * reading "at 09:00 UTC" under a routine that fires at nine in their morning
 * was being told the wrong hour for no reason.
 */
export function cronToHuman(expression) {
  const parts = String(expression ?? "")
    .trim()
    .split(/\s+/);
  if (parts.length !== 5) return null;
  const [min, hour, dom, mon, dow] = parts;
  const isNum = (v) => /^\d+$/.test(v);

  let time;
  if (isNum(min) && isNum(hour)) time = `at ${pad(hour)}:${pad(min)}`;
  else if (min.startsWith("*/") && hour === "*") time = `every ${min.slice(2)} minutes`;
  else if (isNum(min) && hour === "*") time = `at ${min} minutes past every hour`;
  else if (min === "*" && hour === "*") time = "every minute";
  else return null;

  let days;
  if (dow === "1-5") days = "Monday to Friday";
  else if (dow === "0,6" || dow === "6,0") days = "on weekends";
  else if (/^[0-6]$/.test(dow)) days = `on ${DAYS[Number(dow)]}`;
  else if (isNum(dom)) days = `on day ${dom} of the month`;
  else days = "every day";

  const months =
    mon === "*"
      ? ""
      : `, in ${mon
          .split(",")
          .map((m) => MONTHS[Number(m) - 1] ?? m)
          .join(", ")}`;

  return `Runs ${days} ${time}${months}.`;
}

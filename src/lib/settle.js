/**
 * What the folder actually holds, turned into what the app can render.
 *
 * The workspace folder is the source of truth and it is meant to be hand-edited.
 * That is the whole product promise, and it has a cost nobody notices until the
 * window goes blank: every record in it was written by some version of this app,
 * or by a person in a text editor at midnight, and the screens read those
 * records as though every field were guaranteed.
 *
 * They are not. `computer.usage.cpuPct` is three assumptions in one expression -
 * that the record has a `usage`, that it is an object, and that its `cpuPct` is
 * a number - and a JSON file with `"usage": null` takes the React tree down.
 * `?? []` does not help: it catches null and waves an object straight through,
 * which is exactly how a permissions document written by an older build blanked
 * the chat screen (P15).
 *
 * So every collection gets one pass on the way in, here, before the store sees
 * it. Three rules the passes follow:
 *
 * 1. **Repair, never drop.** A malformed record stays in the list where the user
 *    can see it and go fix it. Filtering it out means a file that exists on disk
 *    and nowhere in the app, which is the worst of both.
 *
 * 2. **Coerce to the shape, not to a guess.** A missing meter reads zero, which
 *    is what "we have not measured this" looks like. A missing name stays empty
 *    rather than becoming "Untitled", because inventing a name writes that name
 *    back to the folder on the next mirror.
 *
 * 3. **Same object when nothing changed.** Hydration must not look like an edit,
 *    or opening the app would rewrite every file in the workspace.
 */

import { PREFERENCE_DEFAULTS } from "./appearance.js";

/* -- the primitives ------------------------------------------------------ */

const isPlain = (value) => Boolean(value) && typeof value === "object" && !Array.isArray(value);

/** A string, or the fallback. A number becomes its text; an object does not -
 *  "[object Object]" in a title bar is a bug wearing a hat. */
export function asText(value, fallback = "") {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return fallback;
}

export function asNumber(value, fallback = 0) {
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? n : fallback;
}

/** A meter reads 0-100 whatever the file said. A disk that is 4000% full is a
 *  provider bug, and clamping is what stops it painting off the end of the bar. */
export const asPercent = (value) => Math.min(100, Math.max(0, asNumber(value, 0)));

export const asFlag = (value, fallback = false) =>
  typeof value === "boolean" ? value : fallback;

/** A list of ids or tags. Non-strings are dropped rather than stringified: a
 *  tag that reads "[object Object]" is not a tag the user meant to write. */
export function asStrings(value) {
  if (!Array.isArray(value)) return [];
  return value.filter((entry) => typeof entry === "string" && entry);
}

export const asObject = (value) => (isPlain(value) ? value : {});

export const asOneOf = (value, allowed, fallback) =>
  allowed.includes(value) ? value : fallback;

/**
 * Keep the original object when the settled one says the same thing.
 *
 * Cheap, because it only ever runs on hydration, and worth it: React state that
 * changes identity is a state change, a state change schedules the mirror, and
 * the mirror rewrites every file whose fingerprint moved. A workspace that
 * rewrites itself on open is one that can never be diffed.
 */
function keepIfSame(original, settled) {
  return JSON.stringify(original) === JSON.stringify(settled) ? original : settled;
}

/** Map a settle over a list, preserving the array's identity when nothing moved. */
function settleList(rows, settleOne) {
  if (!Array.isArray(rows)) return [];
  let touched = false;
  const settled = rows.map((row) => {
    const next = settleOne(row);
    if (next !== row) touched = true;
    return next;
  });
  return touched ? settled : rows;
}

/* -- computers ----------------------------------------------------------- */

export const COMPUTER_STATUSES = ["running", "paused", "stopped", "provisioning", "error"];

/**
 * A computer record.
 *
 * Four unguarded reads in the screens make this the most exposed collection in
 * the app: `usage.cpuPct` in two places, `specs.cpu` and `specs.memoryGb`,
 * `assignedAgentIds.length` in the delete confirmation, and `assignedAgentIds
 * .map` in the list. A computer written by hand - which is the point of the
 * folder - has none of them.
 */
export function settleComputer(computer) {
  if (!isPlain(computer)) return computer;
  const usage = asObject(computer.usage);
  const specs = asObject(computer.specs);
  return keepIfSame(computer, {
    ...computer,
    name: asText(computer.name),
    status: asOneOf(computer.status, COMPUTER_STATUSES, "stopped"),
    provider: asText(computer.provider, "docker"),
    usage: {
      cpuPct: asPercent(usage.cpuPct),
      memPct: asPercent(usage.memPct),
      diskPct: asPercent(usage.diskPct),
    },
    specs: {
      cpu: asNumber(specs.cpu, 0),
      memoryGb: asNumber(specs.memoryGb, 0),
      diskGb: asNumber(specs.diskGb, 0),
    },
    assignedAgentIds: asStrings(computer.assignedAgentIds),
    snapshotCount: asNumber(computer.snapshotCount, 0),
  });
}

export const settleComputers = (rows) => settleList(rows, settleComputer);

/* -- routines ------------------------------------------------------------ */

/**
 * A routine.
 *
 * `lastRun.at` is read without a guard once the status says a run happened, so
 * a run with no timestamp is a crash. A run object with nothing in it is not a
 * run, and becomes null - "never run", which is the truth about a record that
 * cannot say when it ran.
 */
export function settleRoutine(routine) {
  if (!isPlain(routine)) return routine;
  const run = asObject(routine.lastRun);
  const lastRun = run.at || run.status ? { ...run, at: asText(run.at) || null } : null;
  // A schedule an agent wrote by hand may be a bare cron string, or missing.
  // Every screen reads `schedule.kind`, so it is always an object with one.
  const schedule =
    typeof routine.schedule === "string"
      ? { kind: "cron", expression: routine.schedule, humanLabel: routine.schedule }
      : { kind: "manual", ...asObject(routine.schedule) };
  return keepIfSame(routine, {
    ...routine,
    name: asText(routine.name),
    tags: asStrings(routine.tags),
    enabled: asFlag(routine.enabled, true),
    // Only when present. Absent means the defaults in shared/routines, and
    // writing them in here would rewrite every routine file to say so.
    ...(routine.mode !== undefined ? { mode: asText(routine.mode) } : {}),
    ...(routine.approval !== undefined ? { approval: asText(routine.approval) } : {}),
    schedule,
    lastRun,
  });
}

export const settleRoutines = (rows) => settleList(rows, settleRoutine);

/* -- memory -------------------------------------------------------------- */

/**
 * A memory.
 *
 * The three numbers are rendered as fact and drive two sort orders, so a string
 * where a count belongs sorts nonsensically rather than loudly. Confidence is a
 * percentage in the UI and clamped like one.
 */
export function settleMemory(memory) {
  if (!isPlain(memory)) return memory;
  return keepIfSame(memory, {
    ...memory,
    title: asText(memory.title),
    body: asText(memory.body),
    tags: asStrings(memory.tags),
    useCount: asNumber(memory.useCount, 0),
    confidence: asPercent(memory.confidence),
    // Which project a memory belongs to, and whether anything is allowed to
    // believe it yet. Both are read by the screen and by the main process, so a
    // record hand-edited into a shape neither expects is settled here rather
    // than being guessed at twice.
    scope: memory.scope === "project" ? "project" : "global",
    folder: typeof memory.folder === "string" && memory.folder ? memory.folder : null,
    pending: memory.pending === true,
  });
}

export const settleMemories = (rows) => settleList(rows, settleMemory);

/* -- threads ------------------------------------------------------------- */

/**
 * A thread.
 *
 * `draft` is forced false on the way in for the same reason a streaming message
 * is settled: a draft has no file, so a record on disk claiming to be one is a
 * contradiction, and honouring it would hide a real conversation from the list
 * and then delete its file on the next mirror. `temporary` is the same
 * contradiction with a different name - a temporary conversation is one that
 * was never written down, so a file claiming to be one is a file that should
 * not exist, and believing it would stop the conversation ever being saved
 * again.
 */
export function settleThread(thread) {
  if (!isPlain(thread)) return thread;
  return keepIfSame(thread, {
    ...thread,
    title: asText(thread.title),
    draft: false,
    // Only when the file actually claims it. Writing the key unconditionally
    // would add `"temporary": false` to every thread record in the folder on
    // the next mirror, which is a rewrite of the whole collection to say
    // nothing.
    ...(thread.temporary ? { temporary: false } : {}),
    pinned: asFlag(thread.pinned),
    unread: asNumber(thread.unread, 0),
    messageCount: asNumber(thread.messageCount, 0),
    agentId: typeof thread.agentId === "string" ? thread.agentId : null,
  });
}

export const settleThreads = (rows) => settleList(rows, settleThread);

/* -- the person ---------------------------------------------------------- */

/**
 * The identity document.
 *
 * `user.preferences.sendOnEnter` and `user.preferences.defaultModelId` are read
 * without a guard in the composer, which means a hand-edited identity file with
 * no `preferences` key stops the user from typing.
 *
 * Each preference is coerced to the type of its own default, which is why
 * `PREFERENCE_DEFAULTS` is the right list to walk: it is already the single
 * statement of what a preference is, and a new option gets this for free.
 * Unknown keys are kept - a preference this build does not know about belongs
 * to a build that does, and dropping it would delete it from the folder.
 */
export function settleUser(user) {
  if (!isPlain(user)) return { preferences: { ...PREFERENCE_DEFAULTS } };
  const stored = asObject(user.preferences);
  const preferences = { ...stored };

  for (const [key, fallback] of Object.entries(PREFERENCE_DEFAULTS)) {
    const value = stored[key];
    if (value === undefined) {
      preferences[key] = fallback;
    } else if (typeof fallback === "boolean") {
      preferences[key] = asFlag(value, fallback);
    } else if (typeof fallback === "number") {
      preferences[key] = asNumber(value, fallback);
    } else if (typeof fallback === "string") {
      // Stricter than `asText` on purpose. Every string preference is an enum
      // or an id, and a number stringified into one - accent `7`, timezone `0`
      // - is a value nothing matches, rendered as a picker showing a blank.
      preferences[key] = typeof value === "string" ? value : fallback;
    } else {
      // A default of null says "no opinion", and the only wrong value for that
      // is one that is neither a string nor absent.
      preferences[key] = typeof value === "string" || value === null ? value : fallback;
    }
  }

  return keepIfSame(user, {
    ...user,
    name: asText(user.name),
    shortName: asText(user.shortName),
    bio: asText(user.bio),
    preferences,
  });
}

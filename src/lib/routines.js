/**
 * The window's view of the scheduler.
 *
 * Thin on purpose. The scheduler is in the backend and keeps time whether
 * a window exists or not, which is the whole point of a routine - so nothing
 * here decides anything. It asks, it commands, and it listens.
 *
 * There is no polling. A routine that starts or finishes says so on the event
 * channel, and the folder watcher picks the written record up as well, so a
 * screen that is open sees a run happen and one that is not misses nothing.
 */

const bridge = typeof window !== "undefined" ? window.routineAPI : null;

export function isSchedulerAvailable() {
  return Boolean(bridge?.run);
}

function unwrap(result) {
  if (!result) throw new Error("The routines bridge did not answer.");
  if (result.ok === false) throw new Error(result.error ?? "That did not work.");
  // `"data" in result`, not `result.data ?? result` - `data` is legitimately
  // null for a schedule that has no next run, and `??` would fall through and
  // hand back the envelope. That mistake drew a broken image in the computers
  // pane for a week; it is not making it into this file.
  return "data" in result ? result.data : result;
}

const call = (name, ...args) => {
  if (!bridge?.[name]) {
    return Promise.reject(new Error("Routines only run in the desktop app."));
  }
  return bridge[name](...args).then(unwrap);
};

export const routines = {
  /** Run one now, whatever its schedule says. Resolves when the turn is done. */
  run: (id) => call("run", id),
  /** When each routine is next due, or - given a routine - when that one is. */
  next: (routine) => call("next", routine),
  /** A schedule in words, so no screen has to parse cron. */
  describe: (routine) => call("describe", routine),
  /** A pass over the folder now, for someone who does not want to wait. */
  tick: () => call("tick"),
  onEvent: (handler) => bridge?.onEvent?.(handler) ?? (() => {}),
};

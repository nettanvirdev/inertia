import { call, subscribe } from "./envelope";

/**
 * The routines bridge: the window's remote control for the scheduler.
 *
 * Deliberately small, and that is the shape the other shell settled on for a
 * reason: everything a routine does happens in the app process whether a
 * window exists or not, so this is three questions and one command rather than
 * a remote control. Run this one now, when is it next due, what does this
 * schedule say - and a stream of events so an open screen keeps up without
 * polling.
 *
 * `lib/routines.js` reads `window.routineAPI` at module-evaluation time and
 * `isSchedulerAvailable()` is `Boolean(bridge?.run)`, so this namespace being
 * installed is what turns the Routines screen from a preview into the real
 * thing. Which is why it is installed only once there is a scheduler behind
 * it: a stub that answered would leave the person looking at a Run button that
 * quietly does nothing at nine o'clock.
 */
export function routinesBridge() {
  return {
    /** Run one now, whatever its schedule says. Resolves when the turn is done. */
    run: (id) => call("routine_run", { id }),

    /**
     * When each routine is next due, or - given a routine - when that one is.
     *
     * The argument is the routine the person is editing, not its id: a dialog
     * showing a schedule halfway through being typed has to ask about what it
     * is about to save, not about what is on disk.
     */
    next: (routine) => call("routine_next", { routine: routine ?? null }),

    /** A schedule in words, so no screen has to parse cron. */
    describe: (routine) => call("routine_describe", { routine: routine ?? null }),

    /** A pass over the folder now, for someone who does not want to wait. */
    tick: () => call("routine_tick"),

    /** Runs starting and finishing, including the ones nobody asked for. */
    onEvent: (callback) => subscribe("routine:event", callback),
  };
}

import { call, subscribe } from "./envelope";

/**
 * The agents an agent started.
 *
 * Snapshot plus a push, rather than a stream of deltas. The tree changes shape
 * while it is being drawn - a run spawns two more halfway through streaming its
 * own reply - and reconciling that in the renderer is a second source of truth
 * waiting to disagree with the first. So `onEvent` carries whole snapshots, one
 * per conversation, coalesced in the backend so four streaming runs cannot put
 * a message on the bridge per chunk.
 *
 * Every method is forgiving about its arguments because the panel calls some of
 * them from effect cleanups, where a conversation can already be gone: a
 * missing id becomes an empty string and the backend answers with an empty
 * list, rather than the call throwing on the way out of a component.
 *
 * # Three methods that answer "not yet"
 *
 * `pause`, `resume` and `restart` are installed and refuse in a sentence. The
 * runtime behind them has no pause gate and no way to relaunch a brief as a
 * second run, and the panel is written for a reply it ignores - it leaves the
 * row exactly as it was, which is the honest drawing of a request that did not
 * happen. Leaving them off the namespace instead would be worse: `lib/crew.js`
 * checks for each method and silently returns `{ paused: false }`, so the bug
 * would look identical and there would be nothing to grep for.
 */
export function crewBridge() {
  return {
    /** Every run under one conversation, in the shape the panel draws. */
    snapshot: (conversationId) =>
      call("crew_snapshot", { conversationId: String(conversationId ?? "") }),

    /**
     * Stop a run, and by default everything underneath it.
     *
     * The subtree is the honest default: a helper whose coordinator has been
     * stopped is working towards a report nobody will read.
     */
    cancel: (runId, options) =>
      call("crew_cancel", {
        runId: String(runId ?? ""),
        options: { descendants: options?.descendants !== false },
      }),

    pause: (runId, options) =>
      call("crew_pause", {
        runId: String(runId ?? ""),
        options: { descendants: options?.descendants !== false },
      }),

    resume: (runId, options) =>
      call("crew_resume", {
        runId: String(runId ?? ""),
        options: { descendants: options?.descendants !== false },
      }),

    restart: (runId) => call("crew_restart", { runId: String(runId ?? "") }),

    /** End the turn, keep the run: it can still be given a new brief. */
    interrupt: (runId) => call("crew_interrupt", { runId: String(runId ?? "") }),

    followUp: (runId, prompt) =>
      call("crew_followup", { runId: String(runId ?? ""), prompt: String(prompt ?? "") }),

    /** Every event from every run in one conversation, merged and in order. */
    timeline: (conversationId) =>
      call("crew_timeline", { conversationId: String(conversationId ?? "") }),

    /**
     * Which run this window is reading.
     *
     * Cheap, idempotent, and sent again with no run when the column closes.
     * Nothing is selected by it yet - the snapshot carries no folded transcript
     * to select - but the panel calls it on every open and close, and a
     * namespace missing the method would be a `is not a function` in an effect.
     */
    watch: (conversationId, runId) =>
      call("crew_watch", {
        conversationId: String(conversationId ?? ""),
        runId: runId ? String(runId) : null,
      }),

    /** A thread was deleted. Its runs have nowhere left to be shown. */
    forget: (conversationId) =>
      call("crew_forget", { conversationId: String(conversationId ?? "") }),

    onEvent: (callback) => subscribe("crew:event", callback),
  };
}

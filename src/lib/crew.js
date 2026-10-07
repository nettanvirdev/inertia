import * as React from "react";

/**
 * The window's view of the team.
 *
 * The backend owns every run: it started the sessions, it holds the abort
 * controllers, and it is the only thing that survives a reload. This is a
 * mirror, refreshed by a push, and it deliberately holds no opinion of its own
 * - a window that decided locally that a run had finished would be a second
 * source of truth, and the first one it disagreed with would be right.
 *
 * A snapshot on mount and a snapshot on every push, rather than deltas. The
 * tree changes shape while it is being drawn, because a run can spawn two more
 * halfway through streaming its reply, and a delta stream would have the
 * window reconstructing a shape the backend already knows.
 */

/**
 * Read at the point of use, not captured at import.
 *
 * The other bridges in this folder bind `window.somethingAPI` to a module
 * constant, which is fine in the desktop app because `main.jsx` installs the
 * bridges before anything else is imported. It is
 * wrong anywhere the module can be imported before the bridge exists - a test,
 * a browser preview - and the failure is silent: an empty panel that looks like
 * an empty team.
 */
const bridge = () => (typeof window !== "undefined" ? window.crewAPI : null) ?? null;

function unwrap(result) {
  if (!result) return [];
  if (result.ok === false) throw new Error(result.error ?? "That did not work.");
  return "data" in result ? result.data : result;
}

/**
 * Whatever the bridge said, as a list of runs.
 *
 * `unwrap` returns the reply as-is when it carries no envelope, so a bridge
 * answering with an object puts one where the panel expects an array. The first
 * thing to touch it is `crewRuns.filter` in ChatView, which means a malformed
 * snapshot does not produce an empty agent panel - it produces a conversation
 * replaced by an error boundary. A bridge that answers nonsense is still no
 * team, and that is what it should look like.
 */
export function asRuns(result) {
  return Array.isArray(result) ? result : [];
}

export async function cancelRun(runId, { descendants = true } = {}) {
  const api = bridge();
  if (!api?.cancel) return { cancelled: false };
  return unwrap(await api.cancel(runId, { descendants }));
}

/**
 * Hold a run at its next boundary, or let it go again.
 *
 * The subtree comes along by default, which is the honest default for the same
 * reason it is when stopping: a helper whose coordinator is paused is working
 * towards a report nobody is currently reading.
 */
export async function pauseRun(runId, { descendants = true } = {}) {
  const api = bridge();
  if (!api?.pause) return { paused: false };
  return unwrap(await api.pause(runId, { descendants }));
}

export async function resumeRun(runId, { descendants = true } = {}) {
  const api = bridge();
  if (!api?.resume) return { resumed: false };
  return unwrap(await api.resume(runId, { descendants }));
}

/** Run the same brief again, as a new run linked to the one it replaces. */
export async function restartRun(runId) {
  const api = bridge();
  if (!api?.restart) return { restarted: false };
  return unwrap(await api.restart(runId));
}

/**
 * Stop a run's current turn and keep it, so it can be given a new brief.
 *
 * Its children keep going, unlike a stop: the person interrupting a
 * coordinator to redirect it usually wants what it already handed out.
 */
export async function interruptRun(runId) {
  const api = bridge();
  if (!api?.interrupt) return { interrupted: false };
  return unwrap(await api.interrupt(runId));
}

/** A new brief for a run that has settled; it continues with its transcript. */
export async function followUpRun(runId, prompt) {
  const api = bridge();
  if (!api?.followUp) return { started: false, reason: "Not available here." };
  return unwrap(await api.followUp(runId, prompt));
}

/**
 * Say which run this window is reading, so its transcript is pushed with the
 * snapshot. Pass no run when the column closes; failing is not worth a word,
 * the column simply shows what it already had.
 */
export async function watchRun(conversationId, runId) {
  const api = bridge();
  if (!api?.watch) return;
  await api.watch(conversationId, runId ?? null).catch(() => {});
}

/** Every event from every run in one conversation, merged and in order. */
export async function loadTimeline(conversationId) {
  const api = bridge();
  if (!api?.timeline) return [];
  return unwrap(await api.timeline(conversationId)) ?? [];
}

export async function forgetConversation(conversationId) {
  const api = bridge();
  if (!api?.forget) return;
  await api.forget(conversationId);
}

/**
 * Every run under one conversation, live.
 *
 * `conversationId` is the thread id: a spawned session is addressed as
 * `thread/run`, however deep it goes, so one subscription covers the whole tree
 * beneath a chat without the panel having to know its shape.
 */
export function useCrew(conversationId) {
  const [runs, setRuns] = React.useState([]);

  React.useEffect(() => {
    const api = bridge();
    if (!conversationId || !api?.snapshot) {
      setRuns([]);
      return undefined;
    }
    let alive = true;

    api
      .snapshot(conversationId)
      .then((result) => {
        // Whatever came back has to be a list. `unwrap` returns the reply
        // as-is when it carries no envelope, so one malformed answer would put
        // an object where the panel expects an array - and the first thing
        // that touches it is `crewRuns.filter` in ChatView, which means the
        // whole conversation renders as an error boundary rather than an empty
        // agent panel. A bridge that answers nonsense is still no team.
        if (alive) setRuns(asRuns(unwrap(result)));
      })
      .catch(() => {
        // A bridge that will not answer is an empty panel, not a broken chat.
        if (alive) setRuns([]);
      });

    const off = api.onEvent?.((payload) => {
      if (!alive || payload?.conversationId !== conversationId) return;
      setRuns(asRuns(payload.runs));
    });

    return () => {
      alive = false;
      off?.();
    };
  }, [conversationId]);

  return runs;
}

/**
 * The runs arranged the way they were created: each under whoever started it.
 *
 * Depth is recomputed from the links rather than trusted from the record,
 * because a run's own `depth` counts sessions - a `task` subagent's child is at
 * depth two - and the panel is drawing the tree it can actually see. A run
 * whose parent is not in this conversation hangs off the root rather than
 * disappearing, which is the failure that would be hardest to notice.
 */
export function asTree(runs) {
  const byId = new Map((runs ?? []).map((run) => [run.id, run]));
  const children = new Map();
  for (const run of runs ?? []) {
    const parent = run.parentId && byId.has(run.parentId) ? run.parentId : null;
    const list = children.get(parent) ?? [];
    list.push(run);
    children.set(parent, list);
  }
  const out = [];
  const seen = new Set();
  const walk = (parentId, depth) => {
    for (const run of children.get(parentId) ?? []) {
      if (seen.has(run.id)) continue;
      seen.add(run.id);
      out.push({ ...run, indent: depth });
      walk(run.id, depth + 1);
    }
  };
  walk(null, 0);

  // Anything the walk could not reach, at the root.
  //
  // Only a cycle can do that - every other shape has a run whose parent is
  // absent, and those are already filed under `null` above - and nothing
  // creates one, because a parent id is the id of a run that already existed
  // when the child was spawned. But "impossible" and "harmless" are different
  // claims, and the harm here is specific: the walk starts at the root, so a
  // cycle is not reachable from it and every run in the conversation vanishes.
  // The panel would say the team was empty while five agents were working,
  // which is worse than any indentation could be wrong.
  for (const run of runs ?? []) {
    if (!seen.has(run.id)) out.push({ ...run, indent: 0 });
  }
  return out;
}

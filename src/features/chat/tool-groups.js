/**
 * Folding a run of calls into one row.
 *
 * An agent orienting itself in an unfamiliar repository reads six or eight
 * files before it says a word, and every one of them was its own card. The
 * transcript became a ladder of near-identical rows - "Read files package.json",
 * "Read files AGENTS.md", "Read files layout.tsx" - that pushed the answer the
 * person was waiting for off the bottom of the screen. Folded, the same run is
 * one line, and every one of them is still there behind it.
 *
 * ── Why every tool folds, and not only `read` ──────────────────────────────
 * It was `read` alone at first, on the argument that every other tool either
 * changes something or costs something, and that folding three edits into
 * "Edited 3 files" hides the one that was wrong. The argument was about a
 * summary that REPLACES its members. This one does not: the row is a lid, each
 * call is still its own openable row underneath, and a run containing a
 * failure opens itself and shows the failure. Once that is true, a search tool
 * called four times in a row is the same ladder as four reads - which is
 * exactly what the person saw when two failed searches, two retries and two
 * thoughts filled the screen for one lookup.
 *
 * Only calls of the SAME tool fold together. A run of searches is one action
 * from the reader's side; a search followed by an edit is two, and the order
 * is the story.
 *
 * A run is strictly consecutive. Anything between two calls - a line of prose,
 * a tool of another name - ends the run.
 *
 * ── Thoughts ──────────────────────────────────────────────────────────────
 * The same problem in a different shape. A model that thinks, calls nothing,
 * and thinks again produces two "Thought for 3s" rows describing one pause, so
 * consecutive thinking is joined into one.
 */

/** Two is a ladder when the rows are identical, which is when this fires. */
const MIN_RUN = 2;

/** The tool whose folded row is about files rather than calls. */
export const GROUPED_TOOL = "read";

/**
 * Message parts, with consecutive calls of one tool collected.
 *
 * Returns render items rather than parts so the caller stays a `map`: either
 * `{ kind: "one", part }` for anything that draws itself, or
 * `{ kind: "run", name, calls }` for a folded run.
 *
 * Keys carry the part's index because that is what the transcript already keys
 * on, and it is the only identifier a prose part has. A folded run takes the
 * index of its first member, which does not move as the run grows - so a run
 * being appended to while the turn streams does not remount what is already
 * drawn.
 */
export function groupParts(parts) {
  const items = [];
  let run = [];
  let thinking = [];

  const flushRun = () => {
    if (!run.length) return;
    if (run.length >= MIN_RUN) {
      items.push({
        kind: "run",
        key: `run-${run[0].key}`,
        name: run[0].part.name,
        calls: run.map((one) => one.part),
      });
    } else {
      for (const one of run) items.push({ kind: "one", key: one.key, part: one.part });
    }
    run = [];
  };

  const flushThoughts = () => {
    if (!thinking.length) return;
    if (thinking.length === 1) {
      items.push({ kind: "one", key: thinking[0].key, part: thinking[0].part });
    } else {
      items.push({ kind: "one", key: thinking[0].key, part: joinThoughts(thinking.map((one) => one.part)) });
    }
    thinking = [];
  };

  const flush = () => {
    flushRun();
    flushThoughts();
  };

  (parts ?? []).forEach((part, index) => {
    const key = `${index}-${part?.callId ?? part?.type ?? "part"}`;
    if (part?.type === "tool" && part?.name) {
      flushThoughts();
      // A different tool is a different action, so the run before it ends.
      if (run.length && run[0].part.name !== part.name) flushRun();
      run.push({ key, part });
      return;
    }
    if (part?.type === "reasoning") {
      flushRun();
      thinking.push({ key, part });
      return;
    }
    flush();
    items.push({ kind: "one", key, part });
  });
  flush();

  return items;
}

/**
 * Several stretches of thinking, as one.
 *
 * The span is the first start to the last end, because that is the pause the
 * person actually sat through - the gaps between them were thinking too.
 */
export function joinThoughts(parts) {
  return {
    type: "reasoning",
    text: parts
      .map((one) => String(one?.text ?? "").trim())
      .filter(Boolean)
      .join("\n\n"),
    startedAt: parts[0]?.startedAt,
    endedAt: parts[parts.length - 1]?.endedAt,
  };
}

/**
 * The state of a run, from the states of its members.
 *
 * A failure anywhere is the run's state, because that is the one thing the
 * reader has to be told without opening anything. Otherwise a run is still
 * running until every member has finished.
 */
export function runState(calls) {
  if (calls.some((one) => one.state === "failed")) return "failed";
  if (calls.some((one) => one.state === "running")) return "running";
  return "done";
}

/**
 * How long the run took, or nothing.
 *
 * Nothing when any member has not reported, rather than a total over the ones
 * that did: an understated number read as fact is worse than no number.
 */
export function runDuration(calls) {
  if (!calls.length || calls.some((one) => one.durationMs == null)) return undefined;
  return calls.reduce((total, one) => total + one.durationMs, 0);
}

/**
 * The last segment of a path, for a label.
 *
 * Both separators: these paths are Windows paths as often as not, and a split
 * on the forward slash alone hands back the whole `D:\work\api\page.tsx`.
 */
export function fileName(call) {
  const raw = call?.metadata?.path ?? call?.args?.filePath ?? call?.args?.path ?? call?.title ?? "";
  const parts = String(raw)
    .replace(/[\\/]+$/, "")
    .split(/[\\/]/);
  return parts[parts.length - 1] || String(raw);
}

/**
 * The run's title: every file, named, in the order they were read.
 *
 * All of them, and not a truncated list with "+4 more" appended, because the
 * row truncates itself: the title is a single non-wrapping line and the browser
 * puts the ellipsis exactly where the pane runs out. Counting here as well would
 * mean deciding at four names what the browser can decide at whatever width the
 * window happens to be, and the count is already on the row beside it.
 */
export function runTitle(calls) {
  if (calls[0]?.name === GROUPED_TOOL) return calls.map(fileName).join(", ");
  // Every other tool says what it touched in its own title, and the same
  // target twice - two attempts at one search - is said once.
  const seen = [];
  for (const call of calls) {
    const title = String(call?.title ?? call?.name ?? "").trim();
    if (title && !seen.includes(title)) seen.push(title);
  }
  return seen.join(", ");
}

/** What the count on the row is counting. */
export function runCount(calls) {
  if (calls[0]?.name === GROUPED_TOOL) return `${calls.length} files`;
  return `${calls.length} calls`;
}

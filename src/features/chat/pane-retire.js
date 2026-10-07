/**
 * Closing a browser view a moment after the pane that showed it goes.
 *
 * The page is a native webview the backend owns, and it outlives the component
 * that positions it, so a pane that unmounts has to close its views or it
 * leaves real browsers running with nothing left to stop them. That is why the
 * close was in the unmount, and it was right about the goal and wrong about
 * the timing.
 *
 * ── The unmount that is not one ───────────────────────────────────────────
 * React does not promise that an unmount means goodbye. In development every
 * component is mounted, torn down and mounted again on purpose, and both
 * halves of that are indistinguishable from the real thing from inside an
 * effect. So opening the browser panel closed the view it had just been opened
 * to show. Nobody noticed while panels were only ever opened by hand and empty
 * when they opened - and then a tool started loading a page BEFORE asking for
 * the panel, and the page was destroyed underneath its own navigation. What
 * came back was `Cannot read properties of undefined (reading 'getURL')`, and
 * a browser pane that offered to open a page it had already loaded.
 *
 * ── The rule ──────────────────────────────────────────────────────────────
 * A pane going away schedules the close; a pane appearing with the same id
 * cancels it. A remount happens in the same frame, so it always wins; a real
 * unmount has nothing to cancel it and the view closes a moment later. The
 * delay is the whole mechanism, so it is short enough that a genuinely closed
 * browser does not keep running and long enough that no remount can lose the
 * race.
 */

/** Timers by pane id. */
const timers = new Map();

/** Long enough for a remount, short enough that a real close feels immediate. */
export const RETIRE_DELAY = 250;

/**
 * This view's pane has gone. Close it, unless it comes back first.
 */
export function retirePane(id, close, delay = RETIRE_DELAY) {
  const key = String(id ?? "");
  if (!key || typeof close !== "function") return;
  keepPane(key);
  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key);
      try {
        close();
      } catch {
        // Closing something already gone is the expected way for this to fail.
      }
    }, delay)
  );
}

/** This view has a pane again - or never lost one. Called on mount. */
export function keepPane(id) {
  const key = String(id ?? "");
  const timer = timers.get(key);
  if (timer === undefined) return false;
  clearTimeout(timer);
  timers.delete(key);
  return true;
}

/** Whether a close is still waiting to happen. */
export function isRetiring(id) {
  return timers.has(String(id ?? ""));
}

/** Test seam. */
export function resetPaneRetire() {
  for (const timer of timers.values()) clearTimeout(timer);
  timers.clear();
}

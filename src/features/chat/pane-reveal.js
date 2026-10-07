/**
 * Showing the person what the agent is looking at.
 *
 * The browser and the terminal beside a conversation are the agent's as much
 * as they are the person's, and until now that was true in the least useful
 * way: a tool could load a page, click through it and read its console while
 * the panel was closed, and the only trace was a tool result making claims
 * about a window nobody could see. Which is exactly what a model inventing an
 * answer also looks like. The whole argument for putting a real browser in
 * this app was that the two of you look at the same one.
 *
 * So a tool that touches a pane asks for it to be brought forward, and three
 * separate things have to happen for that to mean anything:
 *
 *   - the dock has to be open at all             (ChatView owns that)
 *   - the right panel has to be in front         (SideDock owns that)
 *   - the right tab within it has to be active   (WebPane / ShellPane)
 *
 * None of those three can see the other two, and threading a prop from the
 * IPC listener down through the dock into a tab list would put a wire through
 * four components that have no other reason to know about each other. So it is
 * a module: one event, three subscribers, each doing the one part it owns.
 *
 * ── Why the request is remembered rather than shouted ──────────────────────
 * The first version was a plain event, and it could not work: the panes it is
 * addressed to are the ones the dock has not mounted yet. A closed dock mounts
 * nothing, so the sequence is open the dock, THEN mount the pane - and an
 * event fired between those two lands nowhere at all. The agent started a dev
 * server, asked for its tab, and the person who opened the terminal a moment
 * later found the tab strip exactly as they had left it.
 *
 * So the latest request per panel-and-conversation is kept, and a pane that
 * subscribes is told about it immediately. A pane consuming one clears it, so
 * a panel closed and reopened an hour later does not jump back to whatever the
 * agent was doing then.
 *
 * A request, not a command. Every subscriber is free to ignore it - a pane for
 * a different conversation does, and so does a tab id that belongs to a pane
 * that is not mounted.
 */

const listeners = new Set();

/**
 * Requests waiting for a pane that can honour them, in the order they were
 * made.
 *
 * Keyed by the TAB and not only by the panel, which is the difference between
 * two tabs and one. Two background services started in the same turn are two
 * reveals a fraction apart, and keeping only the latest meant the second
 * silently replaced the first: the agent could read three terminals and the
 * person could see two. A Map preserves insertion order, so replaying them all
 * still leaves the newest in front.
 */
const pending = new Map();

const slot = (dock, threadId, tabId) => `${dock}|${threadId ?? ""}|${tabId ?? ""}`;

function tell(listener, detail) {
  try {
    listener(detail);
  } catch {
    // One subscriber throwing must not stop the rest: a reveal that opens the
    // panel but never picks the tab is worse than one that does nothing.
  }
}

/**
 * Bring a tab forward.
 *
 * `dock` is the panel's id in the SideDock ("web" or "shell"), `tabId` the
 * pane's own tab, and `threadId` the conversation it belongs to - without
 * which a second conversation open in another window would pull its panels
 * around in response to this one's tools.
 */
export function revealPane(target) {
  const detail = {
    dock: String(target?.dock ?? ""),
    tabId: String(target?.tabId ?? ""),
    threadId: target?.threadId == null ? null : String(target.threadId),
  };
  if (!detail.dock || !detail.tabId) return;
  pending.set(slot(detail.dock, detail.threadId, detail.tabId), detail);
  for (const listener of [...listeners]) tell(listener, detail);
}

/**
 * Subscribe, and hear anything already waiting.
 *
 * The replay is the point rather than a nicety: a pane subscribes in the
 * effect that runs after it mounts, which is after the request that caused it
 * to be mounted has already been made.
 */
export function onRevealPane(listener) {
  if (typeof listener !== "function") return () => {};
  listeners.add(listener);
  for (const detail of [...pending.values()]) tell(listener, detail);
  return () => listeners.delete(listener);
}

/**
 * Done with it.
 *
 * Called by the pane that actually adopted the tab. Until then the request
 * stays put, because "nobody was listening" and "somebody handled it" are
 * different states and only the pane can tell them apart.
 */
export function clearReveal(dock, threadId, tabId) {
  const key = slot(String(dock ?? ""), threadId == null ? null : String(threadId), tabId);
  if (tabId != null) {
    pending.delete(key);
    return;
  }
  // No tab named: everything waiting for this panel in this conversation.
  const prefix = `${String(dock ?? "")}|${threadId == null ? "" : String(threadId)}|`;
  for (const each of [...pending.keys()]) if (each.startsWith(prefix)) pending.delete(each);
}

/** What is waiting for this panel in this conversation, newest last. */
export function pendingReveals(dock, threadId) {
  const prefix = `${String(dock ?? "")}|${threadId == null ? "" : String(threadId)}|`;
  return [...pending.entries()]
    .filter(([key]) => key.startsWith(prefix))
    .map(([, detail]) => detail);
}

/** Test seam. */
export function resetPaneReveal() {
  listeners.clear();
  pending.clear();
}

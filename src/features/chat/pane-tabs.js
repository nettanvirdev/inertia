/**
 * Several browsers, or several terminals, in one pane.
 *
 * The rules are the same for both and they are the fiddly kind - what happens
 * to the front tab when you close the front tab, what a new tab is called, how
 * a numbering scheme survives closing the middle one - so they live here once,
 * as data, rather than twice inside two components.
 *
 * A tab is `{ id, label }` and the id is what the backend keys a shell or
 * a browser view on. It has to be stable for the life of the tab and unique
 * across conversations, because two conversations open at once are two sets of
 * shells that must not be the same shells.
 */

/**
 * The id for one tab of one kind in one conversation.
 *
 * `chat:<thread>:sh:3`. The thread comes first because that is the part that
 * makes it unique; the rest is only there so a browser and a terminal in the
 * same conversation do not collide.
 */
export function tabId(threadId, kind, serial) {
  return `chat:${threadId}:${kind}:${serial}`;
}

/**
 * A tab's number, for its label.
 *
 * Counted from the highest ever used rather than from the length of the list,
 * because a person who opens three tabs, closes the second, and opens another
 * expects a new one - not a second tab called 2. The serial only ever goes up,
 * which is also what keeps ids from being reused while the backend still
 * holds a shell under the old one.
 */
export function nextSerial(tabs = []) {
  const highest = (tabs ?? []).reduce((top, tab) => Math.max(top, Number(tab?.serial) || 0), 0);
  return highest + 1;
}

/** One more tab, and it becomes the front one - which is why you opened it. */
export function addTab(tabs, { threadId, kind, label }) {
  const list = Array.isArray(tabs) ? tabs : [];
  const serial = nextSerial(list);
  const tab = {
    id: tabId(threadId, kind, serial),
    serial,
    label: label ?? `${kind === "sh" ? "Terminal" : "Tab"} ${serial}`,
  };
  return { tabs: [...list, tab], active: tab.id, opened: tab };
}

/**
 * One fewer tab, and whoever takes the front.
 *
 * The neighbour to the right, falling back to the one on the left - which is
 * what every browser does and what your hand expects when you close a run of
 * tabs from the middle. Closing the last one leaves nothing, and the caller
 * decides what an empty pane means.
 */
export function closeTab(tabs, active, closing) {
  const list = Array.isArray(tabs) ? tabs : [];
  const at = list.findIndex((tab) => tab.id === closing);
  if (at === -1) return { tabs: list, active, closed: null };

  const remaining = list.filter((tab) => tab.id !== closing);
  if (!remaining.length) return { tabs: remaining, active: null, closed: closing };

  // Only the front tab closing moves the front.
  if (active !== closing) return { tabs: remaining, active, closed: closing };
  const next = remaining[Math.min(at, remaining.length - 1)];
  return { tabs: remaining, active: next.id, closed: closing };
}

/**
 * The tab to show, checked against what exists.
 *
 * The same rule the dock uses one level up: an active id pointing at a tab
 * that has gone renders a strip above an empty pane.
 */
export function activeOf(tabs, wanted) {
  const list = Array.isArray(tabs) ? tabs : [];
  if (!list.length) return null;
  return (list.find((tab) => tab.id === wanted) ?? list[0]).id;
}

/**
 * What a tab is called, once it knows what it is showing.
 *
 * A browser tab takes the page's title, or its host when the title is missing
 * or is just the URL again; a terminal takes the last segment of its folder.
 * Falls back to the number it was born with, so a tab is never nameless.
 */
export function labelFor({ kind, serial, title, url, cwd }) {
  if (kind === "job") {
    // The command, cut to something that fits a tab. A dev server's tab
    // saying "npm run dev" is the only label that helps; its pid does not.
    const command = String(title ?? "")
      .trim()
      .split(/\s+/)
      .slice(0, 3)
      .join(" ");
    return command.slice(0, 24) || "Process";
  }
  if (kind === "sh") {
    const folder = String(cwd ?? "").replace(/[\\/]+$/, "");
    const leaf = folder.split(/[\\/]/).filter(Boolean).pop();
    return leaf || `Terminal ${serial}`;
  }
  const clean = String(title ?? "").trim();
  if (clean && clean !== url) return clean;
  try {
    return new URL(String(url)).host || `Tab ${serial}`;
  } catch {
    return `Tab ${serial}`;
  }
}

/**
 * The number in a tab's id, or zero.
 *
 * Parsed rather than tracked, for the one case where a tab arrives from
 * somewhere other than this list: the agent drives `chat:x:web:1` before the
 * pane is open, and the pane has to adopt that id rather than mint a new one -
 * or the person watches an empty tab while the page loads into another.
 */
export function serialOf(id) {
  const match = /:(\d+)$/.exec(String(id ?? ""));
  return match ? Number(match[1]) : 0;
}

/**
 * Which conversation a tab id belongs to, or null if it is not one of ours.
 *
 * Three shapes, because a job tab is keyed by the pid of the process it is
 * showing rather than by a tab number - the pid is what makes it findable
 * again from the side that started it.
 */
export function threadOfTab(id) {
  const match = /^chat:(.+):(?:web:\d+|sh:\d+|job:.+)$/.exec(String(id ?? ""));
  return match ? match[1] : null;
}

/** "web", "sh", "job", or null for an id this app did not make. */
export function kindOfTab(id) {
  const match = /^chat:.+:(web|sh|job):[^:]+$/.exec(String(id ?? ""));
  return match ? match[1] : null;
}

/**
 * Make sure a tab exists, without disturbing the ones that do.
 *
 * Returns the list unchanged when the id is already in it, so a reveal of the
 * tab you are already looking at re-renders nothing.
 */
export function adoptTab(tabs, id, kind) {
  const list = Array.isArray(tabs) ? tabs : [];
  const key = String(id ?? "");
  if (!key || list.some((tab) => tab.id === key)) return list;
  const serial = serialOf(key) || nextSerial(list);
  // The id knows better than the caller. A background process is revealed into
  // the terminal pane, so the kind passed is "sh" - and calling it "Terminal
  // 20412" would put a pid in a tab strip as though it were a tab number.
  return [...list, { id: key, serial, label: labelFor({ kind: kindOfTab(key) ?? kind, serial }) }];
}

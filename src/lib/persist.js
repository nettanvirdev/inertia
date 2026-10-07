import { useCallback, useEffect, useRef, useState } from "react";

/**
 * UI preference persistence.
 *
 * The webview's localStorage lives in the profile Tauri keeps in the app's own
 * data directory, so it survives quit/relaunch and lives per-install - the
 * same store `lib/theme.jsx` has always used for the theme. Keeping it here
 * rather than in the Rust backend means a preference read costs nothing at
 * mount: no IPC round-trip, no async gap where the UI renders the default and
 * then snaps to the stored value.
 *
 * Everything is namespaced under `inertia.ui.` so app data (should any land in
 * localStorage later) can never collide with a remembered toggle, and so
 * clearing preferences would be one prefix scan.
 */

const NS = "inertia.ui.";

/** Reads are total: a corrupt entry, a disabled store or a private window all
 *  fall back to the caller's default rather than throwing on the way up. */
export function readPref(key, fallback) {
  try {
    const raw = localStorage.getItem(NS + key);
    if (raw == null) return fallback;
    return JSON.parse(raw);
  } catch {
    return fallback;
  }
}

export function writePref(key, value) {
  try {
    if (value === undefined) localStorage.removeItem(NS + key);
    else localStorage.setItem(NS + key, JSON.stringify(value));
  } catch {
    /* quota, or storage disabled - a lost preference is never worth a crash */
  }
}

/**
 * A stored value is only as good as its validator. A remembered tab whose pane
 * has since been removed, or an agent id that no longer exists, would otherwise
 * render an empty screen on launch - so `isValid` gates the restore and the
 * caller's default wins whenever the stored value no longer makes sense.
 */
function restore(key, initial, isValid) {
  const fallback = typeof initial === "function" ? initial() : initial;
  // The shell's view is the one restore anything is allowed to overrule: a
  // person who has chosen a screen to open on means it more than the screen
  // they happened to close the window on. See `startupOverride`.
  if (key === PREF.view) {
    const forced = startupOverride(isValid);
    if (forced !== undefined) return forced;
  }
  const stored = readPref(key, undefined);
  if (stored === undefined) return fallback;
  if (isValid && !isValid(stored)) return fallback;
  return stored;
}

/**
 * The startup view, mirrored out of the user's preferences.
 *
 * The real setting lives in the profile inside the workspace folder, which is
 * read over IPC well after the shell has already decided what to render. A
 * preference that arrives late can only correct the screen after the fact, and
 * a window that opens on one view and jumps to another is worse than no setting
 * at all - so the settings pane writes a copy here, where a restore can read it
 * synchronously at the first render.
 *
 * `RESUME_LAST` is the mirror's way of saying "do not override": go back to
 * wherever the window was closed, which is what the app has always done.
 */
export const RESUME_LAST = "last";

function startupOverride(isValid) {
  const choice = readPref(PREF.startupView, RESUME_LAST);
  if (!choice || choice === RESUME_LAST) return undefined;
  // A view that has since been removed must not strand the window on a blank
  // screen, so the caller's own validator gates the override too.
  if (isValid && !isValid(choice)) return undefined;
  return choice;
}

/**
 * `useState` that remembers. Same signature, plus an optional validator.
 *
 *   const [layout, setLayout] = usePersistentState("library.layout", "grid",
 *     (v) => v === "grid" || v === "list");
 */
export function usePersistentState(key, initial, isValid) {
  // the validator is almost always an inline arrow - reading it through a ref
  // keeps it out of the restore path's dependencies
  const validRef = useRef(isValid);
  validRef.current = isValid;

  const [value, setValue] = useState(() => restore(key, initial, isValid));

  // Writing in an effect rather than inside the setter means a functional
  // update, a batched pair of updates and a re-render from above all persist
  // exactly the value that was rendered.
  useEffect(() => {
    writePref(key, value);
  }, [key, value]);

  return [value, setValue];
}

/**
 * The `usePersistentState` shape for a boolean, with a toggle. Used for the
 * collapsed/expanded state of a disclosure.
 */
export function usePersistentToggle(key, initial = false) {
  const [value, setValue] = usePersistentState(key, initial, (v) => typeof v === "boolean");
  const toggle = useCallback(() => setValue((v) => !v), [setValue]);
  return [value, setValue, toggle];
}

/** Keys live here so no two call sites can disagree about a spelling. */
export const PREF = {
  railExpanded: "rail.expanded",
  railRecentOpen: "rail.recent.open",
  railPinnedOpen: "rail.pinned.open",
  view: "shell.view",
  startupView: "shell.startup-view",
  defaultAgentId: "chat.default-agent",
  settingsTab: "settings.tab",
  chatPanelOpen: "chat.panel.open",
  chatPanelSections: "chat.panel.sections",
  libraryLayout: "library.layout",
  libraryTab: "library.tab",
  librarySort: "library.sort",
  libraryAgent: "library.agent",
  agentsLayout: "agents.layout",
  agentsStatus: "agents.status",
  memoryKind: "memory.kind",
  memorySort: "memory.sort",
  memoryAgent: "memory.agent",
  integrationsTab: "integrations.tab",
  integrationsLayout: "integrations.layout",
  secretsRevealed: "secrets.revealed",
  computersKind: "computers.kind",
  computersTab: "computers.detail.tab",
  activeComputerId: "computers.active",
  routinesStatus: "routines.status",
  routinesAgent: "routines.agent",
  activityCategory: "activity.category",
  activitySeverity: "activity.severity",
  activityAgent: "activity.agent",
  activityRange: "activity.range",
  /** Unfinished messages, per thread: `{ [threadId]: { text, at } }`. */
  composerDrafts: "chat.composer.drafts",
  activeThreadId: "chat.active",
};

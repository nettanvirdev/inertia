import * as React from "react";
import { readPref, writePref, PREF } from "@/lib/persist";

/**
 * The half-written message nobody sent yet.
 *
 * A draft lived in the composer's own `useState`, which meant it existed for
 * exactly as long as the composer was mounted. Switching to Computers and back
 * lost it. Clicking another conversation lost it. Closing the app lost it. In
 * every case the text was gone with no warning and no way back, and it is the
 * one piece of state in the app the user typed themselves.
 *
 * So it is kept per thread, in the same localStorage the rest of the UI's
 * preferences use - which in the desktop app is the webview's profile in the
 * app's data directory, so it survives a relaunch.
 *
 * ## Why writes are debounced
 *
 * Every keystroke would otherwise be a JSON serialise of every draft plus a
 * synchronous localStorage write, on the keypress path of the one control that
 * must never feel slow. A quarter of a second of idle is far below what anyone
 * notices and turns a hundred writes into one.
 *
 * The trade is that the last few characters can be lost to a hard kill. So the
 * pending write is also flushed when the window is hidden or closed, which is
 * what a quit, a reload and an alt-tab all go through.
 */

/** Drafts older than this are dropped: a month-old fragment is not a draft. */
const MAX_AGE_MS = 30 * 24 * 60 * 60 * 1000;

/** And no more than this many are kept, newest first. */
const MAX_DRAFTS = 200;

/** How long the composer goes quiet before the draft is written down. */
const WRITE_DELAY_MS = 250;

/** `{ [threadId]: { text, at } }`, validated on the way in. */
function readAll() {
  const stored = readPref(PREF.composerDrafts, {});
  if (!stored || typeof stored !== "object" || Array.isArray(stored)) return {};
  const out = {};
  for (const [id, entry] of Object.entries(stored)) {
    if (typeof entry?.text === "string" && entry.text)
      out[id] = { text: entry.text, at: entry.at ?? 0 };
  }
  return out;
}

/** Forget what is stale or surplus, so this cannot grow without bound. */
function prune(drafts, now = Date.now()) {
  const alive = Object.entries(drafts).filter(([, entry]) => now - (entry.at ?? 0) < MAX_AGE_MS);
  alive.sort((a, b) => (b[1].at ?? 0) - (a[1].at ?? 0));
  return Object.fromEntries(alive.slice(0, MAX_DRAFTS));
}

/**
 * Memory is the answer; storage is only how it survives a relaunch.
 *
 * Reads come from here, not from localStorage, and that is deliberate: a
 * private window, a full quota or a browser with site data disabled all make
 * the write fail, and in every one of those cases the draft should still be
 * on screen when the user comes back from the Computers tab. Losing it at the
 * end of the session is the price of storage being unavailable. Losing it on
 * the next keystroke is a bug.
 */
let cache = null;
let dirty = false;
let timer = null;

function all() {
  if (!cache) cache = readAll();
  return cache;
}

function flush() {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  if (!dirty) return;
  dirty = false;
  writePref(PREF.composerDrafts, cache ?? {});
}

/** Save (or clear, with an empty string) one thread's draft. */
export function saveDraft(threadId, text) {
  if (!threadId) return;
  const next = { ...all() };
  const trimmed = String(text ?? "");
  if (trimmed.trim()) next[threadId] = { text: trimmed, at: Date.now() };
  else delete next[threadId];

  cache = prune(next);
  dirty = true;
  if (timer) clearTimeout(timer);
  timer = setTimeout(flush, WRITE_DELAY_MS);
}

/** One thread's draft, or "". */
export function loadDraft(threadId) {
  if (!threadId) return "";
  return all()[threadId]?.text ?? "";
}

/** Drop a draft outright - the message was sent, or the thread was deleted. */
export function clearDraft(threadId) {
  saveDraft(threadId, "");
}

/** Every thread that has one, so a list can show which chats are unfinished. */
export function draftThreadIds() {
  return Object.keys(all());
}

/**
 * A thread's draft, as state.
 *
 * Reads synchronously on mount so the text is on screen in the first paint
 * rather than appearing a tick later, and follows the thread id so switching
 * conversations swaps drafts rather than carrying one across.
 */
export function useDraft(threadId) {
  const [text, setText] = React.useState(() => loadDraft(threadId));

  // A different conversation is a different draft. Not an effect on `text`,
  // which would fight the user's typing.
  const lastThread = React.useRef(threadId);
  if (lastThread.current !== threadId) {
    lastThread.current = threadId;
    // Safe during render: this is the "derive state from props" escape hatch,
    // and the alternative is one frame showing the previous chat's draft.
    setText(loadDraft(threadId));
  }

  const update = React.useCallback(
    (next) => {
      setText(next);
      saveDraft(threadId, next);
    },
    [threadId]
  );

  const clear = React.useCallback(() => {
    setText("");
    clearDraft(threadId);
  }, [threadId]);

  // The pending write, forced out before the window can go away with it.
  React.useEffect(() => {
    const onHide = () => {
      if (document.visibilityState === "hidden") flush();
    };
    window.addEventListener("beforeunload", flush);
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", onHide);
    return () => {
      window.removeEventListener("beforeunload", flush);
      window.removeEventListener("pagehide", flush);
      document.removeEventListener("visibilitychange", onHide);
      // Unmounting is itself a reason to write: the composer goes away when the
      // user navigates to another screen, which is exactly the case that used
      // to lose the text.
      flush();
    };
  }, []);

  return [text, update, clear];
}

export const _internals = { prune, readAll, MAX_AGE_MS, MAX_DRAFTS };

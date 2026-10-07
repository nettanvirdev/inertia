import * as React from "react";

/**
 * What the app has already asked the backend, kept so asking again is free.
 *
 * There are two caches in this window and they hold different things. The
 * workspace store holds everything that lives in the workspace folder - agents,
 * threads, routines, memories, settings documents - hydrated once when the app
 * opens and mirrored back on every change. That one is why the chat list is
 * instant.
 *
 * This is the other half: the answers to questions only the backend can
 * answer, because they are about this machine rather than about the folder.
 * Whether Docker is runnable. Whether the sandbox image is built. Which voice
 * keys are present. None of that is in the folder, so none of it was in the
 * store - and every screen that needed it fetched it again on mount, into
 * `useState(null)`, behind a spinner. Leaving a settings tab unmounts the
 * pane, throws the answers away, and coming back pays for all of it a second
 * time. That is the blink.
 *
 * So: one map for the whole window, outliving any component that reads it.
 * A screen that has been opened before renders from what is remembered, with
 * no spinner and no round trip, and a refresh goes out behind it only when the
 * answer is old enough to be worth doubting. Stale for a moment and instant is
 * the right trade for "is Docker installed"; it would be the wrong trade for a
 * bank balance, and nothing like that goes in here.
 *
 * The store below is deliberately plain and has no React in it, so it can be
 * tested as what it is: a map with an age on each entry.
 */

/** key -> { value, at } */
const entries = new Map();

/** key -> Set<listener>, so a write reaches every mounted reader. */
const listeners = new Map();

/** key -> Promise, so two panes mounting at once make one request. */
const inflight = new Map();

/** How old an answer may be before a reader refreshes it in the background. */
export const DEFAULT_MAX_AGE_MS = 30_000;

/** What is remembered for `key`, or undefined. */
export function peek(key) {
  return entries.get(key);
}

/** Remember a value, and tell everyone reading that key. */
export function write(key, value, { at = Date.now() } = {}) {
  entries.set(key, { value, at });
  for (const listener of listeners.get(key) ?? []) {
    try {
      listener(value);
    } catch {
      // A reader that throws is a component that unmounted mid-notify. It is
      // not a reason to stop telling the others.
    }
  }
  return value;
}

/** Forget one key, or everything. The next read fetches again. */
export function invalidate(key) {
  if (key === undefined) {
    entries.clear();
    inflight.clear();
    return;
  }
  entries.delete(key);
  inflight.delete(key);
}

export function subscribe(key, listener) {
  const set = listeners.get(key) ?? new Set();
  set.add(listener);
  listeners.set(key, set);
  return () => {
    set.delete(listener);
    if (!set.size) listeners.delete(key);
  };
}

/** Older than `maxAgeMs`, or never fetched at all. */
export function isStale(key, maxAgeMs = DEFAULT_MAX_AGE_MS) {
  const entry = entries.get(key);
  if (!entry) return true;
  return Date.now() - entry.at >= maxAgeMs;
}

/**
 * Fetch `key` unless the same fetch is already going, and remember the answer.
 *
 * Deduplication is the point of `inflight`: two panes mounting in the same
 * frame - or a pane mounting while a refresh is in the air - would otherwise
 * ask the backend the same question twice and race over which answer is
 * written last.
 */
export function load(key, loader) {
  const already = inflight.get(key);
  if (already) return already;
  const promise = Promise.resolve()
    .then(() => loader())
    .then((value) => {
      write(key, value);
      return value;
    })
    .finally(() => {
      if (inflight.get(key) === promise) inflight.delete(key);
    });
  inflight.set(key, promise);
  return promise;
}

/** Tests need an empty cache; nothing else should call this. */
export function _reset() {
  entries.clear();
  listeners.clear();
  inflight.clear();
}

/**
 * Read something the backend knows, from the cache first.
 *
 * `data` is whatever is remembered, available on the very first render, so a
 * screen that has been opened before never shows a spinner again. `loading` is
 * true only when there is genuinely nothing to show - the first time, which is
 * the one time a spinner is honest.
 *
 * `refresh` re-asks and updates every reader; `set` writes a value in without
 * asking, which is what a pane does after it saves something and already knows
 * the answer.
 */
export function useCached(key, loader, { maxAgeMs = DEFAULT_MAX_AGE_MS, enabled = true } = {}) {
  const [data, setData] = React.useState(() => peek(key)?.value);
  const [error, setError] = React.useState(null);
  // The loader is captured per render by callers who write it inline; the
  // cache key is the identity that matters, so the latest one is kept in a
  // ref rather than being made a dependency that refetches on every render.
  const latest = React.useRef(loader);
  latest.current = loader;

  React.useEffect(() => {
    if (!enabled) return undefined;
    const off = subscribe(key, (value) => {
      setData(value);
      setError(null);
    });
    // Whatever landed between the first render and this effect.
    const held = peek(key);
    if (held) setData(held.value);
    if (isStale(key, maxAgeMs)) {
      load(key, () => latest.current()).catch((cause) => setError(cause));
    }
    return off;
  }, [key, maxAgeMs, enabled]);

  const refresh = React.useCallback(() => {
    invalidate(key);
    return load(key, () => latest.current()).catch((cause) => {
      setError(cause);
      return undefined;
    });
  }, [key]);

  const set = React.useCallback((value) => write(key, value), [key]);

  return { data, error, loading: data === undefined && !error, refresh, set };
}

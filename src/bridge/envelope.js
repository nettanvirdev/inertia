import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/**
 * The shape the renderer was written against, over Tauri's transport.
 *
 * The renderer speaks Electron's preload contract: every call answers
 * `{ ok: true, data }` or `{ ok: false, error }`, and every subscription is a
 * synchronous function returning a synchronous unsubscribe. Tauri offers
 * neither - `invoke` resolves or throws, and `listen` returns a promise of an
 * unlisten function. This module is the entire difference between the two, kept
 * in one file so no feature has to know which shell it is running in.
 */

/** True when there is a Tauri runtime to talk to at all. */
export function hasTauri() {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

/**
 * One call, wrapped in the envelope.
 *
 * A throw becomes a value rather than propagating, because a renderer that has
 * to distinguish "empty list" from "the folder is gone" needs the failure to be
 * something it can read. The message is normalised here: Tauri rejects with a
 * string, an Error, or a serialised object depending on how the command failed,
 * and three shapes of failure reaching the UI would mean three ways to render
 * one sentence.
 */
export async function call(command, args) {
  try {
    return { ok: true, data: await tauriInvoke(command, args) };
  } catch (error) {
    return { ok: false, error: message(error) };
  }
}

/** Whatever came back, as a sentence. */
export function message(error) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object") {
    if (typeof error.message === "string") return error.message;
    if (typeof error.error === "string") return error.error;
    try {
      return JSON.stringify(error);
    } catch {
      /* fall through to the generic sentence */
    }
  }
  return "Something went wrong.";
}

/** A call whose failure should throw rather than be carried in an envelope. */
export async function raw(command, args) {
  return tauriInvoke(command, args);
}

/**
 * A Tauri event, as a synchronous subscription.
 *
 * `listen` is async, so an unsubscribe can be asked for before the listener
 * exists. Returning a function that closes over the still-pending promise -
 * rather than awaiting here - keeps the caller's effect cleanup synchronous,
 * and the `cancelled` flag covers the narrow case where a component mounts and
 * unmounts before the listener is attached. Without it that listener would be
 * attached after its own cleanup ran and would never be removed.
 */
export function subscribe(event, callback) {
  if (typeof callback !== "function") return () => {};

  let cancelled = false;
  let detach = null;

  const pending = listen(event, ({ payload }) => {
    if (!cancelled) callback(payload);
  })
    .then((unlisten) => {
      detach = unlisten;
      if (cancelled) unlisten();
      return unlisten;
    })
    .catch(() => () => {});

  return () => {
    cancelled = true;
    if (detach) detach();
    else pending.then((unlisten) => unlisten?.()).catch(() => {});
  };
}

/**
 * A namespace whose every method forwards to one command, in the envelope.
 *
 * `{ status: "ws_status" }` becomes `{ status: (...args) => Promise<envelope> }`.
 * Argument names matter to Tauri, so each entry may instead be a function that
 * maps positional arguments to the named object the command expects.
 */
export function forward(map) {
  const api = {};
  for (const [name, spec] of Object.entries(map)) {
    if (typeof spec === "function") {
      api[name] = (...args) => spec(...args);
    } else if (typeof spec === "string") {
      api[name] = () => call(spec);
    } else {
      const { command, args } = spec;
      api[name] = (...positional) => call(command, args ? args(...positional) : undefined);
    }
  }
  return api;
}

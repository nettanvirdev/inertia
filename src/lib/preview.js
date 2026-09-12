/**
 * The window's half of the browser pane.
 *
 * The page is a native view the main process parks over a rectangle of this
 * window, so nothing here renders it - this only says where the rectangle is,
 * where to go, and reads back what the page did. Everything returns the
 * bridge's `{ ok, data }` unwrapped, and everything is safe to call when there
 * is no bridge at all: the browser preview of this app has no main process,
 * and a pane that throws on import would take the whole screen with it.
 */

const api = () => (typeof window !== "undefined" ? window.previewAPI : null);

export function isPreviewAvailable() {
  return Boolean(api()?.open);
}

function unwrap(result) {
  if (!result) throw new Error("The browser bridge did not answer.");
  if (result.ok === false) throw new Error(result.error ?? "The browser pane failed.");
  return result.data ?? null;
}

const call = (name, ...args) => {
  const bridge = api();
  if (!bridge?.[name]) return Promise.reject(new Error("The browser pane is not available here."));
  return bridge[name](...args).then(unwrap);
};

export const preview = {
  open: (id, url) => call("open", id, url),
  navigate: (id, url) => call("navigate", id, url),
  back: (id) => call("history", id, "back"),
  forward: (id) => call("history", id, "forward"),
  reload: (id) => call("history", id, "reload"),
  stop: (id) => call("history", id, "stop"),
  /**
   * Where the pane is, and whether it should be drawn.
   *
   * Fire and forget, and deliberately so: this is called from a
   * ResizeObserver and on every scroll of the surrounding layout, and a
   * rejected promise nobody handles per frame would fill the console with
   * noise about a window that is simply closing.
   */
  place: (id, rect, visible) => {
    api()?.place?.(id, rect, visible)?.catch?.(() => {});
  },
  state: (id) => call("state", id),
  screenshot: (id) => call("screenshot", id),
  consoleLog: (id, options) => call("consoleLog", id, options),
  networkLog: (id, options) => call("networkLog", id, options),
  devTools: (id, open) => call("devTools", id, open),
  close: (id) => call("close", id),
  /** Browser profiles on this desktop, with how many cookies each holds. */
  cookieSources: () => call("cookieSources"),
  /** Copy one profile's cookies into the pane's shared session. */
  importCookies: (options) => call("importCookies", options),
  onEvent: (handler) => api()?.onEvent?.(handler) ?? (() => {}),
};

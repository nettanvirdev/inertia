/**
 * The window's half of the terminal pane.
 *
 * Output is pushed from the backend, so the interesting call here is
 * `onEvent`; everything else is a request. Safe to call with no bridge - the
 * browser preview of this app has no shell to lend and the pane says so rather
 * than throwing on import.
 */

const api = () => (typeof window !== "undefined" ? window.terminalAPI : null);

export function isTerminalAvailable() {
  return Boolean(api()?.open);
}

function unwrap(result) {
  if (!result) throw new Error("The terminal bridge did not answer.");
  if (result.ok === false) throw new Error(result.error ?? "The terminal failed.");
  return result.data ?? null;
}

const call = (name, ...args) => {
  const bridge = api();
  if (!bridge?.[name]) return Promise.reject(new Error("The terminal is not available here."));
  return bridge[name](...args).then(unwrap);
};

export const terminal = {
  capability: () => call("capability"),
  open: (id, options) => call("open", id, options),
  /**
   * One keystroke, or a pasted block. Fire and forget: this is called for
   * every character the person types, and a rejected promise nobody handles
   * per keypress would fill the console with noise about a pane that is
   * simply closing.
   */
  write: (id, data) => {
    api()?.write?.(id, data)?.catch?.(() => {});
  },
  resize: (id, cols, rows) => {
    api()?.resize?.(id, cols, rows)?.catch?.(() => {});
  },
  run: (id, command) => call("run", id, command),
  interrupt: (id) => call("interrupt", id),
  read: (id, options) => call("read", id, options),
  setCwd: (id, cwd) => call("setCwd", id, cwd),
  close: (id) => call("close", id),
  list: () => call("list"),
  onEvent: (handler) => api()?.onEvent?.(handler) ?? (() => {}),
};

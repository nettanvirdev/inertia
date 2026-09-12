/**
 * The window's view of lifecycle hooks.
 *
 * Read-mostly, like the bridge behind it: what is loaded, what ran, and the
 * two things a person can do from the pane without opening an editor. In a
 * browser tab there is no bridge and every call answers with nothing, so the
 * pane renders its explanation rather than an error.
 */

const api = () => (typeof window !== "undefined" ? window.hooksAPI : null);

export function isHooksAvailable() {
  return Boolean(api()?.list);
}

function unwrap(result) {
  if (!result) throw new Error("The hooks bridge did not answer.");
  if (result.ok === false) throw new Error(result.error ?? "The hooks bridge failed.");
  return result.data ?? result;
}

export const hooks = {
  list: (options) => api()?.list(options).then(unwrap) ?? Promise.resolve(null),
  recent: () => api()?.recent().then(unwrap) ?? Promise.resolve([]),
  writeExample: () => api()?.writeExample().then(unwrap),
  setEnabled: (enabled) => api()?.setEnabled(enabled).then(unwrap),
  reveal: () => api()?.reveal().then(unwrap),
  onRun: (callback) => api()?.onRun?.(callback) ?? (() => {}),
};

import { call, subscribe } from "./envelope";

/**
 * The hooks bridge: lifecycle hooks, over the backend.
 *
 * Read-mostly, like the pane in front of it. A hook is a program, and the
 * place to write a program is an editor, so the only two writes here are the
 * example file - for somebody starting from nothing - and the master switch.
 *
 * `onRun` is a live feed rather than a poll because a hook fires in the middle
 * of a turn, which is exactly when nobody is looking at the settings screen.
 * The backend keeps the last hundred runs so opening the pane afterwards still
 * shows what happened.
 */
export function hooksBridge() {
  return {
    /** `{ handlers, warnings, sources, events, path }` for one working folder. */
    list: (options) => call("hooks_list", { options: options ?? {} }),

    recent: () => call("hooks_recent"),

    /** Refuses to overwrite a file that exists: it is the user's own program. */
    writeExample: () => call("hooks_write_example"),

    setEnabled: (enabled) => call("hooks_set_enabled", { enabled: enabled !== false }),

    reveal: () => call("hooks_reveal"),

    onRun: (callback) => subscribe("hooks:run", callback),
  };
}

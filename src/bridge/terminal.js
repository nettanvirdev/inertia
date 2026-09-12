import { call, subscribe } from "./envelope";

/**
 * The terminal bridge: the shell beside the conversation.
 *
 * Output is pushed rather than polled - a build writes hundreds of lines and a
 * pane that asked for them on a timer would be a pane that stutters - so the
 * interesting method here is `onEvent`. Everything else is a plain request.
 *
 * `write` is the hot path and is deliberately the thinnest of these: every
 * character the person types is one call, so it does nothing but hand the bytes
 * on. `lib/terminal.js` is the half that swallows its rejection, because a
 * rejected promise per keypress on a pane that is simply closing would fill the
 * console with noise about nothing.
 *
 * The events are the same four shapes the renderer was written against:
 * `data` while a command runs, `cwd` when the shell moves, `exit` when it ends,
 * and `reveal` when a tool asks for a tab to be brought forward.
 */
export function terminalBridge() {
  return {
    capability: () => call("terminal_capability"),
    open: (id, options) => call("terminal_open", { id, options: options ?? null }),
    write: (id, data) => call("terminal_write", { id, data: String(data ?? "") }),
    resize: (id, cols, rows) => call("terminal_resize", { id, cols, rows }),
    run: (id, command) => call("terminal_run", { id, command }),
    interrupt: (id) => call("terminal_interrupt", { id }),
    read: (id, options) => call("terminal_read", { id, options: options ?? null }),
    setCwd: (id, cwd) => call("terminal_set_cwd", { id, cwd }),
    close: (id) => call("terminal_close", { id }),
    list: () => call("terminal_list"),

    onEvent: (callback) => subscribe("terminal:event", callback),
  };
}

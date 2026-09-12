import { call, subscribe } from "./envelope";

/**
 * The browser pane's half of the bridge.
 *
 * The pane the window draws is a hole: a measured, empty box that the backend
 * parks a native child webview over. So nothing here carries a page. What goes
 * down is where the hole is and where to go; what comes back is what the page
 * did, on one event channel the pane and `ChatView` both listen to.
 *
 * # Why `place` is the noisy one and must stay cheap
 *
 * `lib/preview.js` calls it from a ResizeObserver, from every scroll of the
 * surrounding layout, and from a 400ms backstop interval, for every open tab.
 * It is fire and forget on that side and it is one move on this one - no page
 * work, no awaits in the command behind it. Anything expensive added here is
 * paid several times a second for as long as the pane is open.
 *
 * # The two gaps, and why they answer rather than being missing
 *
 * `lib/preview.js` binds this namespace once and calls straight through it, so
 * a method that is absent is a TypeError in a pane rather than a message.
 *
 *   - `screenshot`. There is no capture API on a Tauri webview at all, and the
 *     ways to reach one on Windows mean new dependencies and COM plumbing that
 *     nothing in this workspace could test. `browser_read_page` is what the
 *     agent uses instead, and it is better for everything except questions
 *     that are genuinely about pixels.
 *   - `cookieSources` and `importCookies`. Lifting a signed-in session out of
 *     a browser profile on this desktop means reading a SQLite cookie store
 *     and decrypting it against the platform keychain - DPAPI on Windows, the
 *     login keyring elsewhere - and none of the crates that do either are in
 *     this workspace yet. Same answer, and for the same reason, as the
 *     computers bridge gives.
 *
 * `cookieSources` answers an empty list rather than a refusal on purpose: the
 * import dialog draws "no browser profiles" from an empty list, which is the
 * truth about what this build can see, and a rejection there would be an error
 * toast for a dialog the person opened to look around.
 */
const NOT_YET = (what) =>
  `${what} is not wired up in this build yet. The pane itself works - opening pages, reading them, the console and the network log - but this part has no backend.`;

export function previewBridge() {
  return {
    open: (id, url) => call("preview_open", { id, url: url ?? null }),
    navigate: (id, url) => call("preview_navigate", { id, url: String(url ?? "") }),
    history: (id, direction) => call("preview_history", { id, direction }),

    /*
     * `visible` is not the same question as "does the rect have a size". The
     * pane answers the first - this tab is in front, the dock is not being
     * dragged - and the backend answers the second, because a box being
     * animated open passes through sizes that are positive and unusable.
     */
    place: (id, rect, visible) =>
      call("preview_place", { id, rect: rect ?? {}, visible: Boolean(visible) }),

    state: (id) => call("preview_state", { id }),
    list: () => call("preview_list"),

    consoleLog: (id, options) => call("preview_console", { id, options: options ?? {} }),
    networkLog: (id, options) => call("preview_network", { id, options: options ?? {} }),

    devTools: (id, open) => call("preview_dev_tools", { id, open: open !== false }),
    close: (id) => call("preview_close", { id }),

    screenshot: async () => ({ ok: false, error: NOT_YET("Photographing the page") }),
    cookieSources: async () => ({ ok: true, data: [] }),
    importCookies: async () => ({ ok: false, error: NOT_YET("Importing cookies") }),

    onEvent: (callback) => subscribe("preview:event", callback),
  };
}

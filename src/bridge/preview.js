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
 *
 * `cookieSources` and `importCookies` are real, and they are the same two the
 * computers bridge offers - one source, two destinations. This one goes
 * through WebView2's own cookie manager rather than a SQLite file, which is
 * what lets an `httpOnly` session cookie arrive as one. Every pane shares the
 * store, so the import is for the pane and not for a tab.
 *
 * `cookieSources` answers an empty list rather than a refusal when there is
 * nothing to read: the dialog draws "no browser profiles" from an empty list,
 * and a rejection there would be an error toast for a dialog the person opened
 * to look around.
 */
const NOT_YET = (what) =>
  `${what} is not wired up in this build yet. The pane itself works - opening pages, reading them, the console and the network log - but this part has no backend.`;

/** Rises once per placement, for the whole window. See `place` below. */
let placements = 0;
function nextPlacement() {
  placements += 1;
  return placements;
}

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
      call("preview_place", {
        id,
        rect: rect ?? {},
        visible: Boolean(visible),
        // Rising, and compared in the app. These are separate commands
        // answered concurrently, so the order they are handled in is not the
        // order they were sent in - and the one that matters most is the last
        // one, the hide on the way out, which had nothing to correct it if it
        // lost. That is the white rectangle left over the transcript.
        seq: nextPlacement(),
      }),

    state: (id) => call("preview_state", { id }),
    list: () => call("preview_list"),

    consoleLog: (id, options) => call("preview_console", { id, options: options ?? {} }),
    networkLog: (id, options) => call("preview_network", { id, options: options ?? {} }),

    devTools: (id, open) => call("preview_dev_tools", { id, open: open !== false }),
    close: (id) => call("preview_close", { id }),

    screenshot: async () => ({ ok: false, error: NOT_YET("Photographing the page") }),
    cookieSources: () => call("cookie_sources"),
    importCookies: (options) => call("preview_import_cookies", { options: options ?? {} }),

    onEvent: (callback) => subscribe("preview:event", callback),
  };
}

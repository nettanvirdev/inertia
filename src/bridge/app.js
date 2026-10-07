import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { raw, message } from "./envelope";

/**
 * Window control and the small desktop errands.
 *
 * Named `electronAPI` because that is what the renderer calls it, and renaming
 * it would mean editing sixty files to no benefit. Nothing Electron remains
 * behind it.
 *
 * The window methods are fire-and-forget on purpose: the renderer sends them
 * and never awaits, so a rejected promise here would be an unhandled rejection
 * in the console on every close. Each one swallows its own failure.
 */

function appWindow() {
  // Resolved per call rather than once at module scope: `getCurrentWindow()`
  // reads metadata that only exists inside a Tauri webview, so calling it while
  // this module evaluates throws before React ever mounts - which is what the
  // whole app did when opened in a plain browser tab.
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

const ignore = () => {};

/**
 * Makes `.titlebar-drag` actually drag the window.
 *
 * The renderer marks its drag regions with `-webkit-app-region: drag`, which is
 * an Electron extension. WebView2 does not implement it and does not complain
 * about it either - the rule simply does nothing, which is why the titlebar
 * looked right and moved nothing.
 *
 * Honoured here rather than by editing the titlebar, because the class is the
 * renderer's contract and this is the shell's job to keep. One capture-phase
 * listener, so a region added later works without registering anything.
 */
function installDragRegions() {
  if (typeof document === "undefined") return;

  document.addEventListener(
    "mousedown",
    (event) => {
      // Left button only: a right-click on the titlebar belongs to the system
      // menu, and a middle-click drag is not a gesture anyone expects.
      if (event.button !== 0) return;
      // `detail > 1` is the second click of a double-click. Starting a drag
      // there swallows the `dblclick` the titlebar uses to maximise, so the
      // gesture has to be left alone to complete.
      if (event.detail > 1) return;

      const target = event.target;
      if (!(target instanceof Element)) return;
      if (!target.closest(".titlebar-drag")) return;
      // The window controls sit inside the drag region and opt out of it.
      if (target.closest(".titlebar-no-drag")) return;

      appWindow()?.startDragging().catch(ignore);
    },
    true
  );
}

export function appBridge() {
  installDragRegions();

  return {
    minimize: () => appWindow()?.minimize().catch(ignore),
    maximize: () =>
      appWindow()
        ?.toggleMaximize()
        .catch(ignore),
    close: () => appWindow()?.close().catch(ignore),
    beginDrag: () => appWindow()?.startDragging().catch(ignore),

    /**
     * The window's state, now and whenever it changes.
     *
     * A STRING - "normal" | "maximized" | "fullscreen" - because that is what
     * the titlebar compares against. Handing it `{ maximized: true }` left
     * `isMaximized` permanently false: the restore glyph never appeared, the
     * rounded top corners stayed on while the window was squared off against
     * the screen edge, and the drag branch for a maximised window never ran.
     *
     * Tauri has no single window-state event, so it is assembled from the
     * resize stream. The immediate first read matters: a window restored from a
     * maximised session would otherwise draw the wrong glyph until the user
     * touched the frame.
     */
    onWindowState: (callback) => {
      if (typeof callback !== "function") return () => {};
      const win = appWindow();
      if (!win) return () => {};

      let cancelled = false;
      let last = null;
      const report = async () => {
        try {
          const [maximized, fullscreen] = await Promise.all([
            win.isMaximized(),
            win.isFullscreen().catch(() => false),
          ]);
          const state = fullscreen ? "fullscreen" : maximized ? "maximized" : "normal";
          // Only on a change: the resize stream fires continuously while a
          // window is being dragged by its edge, and each one would otherwise
          // re-render the whole titlebar.
          if (!cancelled && state !== last) {
            last = state;
            callback(state);
          }
        } catch {
          /* the window is going away; nothing to report */
        }
      };
      report();

      let detach = null;
      const pending = win
        .onResized(report)
        .then((unlisten) => {
          detach = unlisten;
          if (cancelled) unlisten();
          return unlisten;
        })
        .catch(() => () => {});

      return () => {
        cancelled = true;
        if (detach) detach();
        else pending.then((unlisten) => unlisten?.()).catch(ignore);
      };
    },

    /*
     * These return plain values rather than the `{ ok, data }` envelope the
     * workspace bridge uses, because that is what the renderer reads:
     * `info?.version`, `result?.url`, `result?.saved`. Wrapping them would make
     * every one of those reads silently undefined.
     */

    getAppInfo: () => raw("app_info").catch(() => null),

    /** Only http and https. A `file:` here would be a way to run a program. */
    openExternal: async (url) => {
      const target = String(url ?? "");
      if (!/^https?:\/\//i.test(target)) return { ok: false, error: "Only web links can be opened." };
      try {
        await openUrl(target);
        return { ok: true };
      } catch (error) {
        return { ok: false, error: message(error) };
      }
    },

    showInFolder: async (filePath) => {
      try {
        await revealItemInDir(String(filePath ?? ""));
        return { ok: true };
      } catch (error) {
        return { ok: false, error: message(error) };
      }
    },

    /**
     * A picture from the web, fetched in the backend because the window's
     * content policy will not load one. `null` on failure - the caller draws a
     * broken-image state from that and needs no reason.
     */
    fetchImage: (url) => raw("app_fetch_image", { url: String(url ?? "") }).catch(() => null),

    /** A picture out of a message, onto the disk, where the person chooses. */
    saveImage: (request) =>
      raw("app_save_image", { request: request ?? {} }).catch((error) => ({
        saved: false,
        error: message(error),
      })),

    /** A command out of a code block, run because the person pressed run. */
    runSnippet: (request) =>
      raw("app_run_snippet", { request: request ?? {} }).catch((error) => ({
        ok: false,
        error: message(error),
      })),
    canRun: (lang) => raw("app_can_run", { lang: String(lang ?? "") }).catch(() => false),

    /** What to paste. Goes through the backend because `navigator.clipboard` needs
     *  a user gesture the paste handler does not always have. */
    readClipboard: () => raw("app_clipboard_read").catch(() => ""),

    /**
     * Window zoom, driven by the appearance settings.
     *
     * Tauri exposes this on the webview rather than the window, and it is sent
     * without being awaited, so it swallows its own failure like the window
     * controls above.
     */
    setZoom: (factor) => {
      try {
        getCurrentWebview().setZoom(Number(factor) || 1).catch(ignore);
      } catch {
        /* no webview: a browser tab, where the browser's own zoom applies */
      }
    },
  };
}

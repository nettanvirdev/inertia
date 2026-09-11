import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow, currentMonitor } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

/**
 * `getCurrentWindow()` reads metadata that only exists inside a Tauri webview,
 * and it used to be called as this module evaluated - so importing anything
 * that led here threw before React mounted, and `bun run dev` in a plain
 * browser rendered nothing at all. Resolving to null instead keeps the
 * frontend previewable on its own (the chrome simply does nothing there,
 * which is the honest behaviour when there is no window to drive).
 */
function currentWindowOrNull() {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

const appWindow = currentWindowOrNull();
type AppWindow = NonNullable<typeof appWindow>;

type Bounds = { x: number; y: number; width: number; height: number };
type WindowMode = "normal" | "maximized";

const RESIZE_MS = 220;

/** A few px of slop, because DPI scaling rarely divides evenly. */
const EDGE_SLOP = 2;

function sameBounds(a: Bounds, b: Bounds) {
  return (
    Math.abs(a.x - b.x) <= EDGE_SLOP &&
    Math.abs(a.y - b.y) <= EDGE_SLOP &&
    Math.abs(a.width - b.width) <= EDGE_SLOP &&
    Math.abs(a.height - b.height) <= EDGE_SLOP
  );
}

async function readBounds(win: AppWindow): Promise<Bounds> {
  const [position, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
  return { x: position.x, y: position.y, width: size.width, height: size.height };
}

async function readWorkArea(from: Bounds): Promise<Bounds> {
  const area = await invoke<[number, number, number, number] | null>("get_monitor_work_area");
  if (area) {
    const [x, y, width, height] = area;
    return { x, y, width, height };
  }
  // Non-Windows fallback: the current monitor's full bounds (no taskbar to avoid).
  const monitor = await currentMonitor();
  if (monitor) {
    return {
      x: monitor.position.x,
      y: monitor.position.y,
      width: monitor.size.width,
      height: monitor.size.height,
    };
  }
  return from;
}

/**
 * Drives the frameless window's chrome: current maximize state, and an
 * animated grow/shrink between the restored and maximized bounds.
 *
 * The animation itself is in Rust (`animate_window_to`), not here. Doing it
 * from JS meant two IPC calls per frame - setPosition then setSize - which
 * left the window at its new origin with its old size for an instant every
 * frame, and paced the tween by round-trip latency rather than by the clock.
 *
 * Just as importantly, nothing calls the native `maximize()` / `unmaximize()`
 * any more. Those snap instantly, and calling maximize() to "finish" a tween
 * re-snapped the window to the OS's own maximized rect - a few pixels away
 * from the work area we had just animated to, which is what made the last
 * moment of the animation jump. Here, "maximized" simply means the window
 * occupies the monitor's work area.
 */
export function useWindowChrome() {
  const [mode, setMode] = useState<WindowMode>("normal");
  const restoreBounds = useRef<Bounds | null>(null);
  const animating = useRef(false);

  useEffect(() => {
    if (!appWindow) return;
    const win = appWindow;

    let disposed = false;
    let unlisten: (() => void) | undefined;

    // Resizes we did not drive - an edge drag, or Windows' own Aero Snap -
    // still have to be reflected, or the titlebar glyph lies about the state.
    const sync = async () => {
      const [bounds, native] = await Promise.all([readBounds(win), win.isMaximized()]);
      const work = await readWorkArea(bounds);
      if (!disposed) setMode(native || sameBounds(bounds, work) ? "maximized" : "normal");
    };

    void sync();
    void win
      .onResized(() => {
        if (animating.current) return;
        void sync();
      })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const minimize = useCallback(() => {
    void appWindow?.minimize();
  }, []);

  const close = useCallback(() => {
    void appWindow?.close();
  }, []);

  const toggleMaximize = useCallback(async () => {
    if (animating.current || !appWindow) return;
    const win = appWindow;
    animating.current = true;
    try {
      const from = await readBounds(win);

      // Aero Snap can leave the window genuinely maximized. Animating out of
      // that state without clearing it first means Windows fights every
      // SetWindowPos, so drop the flag - and drop it while the window still
      // fills the screen, so there is nothing to see.
      if (await win.isMaximized()) {
        await win.unmaximize();
        await invoke("animate_window_to", { ...from, durationMs: 0 });
      }

      const to =
        mode === "maximized" ? (restoreBounds.current ?? from) : await readWorkArea(from);

      if (mode !== "maximized") restoreBounds.current = from;

      await invoke("animate_window_to", { ...to, durationMs: RESIZE_MS });
      setMode(mode === "maximized" ? "normal" : "maximized");
    } finally {
      animating.current = false;
    }
  }, [mode]);

  const beginDrag = useCallback(() => {
    void appWindow?.startDragging();
  }, []);

  return { mode, minimize, toggleMaximize, close, beginDrag };
}

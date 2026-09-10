import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow, currentMonitor, PhysicalPosition, PhysicalSize } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

const appWindow = getCurrentWindow();

type Bounds = { x: number; y: number; width: number; height: number };
type WindowMode = "normal" | "maximized";

/** Matches --ease-out from globals.css: fast off the trigger, no overshoot. */
function easeOut(t: number) {
  return 1 - Math.pow(1 - t, 3);
}

const RESIZE_MS = 220;

async function tweenBounds(from: Bounds, to: Bounds, duration: number) {
  const start = performance.now();
  await new Promise<void>((resolve) => {
    function frame(now: number) {
      const t = Math.min(1, (now - start) / duration);
      const e = easeOut(t);
      const x = Math.round(from.x + (to.x - from.x) * e);
      const y = Math.round(from.y + (to.y - from.y) * e);
      const width = Math.round(from.width + (to.width - from.width) * e);
      const height = Math.round(from.height + (to.height - from.height) * e);
      void appWindow.setPosition(new PhysicalPosition(x, y));
      void appWindow.setSize(new PhysicalSize(width, height));
      if (t < 1) requestAnimationFrame(frame);
      else resolve();
    }
    requestAnimationFrame(frame);
  });
}

async function readBounds(): Promise<Bounds> {
  const [position, size] = await Promise.all([appWindow.outerPosition(), appWindow.outerSize()]);
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
 * animated grow/shrink between the restored and maximized bounds instead of
 * the instant snap `Window.maximize()` gives on its own. Never calls the
 * native maximize/unmaximize mid-tween - Windows resists resizing a window
 * that is really flagged maximized, so the whole gesture is done as a plain
 * move + resize and the native call only finalizes the exact target bounds.
 */
export function useWindowChrome() {
  const [mode, setMode] = useState<WindowMode>("normal");
  const restoreBounds = useRef<Bounds | null>(null);
  const animating = useRef(false);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    (async () => {
      const isMax = await appWindow.isMaximized();
      if (!disposed) setMode(isMax ? "maximized" : "normal");

      unlisten = await appWindow.onResized(async () => {
        if (animating.current) return;
        const isMax = await appWindow.isMaximized();
        if (!disposed) setMode(isMax ? "maximized" : "normal");
      });
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const minimize = useCallback(() => {
    void appWindow.minimize();
  }, []);

  const close = useCallback(() => {
    void appWindow.close();
  }, []);

  const toggleMaximize = useCallback(async () => {
    if (animating.current) return;
    animating.current = true;
    try {
      const from = await readBounds();

      if (mode === "maximized") {
        await appWindow.unmaximize();
        const to = restoreBounds.current ?? from;
        await appWindow.setPosition(new PhysicalPosition(from.x, from.y));
        await appWindow.setSize(new PhysicalSize(from.width, from.height));
        await tweenBounds(from, to, RESIZE_MS);
        setMode("normal");
      } else {
        restoreBounds.current = from;
        const to = await readWorkArea(from);
        await tweenBounds(from, to, RESIZE_MS);
        await appWindow.setPosition(new PhysicalPosition(to.x, to.y));
        await appWindow.setSize(new PhysicalSize(to.width, to.height));
        await appWindow.maximize();
        setMode("maximized");
      }
    } finally {
      animating.current = false;
    }
  }, [mode]);

  const beginDrag = useCallback(() => {
    void appWindow.startDragging();
  }, []);

  return { mode, minimize, toggleMaximize, close, beginDrag };
}

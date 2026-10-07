import * as React from "react";
import { readPref, writePref } from "@/lib/persist";
import { setPaneDragging } from "@/features/chat/pane-drag";

/**
 * How wide the right-hand dock is, and dragging to change it.
 *
 * The width was a constant, which is the wrong answer for a column whose whole
 * job varies: a list of files wants a narrow one, a helper's transcript with
 * tool cards in it wants a wide one, and which of those is on screen is the
 * person's choice from one minute to the next.
 *
 * Remembered rather than reset per session - a size someone dragged to is a
 * decision, and asking them to make it again every launch is the kind of thing
 * that makes people stop dragging.
 */

const KEY = "dock.width";

/** Narrow enough to be worth having, wide enough to be worth reading. */
export const MIN_WIDTH = 260;
export const DEFAULT_WIDTH = 344;

/**
 * How much of the window the dock may take.
 *
 * This was a flat 720 pixels, which is a number that made sense next to a
 * conversation and no sense at all next to a browser: a page rendered at 720
 * is a phone, and somebody who opens a site in the dock wants to look at the
 * site. There is no reason for the app to hold an opinion about that - the
 * only thing it has to protect is that the conversation does not vanish, so
 * the ceiling is a share of the window rather than a constant.
 */
export const MAX_SHARE = 0.8;

/** The widest the dock may be in a window this wide. */
export function maxWidth(viewport) {
  const number = Number(viewport);
  if (!Number.isFinite(number) || number <= 0) return MIN_WIDTH;
  // Never below the minimum: in a window narrower than the minimum the two
  // would cross over, and a clamp whose bounds are inverted returns whichever
  // of them it happened to apply last.
  return Math.max(MIN_WIDTH, Math.round(number * MAX_SHARE));
}

export function clampWidth(value, viewport = viewportWidth()) {
  // `Number(null)` and `Number("")` are both 0, which is finite - so a
  // preference that was never written, or was written as null by an older
  // build, used to clamp to the minimum and open the dock at its narrowest.
  // Nothing is not a width; it is the absence of one, and the answer to that
  // is the default.
  const number = value === null || value === "" ? NaN : Number(value);
  if (!Number.isFinite(number)) {
    return Math.min(maxWidth(viewport), Math.max(MIN_WIDTH, DEFAULT_WIDTH));
  }
  return Math.min(maxWidth(viewport), Math.max(MIN_WIDTH, Math.round(number)));
}

/** The window, when there is one. Zero in a test, which `maxWidth` handles. */
function viewportWidth() {
  return typeof window === "undefined" ? 0 : window.innerWidth;
}

export function useDockWidth() {
  const [width, setWidth] = React.useState(() => clampWidth(readPref(KEY, DEFAULT_WIDTH)));
  const [dragging, setDragging] = React.useState(false);

  React.useEffect(() => {
    writePref(KEY, width);
  }, [width]);

  // A window that shrinks takes the ceiling down with it, and a dock left at
  // its old width would push the conversation off the side of its own app.
  React.useEffect(() => {
    const onResize = () => setWidth((current) => clampWidth(current));
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  /**
   * Drag from the handle on the dock's inner edge.
   *
   * Pointer events with capture, not mouse events: capture is what keeps the
   * drag alive when the pointer leaves the two-pixel handle, which it does
   * immediately - and without it a drag stopped the moment it started moving.
   *
   * The width is measured from the right edge of the window rather than
   * accumulated from a delta, so a drag that outruns the pointer for a frame
   * lands where the pointer is rather than a few pixels behind it for ever.
   */
  const onPointerDown = React.useCallback((event) => {
    if (event.button !== 0) return;
    const handle = event.currentTarget;
    try {
      handle.setPointerCapture?.(event.pointerId);
    } catch {
      // A pointer that has already gone. The listeners below still work.
    }
    setDragging(true);
    // The native panes get out of the way for the length of the drag, or the
    // pointer crosses onto a loaded page and this page never hears from it
    // again. See `pane-drag.js` - this is the reason that module exists.
    setPaneDragging(true);

    const move = (moved) => {
      setWidth(clampWidth(window.innerWidth - moved.clientX));
    };
    const stop = () => {
      setDragging(false);
      setPaneDragging(false);
      handle.releasePointerCapture?.(event.pointerId);
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", stop);
      handle.removeEventListener("pointercancel", stop);
    };

    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", stop);
    handle.addEventListener("pointercancel", stop);
  }, []);

  /** Keyboard, because a drag handle nobody can reach is not a control. */
  const onKeyDown = React.useCallback((event) => {
    const step = event.shiftKey ? 48 : 16;
    if (event.key === "ArrowLeft") setWidth((w) => clampWidth(w + step));
    else if (event.key === "ArrowRight") setWidth((w) => clampWidth(w - step));
    else if (event.key === "Home") setWidth(DEFAULT_WIDTH);
    else return;
    event.preventDefault();
  }, []);

  return { width, dragging, onPointerDown, onKeyDown, setWidth };
}

/*
 * There was a `useDockSplit` here, for sharing the dock's height between two
 * stacked sections. The dock holds tabs now - see SideDock for why five panes
 * cannot be stacked - so nothing splits a height any more, and a hook nobody
 * calls is a hook that rots. It is in the history if a split ever comes back.
 */

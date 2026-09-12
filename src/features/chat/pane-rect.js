/**
 * Turning a measured box into something that survives the bridge.
 *
 * This is four lines and it had a bug in it that cost the whole browser pane,
 * so it is worth its own file and its own tests.
 *
 * `getBoundingClientRect()` returns a `DOMRect`. Its `x`, `y`, `width` and
 * `height` are accessors on the prototype, not own properties of the object -
 * and Electron's context bridge does not clone host objects. What arrived in
 * the main process was `{}`. Bounds of `{}` clamp to zero, a zero-sized view
 * is treated as one that is off screen, and the browser was therefore created,
 * navigated, and never once made visible. Every symptom pointed at loading:
 * the page was loading perfectly and being drawn nowhere.
 *
 * The lesson generalises, which is why this is a named function rather than an
 * object literal at the call site: anything crossing the bridge has to be
 * plain data, and a DOM object that looks like plain data is the easiest way
 * to forget that.
 */

/** A DOMRect as four numbers the bridge can carry. */
export function rectOf(rect) {
  return {
    x: Number(rect?.x ?? rect?.left ?? 0) || 0,
    y: Number(rect?.y ?? rect?.top ?? 0) || 0,
    width: Number(rect?.width ?? 0) || 0,
    height: Number(rect?.height ?? 0) || 0,
  };
}

/**
 * Whether a box is big enough to put a browser in.
 *
 * A pane that is collapsed, behind another tab, or mid-animation measures to
 * nothing or nearly nothing, and a view parked at four pixels in the corner of
 * the window is a bug people photograph. The threshold is deliberately larger
 * than one: a `display: none` element measures exactly zero, but an element
 * being animated open passes through sizes that are technically positive and
 * visibly wrong.
 */
export function isShowable(rect) {
  const box = rectOf(rect);
  return box.width > 8 && box.height > 8;
}

/**
 * Telling the native panes that the dock is being dragged.
 *
 * A browser in this app is a Chromium view the main process parks over a hole
 * in the window. It is not in the DOM; it is above it - above every pixel,
 * including the two-pixel handle you drag the dock's edge with. So the moment
 * a drag moves the pointer over a loaded page, the renderer stops receiving
 * pointer events entirely and the drag stalls with the column half-resized.
 *
 * That is not a bug in the drag. It is what a native child view is: the
 * operating system routes the mouse to it, and no amount of `z-index`,
 * `pointer-events` or pointer capture in a document reaches above a window
 * that is not part of that document.
 *
 * The fix is to get the view out of the way for the length of the drag, which
 * needs one bit of state shared between a hook in the dock and a component two
 * levels away that does not otherwise know it exists. Hence a module: not a
 * context, because this is one boolean read by whoever happens to be mounted,
 * and threading a provider through the tree for it would be the larger change.
 *
 * The side effect is that a page does not reflow live while you drag. That is
 * the trade every editor with a native preview makes, and it is the right way
 * round: a drag that finishes is worth more than a page that resizes smoothly
 * during one that does not.
 */

let dragging = false;
const listeners = new Set();

/** Whether a pane edge is being dragged right now. */
export function isPaneDragging() {
  return dragging;
}

/** Called by whatever owns the drag, at both ends of it. */
export function setPaneDragging(next) {
  const value = Boolean(next);
  if (value === dragging) return;
  dragging = value;
  for (const listener of [...listeners]) {
    try {
      listener(value);
    } catch {
      // One pane that throws while being told must not stop the others from
      // being told - the ones left unhidden would swallow the rest of the drag.
    }
  }
}

/** Subscribe. Returns the unsubscribe, so an effect can return it directly. */
export function onPaneDrag(listener) {
  if (typeof listener !== "function") return () => {};
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Test seam: no component should need this, and a leaked listener is a leak. */
export function resetPaneDrag() {
  dragging = false;
  listeners.clear();
}

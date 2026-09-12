import * as React from "react";

/**
 * Whether a thing that just mounted is new to the person, or only to React.
 *
 * A message rising into the transcript is the right motion for a message that
 * has just been sent. It is the wrong motion for the two hundred a restored
 * conversation opens with, for the thirty an "older messages" click reveals,
 * and for the same message mounting again after a switch to another thread
 * and back - React mounts all of those exactly the way it mounts the new one,
 * and a component cannot tell the difference by looking at its own props.
 *
 * So the difference is decided here, from two facts a component does have:
 *
 *   · when the thing was made. Anything made before this module was loaded
 *     was made before anyone was watching this window, and does not arrive;
 *     it is already there. That is what stops a restored transcript, and it
 *     costs one comparison.
 *   · whether it is still happening. A tool call has no timestamp, but one
 *     that is running when its card mounts is one appearing mid-reply, and a
 *     finished one being mounted is history scrolling into view.
 *
 * And one thing kept here: which ids have already been shown. Either fact
 * says "new" the first time and would go on saying it on every remount, so an
 * id is remembered once it has been decided, and a second mount of the same
 * message is never a second arrival. Bounded, because it is keyed on every
 * message and tool call ever drawn.
 */

const seen = new Set();
const SEEN_CAP = 4000;

/** When this window started watching. Nothing made before it arrives. */
let watchingSince = Date.now();

/**
 * Decide, once, for one id.
 *
 * @param {string|null} id  what is mounting; null for something with no
 *   identity, which is decided on the evidence alone and never remembered.
 * @param {{ at?: number, live?: boolean }} evidence  when it was made, and
 *   whether it is still in progress.
 */
export function isArrival(id, { at, live = false } = {}) {
  const key = id == null ? null : String(id);
  const known = key !== null && seen.has(key);
  if (key !== null) {
    seen.add(key);
    while (seen.size > SEEN_CAP) seen.delete(seen.values().next().value);
  }
  if (known) return false;
  if (live) return true;
  const made = Number(at);
  return Number.isFinite(made) && made >= watchingSince;
}

/** Forget everything, and start watching from now. For tests. */
export function resetArrivals(now = Date.now()) {
  seen.clear();
  watchingSince = now;
}

/**
 * The decision, held for the life of one mounted component.
 *
 * Taken in the state initialiser so it is made exactly once, at mount, and a
 * streaming reply re-rendering on every token does not re-ask and does not
 * restart its own entrance. `onAnimationEnd` drops the class once the motion
 * is over: the entrance utilities carry `will-change`, and a transcript must
 * not keep a promoted layer for every message that ever arrived. Only the
 * element's own animation counts - a child's fade bubbling up must not end
 * the parent's early.
 */
export function useArrival(id, evidence) {
  const [arriving, setArriving] = React.useState(() => isArrival(id, evidence));
  const onAnimationEnd = React.useCallback((event) => {
    if (event.target === event.currentTarget) setArriving(false);
  }, []);
  return { arriving, onAnimationEnd: arriving ? onAnimationEnd : undefined };
}

/**
 * A cross-fade for content that is replaced rather than added.
 *
 * Tabs switching, a button giving way to a different button, a result giving
 * way to its raw form. Nothing on the first render - the thing is simply
 * there - and a fade every time `key` changes after that, cleared the same
 * way as an arrival once it has played. Pair it with `key={key}` on the
 * element so the browser sees a new node and starts the animation.
 */
export function useSwap(key) {
  const [last, setLast] = React.useState(key);
  const [swapping, setSwapping] = React.useState(false);
  if (last !== key) {
    setLast(key);
    setSwapping(true);
  }
  const onAnimationEnd = React.useCallback((event) => {
    if (event.target === event.currentTarget) setSwapping(false);
  }, []);
  return { swapping, onAnimationEnd: swapping ? onAnimationEnd : undefined };
}

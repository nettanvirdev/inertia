/**
 * The app's motion vocabulary, in one place.
 *
 * Every duration a component times in JavaScript - how long to keep a closing
 * menu mounted, when a collapsed section may be unmounted - has to agree with
 * the duration the stylesheet animates it over, or the element is torn out of
 * the page a frame before its fade ends, and the eye reads the last frame as a
 * flicker. These numbers are written down here and echoed as custom properties
 * in `globals.css` (`--motion-*`), and a test holds the two in agreement.
 *
 * Three rules, and every animation in the app follows them:
 *
 *   · Move only what the compositor can move alone. `transform` and `opacity`
 *     are animated on the GPU without a layout or a paint; a height, a margin
 *     or a colour is resolved on the main thread every frame, and on a page
 *     that is also streaming a reply that is the difference between smooth and
 *     not. Where a height genuinely has to change - a section opening - it is
 *     done with `grid-template-rows`, which is one layout per frame rather than
 *     a paint of everything underneath it.
 *
 *   · Decelerate. A thing appearing starts fast and lands softly, so it reads
 *     as having arrived rather than as having been placed. Leaving is shorter
 *     than arriving: the person has already decided, and there is nothing to
 *     look at on the way out.
 *
 *   · Reduced motion is a real setting, not a shorter animation. When it is on,
 *     nothing travels and nothing scales; opacity may still fade, briefly, so a
 *     change never appears without warning. See `prefersReducedMotion`.
 */

/** Milliseconds. Keep in step with `--motion-*` in globals.css. */
export const MOTION = Object.freeze({
  /** A hover, a press, a check mark: felt, not watched. */
  instant: 80,
  /** A tooltip, a switch, a chevron turning. */
  fast: 140,
  /** A menu or a popover arriving. The default. */
  base: 200,
  /** A section folding open or closed. */
  collapse: 240,
  /** A column or a panel changing width. */
  panel: 260,
  /** A sheet sliding in from an edge, a dialog settling. */
  slow: 300,
  /** How long a leaving element stays mounted. Shorter than arriving. */
  exit: 150,
  /** The fade under reduced motion: enough that nothing pops. */
  reduced: 100,
});

/**
 * The stylesheet's easing curves, by name.
 *
 * `out` is the curve every arrival uses: nearly all of the travel happens in
 * the first third, so a 200ms menu feels like it snapped to the pointer and
 * then settled, rather than like it slid. `spring` overshoots by a hair and is
 * only for things that toggle - a switch, a check - where the overshoot is the
 * click.
 */
export const EASE = Object.freeze({
  out: "cubic-bezier(0.16, 1, 0.3, 1)",
  inOut: "cubic-bezier(0.65, 0, 0.35, 1)",
  in: "cubic-bezier(0.55, 0, 1, 0.45)",
  spring: "cubic-bezier(0.34, 1.4, 0.64, 1)",
});

/**
 * Whether motion should be reduced right now.
 *
 * Two sources, either one enough: the app's own setting, which `appearance.js`
 * publishes on the root element as `data-reduce-motion`, and the operating
 * system's, which the media query reports. The stylesheet honours both with
 * the same rule; this is the same answer for code that has to time something.
 *
 * Takes the document rather than reaching for the global so it can be asked
 * about any document - including a fake one in a test.
 */
export function prefersReducedMotion(doc = typeof document === "undefined" ? null : document) {
  if (!doc) return false;
  if (doc.documentElement?.dataset?.reduceMotion === "true") return true;
  const view = doc.defaultView;
  try {
    return Boolean(view?.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches);
  } catch {
    return false;
  }
}

/**
 * How long a leaving element should stay mounted.
 *
 * The one number every presence hook needs: the exit duration, or the short
 * reduced-motion fade when motion is off. Never zero - an element removed on
 * the same frame its `opacity` transition starts is removed before the
 * transition has a first frame, and a zero-length timer in React also reorders
 * against the state update that started it.
 */
export function exitDuration(requested = MOTION.exit, reduced = prefersReducedMotion()) {
  const ms = Number(requested);
  if (!Number.isFinite(ms) || ms <= 0) return reduced ? MOTION.reduced : MOTION.exit;
  return reduced ? Math.min(ms, MOTION.reduced) : ms;
}

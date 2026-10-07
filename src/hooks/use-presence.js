import * as React from "react";
import { MOTION, exitDuration } from "@/lib/motion";

/**
 * Keeping an element on the page long enough to watch it leave.
 *
 * React unmounts on the render that says `open = false`, which is the frame
 * before any exit animation could start - so every overlay in the app used to
 * arrive with a small choreography and leave by ceasing to exist. This holds
 * the element for one more beat and tells it which way it is going.
 *
 *   const { mounted, state } = usePresence(open);
 *   if (!mounted) return null;
 *   return <div data-state={state}>…</div>;
 *
 * `state` is one of:
 *
 *   "entering"  the first frame after opening. Styles that start an arrival
 *               from here transition to "open" on the next frame - which is
 *               how a grid-row or opacity transition is given a from-value.
 *   "open"      settled.
 *   "closing"   on its way out; `mounted` goes false when the exit is over.
 *
 * An element that is open when it first mounts starts at "open" with no
 * entrance: a transcript restored with twenty cards already unfolded should
 * not unfold twenty cards. The entrance is for the edge - open going from
 * false to true while the person is looking.
 *
 * The exit is a timer rather than `animationend`, and on purpose: an element
 * whose exit animation was removed by reduced motion, or by a class the caller
 * forgot, would never fire the event and would stay on the page for ever. A
 * timer always ends. Its length comes from `exitDuration`, so under reduced
 * motion the element goes on the short fade rather than the full exit.
 *
 * Reopening mid-exit is the case that used to produce two menus: the timer is
 * cancelled and the state goes back to "open", with nothing remounted.
 */
export function usePresence(open, { exit = MOTION.exit } = {}) {
  const [mounted, setMounted] = React.useState(Boolean(open));
  const [state, setState] = React.useState(open ? "open" : "closed");
  const timer = React.useRef(null);
  const frame = React.useRef(null);
  // The previous value of `open`, so the effect reacts to its edges and not to
  // its level. StrictMode runs every effect twice on mount, and a version
  // keyed on "is this the first run" animated an entrance on the second.
  const was = React.useRef(Boolean(open));

  React.useEffect(() => {
    const before = was.current;
    was.current = Boolean(open);
    clearTimeout(timer.current);
    cancelAnimationFrame(frame.current);

    if (open) {
      setMounted(true);
      if (before) {
        // Already open - a remount, or a reopen that caught the exit in time.
        setState("open");
        return;
      }
      // Two frames, not one: the first paints the element in its "entering"
      // styles, the second flips it to "open" so the transition has a start
      // and an end. A single frame collapses both into the same style
      // recalculation and nothing animates.
      setState("entering");
      frame.current = requestAnimationFrame(() => {
        frame.current = requestAnimationFrame(() => setState("open"));
      });
      return;
    }

    if (!before) return;
    setState("closing");
    timer.current = setTimeout(() => {
      setMounted(false);
      setState("closed");
    }, exitDuration(exit));
  }, [open, exit]);

  React.useEffect(
    () => () => {
      clearTimeout(timer.current);
      cancelAnimationFrame(frame.current);
    },
    []
  );

  return { mounted, state, closing: state === "closing" };
}

import * as React from "react";
import { cn } from "@/lib/utils";
import { usePresence } from "@/hooks/use-presence";
import { MOTION } from "@/lib/motion";

/**
 * A section that folds open and closed.
 *
 * Every disclosure in the app - a tool card's body, a thought, a thread list
 * in the rail, a settings group - was `{open ? <div/> : null}`, which is a
 * section that exists or does not with nothing in between. Folding is the one
 * motion a person reads as "the same thing, shown more of", and without it a
 * card opening is indistinguishable from a card being replaced.
 *
 * ── How the height moves ────────────────────────────────────────────────────
 * `height: auto` cannot be transitioned, and measuring it in JavaScript is a
 * layout read on every toggle plus a stale number the moment the content
 * changes. A grid with one row can: `grid-template-rows` interpolates between
 * `0fr` and `1fr`, and the row's own content decides what `1fr` means, today
 * and after the body has streamed in three more lines. One layout per frame,
 * no measuring, no ResizeObserver. The rule itself is `.collapse-fold` in
 * globals.css, beside the reduced-motion override that turns it into a fade.
 *
 * The inner box is `min-height: 0` so it can actually shrink below its
 * content, and `min-width: 0` so its content can still truncate: a grid item's
 * minimum size is otherwise its content's, and a long thread title in the rail
 * made the row grow to fit it and pushed the pin off the clipped edge. It is
 * `overflow: hidden` only while moving - a settled open section lets a focus
 * ring or a popover's shadow reach past its edge like any other box.
 *
 * ── What is mounted when ────────────────────────────────────────────────────
 * Children are mounted while open and through the closing fold, and unmounted
 * after it - the same memory behaviour the ternary had. A transcript with
 * forty tool cards, each holding a diff, does not keep forty diffs in the tree
 * because they could be opened. `keepMounted` keeps them for a section whose
 * children hold state worth preserving across a fold, such as a form.
 */
export const Collapse = React.forwardRef(function Collapse(
  { open, children, className, innerClassName, keepMounted = false, as: Tag = "div", ...props },
  ref
) {
  const { mounted, state } = usePresence(open, { exit: MOTION.collapse });
  if (!mounted && !keepMounted) return null;

  const settled = state === "open";
  const closed = state === "closed";

  return (
    <Tag
      ref={ref}
      data-state={state}
      aria-hidden={closed || undefined}
      className={cn("collapse-fold", className)}
      {...props}
    >
      <div
        className={cn("min-h-0 min-w-0", !settled && "overflow-hidden", innerClassName)}
        // A closed section that is kept mounted must not be reachable by Tab
        // or a screen reader, or a person can land inside a box of zero height.
        inert={closed || undefined}
      >
        {children}
      </div>
    </Tag>
  );
});

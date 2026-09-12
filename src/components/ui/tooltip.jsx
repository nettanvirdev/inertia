import * as React from "react";
import { cn } from "@/lib/utils";
import { Portal } from "@/components/ui/portal";
import { useAnchoredPosition } from "@/components/ui/use-anchored-position";
import { usePresence } from "@/hooks/use-presence";
import { MOTION } from "@/lib/motion";

/**
 * Clones its single child as the anchor. There is no `title=` fallback
 * anywhere in this app - this is the only tooltip.
 *
 * TooltipDelayGroup is intentionally not shipped: a shared "already warm"
 * timer is not trivial to get right and nothing needs it yet.
 */
/**
 * Whether something above has already put a tooltip on this element.
 *
 * `IconButton` shows its own label on hover, which is what an icon with no
 * words needs. Sixteen call sites already wrapped one in a `Tooltip` to say
 * something better than the label, and without this they would now show two.
 * A context rather than a prop, because the call site that wrapped it should
 * not have to know that the thing it wrapped grew a tooltip of its own.
 *
 * No DOM: a provider renders nothing, so the anchor cloning below is untouched.
 */
export const InsideTooltip = React.createContext(false);

export function Tooltip({
  content,
  side = "top",
  align = "center",
  delay = 400,
  children,
  className,
  disabled,
}) {
  const anchorRef = React.useRef(null);
  const timer = React.useRef(null);
  const [open, setOpen] = React.useState(false);
  const id = React.useId();

  // A short hold so it fades out rather than blinks. Shorter than the other
  // overlays' exit: a tooltip that lingers over the next row you hover is a
  // tooltip pointing at the wrong thing.
  const { mounted, state } = usePresence(open, { exit: MOTION.instant });
  const { style, placedSide, floatingRef } = useAnchoredPosition({
    anchorRef,
    open: mounted,
    side,
    align,
    offset: 6,
  });

  const clear = () => {
    clearTimeout(timer.current);
    timer.current = null;
  };

  React.useEffect(() => clear, []);

  const show = (immediate = false) => {
    if (disabled || !content) return;
    clear();
    if (immediate) setOpen(true);
    else timer.current = setTimeout(() => setOpen(true), delay);
  };

  const hide = () => {
    clear();
    setOpen(false);
  };

  // A tooltip must not swallow Escape from the dialog or menu underneath it,
  // so it listens passively instead of joining the escape ladder.
  React.useEffect(() => {
    if (!open) return;
    const onKeyDown = (e) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [open]);

  const child = React.Children.only(children);
  const childRef = child.props?.ref ?? child.ref; // React 19 puts ref on props

  const setRef = React.useCallback(
    (node) => {
      anchorRef.current = node;
      if (typeof childRef === "function") childRef(node);
      else if (childRef && typeof childRef === "object") childRef.current = node;
    },
    [childRef]
  );

  const chain = (theirs, ours) => (e) => {
    theirs?.(e);
    ours(e);
  };

  const anchor = React.cloneElement(child, {
    ref: setRef,
    "aria-describedby": open ? id : child.props["aria-describedby"],
    onPointerEnter: chain(child.props.onPointerEnter, () => show()),
    onPointerLeave: chain(child.props.onPointerLeave, hide),
    onPointerDown: chain(child.props.onPointerDown, hide),
    onFocus: chain(child.props.onFocus, () => show(true)),
    onBlur: chain(child.props.onBlur, hide),
  });

  return (
    <InsideTooltip.Provider value={true}>
      {anchor}
      {mounted && content ? (
        <Portal>
          <div
            ref={floatingRef}
            id={id}
            role="tooltip"
            data-side={placedSide}
            data-state={state}
            style={style}
            className={cn(
              "pointer-events-none z-50 flex h-7 w-fit items-center text-balance",
              "rounded-md bg-foreground px-2 text-xs text-background",
              // A fade, not the anchored-sheet choreography the other overlays
              // use. Two reasons, and the second is the one that matters.
              //
              // A tooltip is seven pixels tall and already sits beside what it
              // describes, so travelling a few pixels out of the trigger says
              // nothing a fade does not - the movement is for panels that need
              // to be tied to where they came from.
              //
              // And this is the overlay that mounts and unmounts more than all
              // the others put together: every hover over every row in a
              // transcript, hundreds of times a session, often while React is
              // rebuilding the thing underneath. That animation interpolates a
              // transform whose value comes from custom properties, which is a
              // path that cannot be composited and has to be resolved on the
              // main thread every frame - and a substituted transform of
              // exactly that shape was sitting on the stack of the renderer
              // when it died. That is a lead rather than a proof, but a fade
              // is not a compromise here, so there is nothing to weigh.
              "animate-fade-in",
              className
            )}
          >
            {content}
          </div>
        </Portal>
      ) : null}
    </InsideTooltip.Provider>
  );
}

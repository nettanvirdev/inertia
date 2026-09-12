import * as React from "react";
import { cn } from "@/lib/utils";

const Textarea = React.forwardRef(function Textarea(
  { className, autoResize = false, maxRows, rows = 3, value, onChange, ...props },
  ref
) {
  const innerRef = React.useRef(null);
  React.useImperativeHandle(ref, () => innerRef.current, []);

  // Fit the box to its text.
  //
  // Measured from a COLLAPSED height, not from `auto`: a textarea left to size
  // itself inside a flex column reports the space it was given, not the space
  // its text needs, and the box then snaps to maxRows the moment the effect
  // runs early.
  const fit = React.useCallback(() => {
    const el = innerRef.current;
    if (!el) return;
    el.style.height = "0px";
    const styles = window.getComputedStyle(el);
    const lineHeight = parseFloat(styles.lineHeight) || 20;
    const padding =
      parseFloat(styles.paddingTop) + parseFloat(styles.paddingBottom) || 0;
    const next = maxRows
      ? Math.min(el.scrollHeight, lineHeight * maxRows + padding)
      : el.scrollHeight;
    el.style.height = `${next}px`;
    el.style.overflowY = el.scrollHeight > next ? "auto" : "hidden";
  }, [maxRows]);

  // Before paint, so the box never flashes at the wrong size on a keystroke.
  React.useLayoutEffect(() => {
    if (autoResize) fit();
  }, [autoResize, fit, value]);

  // …and again whenever the control's own width changes. That covers the two
  // cases a value-keyed effect cannot see: a window resize reflowing the text,
  // and first mount, where the measurement can land before the layout and the
  // responsive type size have settled and so reads a width that is not real.
  React.useEffect(() => {
    const el = innerRef.current;
    if (!el || !autoResize || typeof ResizeObserver === "undefined") return undefined;
    // Width only: `fit` sets the height, so reacting to height would be a loop.
    let lastWidth = el.clientWidth;
    const observer = new ResizeObserver(() => {
      if (el.clientWidth === lastWidth) return;
      lastWidth = el.clientWidth;
      fit();
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [autoResize, fit]);

  return (
    <textarea
      ref={innerRef}
      rows={rows}
      value={value}
      onChange={onChange}
      className={cn(
        "w-full resize-none rounded-lg px-3 py-2 text-[13px] text-foreground",
        // The input ladder, not the control one. A textarea is a field, and it
        // was sitting two rungs below the single-line field beside it in the
        // same form - close enough to white to disappear, and visibly not the
        // same control as its neighbour.
        "bg-input outline-none transition-colors duration-150 ease-out",
        "hover:bg-input-hover",
        "focus-visible:bg-input-focus",
        "placeholder:text-muted-foreground",
        "selection:bg-foreground selection:text-background",
        "disabled:pointer-events-none disabled:opacity-50",
        "no-scrollbar",
        className
      )}
      {...props}
    />
  );
});

export { Textarea };

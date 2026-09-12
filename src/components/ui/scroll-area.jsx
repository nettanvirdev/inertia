import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * Scrollbars are a seam in a borderless UI, so they are hidden globally. `fade`
 * replaces the missing "there is more" signal with a mask at the edges - a
 * non-colour gradient, which paints nothing and only clips.
 *
 * The mask is applied per edge and only when that edge actually has content
 * hidden behind it. A static mask fades the top of a list that is already
 * scrolled to the top, which means the first row is dimmed for no reason and
 * anything painted there - a hover fill, a sticky heading - appears to dissolve
 * upward. The signal has to be a fact about the scroll position, not decoration.
 */
const FADE_TOP = 12;
const FADE_BOTTOM = 16;

function maskFor(top, bottom) {
  if (!top && !bottom) return undefined;
  const head = top ? `transparent 0, black ${FADE_TOP}px` : "black 0";
  const tail = bottom ? `black calc(100% - ${FADE_BOTTOM}px), transparent 100%` : "black 100%";
  return `linear-gradient(to bottom, ${head}, ${tail})`;
}

/** True/false per edge, kept in sync with scrolling AND with content resizing -
 *  collapsing a section changes what is overflowing without any scroll event. */
function useScrollEdges(ref, enabled) {
  const [edges, setEdges] = React.useState({ top: false, bottom: false });

  const measure = React.useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const { scrollTop, scrollHeight, clientHeight } = el;
    const room = scrollHeight - clientHeight;
    setEdges((prev) => {
      const top = scrollTop > 1;
      const bottom = room > 1 && scrollTop < room - 1;
      return prev.top === top && prev.bottom === bottom ? prev : { top, bottom };
    });
  }, [ref]);

  React.useEffect(() => {
    if (!enabled) return undefined;
    const el = ref.current;
    if (!el) return undefined;

    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    // the container's own height rarely changes; its content's height is what
    // decides whether anything is hidden, so watch that too
    for (const child of el.children) ro.observe(child);
    const mo = new MutationObserver(measure);
    mo.observe(el, { childList: true, subtree: true, characterData: true });

    return () => {
      ro.disconnect();
      mo.disconnect();
    };
  }, [enabled, measure, ref]);

  return [edges, measure];
}

function ScrollArea({ className, children, fade = false, viewportRef, onScroll, style, ...props }) {
  const innerRef = React.useRef(null);
  const setRef = React.useCallback(
    (node) => {
      innerRef.current = node;
      if (typeof viewportRef === "function") viewportRef(node);
      else if (viewportRef) viewportRef.current = node;
    },
    [viewportRef]
  );

  const [edges, measure] = useScrollEdges(innerRef, fade);
  const mask = fade ? maskFor(edges.top, edges.bottom) : undefined;

  return (
    <div
      ref={setRef}
      onScroll={(e) => {
        if (fade) measure();
        onScroll?.(e);
      }}
      style={mask ? { ...style, maskImage: mask, WebkitMaskImage: mask } : style}
      // `scroll-smooth`: programmatic scrolls glide. Anything restoring a
      // position passes `behavior: "auto"` and still cuts - see ChatView.
      className={cn("no-scrollbar scroll-smooth overflow-auto overscroll-contain", className)}
      {...props}
    >
      {children}
    </div>
  );
}

function ScrollAreaX({ className, children, fade = true, viewportRef, ...props }) {
  return (
    <div
      ref={viewportRef}
      className={cn(
        "no-scrollbar scroll-smooth flex overflow-x-auto overflow-y-hidden overscroll-contain",
        fade && "edge-fade-x",
        className
      )}
      {...props}
    >
      {children}
    </div>
  );
}

export { ScrollArea, ScrollAreaX };

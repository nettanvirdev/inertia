import * as React from "react";
import { cn } from "@/lib/utils";
import { Portal, useEscapeLayer, usePrefersReducedMotion } from "@/components/ui/portal";
import { useFocusTrap, useLockBodyScroll } from "@/components/ui/dialog";

const DURATION = 300;

const axis = {
  left: { pos: "inset-y-0 left-0 h-full", hidden: "-translate-x-full" },
  right: { pos: "inset-y-0 right-0 h-full", hidden: "translate-x-full" },
  top: { pos: "inset-x-0 top-0 w-full", hidden: "-translate-y-full" },
  bottom: { pos: "inset-x-0 bottom-0 w-full", hidden: "translate-y-full" },
};

/**
 * The narrow-window drawer. Sits at z-70 over its own z-60 backdrop, so a menu
 * opened from inside it can take z-80 and still clear both.
 *
 * Under `prefers-reduced-motion` the slide is dropped entirely and the panel
 * cross-fades instead - a transform is the thing being reduced, not the change.
 */
export function Sheet({
  open,
  onOpenChange,
  side = "right",
  size,
  className,
  children,
  ariaLabel,
  closeOnOverlay = true,
}) {
  const panelRef = React.useRef(null);
  const reduced = usePrefersReducedMotion();

  // keep the panel mounted through its exit transition
  const [mounted, setMounted] = React.useState(open);
  const [entered, setEntered] = React.useState(false);

  React.useEffect(() => {
    if (open) {
      setMounted(true);
      const raf = requestAnimationFrame(() => setEntered(true));
      return () => cancelAnimationFrame(raf);
    }
    setEntered(false);
    if (!mounted) return;
    const t = setTimeout(() => setMounted(false), reduced ? 0 : DURATION);
    return () => clearTimeout(t);
  }, [open, reduced]); // eslint-disable-line react-hooks/exhaustive-deps

  const close = React.useCallback(() => onOpenChange?.(false), [onOpenChange]);

  const onTabKey = useFocusTrap(panelRef, open);
  useLockBodyScroll(open);
  useEscapeLayer(open, close);

  if (!mounted) return null;

  const geom = axis[side] ?? axis.right;
  const vertical = side === "top" || side === "bottom";
  const sizeStyle = size ? (vertical ? { height: size } : { width: size }) : undefined;

  return (
    <Portal>
      <div
        className={cn(
          "fixed inset-0 z-60 scrim transition-opacity ease-out",
          entered ? "opacity-100" : "opacity-0"
        )}
        style={{ transitionDuration: `${DURATION}ms` }}
        onClick={closeOnOverlay ? close : undefined}
      />
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel}
        tabIndex={-1}
        onKeyDown={onTabKey}
        style={{ transitionDuration: `${DURATION}ms`, ...sizeStyle }}
        className={cn(
          "fixed z-70 flex flex-col overflow-hidden bg-sidebar outline-none",
          geom.pos,
          !vertical && !size && "w-[19rem] max-w-[85vw]",
          vertical && !size && "h-[60dvh]",
          reduced
            ? cn("transition-opacity ease-out", entered ? "opacity-100" : "opacity-0")
            : cn(
                "transition-transform ease-out",
                entered ? "translate-x-0 translate-y-0" : geom.hidden
              ),
          className
        )}
      >
        {children}
      </div>
    </Portal>
  );
}

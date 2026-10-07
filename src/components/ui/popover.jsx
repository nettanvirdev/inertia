import * as React from "react";
import { cn } from "@/lib/utils";
import { useDismiss } from "@/hooks/use-dismiss";
import { usePresence } from "@/hooks/use-presence";
import { Portal, useEscapeLayer } from "@/components/ui/portal";
import { useAnchoredPosition } from "@/components/ui/use-anchored-position";

/**
 * The generic anchored floating surface. Every menu, select sheet and tooltip
 * is this component plus a role.
 *
 * Elevation is the `overlay-surface` utility, which is a background colour and
 * nothing else. No ring, no shadow: a panel is legible because it is a clear
 * step up the surface ladder from whatever it covers, which is the same way
 * every other surface in this UI separates.
 */
export const Popover = React.forwardRef(function Popover(
  {
    open,
    onOpenChange,
    anchorRef,
    side = "bottom",
    align = "start",
    offset = 8,
    matchWidth = false,
    className,
    children,
    ariaLabel,
    role = "dialog",
    dismissOnOutside = true,
    ...props
  },
  ref
) {
  // Held on the page through its exit, so a closing menu folds back into the
  // control it came out of rather than ceasing to exist. Positioning follows
  // `mounted`, not `open`: a layer that is still visible has to stay where it
  // was, and the hook hides anything it is told is closed.
  const { mounted, state } = usePresence(open);
  const { style, placedSide, floatingRef } = useAnchoredPosition({
    anchorRef,
    open: mounted,
    side,
    align,
    offset,
    matchWidth,
  });

  const close = React.useCallback(() => onOpenChange?.(false), [onOpenChange]);

  // stable array identity - useDismiss re-binds its listeners whenever it changes
  const dismissRefs = React.useMemo(() => [floatingRef, anchorRef], [floatingRef, anchorRef]);
  useDismiss(dismissRefs, open && dismissOnOutside, close);
  useEscapeLayer(open, close);

  React.useImperativeHandle(ref, () => floatingRef.current, [open]);

  if (!mounted) return null;

  return (
    <Portal>
      <div
        ref={floatingRef}
        role={role}
        aria-label={ariaLabel}
        aria-hidden={state === "closing" || undefined}
        data-side={placedSide}
        data-state={state}
        style={style}
        className={cn(
          "z-50 min-w-32 overflow-y-auto no-scrollbar outline-none",
          "overlay-surface rounded-2xl p-1.5",
          "animate-overlay-in",
          className
        )}
        {...props}
      >
        {children}
      </div>
    </Portal>
  );
});

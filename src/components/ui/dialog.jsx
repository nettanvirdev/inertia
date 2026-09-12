import * as React from "react";
import { X } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Portal, useEscapeLayer } from "@/components/ui/portal";
import { usePresence } from "@/hooks/use-presence";

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

function focusables(root) {
  if (!root) return [];
  return Array.from(root.querySelectorAll(FOCUSABLE)).filter(
    (el) => el.offsetWidth || el.offsetHeight || el.getClientRects().length,
  );
}

/**
 * Real trap: Tab and Shift+Tab cycle inside `containerRef`, focus moves in on
 * open and back to the opener on close.
 *
 * Deliberately keydown-based rather than a `focusin` guard - our own selects and
 * menus portal OUT of the panel while staying children of it in the React tree,
 * and a focusin guard would yank focus back out of them. The containment check
 * below lets those bubbled events through untouched.
 */
export function useFocusTrap(containerRef, active) {
  const restoreTo = React.useRef(null);

  React.useEffect(() => {
    if (!active) return;
    restoreTo.current = document.activeElement;

    const raf = requestAnimationFrame(() => {
      const node = containerRef.current;
      if (!node || node.contains(document.activeElement)) return;
      (focusables(node)[0] ?? node)?.focus({ preventScroll: true });
    });

    return () => {
      cancelAnimationFrame(raf);
      const target = restoreTo.current;
      if (target && document.contains(target))
        target.focus?.({ preventScroll: true });
    };
  }, [active, containerRef]);

  return React.useCallback(
    (e) => {
      if (e.key !== "Tab") return;
      const node = containerRef.current;
      if (!node || !node.contains(e.target)) return;
      const items = focusables(node);
      if (!items.length) {
        e.preventDefault();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus({ preventScroll: true });
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus({ preventScroll: true });
      }
    },
    [containerRef],
  );
}

/* ── body scroll lock (ref-counted, so nested layers cannot unlock early) ──── */

let lockCount = 0;
let previousOverflow = "";

export function useLockBodyScroll(active) {
  React.useEffect(() => {
    if (!active) return;
    if (lockCount === 0) {
      previousOverflow = document.body.style.overflow;
      document.body.style.overflow = "hidden";
    }
    lockCount += 1;
    return () => {
      lockCount -= 1;
      if (lockCount === 0) document.body.style.overflow = previousOverflow;
    };
  }, [active]);
}

/* ── dialog ───────────────────────────────────────────────────────────────── */

const sizes = {
  sm: "max-w-[26rem]",
  md: "max-w-lg",
  lg: "max-w-2xl",
  xl: "max-w-4xl",
  full: "w-[calc(100vw-4rem)] max-w-[80rem] h-[min(54rem,calc(100dvh-6rem))] p-0",
};

const DialogContext = React.createContext(null);

export function useDialog() {
  return React.useContext(DialogContext);
}

export function Dialog({
  open,
  onOpenChange,
  size = "md",
  className,
  children,
  closeOnOverlay = true,
  showClose = true,
  ariaLabel,
}) {
  const panelRef = React.useRef(null);
  const baseId = React.useId();
  const [labelled, setLabelled] = React.useState(false);
  const [described, setDescribed] = React.useState(false);

  const close = React.useCallback(() => onOpenChange?.(false), [onOpenChange]);

  const onTabKey = useFocusTrap(panelRef, open);
  useLockBodyScroll(open);
  useEscapeLayer(open, close);

  const ctx = React.useMemo(
    () => ({
      close,
      titleId: `${baseId}-title`,
      descriptionId: `${baseId}-description`,
      setLabelled,
      setDescribed,
    }),
    [close, baseId],
  );

  // Mounted one beat past closing so the panel can settle back out and the
  // scrim can fade rather than blink. The focus trap and the scroll lock
  // follow `open` above: a closing dialog has already given focus back.
  const { mounted, state } = usePresence(open);
  if (!mounted) return null;

  return (
    <DialogContext.Provider value={ctx}>
      <Portal>
        <div className={cn("fixed inset-0 z-50", state === "closing" && "pointer-events-none")}>
          {/* scrim: fade only, never blur */}
          <div
            data-state={state}
            className="absolute inset-0 scrim animate-fade-in"
            onClick={closeOnOverlay ? close : undefined}
          />
          {/* pointer-events pass through the padding so the scrim still catches clicks */}
          <div className="pointer-events-none absolute inset-0 grid place-items-center p-4">
            <div
              ref={panelRef}
              role="dialog"
              aria-modal="true"
              aria-hidden={state === "closing" || undefined}
              aria-label={labelled ? undefined : ariaLabel}
              aria-labelledby={labelled ? ctx.titleId : undefined}
              aria-describedby={described ? ctx.descriptionId : undefined}
              data-state={state}
              tabIndex={-1}
              onKeyDown={onTabKey}
              className={cn(
                // The panel never grows past the viewport: a tall body (a long
                // diff, a long list) scrolls inside `DialogBody` instead of
                // running off the bottom of the screen where nothing can reach it.
                "pointer-events-auto relative flex max-h-[calc(100dvh-2rem)] w-full max-w-[calc(100%-2rem)] flex-col",
                "overlay-surface rounded-3xl p-5 outline-none",
                "animate-overlay-in",
                sizes[size] ?? sizes.md,
                className,
              )}
            >
              {showClose ? (
                <DialogClose className="absolute top-4 right-4" />
              ) : null}
              {children}
            </div>
          </div>
        </div>
      </Portal>
    </DialogContext.Provider>
  );
}

export function DialogTitle({ className, children, ...props }) {
  const ctx = useDialog();
  React.useEffect(() => {
    ctx?.setLabelled(true);
    return () => ctx?.setLabelled(false);
  }, [ctx]);

  return (
    <h2
      id={ctx?.titleId}
      className={cn(
        "text-base leading-snug font-medium text-foreground",
        className,
      )}
      {...props}
    >
      {children}
    </h2>
  );
}

export function DialogDescription({ className, children, ...props }) {
  const ctx = useDialog();
  React.useEffect(() => {
    ctx?.setDescribed(true);
    return () => ctx?.setDescribed(false);
  }, [ctx]);

  return (
    <p
      id={ctx?.descriptionId}
      className={cn(
        "mt-2 text-[13px] leading-relaxed text-muted-foreground",
        className,
      )}
      {...props}
    >
      {children}
    </p>
  );
}

/** 8px down from the title. */
export function DialogBody({ className, children, ...props }) {
  return (
    <div className={cn("mt-2 min-h-0 flex-1", className)} {...props}>
      {children}
    </div>
  );
}

/** 20px down from the body; reversed on mobile so the primary sits under the thumb. */
export function DialogFooter({ className, children, ...props }) {
  return (
    <div
      className={cn(
        "mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end",
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}

export function DialogClose({ className, children, onClick, ...props }) {
  const ctx = useDialog();
  return (
    <button
      type="button"
      aria-label="Close"
      onClick={(e) => {
        onClick?.(e);
        if (!e.defaultPrevented) ctx?.close();
      }}
      className={cn(
        "grid size-6 shrink-0 place-items-center rounded-full text-muted-foreground",
        "transition-colors duration-150 ease-out hover:fill-close hover:text-foreground",
        "outline-none focus-visible:fill-close",
        className,
      )}
      {...props}
    >
      {children ?? <X className="size-3.5" />}
    </button>
  );
}

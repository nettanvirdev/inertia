import * as React from "react";
import { CircleAlert, CircleCheck, TriangleAlert, X } from "@/components/icons";
import { cn } from "@/lib/utils";
import { Portal } from "@/components/ui/portal";
import { Button } from "@/components/ui/button";

const MAX_VISIBLE = 4;
const EXIT_MS = 160;

/** The accent is a glyph. It never fills the toast. */
const variants = {
  default: { icon: null, ink: "" },
  success: { icon: CircleCheck, ink: "text-success-ink" },
  warning: { icon: TriangleAlert, ink: "text-warning-ink" },
  danger: { icon: CircleAlert, ink: "text-destructive-ink" },
};

const ToastContext = React.createContext(null);

export function useToast() {
  const ctx = React.useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>");
  return ctx;
}

export function ToastProvider({ children, max = MAX_VISIBLE }) {
  const [toasts, setToasts] = React.useState([]);
  const nextId = React.useRef(0);

  const dismiss = React.useCallback((id) => {
    setToasts((list) => list.map((t) => (t.id === id ? { ...t, exiting: true } : t)));
  }, []);

  const remove = React.useCallback((id) => {
    setToasts((list) => list.filter((t) => t.id !== id));
  }, []);

  const toast = React.useCallback(
    (options = {}) => {
      const id = `toast-${(nextId.current += 1)}`;
      setToasts((list) => {
        const next = [...list, { id, variant: "default", duration: 4000, ...options }];
        const live = next.filter((t) => !t.exiting);
        if (live.length <= max) return next;
        // oldest beyond the cap starts its exit immediately
        const doomed = new Set(live.slice(0, live.length - max).map((t) => t.id));
        return next.map((t) => (doomed.has(t.id) ? { ...t, exiting: true } : t));
      });
      return id;
    },
    [max]
  );

  const value = React.useMemo(() => ({ toast, dismiss }), [toast, dismiss]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      {toasts.length ? (
        <Portal>
          <div
            aria-live="polite"
            aria-atomic="false"
            className="pointer-events-none fixed right-4 bottom-4 z-50 flex flex-col items-end gap-2"
          >
            {toasts.map((t) => (
              <ToastRow key={t.id} toast={t} onDismiss={dismiss} onRemove={remove} />
            ))}
          </div>
        </Portal>
      ) : null}
    </ToastContext.Provider>
  );
}

function ToastRow({ toast, onDismiss, onRemove }) {
  const { id, title, description, variant = "default", duration = 4000, action, exiting } = toast;
  const { icon: Icon, ink } = variants[variant] ?? variants.default;

  const timer = React.useRef(null);
  const startedAt = React.useRef(0);
  const remaining = React.useRef(duration);

  const clear = () => {
    clearTimeout(timer.current);
    timer.current = null;
  };

  const resume = React.useCallback(() => {
    if (exiting || duration <= 0 || timer.current) return;
    startedAt.current = Date.now();
    timer.current = setTimeout(() => onDismiss(id), Math.max(0, remaining.current));
  }, [exiting, duration, id, onDismiss]);

  const pause = React.useCallback(() => {
    if (!timer.current) return;
    remaining.current -= Date.now() - startedAt.current;
    clear();
  }, []);

  React.useEffect(() => {
    resume();
    return clear;
  }, [resume]);

  React.useEffect(() => {
    if (!exiting) return;
    clear();
    const t = setTimeout(() => onRemove(id), EXIT_MS);
    return () => clearTimeout(t);
  }, [exiting, id, onRemove]);

  /**
   * A toast with nothing but a title is one line of text, and one line of text
   * wants its icon and its close button on the same optical centre. With a
   * description it becomes a block, and both want to sit on the FIRST line
   * instead. Two layouts, chosen from the content, rather than one layout that
   * is a compromise for both.
   */
  const dense = !description && !action;

  return (
    <div
      role="status"
      onPointerEnter={pause}
      onPointerLeave={resume}
      onFocusCapture={pause}
      onBlurCapture={resume}
      className={cn(
        // Hugs its text rather than padding a fixed width out to the close
        // button, which is what leaves two words marooned at one end of a box.
        // The stack is right-aligned, so ragged widths still line up on an edge.
        "pointer-events-auto flex w-max max-w-[22rem] min-w-[14rem] gap-2.5",
        "overlay-plain rounded-2xl py-2.5 pr-2.5 pl-3.5",
        dense ? "items-center" : "items-start",
        exiting ? "animate-fade-out" : "animate-slide-up"
      )}
    >
      {Icon ? (
        <Icon className={cn("size-4 shrink-0", dense ? null : "mt-0.5", ink)} aria-hidden="true" />
      ) : null}

      <div className="flex min-w-0 flex-1 flex-col gap-1 py-px">
        {title ? <div className="text-[13px] leading-5 text-foreground">{title}</div> : null}
        {description ? (
          <div className="text-[11px] leading-relaxed text-muted-foreground">{description}</div>
        ) : null}
        {action ? (
          <div className="mt-1.5 flex">
            {React.isValidElement(action) ? (
              action
            ) : (
              <Button
                variant="ghost"
                size="xs"
                className="-ml-2"
                onClick={() => {
                  action.onClick?.();
                  onDismiss(id);
                }}
              >
                {action.label}
              </Button>
            )}
          </div>
        ) : null}
      </div>

      <button
        type="button"
        aria-label="Dismiss"
        onClick={() => onDismiss(id)}
        className={cn(
          "grid size-5 shrink-0 place-items-center rounded-full text-muted-foreground",
          "transition-colors duration-150 ease-out hover:fill-close hover:text-foreground",
          "outline-none focus-visible:fill-close",
          dense ? null : "mt-px"
        )}
      >
        <X className="size-3" />
      </button>
    </div>
  );
}

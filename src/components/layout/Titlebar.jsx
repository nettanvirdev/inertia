import { useEffect } from "react";
import { Minus, Square, X, Copy } from "@/components/icons";
import { cn } from "@/lib/utils";

/**
 * Frameless-window chrome. 32px tall, flat, no border - the surface step to
 * the shell below does the separating. Window controls are the one place a
 * red hover is allowed, because it is the OS convention users aim for.
 *
 * The bar owns no identity of its own: it used to carry a logo and the word
 * "Inertia" directly above the sidebar's logo and the word "Inertia", which is
 * the same badge twice in one corner. The sidebar's header now moves up into
 * this strip as `leading`, so there is one brand mark, and the row below the
 * strip starts with New chat on the left and the agent name on the right - the
 * two of them finally on one line.
 */
export function Titlebar({
  leading = null,
  windowState = "normal",
  onWindowStateChange,
  children,
}) {
  useEffect(() => {
    const unsubscribe = window.electronAPI?.onWindowState?.((state) =>
      onWindowStateChange?.(state)
    );
    return () => unsubscribe?.();
  }, [onWindowStateChange]);

  const isMaximized = windowState === "maximized" || windowState === "fullscreen";
  const dragEnabled = windowState === "normal";

  const handleMouseDown = (e) => {
    if (e.target.closest(".titlebar-no-drag")) return;
    if (isMaximized) window.electronAPI?.beginDrag?.();
  };

  return (
    <header
      onMouseDown={handleMouseDown}
      onDoubleClick={() => window.electronAPI?.maximize?.()}
      className={cn(
        "flex h-8 min-h-8 select-none items-center justify-between bg-titlebar",
        dragEnabled && "titlebar-drag",
        !isMaximized && "rounded-t-window"
      )}
    >
      {leading}

      <div className="titlebar-no-drag flex items-center gap-1 pr-1">
        {children}
        <div className="flex h-8">
          <WindowButton onClick={() => window.electronAPI?.minimize?.()} label="Minimize">
            <Minus className="size-3.5" />
          </WindowButton>
          <WindowButton
            onClick={() => window.electronAPI?.maximize?.()}
            label={isMaximized ? "Restore" : "Maximize"}
          >
            {/* Two glyphs, so React swaps the element and the newcomer pops
                in - the one place in the strip that answers a click with a
                changed mark rather than a changed colour. */}
            {isMaximized ? (
              <Copy className="size-3 animate-pop-in" />
            ) : (
              <Square className="size-2.5 animate-pop-in" />
            )}
          </WindowButton>
          <WindowButton
            onClick={() => window.electronAPI?.close?.()}
            label="Close"
            className={cn(
              "hover:bg-destructive hover:text-destructive-foreground",
              !isMaximized && "rounded-tr-window"
            )}
          >
            <X className="size-3.5" />
          </WindowButton>
        </div>
      </div>
    </header>
  );
}

/**
 * A titlebar control: 44 x 32, flat, square. Exported because the sidebar's
 * collapse toggle now lives in this same strip - sharing the component is the
 * only way the left control and the window controls on the right cannot drift
 * apart in size, weight or hover behaviour.
 */
export function TitlebarButton({ children, onClick, label, className, ...props }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className={cn(
        "inline-flex h-8 w-11 items-center justify-center text-muted-foreground",
        "transition-colors duration-150 ease-out hover:fill-nav hover:text-foreground",
        "outline-none focus-visible:fill-nav",
        className
      )}
      {...props}
    >
      {children}
    </button>
  );
}

const WindowButton = TitlebarButton;

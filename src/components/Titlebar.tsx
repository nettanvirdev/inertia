import { useWindowChrome } from "@/hooks/useWindowChrome";
import { Minus, Square, Copy, X } from "@/lib/icons";
import { cn } from "@/lib/utils";

/**
 * Frameless-window chrome, 32px tall, flat - the surface step to the shell
 * below does the separating instead of a border. Dragging and the maximize
 * gesture are both handled here rather than left to the webview's default
 * `data-tauri-drag-region` behaviour, because that region's built-in
 * double-click snaps the window instantly - and the whole point of this bar
 * is the animated grow/shrink in `useWindowChrome`.
 */
export function Titlebar({ title }: { title?: string }) {
  const { mode, minimize, toggleMaximize, close, beginDrag } = useWindowChrome();
  const isMaximized = mode === "maximized";

  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest(".titlebar-no-drag")) return;
    beginDrag();
  };

  return (
    <header
      onMouseDown={handleMouseDown}
      onDoubleClick={(e) => {
        if ((e.target as HTMLElement).closest(".titlebar-no-drag")) return;
        void toggleMaximize();
      }}
      className="relative flex h-8 min-h-8 select-none items-center justify-between bg-titlebar"
    >
      <div className="panel-gutter flex min-w-0 items-center text-[13px] font-medium text-foreground-secondary">
        {title}
      </div>

      <div className="titlebar-no-drag relative flex h-8 items-center">
        <WindowButton onClick={minimize} label="Minimize">
          <Minus className="size-3.5" />
        </WindowButton>
        <WindowButton onClick={() => void toggleMaximize()} label={isMaximized ? "Restore" : "Maximize"}>
          {isMaximized ? (
            <Copy className="size-3 animate-pop-in" />
          ) : (
            <Square className="size-2.5 animate-pop-in" />
          )}
        </WindowButton>
        <WindowButton onClick={close} label="Close" tone="danger">
          <X className="size-3.5" />
        </WindowButton>
      </div>
    </header>
  );
}

function WindowButton({
  children,
  onClick,
  label,
  tone = "neutral",
  className,
}: {
  children: React.ReactNode;
  onClick: () => void;
  label: string;
  tone?: "neutral" | "danger";
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className={cn(
        "inline-flex h-8 w-11 items-center justify-center text-muted-foreground",
        "outline-none transition-colors duration-150 ease-out",
        // Each tone owns its whole hover/focus treatment as one block rather
        // than layering a shared neutral hover under a danger override: two
        // `hover:` background utilities on the same element both compile to
        // real CSS rules, and whichever one Tailwind happens to emit later
        // wins the cascade regardless of which reads as more "specific" here
        // - which is how the close button's red hover was silently getting
        // painted over by the neutral grey one.
        tone === "danger"
          ? "hover:bg-destructive hover:text-destructive-foreground focus-visible:bg-destructive focus-visible:text-destructive-foreground"
          : "hover:fill-nav hover:text-foreground focus-visible:fill-nav",
        className,
      )}
    >
      {children}
    </button>
  );
}

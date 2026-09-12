import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, X } from "@/components/icons";
import { cn } from "@/lib/utils";

/**
 * Resolved per call rather than once at module scope. `getCurrentWindow()`
 * reads metadata that only exists inside a Tauri webview, so calling it while
 * the module evaluates throws before React ever mounts - which is what the
 * whole setup screen did when opened through `bun run dev:setup-ui` in a
 * plain browser. Failing quietly here keeps that preview working; inside the
 * installer the call always succeeds.
 */
function setupWindow() {
  try {
    return getCurrentWindow();
  } catch {
    return null;
  }
}

/**
 * The installer's chrome: the app's title bar with the maximize control taken
 * out, because the window is fixed at 460x560. Deliberately not a shared
 * component with the app's `Titlebar` - that one exists to drive the animated
 * maximize, which is the one thing this window must never do.
 */
export function SetupTitlebar({ busy, title = "Setup" }: { busy: boolean; title?: string }) {
  return (
    <header
      onMouseDown={(e) => {
        if (e.button !== 0) return;
        if ((e.target as HTMLElement).closest(".titlebar-no-drag")) return;
        void setupWindow()?.startDragging();
      }}
      className="relative flex h-8 min-h-8 select-none items-center justify-between bg-titlebar"
    >
      {/* pl-3.5 puts the word on 14px, the same left edge the app's own title
          bar starts its mark from, so the two windows read as one product.
          It used to be `panel-gutter`, a class that is not defined in this
          stylesheet or the app's - so it resolved to nothing and the word sat
          flat against the window edge. */}
      <div className="flex min-w-0 items-center pl-3.5 text-[13px] font-medium text-foreground-secondary">
        {title}
      </div>

      <div className="titlebar-no-drag relative flex h-8 items-center">
        <button
          type="button"
          onClick={() => void setupWindow()?.minimize()}
          aria-label="Minimize"
          className="inline-flex h-8 w-11 items-center justify-center text-muted-foreground outline-none transition-colors duration-150 ease-out hover:fill-nav hover:text-foreground focus-visible:fill-nav"
        >
          <Minus className="size-3.5" />
        </button>
        <button
          type="button"
          onClick={() => void setupWindow()?.close()}
          // Closing mid-run would leave a half-written (or half-deleted)
          // install directory behind. The window has no other exit, so the
          // control stays visible and inert rather than disappearing for the
          // few seconds the work takes.
          disabled={busy}
          aria-label={busy ? "Close (unavailable while installing)" : "Close"}
          className={cn(
            "inline-flex h-8 w-11 items-center justify-center text-muted-foreground",
            "outline-none transition-colors duration-150 ease-out",
            busy
              ? "opacity-35"
              : "hover:bg-destructive hover:text-destructive-foreground focus-visible:bg-destructive focus-visible:text-destructive-foreground",
          )}
        >
          <X className="size-3.5" />
        </button>
      </div>
    </header>
  );
}

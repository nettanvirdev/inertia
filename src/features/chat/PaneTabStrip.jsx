import * as React from "react";
import { Plus, X } from "@/components/icons";
import { IconButton } from "@/components/ui/icon-button";
import { cn } from "@/lib/utils";

/**
 * The tab strip inside a pane.
 *
 * Distinct from the dock's own strip one level up, and deliberately quieter:
 * that one chooses which PANE you are looking at, this one chooses which
 * browser or which terminal within it. Two strips of equal weight stacked on
 * each other would read as one confusing double row, so this is smaller, has
 * no icons, and only appears when there is more than one thing to choose
 * between - a single tab needs no tab.
 *
 * The close button is on every tab rather than only the active one. A row of
 * controls that appear on hover is a row you cannot aim at, and these are
 * small enough already.
 *
 * ── Why the padding here is not a free choice ─────────────────────────────
 * `px-2` matches the toolbar directly beneath this strip, and the `+` is the
 * same `IconButton` at the same size as the buttons in that toolbar. So the
 * `+` and the `x` below it sit on one vertical line: same 8px inset, same 24px
 * target, same centre. They were a hand-rolled 20px button in a 4px-padded row
 * over a 24px button in an 8px-padded one, which put their centres six pixels
 * apart - close enough to look like a mistake rather than a difference, which
 * is the worst distance for two things to be.
 */
export function PaneTabStrip({ tabs, active, onSelect, onClose, onAdd, addLabel = "New tab" }) {
  if (!tabs?.length) return null;
  const single = tabs.length === 1;

  return (
    <div
      role="tablist"
      className="flex shrink-0 items-center gap-0.5 overflow-x-auto border-b border-border-subtle px-2 py-1 no-scrollbar"
    >
      {single ? null : (
        tabs.map((tab) => (
          <div
            key={tab.id}
            className={cn(
              "group/tab flex shrink-0 items-center gap-1 rounded-sm pl-2 pr-1 transition-colors duration-100",
              tab.id === active ? "fill-control" : "hover:fill-control/60"
            )}
          >
            <button
              type="button"
              role="tab"
              aria-selected={tab.id === active}
              onClick={() => onSelect(tab.id)}
              // Middle click closes, the way it does in every browser.
              onAuxClick={(event) => {
                if (event.button === 1) {
                  event.preventDefault();
                  onClose(tab.id);
                }
              }}
              className={cn(
                "max-w-[120px] truncate py-1 text-[11px] outline-none",
                tab.id === active ? "text-foreground" : "text-muted-foreground"
              )}
              title={tab.label}
            >
              {tab.label}
            </button>
            <button
              type="button"
              aria-label={`Close ${tab.label}`}
              onClick={() => onClose(tab.id)}
              className="flex size-4 shrink-0 items-center justify-center rounded-sm text-muted-foreground opacity-0 outline-none transition-opacity duration-100 hover:text-foreground group-hover/tab:opacity-100 focus-visible:opacity-100"
            >
              <X className="size-2.5" />
            </button>
          </div>
        ))
      )}
      <IconButton
        size="sm"
        label={addLabel}
        onClick={onAdd}
        className={cn(single && "ml-auto")}
      >
        <Plus />
      </IconButton>
    </div>
  );
}

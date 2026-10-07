import * as React from "react";
import { Icon } from "@/components/icons";
import { cn } from "@/lib/utils";
import { MIN_WIDTH, useDockWidth } from "@/features/chat/use-dock-width";
import { activeTab, moveTab, orderTabs } from "@/features/chat/dock-tabs.js";
import { onRevealPane } from "@/features/chat/pane-reveal";
import { readPref, writePref } from "@/lib/persist";

/**
 * The column to the right of the conversation, and everything that lives in it.
 *
 * ── Why tabs, having been a stack ──────────────────────────────────────────
 * There used to be two things in here - what this thread is working with, and
 * what the helpers are doing - and they were stacked, with a divider between
 * them. That was right for two. It is not right for five: a browser, a
 * terminal, the thread's files, the crew, each squashed into a fifth of a
 * column that is three hundred pixels wide, is four panes none of which can be
 * read. So they are tabs, and the whole column belongs to whichever one you
 * are looking at.
 *
 * ── Why every open pane stays mounted ──────────────────────────────────────
 * Only the front one is visible; the rest are `hidden`, not unmounted. Two
 * reasons, and the first is a bug that would otherwise be very hard to find.
 * The browser pane is a native view positioned over a hole in this window, and
 * it reports where that hole is from a ResizeObserver - a hidden box measures
 * to nothing, so hiding the tab hides the view, which is exactly right, and
 * unmounting it would instead leave the view floating over the transcript with
 * nothing left to tell it to move. The second is ordinary: a terminal you
 * tabbed away from and back should not have forgotten what you scrolled to.
 *
 * ── Sections are passed as data, not as children ───────────────────────────
 * This took `children` at first and worked out what was open by counting them,
 * which is wrong in a way worth writing down: `React.Children.toArray` hands
 * back the *elements*, and a section that renders `null` when it is closed is
 * still a perfectly truthy element. So the dock was always open - a full-width
 * band of sidebar down the side of the window with nothing in it.
 *
 * A list of `{ id, label, icon, open, node }` cannot be wrong that way.
 */

const ORDER_KEY = "dock.order";
const ACTIVE_KEY = "dock.active";

export function SideDock({ sections = [], className }) {
  const { width, dragging, onPointerDown, onKeyDown } = useDockWidth();

  const [order, setOrder] = React.useState(() => {
    const stored = readPref(ORDER_KEY, null);
    return Array.isArray(stored) ? stored.filter((id) => typeof id === "string") : [];
  });
  const [picked, setPicked] = React.useState(() => readPref(ACTIVE_KEY, null));
  const [dragId, setDragId] = React.useState(null);

  const shown = React.useMemo(
    () =>
      orderTabs(
        sections.filter((section) => section?.open && section?.node),
        order
      ),
    [sections, order]
  );
  const open = shown.length > 0;
  // Resolved rather than stored: the front tab can close, and an active id
  // pointing at a pane that is gone renders a strip above an empty column.
  const active = activeTab(shown, picked);

  React.useEffect(() => {
    if (active) writePref(ACTIVE_KEY, active);
  }, [active]);

  // A tool touched a panel: put it in front. See `pane-reveal.js` - this is
  // one of the three things that have to happen, and the only one this
  // component owns.
  React.useEffect(() => onRevealPane(({ dock }) => setPicked(dock)), []);

  const reorder = (moved, before) => {
    // Built from what is on screen rather than patched into the stored list,
    // so an order carrying ids from panes that no longer exist cannot decide
    // where a tab lands today.
    const next = moveTab(
      shown.map((section) => section.id),
      moved,
      before
    );
    setOrder(next);
    writePref(ORDER_KEY, next);
  };

  return (
    <div
      className={cn(
        "relative flex min-h-0 shrink-0 overflow-hidden",
        // Painted only while there is a sidebar. A zero-width box paints
        // nothing today, but a border added here later would, and the next
        // person should not have to rediscover why the window has an edge.
        open && "bg-sidebar",
        // No width transition while dragging: an eased width chases the
        // pointer a frame behind and reads as the column being reluctant.
        !dragging && "transition-[width] duration-[var(--motion-panel)] ease-[var(--ease-out)]",
        className
      )}
      style={{ width: open ? width : 0 }}
    >
      {/* The width handle, on the dock's inner edge, wider in hit area than it
          is in ink. Only when there is a column to resize. */}
      {open ? (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize the panel"
          aria-valuenow={width}
          aria-valuemin={MIN_WIDTH}
          tabIndex={0}
          onPointerDown={onPointerDown}
          onKeyDown={onKeyDown}
          className="group/handle absolute inset-y-0 left-0 z-30 w-2 -translate-x-1 cursor-col-resize touch-none select-none outline-none"
        >
          <span
            aria-hidden="true"
            className={cn(
              "absolute inset-y-0 left-1 w-px transition-colors duration-100",
              dragging ? "bg-accent-solid" : "bg-transparent group-hover/handle:bg-border-strong",
              "group-focus-visible/handle:bg-accent-solid"
            )}
          />
        </div>
      ) : null}

      {/* Fixed inner width, so content does not reflow through the whole width
          transition and read as the column settling after it opens. The fade
          rides on the column widening, so the content arrives with it rather
          than being uncovered by it. Opacity only - see the panels below. */}
      <div
        className={cn("relative flex min-h-0 w-full flex-col", open && "animate-fade-in")}
        style={{ width }}
      >
        {shown.length > 1 ? (
          <div
            role="tablist"
            aria-label="Panels"
            className="flex shrink-0 items-center gap-0.5 overflow-x-auto border-b border-border-subtle px-1.5 py-1 no-scrollbar"
            // A drop on the strip itself, past the last tab, means the end.
            onDragOver={(event) => event.preventDefault()}
            onDrop={() => {
              if (dragId) reorder(dragId, null);
              setDragId(null);
            }}
          >
            {shown.map((section) => (
              <button
                key={section.id}
                type="button"
                role="tab"
                aria-selected={section.id === active}
                draggable
                onDragStart={() => setDragId(section.id)}
                onDragEnd={() => setDragId(null)}
                onDragOver={(event) => event.preventDefault()}
                onDrop={(event) => {
                  // Stopped, or the strip's own handler runs next and moves
                  // the tab to the end instead of to where it was dropped.
                  event.stopPropagation();
                  if (dragId && dragId !== section.id) reorder(dragId, section.id);
                  setDragId(null);
                }}
                onClick={() => setPicked(section.id)}
                className={cn(
                  "flex shrink-0 items-center gap-1.5 rounded-sm px-2 py-1 text-[12px] outline-none",
                  "transition-[color,background-color,opacity] duration-[var(--motion-fast)] ease-[var(--ease-out)]",
                  section.id === active
                    ? "fill-control text-foreground"
                    : "text-muted-foreground hover:text-foreground",
                  dragId === section.id && "opacity-50"
                )}
              >
                {section.icon ? <Icon name={section.icon} className="size-3.5" /> : null}
                {section.label ?? section.id}
              </button>
            ))}
          </div>
        ) : null}

        {/* The pane coming forward fades, and fades only. The browser pane
            positions a native view from its own box's getBoundingClientRect,
            which a transform would shift - so nothing here may scale or
            slide. Opacity is invisible to that measurement. */}
        {shown.map((section) => (
          <section
            key={section.id}
            role={shown.length > 1 ? "tabpanel" : undefined}
            hidden={section.id !== active}
            className={cn(
              "min-h-0 flex-1 flex-col overflow-hidden",
              section.id === active && "flex animate-fade-in"
            )}
          >
            {section.node}
          </section>
        ))}
      </div>
    </div>
  );
}

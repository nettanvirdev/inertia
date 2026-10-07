import * as React from "react";
import { cn } from "@/lib/utils";
import { MenuPanel, MenuItem, MenuLabel, MenuSeparator } from "@/components/ui/dropdown-menu";

/**
 * Right-click menu. It is the dropdown, anchored to a point instead of an
 * element - `items` are the same MenuItem rows, never a second visual language.
 *
 * `items` may be an array of descriptors
 * (`{ type: "separator" | "label" } | { icon, label, shortcut, danger, disabled,
 * checked, onSelect }`), or a render function `({ close }) => nodes` when you
 * need submenus or custom rows.
 */
export function ContextMenu({ items, children, className, panelClassName }) {
  const [point, setPoint] = React.useState(null);

  // A virtual anchor: useAnchoredPosition only ever calls getBoundingClientRect,
  // and `contains` keeps useDismiss's outside-click test happy.
  const anchorRef = React.useRef(null);
  if (point && (!anchorRef.current || anchorRef.current.__point !== point)) {
    anchorRef.current = {
      __point: point,
      contains: () => false,
      getBoundingClientRect: () => ({
        x: point.x,
        y: point.y,
        top: point.y,
        left: point.x,
        right: point.x,
        bottom: point.y,
        width: 0,
        height: 0,
      }),
    };
  }

  const close = React.useCallback(() => setPoint(null), []);

  const onContextMenu = (e) => {
    // An empty menu should not appear at all - a right-click that opens a blank
    // box reads as a bug. A render function might produce rows we cannot count
    // ahead of time, so only a provably empty array is suppressed.
    if (Array.isArray(items) && items.filter(Boolean).length === 0) return;
    e.preventDefault();
    e.stopPropagation();
    setPoint({ x: e.clientX, y: e.clientY });
  };

  const rows =
    typeof items === "function"
      ? items({ close })
      : Array.isArray(items)
        ? items.map((item, i) => {
            if (!item) return null;
            if (item.type === "separator") return <MenuSeparator key={`s-${i}`} />;
            if (item.type === "label") return <MenuLabel key={`l-${i}`}>{item.label}</MenuLabel>;
            return (
              <MenuItem
                key={item.id ?? `i-${i}`}
                icon={item.icon}
                shortcut={item.shortcut}
                danger={item.danger}
                disabled={item.disabled}
                checked={item.checked}
                onSelect={item.onSelect}
              >
                {item.label}
              </MenuItem>
            );
          })
        : items;

  return (
    <>
      <div className={cn("contents", className)} onContextMenu={onContextMenu}>
        {children}
      </div>
      <MenuPanel
        open={Boolean(point)}
        onOpenChange={(next) => !next && close()}
        anchorRef={anchorRef}
        side="bottom"
        align="start"
        offset={2}
        className={panelClassName}
      >
        {rows}
      </MenuPanel>
    </>
  );
}

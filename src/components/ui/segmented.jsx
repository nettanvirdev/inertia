import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * app-controls gives this control two sizes, 44px and 38px, which are the right
 * heights for a control that is the subject of the screen it sits on - a
 * settings pane, a view switcher with a whole page under it. They are the wrong
 * heights for a toolbar, where this is one of six things in a 3.5rem header, so
 * the app's own 28px and 32px stay. Four sizes, and the choice is about what
 * the control is competing with rather than about taste.
 */
const SIZES = {
  xs: { track: "h-7 p-0.5", item: "h-6 gap-1.5 px-2.5 text-xs", icon: "[&_svg]:size-3.5" },
  md: { track: "h-8 p-0.5", item: "h-7 gap-1.5 px-3 text-[13px]", icon: "[&_svg]:size-4" },
  lg: {
    track: "h-[2.375rem] p-1",
    item: "h-[1.875rem] gap-2 px-4 text-[13px]",
    icon: "[&_svg]:size-4",
  },
  xl: { track: "h-11 p-1", item: "h-9 gap-2 px-5 text-sm", icon: "[&_svg]:size-4" },
};

// The spec's two shapes. 12px rather than the 11px it names, which is the
// nearest rung on the radius ramp and is what its own note recommends.
const SHAPES = {
  pill: { track: "rounded-full", item: "rounded-full" },
  square: { track: "rounded-md", item: "rounded-sm" },
};

/** options: [{ value, label, icon }] - `icon` is a React node, not a component. */
function Segmented({
  value,
  onChange,
  options = [],
  size = "md",
  shape = "pill",
  fullWidth = false,
  className,
  label,
  ...props
}) {
  const trackRef = React.useRef(null);
  const s = SIZES[size] ?? SIZES.md;
  const shp = SHAPES[shape] ?? SHAPES.pill;

  function move(delta) {
    const enabled = options.filter((o) => !o.disabled);
    if (!enabled.length) return;
    const at = enabled.findIndex((o) => o.value === value);
    const next = enabled[(at + delta + enabled.length) % enabled.length];
    onChange?.(next.value);
    trackRef.current?.querySelector(`[data-value="${CSS.escape(String(next.value))}"]`)?.focus();
  }

  function handleKeyDown(e) {
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      move(1);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      move(-1);
    }
  }

  return (
    <div
      ref={trackRef}
      role="tablist"
      aria-label={label}
      onKeyDown={handleKeyDown}
      className={cn(
        "items-center fill-control",
        fullWidth ? "flex w-full" : "inline-flex shrink-0",
        shp.track,
        s.track,
        className
      )}
      {...props}
    >
      {options.map((opt) => {
        const selected = opt.value === value;
        return (
          <button
            key={opt.value}
            type="button"
            role="tab"
            data-value={opt.value}
            aria-selected={selected}
            disabled={opt.disabled}
            tabIndex={selected ? 0 : -1}
            onClick={() => onChange?.(opt.value)}
            className={cn(
              "inline-flex items-center justify-center font-normal outline-none",
              fullWidth ? "flex-1" : "shrink-0",
              shp.item,
              "transition-colors duration-150 ease-out",
              "focus-visible:fill-control-hover",
              "disabled:pointer-events-none disabled:opacity-40",
              "[&_svg]:shrink-0",
              s.item,
              s.icon,
              // the selected label takes the accent, the track does not: a
              // filled segment would make every toolbar in the app shout
              // The thumb is a raised surface, so it is the raised surface: fill,
              // hairline and the smallest shadow, exactly as a card would be.
              // In light that is the whole reason it is visible at all.
              selected
                ? "accent-ink bg-card-lighter shadow-sm dark:bg-muted"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            {opt.icon}
            {opt.label ? <span>{opt.label}</span> : null}
          </button>
        );
      })}
    </div>
  );
}

export { Segmented };

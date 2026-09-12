import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * No underline, no sliding bar: the fill IS the indicator.
 * `items` = `[{ value, label, icon, badge, disabled }]`.
 */

const sizes = {
  sm: "h-7 px-2 text-xs gap-1.5",
  md: "h-8 px-3 text-[13px] gap-2",
};

function renderIcon(icon, className) {
  if (!icon) return null;
  if (React.isValidElement(icon)) {
    return React.cloneElement(icon, { className: cn(className, icon.props.className) });
  }
  const Icon = icon;
  return <Icon className={className} />;
}

export function Tabs({
  value,
  onChange,
  items = [],
  variant = "strip",
  size = "sm",
  className,
  ariaLabel,
  idPrefix = "inertia",
}) {
  const listRef = React.useRef(null);
  const enabled = items.filter((i) => !i.disabled);

  // keep the active tab visible in the scrolling strip
  React.useEffect(() => {
    const el = listRef.current?.querySelector('[data-state="active"]');
    el?.scrollIntoView({ inline: "nearest", block: "nearest" });
  }, [value]);

  const move = (delta) => {
    if (!enabled.length) return;
    const i = enabled.findIndex((t) => t.value === value);
    const next = enabled[(i + delta + enabled.length) % enabled.length];
    if (next) {
      onChange?.(next.value);
      requestAnimationFrame(() => {
        listRef.current
          ?.querySelector(`#${CSS.escape(`${idPrefix}-tab-${next.value}`)}`)
          ?.focus({ preventScroll: true });
      });
    }
  };

  const onKeyDown = (e) => {
    switch (e.key) {
      case "ArrowRight":
        e.preventDefault();
        move(1);
        break;
      case "ArrowLeft":
        e.preventDefault();
        move(-1);
        break;
      case "Home":
        e.preventDefault();
        if (enabled[0]) onChange?.(enabled[0].value);
        break;
      case "End":
        e.preventDefault();
        if (enabled.length) onChange?.(enabled[enabled.length - 1].value);
        break;
      default:
        break;
    }
  };

  return (
    <div
      ref={listRef}
      role="tablist"
      aria-label={ariaLabel}
      onKeyDown={onKeyDown}
      className={cn(
        "flex items-center gap-1 overflow-x-auto no-scrollbar",
        variant === "strip" && "edge-fade-x",
        className
      )}
    >
      {items.map((item) => {
        const active = item.value === value;
        return (
          <button
            key={String(item.value)}
            id={`${idPrefix}-tab-${item.value}`}
            type="button"
            role="tab"
            aria-selected={active}
            aria-controls={`${idPrefix}-panel-${item.value}`}
            data-state={active ? "active" : "inactive"}
            tabIndex={active ? 0 : -1}
            disabled={item.disabled}
            onClick={() => onChange?.(item.value)}
            className={cn(
              "inline-flex shrink-0 items-center whitespace-nowrap font-normal outline-none",
              "transition-colors duration-150 ease-out",
              "focus-visible:fill-nav",
              "disabled:pointer-events-none disabled:opacity-40",
              "[&_svg]:size-3.5 [&_svg]:shrink-0",
              variant === "pill" ? "rounded-full" : "rounded-lg",
              sizes[size] ?? sizes.sm,
              active
                ? "bg-muted font-medium text-foreground"
                : "text-muted-foreground hover:fill-nav hover:text-foreground"
            )}
          >
            {renderIcon(item.icon, "size-3.5")}
            {item.label}
            {item.badge != null ? (
              <span
                className={cn(
                  "ml-0.5 text-[11px] font-semibold tabular-nums",
                  active ? "text-foreground" : "text-muted-foreground"
                )}
              >
                {item.badge}
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}

export function TabPanel({ value, activeValue, children, className, idPrefix = "inertia", ...props }) {
  if (value !== activeValue) return null;
  return (
    <div
      role="tabpanel"
      id={`${idPrefix}-panel-${value}`}
      aria-labelledby={`${idPrefix}-tab-${value}`}
      tabIndex={0}
      className={cn("outline-none", className)}
      {...props}
    >
      {children}
    </div>
  );
}

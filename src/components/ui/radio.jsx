import * as React from "react";
import { cn } from "@/lib/utils";

const RadioContext = React.createContext(null);

/**
 * Roving tabindex: the group is a single tab stop and arrows move between items.
 * Order is read from the DOM rather than a registry so nesting and conditional
 * items can never scramble it.
 */
function RadioGroup({ value, onChange, children, className, label, ...props }) {
  const groupRef = React.useRef(null);

  const enabledItems = React.useCallback(() => {
    const root = groupRef.current;
    if (!root) return [];
    return Array.from(root.querySelectorAll('[role="radio"]')).filter((n) => !n.disabled);
  }, []);

  const move = React.useCallback(
    (node, delta) => {
      const list = enabledItems();
      if (!list.length) return;
      const at = list.indexOf(node);
      const next = list[(at + delta + list.length) % list.length];
      next.focus();
      next.click();
    },
    [enabledItems]
  );

  // When nothing is selected the first enabled item holds the group's tab stop.
  const isFirst = React.useCallback((node) => enabledItems()[0] === node, [enabledItems]);

  const ctx = React.useMemo(
    () => ({ value, onChange, move, isFirst }),
    [value, onChange, move, isFirst]
  );

  return (
    <RadioContext.Provider value={ctx}>
      <div
        ref={groupRef}
        role="radiogroup"
        aria-label={label}
        className={cn("flex flex-col gap-1", className)}
        {...props}
      >
        {children}
      </div>
    </RadioContext.Provider>
  );
}

function RadioItem({ value, label, description, disabled = false, className, ...props }) {
  const ctx = React.useContext(RadioContext);
  const ref = React.useRef(null);
  const selected = ctx?.value === value;
  const [fallbackStop, setFallbackStop] = React.useState(false);

  React.useLayoutEffect(() => {
    if (ctx?.value == null && !disabled) setFallbackStop(!!ctx?.isFirst(ref.current));
    else setFallbackStop(false);
  }, [ctx, disabled]);

  function handleKeyDown(e) {
    if (e.key === "ArrowDown" || e.key === "ArrowRight") {
      e.preventDefault();
      ctx?.move(ref.current, 1);
    } else if (e.key === "ArrowUp" || e.key === "ArrowLeft") {
      e.preventDefault();
      ctx?.move(ref.current, -1);
    } else if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      ctx?.onChange?.(value);
    }
  }

  return (
    <button
      ref={ref}
      type="button"
      role="radio"
      aria-checked={selected}
      disabled={disabled}
      tabIndex={selected || fallbackStop ? 0 : -1}
      onClick={() => ctx?.onChange?.(value)}
      onKeyDown={handleKeyDown}
      className={cn(
        "flex w-full items-start gap-2.5 rounded-lg px-2 py-1.5 text-left outline-none",
        "transition-colors duration-150 ease-out hover:fill-nav",
        "focus-visible:fill-nav",
        "disabled:pointer-events-none disabled:opacity-40",
        className
      )}
      {...props}
    >
      <span
        aria-hidden="true"
        className={cn(
          "mt-px flex size-[18px] shrink-0 items-center justify-center rounded-full",
          "transition-colors duration-150 ease-out",
          // Both states are the same circle; only the fill changes. The
          // selected one is solid ink with the dot punched out of it in the
          // page colour, which keeps the silhouette identical - the eye reads
          // a change of shape as a change of kind.
          selected ? "bg-foreground" : "fill-secondary"
        )}
      >
        {selected ? <span className="size-2 rounded-full bg-background" /> : null}
      </span>
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="text-[13px] text-foreground">{label}</span>
        {description ? (
          <span className="text-xs leading-relaxed text-muted-foreground">{description}</span>
        ) : null}
      </span>
    </button>
  );
}

export { RadioGroup, RadioItem };

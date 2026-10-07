import * as React from "react";
import { Check, Minus } from "@/components/icons";
import { cn } from "@/lib/utils";

// 18px at 3px, from app-controls - which specifies 2.7px and then says in its
// own notes that 3 is the nearest rung. `rounded-xs` IS 3px, and moves with the
// corner setting instead of ignoring it.
const SIZES = {
  sm: "size-4 rounded-xs",
  md: "size-[18px] rounded-xs",
};

function Checkbox({
  checked = false,
  indeterminate = false,
  onCheckedChange,
  disabled = false,
  size = "md",
  className,
  id,
  label,
  ...props
}) {
  const on = indeterminate || checked;

  function toggle() {
    if (disabled) return;
    onCheckedChange?.(indeterminate ? true : !checked);
  }

  function handleKeyDown(e) {
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      toggle();
    }
  }

  return (
    <button
      id={id}
      type="button"
      role="checkbox"
      aria-checked={indeterminate ? "mixed" : checked}
      aria-label={label}
      disabled={disabled}
      data-state={indeterminate ? "indeterminate" : checked ? "checked" : "unchecked"}
      onClick={toggle}
      onKeyDown={handleKeyDown}
      className={cn(
        "inline-flex shrink-0 items-center justify-center outline-none",
        "transition-colors duration-150 ease-out",
        "focus-visible:fill-secondary-hover",
        "disabled:pointer-events-none disabled:opacity-40",
        // Unchecked is the secondary fill - the firmest of the quiet rungs,
        // because an empty box is the one control that has nothing else to
        // show for itself once the outline is gone.
        on ? "bg-foreground text-background" : "fill-secondary hover:fill-secondary-hover",
        SIZES[size] ?? SIZES.md,
        className
      )}
      {...props}
    >
      {/* The mark pops in rather than appears: a tick is a small thing to
          notice, and a little scale is what makes the box answer the click. */}
      {indeterminate ? (
        <Minus className="size-3 animate-pop-in" aria-hidden="true" />
      ) : checked ? (
        <Check className="size-3 animate-pop-in" aria-hidden="true" />
      ) : null}
    </button>
  );
}

export { Checkbox };

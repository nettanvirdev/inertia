import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * ON is the inverted foreground, not a brand hue: a near-black brand colour on
 * a dark track reads as OFF. Inverted foreground can never be ambiguous, which
 * is why the spec's blue is not used here - it has only a light mode, and blue
 * on #181818 is the one thing this control cannot afford to be.
 *
 * OFF is a fixed grey rather than a translucent one, per the spec. A track made
 * of alpha is a different grey on a card than on the ground, and a switch has
 * to look like itself everywhere on the screen. Dark keeps the alpha, for the
 * reason in the stylesheet: #c4c4c4 there is brighter than ON.
 */
const SIZES = {
  sm: {
    track: "h-[18px] w-8",
    thumb: "size-3.5",
    on: "translate-x-[14px]",
    off: "translate-x-[2px]",
  },
  // 42x20 with a 16px knob, from app-controls.
  md: {
    track: "h-5 w-[42px]",
    thumb: "size-4",
    on: "translate-x-[24px]",
    off: "translate-x-[2px]",
  },
};

function Switch({
  checked = false,
  onCheckedChange,
  disabled = false,
  size = "md",
  className,
  id,
  label,
  ...props
}) {
  const s = SIZES[size] ?? SIZES.md;

  function toggle() {
    if (disabled) return;
    onCheckedChange?.(!checked);
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
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      data-state={checked ? "checked" : "unchecked"}
      onClick={toggle}
      onKeyDown={handleKeyDown}
      className={cn(
        "relative inline-flex shrink-0 items-center rounded-full outline-none",
        "transition-colors duration-150 ease-out",
        "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-foreground/40",
        "disabled:pointer-events-none disabled:opacity-40",
        checked ? "accent-fill" : "bg-switch-off hover:bg-switch-off-hover",
        s.track,
        className
      )}
      {...props}
    >
      <span
        aria-hidden="true"
        className={cn(
          "pointer-events-none rounded-full bg-background",
          // The one overshoot in the app: the knob lands a hair past its
          // stop and settles, which is what makes a flick feel like a click.
          "transition-transform duration-[var(--motion-base)] ease-[var(--ease-spring)]",
          s.thumb,
          checked ? s.on : s.off
        )}
      />
    </button>
  );
}

export { Switch };

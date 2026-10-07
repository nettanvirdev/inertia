import * as React from "react";
import { cn } from "@/lib/utils";

function clamp(n, min, max) {
  return Math.min(max, Math.max(min, n));
}

/**
 * `tone` is about what the filled part of the track means.
 *
 * On a setting it is the value, and the value is the point - it takes the
 * foreground. On a scrubber under a chart it is only elapsed time, and a
 * full-width white bar there is louder than the coloured bars it is meant to
 * serve; "muted" drops it below them.
 */
const TONE = {
  default: { fill: "bg-foreground", thumb: "bg-foreground size-3.5" },
  muted: { fill: "bg-muted-foreground", thumb: "bg-muted-foreground size-3" },
};

function Slider({
  value = 0,
  onChange,
  min = 0,
  max = 100,
  step = 1,
  disabled = false,
  tone = "default",
  className,
  label,
  formatValue,
  ...props
}) {
  const skin = TONE[tone] ?? TONE.default;
  const trackRef = React.useRef(null);
  const dragging = React.useRef(false);

  const pct = max > min ? ((clamp(value, min, max) - min) / (max - min)) * 100 : 0;

  const commit = React.useCallback(
    (clientX) => {
      const rect = trackRef.current?.getBoundingClientRect();
      if (!rect || rect.width === 0) return;
      const ratio = clamp((clientX - rect.left) / rect.width, 0, 1);
      const raw = min + ratio * (max - min);
      const snapped = clamp(Math.round(raw / step) * step, min, max);
      // step can be fractional - kill float dust so 0.30000000000000004 never ships
      const decimals = (String(step).split(".")[1] || "").length;
      onChange?.(Number(snapped.toFixed(decimals)));
    },
    [min, max, step, onChange]
  );

  React.useEffect(() => {
    if (disabled) return undefined;
    function onMove(e) {
      if (dragging.current) commit(e.clientX);
    }
    function onUp() {
      dragging.current = false;
    }
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [commit, disabled]);

  function handlePointerDown(e) {
    if (disabled) return;
    dragging.current = true;
    e.currentTarget.focus();
    commit(e.clientX);
  }

  function handleKeyDown(e) {
    if (disabled) return;
    const big = (max - min) / 10;
    const map = {
      ArrowRight: step,
      ArrowUp: step,
      ArrowLeft: -step,
      ArrowDown: -step,
      PageUp: big,
      PageDown: -big,
    };
    if (e.key in map) {
      e.preventDefault();
      onChange?.(clamp(value + map[e.key], min, max));
    } else if (e.key === "Home") {
      e.preventDefault();
      onChange?.(min);
    } else if (e.key === "End") {
      e.preventDefault();
      onChange?.(max);
    }
  }

  return (
    <div
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuenow={value}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuetext={formatValue ? formatValue(value) : undefined}
      aria-disabled={disabled || undefined}
      onPointerDown={handlePointerDown}
      onKeyDown={handleKeyDown}
      className={cn(
        "group relative flex h-4 w-full touch-none items-center rounded-full outline-none select-none",
        "focus-visible:fill-control-hover",
        disabled && "pointer-events-none opacity-40",
        className
      )}
      {...props}
    >
      <div ref={trackRef} className="relative h-1 w-full rounded-full fill-track">
        <div className={cn("h-full rounded-full", skin.fill)} style={{ width: `${pct}%` }} />
        <div
          className={cn(
            "absolute top-1/2 -translate-x-1/2 -translate-y-1/2 rounded-full",
            skin.thumb
          )}
          style={{ left: `${pct}%` }}
        />
      </div>
    </div>
  );
}

export { Slider };

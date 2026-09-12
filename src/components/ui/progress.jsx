import * as React from "react";
import { cn } from "@/lib/utils";

function pct(value, max) {
  if (!max || max <= 0) return 0;
  return Math.min(100, Math.max(0, (value / max) * 100));
}

function Progress({ value = 0, max = 100, indeterminate = false, className, label, ...props }) {
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={indeterminate ? undefined : Math.min(max, Math.max(0, value))}
      className={cn("h-1 w-full overflow-hidden rounded-full fill-track", className)}
      {...props}
    >
      <div
        className={cn(
          "h-full rounded-full bg-foreground",
          indeterminate ? "w-1/3 animate-soft-pulse" : "transition-[width] duration-200 ease-out"
        )}
        style={indeterminate ? undefined : { width: `${pct(value, max)}%` }}
      />
    </div>
  );
}

const TONES = {
  neutral: "bg-foreground",
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-destructive",
  info: "bg-info",
};

/** A labelled resource gauge (CPU / RAM) - the bar plus its readout as one unit. */
function Meter({ value = 0, max = 100, label, tone = "neutral", formatValue, className, ...props }) {
  const p = pct(value, max);
  return (
    <div className={cn("flex w-full flex-col gap-1.5", className)} {...props}>
      {(label || formatValue) && (
        <div className="flex items-baseline justify-between gap-2 text-[11px]">
          <span className="text-muted-foreground">{label}</span>
          <span className="tabular-nums text-foreground">
            {formatValue ? formatValue(value) : `${Math.round(p)}%`}
          </span>
        </div>
      )}
      <div
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={max}
        aria-valuenow={value}
        className="h-1 w-full overflow-hidden rounded-full fill-track"
      >
        <div
          className={cn("h-full rounded-full transition-[width] duration-200 ease-out", TONES[tone] ?? TONES.neutral)}
          style={{ width: `${p}%` }}
        />
      </div>
    </div>
  );
}

export { Progress, Meter };

import * as React from "react";
import { cn } from "@/lib/utils";
import { formatTokens } from "@shared/usage";

/**
 * How full the model's context is.
 *
 * A number the app always had and never showed. The loop knows the window,
 * the provider reports how much of it the last request used, and until this
 * existed the first the person heard of either was a warning that the oldest
 * messages had been summarised - which is the moment it is too late to do
 * anything about it. Claude Code has `/context`, Codex has a tool the model
 * calls; a small ring in the header is the version that needs no asking.
 *
 * Three colours and no more. Quiet until 80%, amber from there, red at 85%:
 * amber is "wrap this up or start a new chat", red is "the next step will be
 * summarised", and 85% is the number that is actually true of the second one.
 * Both are things a person can act on, which is the test for putting a colour
 * on anything.
 */

const AMBER_AT = 0.8;
// The same number automatic compaction fires at, deliberately. Red used to be
// 0.95, above the threshold - so the band could only be reached with automatic
// compaction switched off, and the one colour that means "this is about to
// happen" was the one nobody saw.
const RED_AT = 0.85;

export function ContextMeter({ context, className }) {
  const used = Number(context?.used) || 0;
  const window = Number(context?.window) || 0;
  if (!window) return null;
  const ratio = Math.min(1, used / window);
  const percent = Math.round(ratio * 100);
  const tone = ratio >= RED_AT ? "text-destructive-ink" : ratio >= AMBER_AT ? "text-warning-ink" : "text-muted-foreground";

  // A ring is read at a glance the way a battery is; a bar this small is a
  // line of a different length. Radius 5 in a 14px box, so the stroke sits
  // inside the pill's text height.
  const radius = 5;
  const circumference = 2 * Math.PI * radius;

  return (
    <span
      className={cn(
        "hidden shrink-0 items-center gap-1.5 rounded-full fill-control px-2 py-0.5 text-[11px] tabular-nums md:inline-flex",
        tone,
        className
      )}
      title={`Context: ${formatTokens(used)} of ${formatTokens(window)} tokens used (${percent}%)${
        ratio >= AMBER_AT ? ". Older messages will be summarised soon." : ""
      }`}
      aria-label={`Context ${percent}% full`}
    >
      <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true" className="shrink-0">
        <circle cx="7" cy="7" r={radius} fill="none" stroke="currentColor" strokeOpacity="0.25" strokeWidth="2" />
        <circle
          cx="7"
          cy="7"
          r={radius}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={circumference * (1 - ratio)}
          transform="rotate(-90 7 7)"
          style={{ transition: "stroke-dashoffset 400ms ease-out" }}
        />
      </svg>
      {percent}%
    </span>
  );
}

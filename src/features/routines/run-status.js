import { RUN_STATUS_META } from "@/data";

/**
 * Run status → the palette classes it is allowed to spend. Status is the one
 * place colour is permitted, so it is centralised here rather than re-derived
 * in every routine surface.
 */
const TONES = {
  success: {
    badge: "success",
    dot: "bg-emerald-500",
    ink: "text-emerald-600 dark:text-emerald-400",
    bar: "bg-emerald-500",
  },
  warning: {
    badge: "warning",
    dot: "bg-amber-500",
    ink: "text-amber-600 dark:text-amber-400",
    bar: "bg-amber-500",
  },
  error: {
    badge: "danger",
    dot: "bg-red-500",
    ink: "text-red-600 dark:text-red-400",
    bar: "bg-red-500",
  },
  running: {
    badge: "info",
    dot: "bg-blue-500",
    ink: "text-blue-600 dark:text-blue-400",
    bar: "bg-blue-500",
  },
};

const NEVER = {
  badge: "neutral",
  dot: "bg-muted-foreground/50",
  ink: "text-muted-foreground",
  bar: "fill-track",
};

export function runTone(status) {
  return TONES[status] ?? NEVER;
}

export function runLabel(status) {
  return RUN_STATUS_META[status]?.label ?? "Never run";
}

/** Percentage of finished runs that succeeded, or null when nothing has run. */
export function successRate(routine) {
  const done = (routine?.runHistory ?? []).filter((r) => r.status !== "running");
  if (!done.length) return null;
  const ok = done.filter((r) => r.status === "success").length;
  return Math.round((ok / done.length) * 100);
}

/** Absolute UTC stamp, used wherever a relative time needs a precise twin. */
export function absoluteTime(iso) {
  if (!iso) return "-";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "-";
  return d.toISOString().replace("T", " ").slice(0, 16) + " UTC";
}

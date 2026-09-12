/**
 * The fixed parts of the vocabulary the screens are written against.
 *
 * Per-kind labels and icons, the model catalogue, and the time and number
 * formatters every list uses. Icons are icon-set *names* as strings; components
 * map them to real components themselves.
 *
 * It was the demo data once, and the seed arrays are gone: a fresh install
 * starts on an empty workspace and every record on screen is read back from it.
 * What is left is the part that was never data about a user - the meanings.
 */

import { MODELS } from './models.js';

export * from './agents.js';
export * from './models.js';
export * from './computers.js';
export * from './routines.js';
export * from './memories.js';
export * from './activity.js';
export * from './user.js';
export * from './library.js';


export function getModel(id) {
  return MODELS.find((m) => m.id === id);
}

/* ── Time formatting ───────────────────────────────────────────────────── */

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/**
 * "just now" / "3m ago" / "2h ago" / "Yesterday" / "Mar 4" / "Mar 4, 2025".
 * Compares against the wall clock, because the strings it produces are read by
 * someone looking at records they may have made moments ago. Pass `nowIso` to
 * pin the comparison when you need a deterministic result.
 */
export function relativeTime(iso, nowIso) {
  if (!iso) return '-';
  const then = new Date(iso);
  const now = nowIso == null ? new Date() : new Date(nowIso);
  if (Number.isNaN(then.getTime())) return '-';

  const diffMs = now.getTime() - then.getTime();

  if (diffMs < 0) {
    const ahead = Math.abs(diffMs);
    const mins = Math.round(ahead / 60_000);
    if (mins < 1) return 'in a moment';
    if (mins < 60) return `in ${mins}m`;
    const hours = Math.round(mins / 60);
    if (hours < 24) return `in ${hours}h`;
    const days = Math.round(hours / 24);
    if (days === 1) return 'Tomorrow';
    if (days < 7) return `in ${days}d`;
    return `${MONTHS[then.getUTCMonth()]} ${then.getUTCDate()}`;
  }

  const seconds = Math.floor(diffMs / 1000);
  if (seconds < 45) return 'just now';

  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;

  const hours = Math.floor(minutes / 60);
  if (hours < 24 && then.getUTCDate() === now.getUTCDate()) return `${hours}h ago`;

  const startOfToday = Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate());
  const startOfThen = Date.UTC(then.getUTCFullYear(), then.getUTCMonth(), then.getUTCDate());
  const dayDiff = Math.round((startOfToday - startOfThen) / 86_400_000);

  if (dayDiff === 0) return `${hours}h ago`;
  if (dayDiff === 1) return 'Yesterday';
  if (dayDiff < 7) return `${dayDiff}d ago`;

  const label = `${MONTHS[then.getUTCMonth()]} ${then.getUTCDate()}`;
  return then.getUTCFullYear() === now.getUTCFullYear() ? label : `${label}, ${then.getUTCFullYear()}`;
}

/** "1m 14s" / "412ms" / "2h 03m" - for tool durations and routine runtimes. */
export function formatDuration(ms) {
  if (ms == null) return '-';
  if (ms < 1000) return `${ms}ms`;
  const totalSeconds = Math.round(ms / 1000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}m ${String(seconds).padStart(2, '0')}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${String(minutes % 60).padStart(2, '0')}m`;
}

/** 91402755 -> "91.4M" */
export function formatCompactNumber(value) {
  if (value == null) return '-';
  const abs = Math.abs(value);
  if (abs >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(1)}B`;
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (abs >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

/** 94218640 -> "89.9 MB" */
export function formatBytes(bytes) {
  if (bytes == null) return '-';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

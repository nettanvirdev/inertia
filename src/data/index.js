/**
 * Inertia demo data - single entry point.
 *
 * Everything here is static, deterministic and framework-free. Icons are icon-set
 * *names* as strings; components map them to real components themselves.
 *
 * All timestamps are fixed ISO strings clustered around NOW so the fixtures
 * stay consistent with each other. Formatters still compare them against the
 * wall clock, so seeded rows age like anything else the user made. Nothing
 * calls Date.now() at module scope.
 */

export * from './agents.js';
export * from './models.js';
export * from './threads.js';
export * from './computers.js';
export * from './routines.js';
export * from './memories.js';
export * from './activity.js';
export * from './integrations.js';
export * from './user.js';
export * from './library.js';

import { AGENTS } from './agents.js';
import { MODELS, PROVIDERS } from './models.js';
import { THREADS, MESSAGES } from './threads.js';
import { ROUTINES } from './routines.js';
import { MEMORIES } from './memories.js';
import { ACTIVITY } from './activity.js';
import { INTEGRATIONS } from './integrations.js';
import { CURRENT_USER } from './user.js';
import { SHORTCUTS } from './shortcuts.js';
import { LIBRARY_ITEMS, LIBRARY_CATEGORY_META, LIBRARY_CATEGORIES } from './library.js';

/**
 * The instant the seed fixtures below are written around. It exists so the
 * fixture files can cluster their timestamps somewhere sensible, and so a test
 * can pin a comparison. It is deliberately NOT the clock any formatter uses:
 * a record the user created a second ago has to read as "just now", and it
 * cannot if every comparison is made against a date frozen at authoring time.
 */
export const NOW = '2026-09-01T14:20:00Z';

/* ── Entity lookups ────────────────────────────────────────────────────── */

export function getAgent(id) {
  return AGENTS.find((b) => b.id === id);
}

export function getThread(id) {
  return THREADS.find((t) => t.id === id);
}

export function getMessages(threadId) {
  return MESSAGES[threadId] ?? [];
}

export function getThreadsForAgent(agentId) {
  return THREADS.filter((t) => t.agentId === agentId).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

export function getRoutinesForAgent(agentId) {
  return ROUTINES.filter((r) => r.agentId === agentId);
}

export function getMemoriesForAgent(agentId) {
  return MEMORIES.filter((m) => m.agentId === agentId).sort((a, b) => {
    if (a.pinned !== b.pinned) return Number(b.pinned) - Number(a.pinned);
    return b.lastUsedAt.localeCompare(a.lastUsedAt);
  });
}

export function getModel(id) {
  return MODELS.find((m) => m.id === id);
}

export function getProvider(id) {
  return PROVIDERS.find((p) => p.id === id);
}

export function getRoutine(id) {
  return ROUTINES.find((r) => r.id === id);
}

export function getMemory(id) {
  return MEMORIES.find((m) => m.id === id);
}

export function getIntegration(id) {
  return INTEGRATIONS.find((i) => i.id === id);
}

export function getActivityForAgent(agentId) {
  return ACTIVITY.filter((a) => a.agentId === agentId);
}

export function getActivityForComputer(computerId) {
  return ACTIVITY.filter((a) => a.computerId === computerId);
}

/* ── Aggregates ────────────────────────────────────────────────────────────
 *
 * `getAppStats` and `getAgentDetail` used to live here and are gone. Both
 * summed over the seed arrays, which are empty now that a fresh install starts
 * on an empty workspace, and neither was called from anywhere. `getAgentDetail`
 * had also been broken for some time: it called `getComputer`, which does not
 * exist in this module - computers are the main process's, not the seed's - so
 * any agent with a machine attached would have thrown a ReferenceError. A dead
 * function that would crash if it were ever wired up is worse than no function.
 */

/* ── Command palette search ────────────────────────────────────────────── */

function matches(query, ...fields) {
  const q = query.trim().toLowerCase();
  if (!q) return false;
  return fields.some((f) => typeof f === 'string' && f.toLowerCase().includes(q));
}

/**
 * Grouped fuzzy-ish search across every entity the palette can jump to.
 * Returns `{ query, total, groups: [{ id, label, icon, items: [...] }] }`.
 * Each item is `{ id, type, title, subtitle, icon, agentId }`.
 */
export function searchAll(query) {
  const q = (query ?? '').trim();
  if (!q) {
    return { query: '', total: 0, groups: [] };
  }

  const agents = AGENTS.filter((b) => matches(q, b.name, b.handle, b.role, b.description, ...b.tags)).map((b) => ({
    id: b.id,
    type: 'agent',
    title: b.name,
    subtitle: b.role,
    icon: 'Agent',
    agentId: b.id,
  }));

  const threads = THREADS.filter((t) => matches(q, t.title, t.preview)).map((t) => ({
    id: t.id,
    type: 'thread',
    title: t.title,
    subtitle: getAgent(t.agentId)?.name ?? 'Unassigned',
    icon: 'MessageCircle',
    agentId: t.agentId,
  }));

  const routines = ROUTINES.filter((r) => matches(q, r.name, r.description, ...r.tags)).map((r) => ({
    id: r.id,
    type: 'routine',
    title: r.name,
    subtitle: r.schedule.humanLabel,
    icon: r.icon,
    agentId: r.agentId,
  }));

  const memories = MEMORIES.filter((m) => matches(q, m.title, m.body, ...m.tags)).map((m) => ({
    id: m.id,
    type: 'memory',
    title: m.title,
    subtitle: getAgent(m.agentId)?.name ?? '',
    icon: 'Brain',
    agentId: m.agentId,
  }));

  const integrations = INTEGRATIONS.filter((i) => matches(q, i.name, i.description, i.category)).map((i) => ({
    id: i.id,
    type: 'integration',
    title: i.name,
    subtitle: i.connected ? `Connected · ${i.toolCount} tools` : 'Not connected',
    icon: i.icon,
    agentId: null,
  }));

  const groups = [
    { id: 'agents', label: 'Agents', icon: 'Agent', items: agents },
    { id: 'threads', label: 'Threads', icon: 'MessageCircle', items: threads },
    { id: 'routines', label: 'Routines', icon: 'Repeat', items: routines },
    { id: 'memories', label: 'Memories', icon: 'Brain', items: memories },
    { id: 'integrations', label: 'Integrations', icon: 'Plug', items: integrations },
  ].filter((g) => g.items.length > 0);

  return {
    query: q,
    total: groups.reduce((sum, g) => sum + g.items.length, 0),
    groups,
  };
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

export const DATA = {
  NOW,
  AGENTS,
  MODELS,
  PROVIDERS,
  THREADS,
  MESSAGES,
  ROUTINES,
  MEMORIES,
  ACTIVITY,
  INTEGRATIONS,
  CURRENT_USER,
  SHORTCUTS,
  LIBRARY_ITEMS,
  LIBRARY_CATEGORY_META,
  LIBRARY_CATEGORIES,
};

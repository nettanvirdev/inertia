/*
 * What a group conversation is allowed to do on its own.
 *
 * Shared because both halves need the same numbers and neither can be the one
 * that decides them: the main process enforces these, and the settings screen
 * shows a person what they are. Two copies would drift, and the drift would
 * show up as a switch that says one thing while the app does another.
 */

/** How many agent turns may pass without the person, before the floor returns. */
export const MAX_HOPS = 8;

/** How many agents may be in one conversation at once. */
export const MAX_AGENTS = 6;

/**
 * An agent's name as it is written after `@`.
 *
 * Lowercased and hyphenated so "Folder Organizer" is one token. The same rule
 * the composer uses for its pills, kept here because the main process reads
 * the agents' own replies for these and both sides must agree on the spelling.
 */
export function slugOf(name) {
  return String(name ?? "")
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "");
}

/**
 * The ids of the agents `text` names with `@`, in the order they are named,
 * without repeats. The whole hyphenated word after the `@` is the handle, so
 * "@nova-reyes" is Nova Reyes and not Nova followed by a word. A handle glued
 * to a letter is not a mention: an email address is not asking anybody anything.
 */
export function mentionedAgents(text, agents = []) {
  const value = String(text ?? "");
  if (!value.includes("@")) return [];
  const byHandle = new Map();
  for (const agent of agents) {
    const slug = slugOf(agent?.name);
    if (agent?.id && slug && !byHandle.has(slug)) byHandle.set(slug, agent.id);
  }
  const found = [];
  const pattern = /(^|[^\p{L}\p{N}])@([\p{L}\p{N}-]+)/gu;
  for (const match of value.matchAll(pattern)) {
    const id = byHandle.get(match[2].toLowerCase().replace(/-+$/, ""));
    if (id && !found.includes(id)) found.push(id);
  }
  return found;
}

export const DEFAULT_GROUP = {
  /** The person's own words about how their agents should collaborate. */
  prompt: "",
  permissions: {
    /** May an agent bring another one in? */
    canInvite: true,
    /** May an agent hand the conversation over, name and face and all? */
    canHandover: true,
    /** May an agent decide it is done and step out? */
    canLeave: true,
    maxAgents: MAX_AGENTS,
    maxHops: MAX_HOPS,
  },
};

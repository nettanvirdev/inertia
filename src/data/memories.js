/**
 * Per-agent long-term memory entries. Mixed provenance and confidence so the UI
 * can show pinned, learned, stale and low-confidence states.
 */

/**
 * Empty on purpose.
 *
 * This was a roster of invented memories, and it was not merely unused - it was
 * what a brand new workspace got written into it on first open. Which meant a
 * fresh install opened onto someone else's work, and a real folder ended up
 * holding records nobody made and nothing could act on.
 *
 * The app handles an empty workspace everywhere; that is what the first launch
 * should look like.
 */
export const MEMORIES = [];

export const MEMORY_KIND_META = {
  fact: { label: 'Fact', icon: 'Lightbulb' },
  preference: { label: 'Preference', icon: 'Heart' },
  contact: { label: 'Contact', icon: 'UserRound' },
  project: { label: 'Project', icon: 'FolderKanban' },
  'credential-note': { label: 'Credential note', icon: 'KeyRound' },
  handover: { label: 'Where we left off', icon: 'Brain' },
};

export const MEMORY_SOURCE_META = {
  learned: { label: 'Learned', icon: 'Brain' },
  pinned: { label: 'Pinned by user', icon: 'Pin' },
  imported: { label: 'Imported', icon: 'Download' },
};

/** Below this, the UI shows a "low confidence" treatment. */
export const LOW_CONFIDENCE_THRESHOLD = 0.5;

/** Memories unused for longer than this many days read as stale. */
export const STALE_AFTER_DAYS = 21;

export function getMemoryById(id) {
  return MEMORIES.find((m) => m.id === id);
}

export function getMemoriesByAgent(agentId) {
  return MEMORIES.filter((m) => m.agentId === agentId);
}

export function getPinnedMemories(agentId) {
  return MEMORIES.filter((m) => m.pinned && (!agentId || m.agentId === agentId));
}

export function getLowConfidenceMemories() {
  return MEMORIES.filter((m) => m.confidence < LOW_CONFIDENCE_THRESHOLD);
}

/**
 * Memories whose lastUsedAt is older than STALE_AFTER_DAYS relative to `nowIso`,
 * which is the wall clock unless a caller pins it. Staleness that was measured
 * against a hardcoded date would freeze the day a memory stops counting as
 * fresh, and nothing seeded would ever go stale.
 */
export function getStaleMemories(nowIso) {
  const cutoff = (nowIso == null ? Date.now() : new Date(nowIso).getTime()) - STALE_AFTER_DAYS * 86_400_000;
  return MEMORIES.filter((m) => new Date(m.lastUsedAt).getTime() < cutoff);
}

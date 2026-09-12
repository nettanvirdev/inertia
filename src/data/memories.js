/**
 * How a memory is labelled, and when it counts as stale or unsure.
 *
 * The memories themselves are the workspace's, written by the capture pass and
 * the save tool. This is what the screen reads them with.
 */

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

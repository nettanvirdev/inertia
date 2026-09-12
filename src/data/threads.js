/**
 * Conversations between the user and their agents.
 *
 * MESSAGES[threadId] is an ordered array of blocks. A block is either
 * `type: 'message'` (role user | agent | system) or `type: 'tool'` - a tool-call
 * card the UI renders inline with its own state, timing, input and output.
 */

/**
 * Empty on purpose.
 *
 * This was a roster of invented conversations, and it was not merely unused - it was
 * what a brand new workspace got written into it on first open. Which meant a
 * fresh install opened onto someone else's work, and a real folder ended up
 * holding records nobody made and nothing could act on.
 *
 * The app handles an empty workspace everywhere; that is what the first launch
 * should look like.
 */
export const THREADS = [];

/**
 * Empty on purpose.
 *
 * This was a roster of invented transcripts, and it was not merely unused - it was
 * what a brand new workspace got written into it on first open. Which meant a
 * fresh install opened onto someone else's work, and a real folder ended up
 * holding records nobody made and nothing could act on.
 *
 * The app handles an empty workspace everywhere; that is what the first launch
 * should look like.
 */
export const MESSAGES = {};

export const TOOL_META = {
  terminal: { label: 'Terminal', icon: 'SquareTerminal' },
  browser: { label: 'Browser', icon: 'Globe' },
  files: { label: 'Files', icon: 'FolderTree' },
  desktop: { label: 'Desktop', icon: 'Monitor' },
  search: { label: 'Web search', icon: 'Search' },
  delegate: { label: 'Delegate', icon: 'Users' },
};

export const TOOL_STATE_META = {
  running: { label: 'Running', icon: 'Loader' },
  success: { label: 'Done', icon: 'Check' },
  error: { label: 'Failed', icon: 'X' },
};

export function getThreadById(id) {
  return THREADS.find((t) => t.id === id);
}

export function getMessagesForThread(threadId) {
  return MESSAGES[threadId] ?? [];
}

export function getThreadsByAgent(agentId) {
  return THREADS.filter((t) => t.agentId === agentId);
}

export function getPinnedThreads() {
  return THREADS.filter((t) => t.pinned);
}

export function getUnreadThreads() {
  return THREADS.filter((t) => t.unread);
}

/** Newest first. */
export function getSortedThreads() {
  return THREADS.slice().sort((a, b) => {
    if (a.pinned !== b.pinned) return Number(b.pinned) - Number(a.pinned);
    return b.updatedAt.localeCompare(a.updatedAt);
  });
}

export function getToolCalls(threadId) {
  return getMessagesForThread(threadId).filter((m) => m.type === 'tool');
}

/** The thread currently mid-stream, if any - used to demo the live state. */
export function getStreamingThread() {
  for (const [threadId, blocks] of Object.entries(MESSAGES)) {
    if (blocks.some((b) => b.status === 'streaming' || (b.type === 'tool' && b.state === 'running'))) {
      return getThreadById(threadId);
    }
  }
  return undefined;
}

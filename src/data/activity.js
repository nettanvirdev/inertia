/**
 * Audit log (newest first) plus the per-agent permission matrix.
 */

/**
 * Empty on purpose.
 *
 * This was a roster of invented events, and it was not merely unused - it was
 * what a brand new workspace got written into it on first open. Which meant a
 * fresh install opened onto someone else's work, and a real folder ended up
 * holding records nobody made and nothing could act on.
 *
 * The app handles an empty workspace everywhere; that is what the first launch
 * should look like.
 */
export const ACTIVITY = [];

export const ACTIVITY_CATEGORY_META = {
  permission: { label: 'Permission', icon: 'ShieldAlert' },
  tool: { label: 'Tool use', icon: 'Wrench' },
  // What went wrong during a turn: a tool that failed, a provider that
  // refused, a turn that ended in an error. Its own category rather than a
  // severity on "tool use", because "show me what failed" is the question
  // this feed is opened to answer, and a filter has to be able to ask it.
  failure: { label: 'Failure', icon: 'TriangleAlert' },
  routine: { label: 'Routine', icon: 'Repeat' },
  thread: { label: 'Thread', icon: 'MessageCircle' },
  memory: { label: 'Memory', icon: 'Brain' },
  computer: { label: 'Computer', icon: 'Monitor' },
  integration: { label: 'Integration', icon: 'Plug' },
  auth: { label: 'Auth', icon: 'KeyRound' },
};

export const SEVERITY_META = {
  // `wash` is here rather than derived because the row that draws these used
  // to build its tint by concatenating "1a" onto the hex - a trick that works
  // only while the value IS a hex, and would have silently produced no
  // background at all the moment one became a token.
  info: { label: 'Info', color: 'var(--info)', wash: 'var(--info-wash)', icon: 'Info' },
  success: { label: 'Success', color: 'var(--success)', wash: 'var(--success-wash)', icon: 'CircleCheck' },
  warning: { label: 'Warning', color: 'var(--warning)', wash: 'var(--warning-wash)', icon: 'TriangleAlert' },
  danger: { label: 'Blocked', color: 'var(--destructive)', wash: 'var(--destructive-wash)', icon: 'OctagonAlert' },
};

export function getActivityByAgent(agentId) {
  return ACTIVITY.filter((a) => a.agentId === agentId);
}

export function getActivityByCategory(category) {
  return ACTIVITY.filter((a) => a.category === category);
}

export function getActivityByComputer(computerId) {
  return ACTIVITY.filter((a) => a.computerId === computerId);
}

export function getBlockedActivity() {
  return ACTIVITY.filter((a) => a.severity === 'danger');
}


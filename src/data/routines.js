/**
 * Routines - plain Markdown playbooks an agent runs on a schedule, a trigger, or on demand.
 */

/**
 * Empty on purpose.
 *
 * This was a roster of invented routines, and it was not merely unused - it was
 * what a brand new workspace got written into it on first open. Which meant a
 * fresh install opened onto someone else's work, and a real folder ended up
 * holding records nobody made and nothing could act on.
 *
 * The app handles an empty workspace everywhere; that is what the first launch
 * should look like.
 */
export const ROUTINES = [];

export const SCHEDULE_KIND_META = {
  cron: { label: 'Scheduled', icon: 'CalendarClock' },
  interval: { label: 'Interval', icon: 'Timer' },
  manual: { label: 'Manual', icon: 'Hand' },
  trigger: { label: 'Triggered', icon: 'Zap' },
};

export const RUN_STATUS_META = {
  success: { label: 'Succeeded', icon: 'CircleCheck' },
  warning: { label: 'Partial', icon: 'TriangleAlert' },
  error: { label: 'Failed', icon: 'CircleX' },
  running: { label: 'Running', icon: 'Loader' },
};

export function getRoutineById(id) {
  return ROUTINES.find((r) => r.id === id);
}

export function getRoutinesByAgent(agentId) {
  return ROUTINES.filter((r) => r.agentId === agentId);
}

export function getEnabledRoutines() {
  return ROUTINES.filter((r) => r.enabled);
}

export function getRunningRoutines() {
  return ROUTINES.filter((r) => r.lastRun && r.lastRun.status === 'running');
}

export function getFailingRoutines() {
  return ROUTINES.filter((r) => r.lastRun && r.lastRun.status === 'error');
}

/** Percentage of successful runs in a routine's history, or null when never run. */
export function getRoutineSuccessRate(routineId) {
  const routine = getRoutineById(routineId);
  if (!routine || routine.runHistory.length === 0) return null;
  const done = routine.runHistory.filter((r) => r.status !== 'running');
  if (done.length === 0) return null;
  const ok = done.filter((r) => r.status === 'success').length;
  return Math.round((ok / done.length) * 100);
}

/** Upcoming scheduled runs, soonest first. */
export function getUpcomingRuns() {
  return ROUTINES.filter((r) => r.enabled && r.schedule.nextRunAt)
    .slice()
    .sort((a, b) => a.schedule.nextRunAt.localeCompare(b.schedule.nextRunAt));
}

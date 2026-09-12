/**
 * How a routine's schedule and its last run are labelled.
 *
 * Routines are plain Markdown playbooks an agent runs on a schedule, a trigger,
 * or on demand, and they live in the workspace. This is the vocabulary the
 * screen draws them with.
 */

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

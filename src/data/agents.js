/**
 * The persistent AI teammates the single user works with.
 * Ids are referenced by threads, routines, memories, activity and permissions.
 */

/**
 * Empty on purpose.
 *
 * This was a roster of invented teammates, and it was not merely unused - it was
 * what a brand new workspace got written into it on first open. Which meant a
 * fresh install opened onto someone else's work, and a real folder ended up
 * holding records nobody made and nothing could act on.
 *
 * The app handles an empty workspace everywhere; that is what the first launch
 * should look like.
 */
export const AGENTS = [];

/**
 * Status, as tokens rather than hex.
 *
 * These were `#22c55e` and friends - Tailwind's stock palette, surviving as
 * data after the class names were moved onto the status ramp. Two things were
 * wrong with that. They are the same colour in both themes, so a dot tuned to
 * read on white was the brightest thing on a dark screen; and they are a
 * second, invisible definition of "success", which is how one green drifts away
 * from another.
 *
 * They are `var(--...)` because these are spent as inline style, not as
 * classes - the value goes straight into `backgroundColor`, and a custom
 * property resolves there exactly like a colour, per theme, for free.
 */
export const AGENT_STATUS_META = {
  online: { label: "Online", color: "var(--success)", icon: "Circle" },
  busy: { label: "Working", color: "var(--warning)", icon: "Loader" },
  // Two greys rather than two colours, and the dimmer one is off. Idle is a
  // thing that could answer; offline is a thing that could not.
  idle: { label: "Idle", color: "var(--muted-foreground)", icon: "Moon" },
  offline: { label: "Offline", color: "var(--border-strong)", icon: "PowerOff" },
};

export function getAgentById(id) {
  return AGENTS.find((b) => b.id === id);
}

export function getAgentByHandle(handle) {
  const normalized = handle.startsWith("@") ? handle : `@${handle}`;
  return AGENTS.find((b) => b.handle === normalized);
}

export function getAgentsByStatus(status) {
  return AGENTS.filter((b) => b.status === status);
}

export function getActiveAgents() {
  return AGENTS.filter((b) => b.status === "online" || b.status === "busy");
}

export function getAgentsForComputer(computerId) {
  return AGENTS.filter((b) => b.computerId === computerId);
}

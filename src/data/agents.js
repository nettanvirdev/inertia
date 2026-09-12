/**
 * How an agent's status is drawn.
 *
 * The invented teammates that used to sit here are gone - the agents on screen
 * are the workspace's own, read back from `agents/`. What is left is the
 * vocabulary: one entry per status, so the dot beside a name means the same
 * thing on every screen that draws one.
 */

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

/**
 * What the computers screens need to know that is not a machine.
 *
 * This file used to hold six invented machines, a fabricated file tree, three
 * scripted terminal sessions and eight browser tabs - a demo of a feature that
 * did not exist. All of it is gone. The panes now read a real provider, so a
 * fixture here would be a second answer to a question that has a real one, and
 * the first thing a person would see on a fresh install would be five machines
 * they never made and cannot start.
 *
 * What is left is vocabulary: how to draw a status, what to call a provider.
 * Both are lists the UI needs and neither is a claim about anything existing.
 */

/**
 * The three places a machine can be.
 *
 * Three, and only three. A provider in this list is one the picker offers, so
 * naming one the app cannot make is how someone chooses a machine that will
 * never exist. The real list comes from the main process at runtime, with
 * whether each one can run right now; this is only what to call them before
 * that answer arrives.
 */
export const PROVIDER_META = {
  local: { label: 'Local', icon: 'Laptop' },
  docker: { label: 'Docker', icon: 'Container' },
  daytona: { label: 'Daytona', icon: 'Cloud' },
};

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
export const COMPUTER_STATUS_META = {
  running: { label: 'Running', color: 'var(--success)', icon: 'Play' },
  paused: { label: 'Paused', color: 'var(--warning)', icon: 'Pause' },
  stopped: { label: 'Stopped', color: 'var(--muted-foreground)', icon: 'Square' },
  provisioning: { label: 'Provisioning', color: 'var(--info)', icon: 'Loader' },
  // Two states the fixtures never had, because nothing could go wrong in them.
  error: { label: 'Failed', color: 'var(--destructive)', icon: 'TriangleAlert' },
  missing: { label: 'Gone', color: 'var(--destructive)', icon: 'CircleX' },
};

/** A workspace with no computers in it, which is what a new one has. */
export const COMPUTERS = [];

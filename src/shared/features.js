/**
 * Which optional parts of the app are switched on.
 *
 * One answer, asked by everything that has to agree: the navigation rail, the
 * command palette, the router and the window title - and the backend reads the
 * same preference, by the same rule, when it decides which tools a turn holds.
 * A feature visible in one of those and gone
 * from another is the kind of half-removed thing that teaches a person not to
 * trust any switch in the app.
 *
 * Written as a predicate over preferences rather than as a list of booleans so
 * that "off" always means the same thing: the preference is explicitly false.
 * An absent preference is on, because a workspace saved before a feature existed
 * should get the feature rather than a blank screen.
 */

export const FEATURES = {
  memory: {
    preference: "memory",
    label: "Memory",
  },
};

export function isFeatureOn(preferences, name) {
  const feature = FEATURES[name];
  if (!feature) return true;
  return preferences?.[feature.preference] !== false;
}

/**
 * Keep the entries whose feature is on.
 *
 * `needs` names a feature; an entry without one is always there. Anything with
 * an `id` and an optional `needs` works, so the rail's rows and the palette's
 * commands go through the same filter without either knowing about the other.
 */
export function visibleItems(items, preferences) {
  return (items ?? []).filter((item) => !item?.needs || isFeatureOn(preferences, item.needs));
}

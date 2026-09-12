import { useEffect, useState } from "react";

/**
 * The current time, as an ISO string, kept fresh.
 *
 * The store used to hand out a `now` on its context value. That was wrong in a
 * quiet way: a timestamp captured into a memoized context is frozen for the
 * life of the provider, so the app read its clock off whenever the window
 * happened to open. Removing it was right, but three views were still
 * destructuring `now` from `useApp()` and getting `undefined`, which turned
 * "Today" and an axis label into `new Date(undefined)` and took Library,
 * Routines and Activity down to a blank window.
 *
 * So the clock is a hook rather than state. It re-renders on its own schedule,
 * which is also what makes a relative label stop saying "2 minutes ago" an hour
 * later.
 *
 * The default tick is a minute because everything reading this formats to
 * minute resolution at best. A view that needs finer can ask for it.
 */
export function useNow(intervalMs = 60_000) {
  const [now, setNow] = useState(() => new Date().toISOString());

  useEffect(() => {
    if (!Number.isFinite(intervalMs) || intervalMs <= 0) return undefined;
    const timer = setInterval(() => setNow(new Date().toISOString()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);

  return now;
}

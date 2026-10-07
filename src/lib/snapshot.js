/**
 * The window's view of a turn's changes.
 *
 * Read at the point of use, like the crew bridge, so a browser preview with
 * no backend gets an honest "not available" rather than a throw.
 */
const bridge = () => (typeof window !== "undefined" ? window.snapshotAPI : null) ?? null;

export function isSnapshotAvailable() {
  return Boolean(bridge()?.revert);
}

function unwrap(result) {
  if (!result) return null;
  if (result.ok === false) throw new Error(result.error ?? "That did not work.");
  return "data" in result ? result.data : result;
}

/** The unified diff for one file of a turn, or for the whole turn. */
export async function loadDiff({ cwd, from, to, file }) {
  const api = bridge();
  if (!api?.diff) return "";
  return (await unwrap(await api.diff({ cwd, from, to, file }))) ?? "";
}

/** Put the folder back the way it was before the turn. */
export async function revertTurn({ cwd, to, since }) {
  const api = bridge();
  if (!api?.revert) return { reverted: false, reason: "Not available here." };
  return unwrap(await api.revert({ cwd, to, since }));
}

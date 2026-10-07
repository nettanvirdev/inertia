/**
 * The project bridge: what a folder has, and offering it what it does not.
 *
 * Shaped like the other bridges in this folder - a lookup that tolerates the
 * API being absent, and `unwrap` on the `{ ok, data }` envelope every bridge
 * call answers with. Outside the desktop app every call answers "nothing to do",
 * which is the right answer for a browser tab that has no folders at all.
 */

function bridge() {
  return typeof window !== "undefined" ? window.projectAPI : null;
}

function unwrap(answer) {
  if (!answer) return null;
  if (answer.ok === false) throw new Error(answer.error || "That did not work.");
  return answer.ok === true ? answer.data : answer;
}

/**
 * Whether to do anything about this folder right now, and what.
 *
 * The rule lives in the backend on purpose. A second copy of "when do we
 * offer this" in the window would eventually disagree with the first, and the
 * disagreement would show up as a repository quietly gaining a file.
 */
export async function decideProjectSetup(cwd, { mode, moment }) {
  const api = bridge();
  if (!api || !cwd) return { action: "none" };
  try {
    return unwrap(await api.decide(cwd, { mode, moment })) ?? { action: "none" };
  } catch {
    return { action: "none" };
  }
}

/** Write the starter files. Throws with something worth showing if it cannot. */
export async function createProjectSetup(cwd, options = {}) {
  const api = bridge();
  if (!api) throw new Error("Setting a project up needs the desktop app.");
  return unwrap(await api.create(cwd, options));
}

/** Remember that this folder was offered and turned down, so it is not asked again. */
export async function declineProjectSetup(cwd) {
  const api = bridge();
  if (!api || !cwd) return { declined: false };
  try {
    return unwrap(await api.decline(cwd)) ?? { declined: false };
  } catch {
    return { declined: false };
  }
}

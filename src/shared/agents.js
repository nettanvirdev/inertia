/**
 * What it means for an agent to be paused.
 *
 * Pausing used to be a word on a badge. The button wrote `status: "offline"`
 * onto the agent's record, the header swapped to Resume, and nothing anywhere
 * else read it - so a paused agent still answered messages and its routines
 * still fired at nine in the morning. Two lines of state, one of them a lie.
 *
 * The predicate lives here because both halves need the same answer. The main
 * process is the only place a pause can be enforced, since a window can be
 * reloaded and a routine runs with no window at all; the window is the only
 * place it can be explained to the person who set it. A second copy of "what
 * counts as paused" in either half is how the two would drift.
 *
 * `status === "offline"` rather than a new `paused` flag, deliberately. That is
 * the value the pause button has always written, so every agent already paused
 * in a workspace on disk is still paused after this change, and there is one
 * field rather than two that can contradict each other.
 */
export function isPaused(agent) {
  return agent?.status === "offline";
}

/**
 * Why the work did not start, and what to do about it.
 *
 * A refusal that only says no is indistinguishable from a bug. The person who
 * paused this agent may have done it a week ago and on another screen, so the
 * message names the agent and says where the switch is rather than leaving them
 * to hunt for it.
 */
/**
 * Is this record protected from being changed by an agent?
 *
 * Protection is a statement about who may change a thing, not about what it
 * is. The agent that ships with the app carries it so a model cannot delete
 * the thing that knows how the app works, and the user can put it on anything
 * they rely on - the teammate a dozen routines point at, the skill that took
 * an afternoon to write.
 *
 * The person is always the authority. The switch is on the record's own
 * screen, and nothing an agent can do turns it off: a model that could unlock
 * what it may not delete is not a lock.
 */
export function isProtected(record) {
  return record?.protected === true;
}

/** Why a change was refused, and what to do about it. */
export function protectedReason(record, what = "record") {
  const name = String(record?.name ?? record?.title ?? "").trim() || `That ${what}`;
  return (
    `${name} is protected, so it cannot be changed or removed from here. ` +
    `Open it in Inertia and turn off Protected first, if that is really what you want.`
  );
}

export function pausedReason(agent) {
  const name = String(agent?.name ?? "").trim() || "That agent";
  return (
    `${name} is paused, so it will not start new work. ` +
    "Open it under Agents and press Resume to let it run again."
  );
}

/**
 * The agent a new conversation starts with.
 *
 * One answer, in one place, because "New chat" is in four: the rail, the
 * keyboard shortcut, the command palette and the button on an empty transcript.
 * Three of them read the setting and the fourth started a thread with whichever
 * agent the person had last looked at - which, on a fresh window, is simply the
 * first one in the folder. A workspace whose agents begin with Astrophysicist
 * got Astrophysicist however plainly the setting said Inertia Dev.
 *
 * The chosen agent, or the first one when that agent has since been deleted:
 * a setting pointing at nothing must not start a thread addressed to nobody.
 */
export function defaultAgent(agents, preferredId) {
  const list = Array.isArray(agents) ? agents : [];
  return list.find((agent) => agent?.id === preferredId) ?? list[0] ?? null;
}

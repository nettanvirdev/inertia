/*
 * Reading a group conversation's room.
 *
 * Two questions the header asks every render - whose conversation is this, and
 * who else is in it - kept out of the component so they can be tested. The
 * suite runs in node with no document, so anything that has to be sure of an
 * answer cannot live in a .jsx file.
 */

/**
 * The agent the conversation currently belongs to.
 *
 * Three answers in order of how current they are: whoever the room says is due
 * to speak, then whoever spoke last, then the agent the thread was started
 * with. The middle one is what makes a handover visible after the turn has
 * ended - the floor has gone back to the person by then, and the name at the
 * top of the thread should still be the one that took it over.
 */
export function speakingAgent({ thread, messages = [], agents = [] }) {
  if (!thread) return null;
  const byId = new Map(agents.map((one) => [one.id, one]));
  const fromRoom = thread.room?.active ? byId.get(thread.room.active) : null;
  if (fromRoom) return fromRoom;

  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    if (message?.role !== "agent" || !message.agentId) continue;
    const spoke = byId.get(message.agentId);
    if (spoke) return spoke;
    break;
  }
  return byId.get(thread.agentId) ?? null;
}

/** The agents in the room, in the order they arrived, minus the one speaking. */
export function othersIn(room, agents = [], speaking = null) {
  if (!room?.roster?.length) return [];
  const byId = new Map(agents.map((one) => [one.id, one]));
  return room.roster
    .filter((id) => id !== speaking)
    .map((id) => byId.get(id))
    .filter(Boolean);
}

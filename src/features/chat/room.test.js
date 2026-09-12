import { describe, expect, it } from "vitest";
import { othersIn, speakingAgent } from "./room.js";

/**
 * Whose name is at the top of a group conversation.
 *
 * The question is not "who is the thread's agent" - in a room that answer goes
 * stale the moment somebody hands over - and it is not "who has the floor"
 * either, because between turns nobody does. It is "whose conversation is this
 * now", and the three sources have to be read in the right order.
 */

const AGENTS = [
  { id: "a1", name: "Nova" },
  { id: "a2", name: "Iris" },
  { id: "a3", name: "Wren" },
];

describe("whose conversation this is", () => {
  it("is the thread's own agent before anybody has spoken", () => {
    const thread = { agentId: "a1" };
    expect(speakingAgent({ thread, messages: [], agents: AGENTS })?.id).toBe("a1");
  });

  it("is whoever spoke last, which is how a handover stays visible", () => {
    const thread = { agentId: "a1", room: { roster: ["a1", "a2"], active: null } };
    const messages = [
      { role: "user", content: "hello" },
      { role: "agent", agentId: "a1", content: "Iris should take this." },
      { role: "agent", agentId: "a2", content: "I have it." },
    ];
    expect(speakingAgent({ thread, messages, agents: AGENTS })?.id).toBe("a2");
  });

  it("is whoever is due to speak, when somebody is", () => {
    const thread = { agentId: "a1", room: { roster: ["a1", "a3"], active: "a3" } };
    const messages = [{ role: "agent", agentId: "a1", content: "over to Wren" }];
    expect(speakingAgent({ thread, messages, agents: AGENTS })?.id).toBe("a3");
  });

  it("falls back to the thread's agent when the last speaker has been deleted", () => {
    const thread = { agentId: "a1" };
    const messages = [{ role: "agent", agentId: "gone", content: "..." }];
    expect(speakingAgent({ thread, messages, agents: AGENTS })?.id).toBe("a1");
  });

  it("ignores the person's own messages", () => {
    const thread = { agentId: "a1" };
    const messages = [
      { role: "agent", agentId: "a2", content: "mine" },
      { role: "user", content: "thanks" },
    ];
    expect(speakingAgent({ thread, messages, agents: AGENTS })?.id).toBe("a2");
  });

  it("has nothing to say about a thread that is not there", () => {
    expect(speakingAgent({ thread: null, agents: AGENTS })).toBeNull();
  });
});

describe("who else is here", () => {
  const room = { roster: ["a1", "a2", "a3"] };

  it("is everybody but the one speaking, in the order they arrived", () => {
    expect(othersIn(room, AGENTS, "a1").map((one) => one.id)).toEqual(["a2", "a3"]);
  });

  it("drops an agent that has since been deleted", () => {
    expect(othersIn({ roster: ["a1", "gone"] }, AGENTS, "a1")).toEqual([]);
  });

  it("is empty outside a group", () => {
    expect(othersIn(null, AGENTS, "a1")).toEqual([]);
    expect(othersIn({ roster: [] }, AGENTS, "a1")).toEqual([]);
  });
});

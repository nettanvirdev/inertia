import { describe, it, expect } from "vitest";
import { defaultAgent, isPaused, pausedReason } from "./agents.js";

/**
 * One predicate, two processes.
 *
 * The value that means "paused" is written by a button in the window and read
 * by the agent bridge and the scheduler in main. These tests exist so that a
 * change to what counts as paused cannot happen in one half only.
 */
describe("isPaused", () => {
  it("reads the value the pause button writes", () => {
    expect(isPaused({ id: "a", status: "offline" })).toBe(true);
  });

  it("leaves every other status running", () => {
    expect(isPaused({ status: "idle" })).toBe(false);
    expect(isPaused({ status: "busy" })).toBe(false);
    expect(isPaused({})).toBe(false);
  });

  it("treats a turn with no agent at all as not paused", () => {
    // A chat with no agent chosen is a plain conversation with the workspace
    // default, and refusing it because there is nothing to be paused would
    // stop a perfectly ordinary message.
    expect(isPaused(null)).toBe(false);
    expect(isPaused(undefined)).toBe(false);
  });
});

describe("pausedReason", () => {
  it("names the agent and says where the switch is", () => {
    const reason = pausedReason({ name: "Ada", status: "offline" });
    expect(reason).toContain("Ada");
    expect(reason).toMatch(/Resume/);
  });

  it("still reads as a sentence for a record with no name", () => {
    expect(pausedReason({ status: "offline" })).toMatch(/^That agent is paused/);
  });
});

describe("defaultAgent", () => {
  const agents = [{ id: "a" }, { id: "b" }];

  it("is the one the setting names", () => {
    expect(defaultAgent(agents, "b")?.id).toBe("b");
  });

  it("falls back to the first when that agent is gone", () => {
    expect(defaultAgent(agents, "gone")?.id).toBe("a");
    expect(defaultAgent(agents, null)?.id).toBe("a");
  });

  it("has nothing to offer when there are no agents", () => {
    expect(defaultAgent([], "a")).toBeNull();
    expect(defaultAgent(undefined, "a")).toBeNull();
  });
});

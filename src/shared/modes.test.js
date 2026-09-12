import { describe, expect, it } from "vitest";

import { MODES, DEFAULT_MODE, modeOf, isMode, toolsForMode, promptForMode } from "./modes.js";
import { costOf, priceFor, CACHE_READ_MULTIPLIER, CACHE_WRITE_MULTIPLIER } from "./usage.js";

/**
 * The modes, and the thing that makes them real.
 *
 * The picker existed for months and did nothing: the value never left the
 * composer, so Chat could scaffold a project and Plan could start building. The
 * tests that matter here are the ones that would fail if it went back to being
 * decoration - that Chat cannot write, that Plan cannot build, and that
 * Autonomous can.
 */

const ALL = [
  "read",
  "ls",
  "grep",
  "glob",
  "write",
  "edit",
  "patch",
  "shell",
  "task",
  "spawn",
  "collect",
  "team",
  "agent_send",
  "todowrite",
  "present_plan",
  "computer_act",
  "computer_observe",
  "inertia_save",
  // A tool nothing in the table has heard of - an MCP server's, say.
  "acme_fetch_invoice",
];

const idsIn = (mode) => toolsForMode(ALL.map((id) => ({ id })), mode).map((tool) => tool.id);

describe("what each mode may do", () => {
  it("chat cannot change anything", () => {
    const tools = idsIn("chat");
    for (const forbidden of ["write", "edit", "patch", "shell", "task", "computer_act", "inertia_save"]) {
      expect(tools).not.toContain(forbidden);
    }
  });

  it("closes the hole a spawned run would open in chat and plan", () => {
    // A spawned run holds full tools of its own, so leaving `spawn` available
    // in a mode that withholds `write` would mean the turn that may not write a
    // file simply asks a helper to write it.
    for (const mode of ["chat", "plan"]) {
      const tools = idsIn(mode);
      for (const forbidden of ["spawn", "collect", "team", "agent_send"]) {
        expect(tools).not.toContain(forbidden);
      }
    }
    expect(idsIn("autonomous")).toContain("spawn");
  });

  it("chat can still look things up", () => {
    const tools = idsIn("chat");
    for (const allowed of ["read", "ls", "grep", "glob", "computer_observe"]) {
      expect(tools).toContain(allowed);
    }
  });

  it("plan cannot build, but can write the plan down", () => {
    const tools = idsIn("plan");
    expect(tools).not.toContain("write");
    expect(tools).not.toContain("shell");
    expect(tools).toContain("present_plan");
    expect(tools).toContain("todowrite");
    expect(tools).toContain("read");
  });

  it("only plan can present a plan", () => {
    // A mode that could approve its own plan would make the approval a
    // formality, and Chat presenting one would be a card nobody asked for.
    expect(idsIn("chat")).not.toContain("present_plan");
    expect(idsIn("autonomous")).not.toContain("present_plan");
    expect(idsIn("plan")).toContain("present_plan");
  });

  it("autonomous holds everything else", () => {
    const tools = idsIn("autonomous");
    for (const allowed of ["write", "edit", "patch", "shell", "task", "computer_act", "inertia_save"]) {
      expect(tools).toContain(allowed);
    }
  });

  it("a tool nobody listed is available everywhere", () => {
    // Withholding rather than allow-listing: an MCP server's tool should not be
    // invisible in Chat because a list in this file was not updated for it.
    for (const mode of ["chat", "plan", "autonomous"]) {
      expect(idsIn(mode)).toContain("acme_fetch_invoice");
    }
  });

  it("takes bare ids as well as tool objects", () => {
    expect(toolsForMode(["read", "write"], "chat")).toEqual(["read"]);
  });
});

describe("the mode table itself", () => {
  it("starts a conversation in chat", () => {
    expect(DEFAULT_MODE).toBe("chat");
    expect(isMode(DEFAULT_MODE)).toBe(true);
  });

  it("falls back rather than throwing on a value from disk", () => {
    // A thread record is a file a person can edit.
    expect(modeOf("nonsense").id).toBe(DEFAULT_MODE);
    expect(modeOf(undefined).id).toBe(DEFAULT_MODE);
    expect(isMode("nonsense")).toBe(false);
  });

  it("gives every mode a prompt that says what the turn is for", () => {
    for (const mode of MODES) {
      expect(promptForMode(mode.id).length).toBeGreaterThan(200);
      expect(mode.hint).toBeTruthy();
    }
  });
});

describe("what a turn costs, with caching", () => {
  const usage = { input: 100_000, output: 10_000, cached: 80_000, cacheWrite: 5_000, requests: 1 };

  it("prices cached reads and cache writes apart from fresh input", () => {
    const price = { input: 3, output: 15 };
    const fresh = 100_000 - 80_000 - 5_000;
    const expected =
      (fresh * 3 + 80_000 * 3 * CACHE_READ_MULTIPLIER + 5_000 * 3 * CACHE_WRITE_MULTIPLIER + 10_000 * 15) /
      1_000_000;
    expect(costOf(usage, price)).toBeCloseTo(expected, 6);
  });

  it("uses the rate the user typed instead of the multiplier", () => {
    const price = { input: 3, output: 15, cachedInput: 0.3, cacheWrite: 3.75 };
    const fresh = 100_000 - 80_000 - 5_000;
    const expected = (fresh * 3 + 80_000 * 0.3 + 5_000 * 3.75 + 10_000 * 15) / 1_000_000;
    expect(costOf(usage, price)).toBeCloseTo(expected, 6);
  });

  it("treats a cached rate of zero as free, not as unset", () => {
    // A gateway that does not charge for cache reads is a real thing, and
    // "0" has to survive the round trip or it silently becomes a tenth of input.
    const free = costOf(usage, { input: 3, output: 15, cachedInput: 0 });
    const tenth = costOf(usage, { input: 3, output: 15 });
    expect(free).toBeLessThan(tenth);
    expect(free).toBeCloseTo((15_000 * 3 + 5_000 * 6 + 10_000 * 15) / 1_000_000, 6);
  });

  it("carries the cached rates off a workspace price", () => {
    const price = priceFor("my-model", {
      "my-model": { input: 1, output: 2, cachedInput: 0.1, cacheWrite: 1.25 },
    });
    expect(price).toMatchObject({ input: 1, output: 2, cachedInput: 0.1, cacheWrite: 1.25 });
  });

  it("leaves the cached rates off when nobody set them", () => {
    const price = priceFor("my-model", { "my-model": { input: 1, output: 2 } });
    expect(price.cachedInput).toBeUndefined();
    expect(price.cacheWrite).toBeUndefined();
  });

  it("still refuses to price a model nobody named", () => {
    expect(priceFor("some-local-thing", {})).toBeNull();
  });
});

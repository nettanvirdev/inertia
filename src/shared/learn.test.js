import { describe, expect, it } from "vitest";
import { LEARN_STANDARDS, learnPrompt } from "./learn.js";
import { SENDS_A_TURN, parseCommand } from "./commands.js";

describe("learnPrompt", () => {
  it("means this conversation when nothing is named", () => {
    const prompt = learnPrompt("");
    expect(prompt).toContain("what we just did in this conversation");
    expect(prompt).toContain("Read back over it first");
  });

  it("carries what was named into the instruction", () => {
    const prompt = learnPrompt("the release process in scripts/release.mjs");
    expect(prompt).toContain("scripts/release.mjs");
    expect(prompt).toContain("Gather what you need first");
  });

  it("always asks for the skill to be saved, not merely described", () => {
    for (const subject of ["", "something"]) {
      expect(learnPrompt(subject)).toContain("inertia_save");
    }
  });

  it("tells it to update an existing skill rather than add a second", () => {
    expect(learnPrompt("")).toContain("inertia_read");
  });

  it("carries the standards, which are the point", () => {
    expect(learnPrompt("")).toContain(LEARN_STANDARDS);
  });

  it("survives nonsense where a subject should be", () => {
    expect(typeof learnPrompt(null)).toBe("string");
    expect(typeof learnPrompt(undefined)).toBe("string");
  });
});

describe("/learn is a command that sends a turn", () => {
  it("parses like any other command", () => {
    expect(parseCommand("/learn")).toMatchObject({ name: "learn", args: "" });
    expect(parseCommand("/learn the deploy steps")).toMatchObject({
      name: "learn",
      args: "the deploy steps",
    });
  });

  it("is declared as one that sends, so the composer substitutes rather than swallows", () => {
    expect(SENDS_A_TURN.has("learn")).toBe(true);
    expect(SENDS_A_TURN.has("compact")).toBe(false);
  });
});

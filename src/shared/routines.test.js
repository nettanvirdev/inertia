import { describe, expect, it } from "vitest";
import {
  ROUTINE_DEFAULT_APPROVAL,
  ROUTINE_DEFAULT_MODE,
  routineApproval,
  routineMode,
} from "./routines.js";

/**
 * A routine runs with every tool, held back by the person's rules, unless its
 * record says otherwise. Running without asking is something the person turns
 * on for a routine they have read, never the default.
 */
describe("what a routine runs as", () => {
  it("is autonomous and asks when the record says nothing", () => {
    expect(routineMode({})).toBe("autonomous");
    expect(routineApproval({})).toBe("ask");
    expect(routineMode(undefined)).toBe(ROUTINE_DEFAULT_MODE);
    expect(routineApproval(null)).toBe(ROUTINE_DEFAULT_APPROVAL);
  });

  it("is what the record says when it says something real", () => {
    expect(routineMode({ mode: "chat" })).toBe("chat");
    expect(routineApproval({ approval: "ask" })).toBe("ask");
    expect(routineApproval({ approval: "edits" })).toBe("edits");
    expect(routineApproval({ approval: "auto" })).toBe("auto");
  });

  it("falls back rather than trusting a hand-edited value", () => {
    // These come off a file a person can edit; an unknown word must not
    // become a mode the loop has no tools for or an approval nothing checks.
    expect(routineMode({ mode: "yolo" })).toBe("autonomous");
    expect(routineApproval({ approval: "never" })).toBe("ask");
  });
});

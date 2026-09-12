import { describe, expect, it } from "vitest";
import {
  ROUTINE_DEFAULT_APPROVAL,
  ROUTINE_DEFAULT_MODE,
  routineApproval,
  routineMode,
} from "./routines.js";

/**
 * A routine runs with every tool and without asking unless its record says
 * otherwise. The defaults are the point: a scheduled job that stops for a
 * person is not a scheduled job.
 */
describe("what a routine runs as", () => {
  it("is autonomous and never asks when the record says nothing", () => {
    expect(routineMode({})).toBe("autonomous");
    expect(routineApproval({})).toBe("auto");
    expect(routineMode(undefined)).toBe(ROUTINE_DEFAULT_MODE);
    expect(routineApproval(null)).toBe(ROUTINE_DEFAULT_APPROVAL);
  });

  it("is what the record says when it says something real", () => {
    expect(routineMode({ mode: "chat" })).toBe("chat");
    expect(routineApproval({ approval: "ask" })).toBe("ask");
    expect(routineApproval({ approval: "edits" })).toBe("edits");
  });

  it("falls back rather than trusting a hand-edited value", () => {
    // These come off a file a person can edit; an unknown word must not
    // become a mode the loop has no tools for or an approval nothing checks.
    expect(routineMode({ mode: "yolo" })).toBe("autonomous");
    expect(routineApproval({ approval: "never" })).toBe("auto");
  });
});

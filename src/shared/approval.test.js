import { describe, expect, it } from "vitest";

import {
  ALWAYS_ASK,
  APPROVALS,
  DEFAULT_APPROVAL,
  approvalOf,
  isApproval,
  promptForApproval,
  resolveAsk,
} from "./approval.js";

/**
 * The dial only ever loosens an `ask`. Every case here is a way it might be
 * tempted to do more than that, and the answer is that it does not.
 */

describe("the three positions", () => {
  it("default to asking, and know their own names", () => {
    expect(DEFAULT_APPROVAL).toBe("ask");
    expect(APPROVALS.map((a) => a.id)).toEqual(["ask", "edits", "auto"]);
    expect(isApproval("edits")).toBe(true);
    expect(isApproval("yolo")).toBe(false);
    expect(approvalOf("nonsense").id).toBe("ask");
  });
});

describe("what an ask becomes", () => {
  it("is still an ask on the default position", () => {
    expect(resolveAsk("ask", "edit")).toBe("ask");
    expect(resolveAsk("ask", "shell")).toBe("ask");
    expect(resolveAsk(undefined, "shell")).toBe("ask");
  });

  it("waves edits through under accept-edits, and nothing else", () => {
    expect(resolveAsk("edits", "edit")).toBe("allow");
    expect(resolveAsk("edits", "shell")).toBe("ask");
    expect(resolveAsk("edits", "computer")).toBe("ask");
    // An edit outside the working folder is not "an edit in this project".
    expect(resolveAsk("edits", "external_directory")).toBe("ask");
  });

  it("waves everything through under never-ask, except what may never be", () => {
    expect(resolveAsk("auto", "shell")).toBe("allow");
    expect(resolveAsk("auto", "external_directory")).toBe("allow");
    expect(resolveAsk("auto", "mcp")).toBe("allow");
    for (const key of ALWAYS_ASK) expect(resolveAsk("auto", key)).toBe("ask");
  });

  it("stops a loop under never-ask rather than letting it run", () => {
    // The guard's question is "did you mean to?" and with nobody to answer,
    // the loop ends. A dial that silenced the guard would be worse than none.
    expect(resolveAsk("auto", "doom_loop")).toBe("deny");
    expect(resolveAsk("edits", "doom_loop")).toBe("ask");
  });
});

describe("what the model is told", () => {
  it("tells an unwatched model that it is the one checking", () => {
    expect(promptForApproval("auto")).toMatch(/you are/);
    expect(promptForApproval("edits")).toMatch(/still stop/);
    expect(promptForApproval("ask")).toMatch(/as written/);
  });
});

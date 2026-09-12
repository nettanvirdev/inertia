import { describe, expect, it } from "vitest";
import { asRuns, asTree } from "./crew";

/**
 * The renderer's half of the crew bridge.
 *
 * Only the pure parts are here. `useCrew` is a hook and this suite runs in node
 * with no DOM, so what is testable is the two rules the hook leans on - and
 * they are the two that decide whether a malformed reply is an empty panel or a
 * broken conversation.
 */

describe("what came back over the bridge", () => {
  it("is a list, or it is nothing", () => {
    // The failure this prevents is not a missing panel. `crewRuns.filter` is
    // the first thing ChatView does with it, so an object here replaces the
    // whole conversation with an error boundary - the chat is gone because the
    // agent sidebar could not be drawn.
    expect(asRuns([{ id: "run-1" }])).toEqual([{ id: "run-1" }]);
    for (const bad of [undefined, null, {}, { runs: [] }, "runs", 0, true]) {
      expect(asRuns(bad), String(bad)).toEqual([]);
    }
  });

  it("does not copy a list it was given", () => {
    // Cheap to get wrong and needless: the rows are already a fresh structure
    // out of the bridge, and copying every snapshot is work per push.
    const rows = [{ id: "run-1" }];
    expect(asRuns(rows)).toBe(rows);
  });
});

describe("the tree", () => {
  const run = (id, parentId = null) => ({
    id,
    parentId,
    agentName: id,
    status: "running",
    startedAt: 0,
  });

  it("indents a child under whoever started it", () => {
    const tree = asTree([run("a"), run("b", "a"), run("c", "b")]);
    expect(tree.map((r) => [r.id, r.indent])).toEqual([
      ["a", 0],
      ["b", 1],
      ["c", 2],
    ]);
  });

  it("keeps an orphan rather than losing it", () => {
    // A parent can be forgotten while its child is still running - the run
    // table is capped. Dropping the child would hide live work; showing it at
    // the root is merely wrong about the indentation.
    const tree = asTree([run("b", "gone")]);
    expect(tree.map((r) => r.id)).toEqual(["b"]);
  });

  it("survives a cycle instead of hanging", () => {
    // Nothing should be able to produce one. Something that walks parent links
    // until it reaches the root must still terminate if anything ever does.
    const tree = asTree([run("a", "b"), run("b", "a")]);
    expect(tree).toHaveLength(2);
    for (const row of tree) expect(Number.isFinite(row.indent)).toBe(true);
  });
});

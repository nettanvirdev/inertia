import { describe, expect, it } from "vitest";
import {
  fileName,
  groupParts,
  joinThoughts,
  runCount,
  runDuration,
  runState,
  runTitle,
} from "./tool-groups.js";

const call = (name, title, extra = {}) => ({
  type: "tool",
  name,
  callId: `${name}-${title}`,
  title,
  state: "done",
  durationMs: 1,
  ...extra,
});

const read = (title, extra = {}) => ({
  type: "tool",
  name: "read",
  callId: title,
  title,
  state: "done",
  durationMs: 1,
  ...extra,
});

describe("groupParts", () => {
  it("leaves a message with no parts alone", () => {
    expect(groupParts(undefined)).toEqual([]);
    expect(groupParts([])).toEqual([]);
  });

  it("folds a run of reads into one item", () => {
    const items = groupParts([read("a.ts"), read("b.ts"), read("c.ts")]);
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe("run");
    expect(items[0].calls).toHaveLength(3);
  });

  it("folds a pair, because two identical rows are already a ladder", () => {
    const items = groupParts([read("a.ts"), read("b.ts")]);
    expect(items.map((one) => one.kind)).toEqual(["run"]);
  });

  it("folds any tool, not only read", () => {
    // Two failed searches and two retries filled the screen for one lookup.
    const items = groupParts([
      call("exa_search", "quantum", { state: "failed" }),
      call("exa_search", "quantum", { state: "failed" }),
      call("exa_search", "quantum"),
    ]);
    expect(items).toHaveLength(1);
    expect(items[0].name).toBe("exa_search");
    expect(runState(items[0].calls)).toBe("failed");
  });

  it("does not fold two different tools together", () => {
    const items = groupParts([call("exa_search", "a"), call("firecrawl", "b")]);
    expect(items.map((one) => one.kind)).toEqual(["one", "one"]);
  });

  it("starts a new run where the tool changes", () => {
    const items = groupParts([
      call("exa_search", "a"),
      call("exa_search", "b"),
      call("firecrawl", "c"),
      call("firecrawl", "d"),
    ]);
    expect(items.map((one) => one.name)).toEqual(["exa_search", "firecrawl"]);
  });

  it("does not fold across something the agent did in between", () => {
    const items = groupParts([
      read("a.ts"),
      read("b.ts"),
      read("c.ts"),
      { type: "reasoning", text: "hm" },
      read("d.ts"),
      read("e.ts"),
      read("f.ts"),
    ]);
    expect(items.map((one) => one.kind)).toEqual(["run", "one", "run"]);
    expect(items[0].calls).toHaveLength(3);
    expect(items[2].calls).toHaveLength(3);
  });

  it("keeps every member openable, which is what makes folding safe", () => {
    // The old rule was that only `read` folds, because a summary that REPLACES
    // its members hides the edit that went wrong. This one does not replace
    // them: the row is a lid.
    const write = call("write", "a.ts");
    const items = groupParts([write, call("write", "b.ts"), call("write", "c.ts")]);
    expect(items[0].calls.map((one) => one.title)).toEqual(["a.ts", "b.ts", "c.ts"]);
  });

  it("keeps prose and tool calls in the order they happened", () => {
    const items = groupParts([
      { type: "text", text: "first" },
      read("a.ts"),
      read("b.ts"),
      read("c.ts"),
      { type: "text", text: "last" },
    ]);
    expect(items.map((one) => one.kind)).toEqual(["one", "run", "one"]);
    expect(items[0].part.text).toBe("first");
    expect(items[2].part.text).toBe("last");
  });

  it("gives every item a distinct key", () => {
    const items = groupParts([
      { type: "text", text: "a" },
      { type: "text", text: "b" },
      read("a.ts"),
      read("b.ts"),
      read("c.ts"),
    ]);
    expect(new Set(items.map((one) => one.key)).size).toBe(items.length);
  });

  it("keeps a run's key steady as the run grows", () => {
    const first = groupParts([read("a.ts"), read("b.ts"), read("c.ts")])[0];
    const second = groupParts([read("a.ts"), read("b.ts"), read("c.ts"), read("d.ts")])[0];
    expect(second.key).toBe(first.key);
  });
});

describe("thinking, joined", () => {
  it("makes one pause of two", () => {
    // "Thought for 5s" above "Thought for 3s" describes one pause twice.
    const items = groupParts([
      { type: "reasoning", text: "first", startedAt: 10, endedAt: 20 },
      { type: "reasoning", text: "second", startedAt: 20, endedAt: 40 },
    ]);
    expect(items).toHaveLength(1);
    expect(items[0].part.text).toBe("first\n\nsecond");
  });

  it("spans from the first start to the last end", () => {
    const joined = joinThoughts([
      { type: "reasoning", text: "a", startedAt: 10, endedAt: 20 },
      { type: "reasoning", text: "b", startedAt: 25, endedAt: 40 },
    ]);
    // The gap between them was thinking too.
    expect(joined.startedAt).toBe(10);
    expect(joined.endedAt).toBe(40);
  });

  it("drops the empty ones rather than leaving blank lines", () => {
    const joined = joinThoughts([
      { type: "reasoning", text: "a" },
      { type: "reasoning", text: "   " },
      { type: "reasoning", text: "b" },
    ]);
    expect(joined.text).toBe("a\n\nb");
  });

  it("does not join across a tool call", () => {
    const items = groupParts([
      { type: "reasoning", text: "a" },
      read("x.ts"),
      { type: "reasoning", text: "b" },
    ]);
    expect(items.map((one) => one.kind)).toEqual(["one", "one", "one"]);
  });

  it("leaves a lone thought exactly as it was", () => {
    const only = { type: "reasoning", text: "a", startedAt: 1, endedAt: 2 };
    expect(groupParts([only])[0].part).toBe(only);
  });
});

describe("runCount", () => {
  it("counts files for reads and calls for everything else", () => {
    expect(runCount([read("a.ts"), read("b.ts")])).toBe("2 files");
    expect(runCount([call("exa_search", "a"), call("exa_search", "b")])).toBe("2 calls");
  });
});

describe("runState", () => {
  it("is failed if any member failed, whatever the rest did", () => {
    expect(runState([read("a"), read("b", { state: "failed" })])).toBe("failed");
    expect(runState([read("a", { state: "running" }), read("b", { state: "failed" })])).toBe(
      "failed"
    );
  });

  it("is running until every member has finished", () => {
    expect(runState([read("a"), read("b", { state: "running" })])).toBe("running");
    expect(runState([read("a"), read("b")])).toBe("done");
  });
});

describe("runDuration", () => {
  it("adds the members up", () => {
    expect(runDuration([read("a", { durationMs: 3 }), read("b", { durationMs: 4 })])).toBe(7);
  });

  it("reports nothing rather than an understated total", () => {
    expect(
      runDuration([read("a", { durationMs: 3 }), read("b", { durationMs: null })])
    ).toBeUndefined();
    expect(runDuration([])).toBeUndefined();
  });
});

describe("fileName", () => {
  it("takes the last segment of a Windows path", () => {
    expect(fileName({ metadata: { path: "D:\\work\\api\\page.tsx" } })).toBe("page.tsx");
  });

  it("takes the last segment of a posix path", () => {
    expect(fileName({ args: { filePath: "/home/a/b/layout.tsx" } })).toBe("layout.tsx");
  });

  it("falls back to the title when there is no path", () => {
    expect(fileName({ title: "AGENTS.md" })).toBe("AGENTS.md");
  });
});

describe("runTitle", () => {
  it("names them all when they fit", () => {
    expect(runTitle([read("a.ts"), read("b.ts"), read("c.ts")])).toBe("a.ts, b.ts, c.ts");
  });

  it("names them all and lets the row do the truncating", () => {
    const calls = ["a", "b", "c", "d", "e"].map((one) => read(`${one}.ts`));
    expect(runTitle(calls)).toBe("a.ts, b.ts, c.ts, d.ts, e.ts");
  });

  it("says a repeated target once", () => {
    // Two attempts at one search are one thing tried twice, and the row saying
    // it twice reads as two different searches.
    const calls = [
      call("exa_search", "quantum computing", { state: "failed" }),
      call("exa_search", "quantum computing"),
    ];
    expect(runTitle(calls)).toBe("quantum computing");
  });
});

import { describe, expect, it } from "vitest";
import { WINDOW_SIZE, isWindowed, windowOf } from "./transcript-window.js";

const list = (n) => Array.from({ length: n }, (_, i) => ({ id: `m${i}` }));

describe("windowOf", () => {
  it("leaves a short conversation completely alone", () => {
    const blocks = list(5);
    const { visible, hidden } = windowOf(blocks);
    expect(visible).toBe(blocks);
    expect(hidden).toBe(0);
  });

  it("keeps the newest, because a conversation is read from the bottom", () => {
    const { visible, hidden } = windowOf(list(100), 10);
    expect(visible).toHaveLength(10);
    expect(visible[0].id).toBe("m90");
    expect(visible[9].id).toBe("m99");
    expect(hidden).toBe(90);
  });

  it("keeps exactly the window when the list is exactly that long", () => {
    const { visible, hidden } = windowOf(list(WINDOW_SIZE));
    expect(visible).toHaveLength(WINDOW_SIZE);
    expect(hidden).toBe(0);
  });

  it("shows more of the same tail as the count grows, never a different part", () => {
    const blocks = list(100);
    const first = windowOf(blocks, 10).visible;
    const second = windowOf(blocks, 20).visible;
    expect(second.slice(10)).toEqual(first);
  });

  it("does not shift what is visible when the conversation grows", () => {
    // The reader is looking at the tail; a turn appends. What they can see is
    // still the tail, which is the point of counting rather than indexing.
    const before = windowOf(list(50), 10).visible.at(-1).id;
    const after = windowOf(list(55), 10).visible.at(-1).id;
    expect(before).toBe("m49");
    expect(after).toBe("m54");
  });

  it("survives nonsense rather than throwing inside a render", () => {
    expect(windowOf(undefined)).toEqual({ visible: [], hidden: 0 });
    expect(windowOf(null, 0)).toEqual({ visible: [], hidden: 0 });
    expect(windowOf(list(3), -5).visible).toHaveLength(3);
  });
});

describe("isWindowed", () => {
  it("is false until something is actually being held back", () => {
    expect(isWindowed(list(WINDOW_SIZE))).toBe(false);
    expect(isWindowed(list(WINDOW_SIZE + 1))).toBe(true);
  });
});

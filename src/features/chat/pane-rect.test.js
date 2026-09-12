import { describe, expect, it } from "vitest";
import { isShowable, rectOf } from "./pane-rect.js";

/**
 * A DOMRect, as the browser makes one: the numbers live on the prototype, and
 * the object has no own properties at all. This is the shape that broke the
 * browser pane - it looks like data and does not survive being cloned.
 */
function domRectLike({ x, y, width, height }) {
  const proto = {
    get x() {
      return x;
    },
    get y() {
      return y;
    },
    get width() {
      return width;
    },
    get height() {
      return height;
    },
    get left() {
      return x;
    },
    get top() {
      return y;
    },
  };
  return Object.create(proto);
}

describe("rectOf", () => {
  it("reads the numbers off the prototype, where a DOMRect keeps them", () => {
    const rect = domRectLike({ x: 12, y: 40, width: 800, height: 600 });
    expect(Object.keys(rect)).toEqual([]);
    expect(rectOf(rect)).toEqual({ x: 12, y: 40, width: 800, height: 600 });
  });

  it("produces something that actually survives a clone", () => {
    const rect = domRectLike({ x: 1, y: 2, width: 3, height: 4 });
    // The bug, demonstrated: the DOMRect clones to nothing.
    expect(JSON.parse(JSON.stringify(rect))).toEqual({});
    // The fix.
    expect(JSON.parse(JSON.stringify(rectOf(rect)))).toEqual({ x: 1, y: 2, width: 3, height: 4 });
  });

  it("takes left and top when there is no x and y", () => {
    expect(rectOf({ left: 5, top: 6, width: 10, height: 10 })).toMatchObject({ x: 5, y: 6 });
  });

  it("answers with zeros rather than NaN for nonsense", () => {
    expect(rectOf(null)).toEqual({ x: 0, y: 0, width: 0, height: 0 });
    expect(rectOf({ width: NaN, height: "tall" })).toEqual({ x: 0, y: 0, width: 0, height: 0 });
  });
});

describe("isShowable", () => {
  it("is true for a pane with room in it", () => {
    expect(isShowable(domRectLike({ x: 0, y: 0, width: 400, height: 300 }))).toBe(true);
  });

  it("is false for a hidden tab, which measures to nothing", () => {
    expect(isShowable(domRectLike({ x: 0, y: 0, width: 0, height: 0 }))).toBe(false);
  });

  it("is false for the sliver a pane passes through while it opens", () => {
    expect(isShowable({ x: 0, y: 0, width: 4, height: 300 })).toBe(false);
  });
});

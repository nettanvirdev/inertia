import { describe, expect, it } from "vitest";
import {
  DEFAULT_WIDTH,
  MAX_SHARE,
  MIN_WIDTH,
  clampWidth,
  maxWidth,
} from "@/features/chat/use-dock-width";

describe("how wide the dock may be", () => {
  it("lets it take most of the window", () => {
    // The old ceiling was a flat 720. A browser in a 1920px window was pinned
    // to a phone-sized viewport with a thousand pixels of empty chat beside it.
    expect(maxWidth(1920)).toBe(Math.round(1920 * MAX_SHARE));
    expect(maxWidth(1920)).toBeGreaterThan(720);
  });

  it("still leaves the conversation on screen", () => {
    expect(maxWidth(1600)).toBeLessThan(1600);
  });

  it("never lets the ceiling fall under the floor", () => {
    // In a window narrower than the minimum the two bounds cross, and a clamp
    // with inverted bounds returns whichever it applied last.
    expect(maxWidth(200)).toBe(MIN_WIDTH);
    expect(clampWidth(400, 200)).toBe(MIN_WIDTH);
  });

  it("holds a width inside the bounds", () => {
    expect(clampWidth(500, 1600)).toBe(500);
    expect(clampWidth(10, 1600)).toBe(MIN_WIDTH);
    expect(clampWidth(9999, 1600)).toBe(maxWidth(1600));
  });

  it("rounds, because a fractional column is a blurry edge", () => {
    expect(clampWidth(500.6, 1600)).toBe(501);
  });

  it("falls back to the default rather than to NaN", () => {
    // A stored preference can be anything: a string, a null, a value written
    // by an older build.
    expect(clampWidth(undefined, 1600)).toBe(DEFAULT_WIDTH);
    expect(clampWidth("wide", 1600)).toBe(DEFAULT_WIDTH);
    expect(clampWidth(null, 1600)).toBe(DEFAULT_WIDTH);
  });

  it("treats a window of no width as a window it knows nothing about", () => {
    expect(maxWidth(0)).toBe(MIN_WIDTH);
    expect(maxWidth(NaN)).toBe(MIN_WIDTH);
  });
});

import { describe, expect, it } from "vitest";
import { DARK_ANSI, LIGHT_ANSI, isDark, xtermTheme } from "./xterm-theme.js";

describe("isDark", () => {
  it("reads a hex ground", () => {
    expect(isDark("#111111")).toBe(true);
    expect(isDark("#fafafa")).toBe(false);
  });

  it("reads what getComputedStyle actually returns", () => {
    expect(isDark("rgb(20, 20, 24)")).toBe(true);
    expect(isDark("rgba(250, 250, 250, 1)")).toBe(false);
  });

  it("weighs green more heavily than blue, as an eye does", () => {
    // Same numeric value in each channel alone: green reads as light, blue
    // does not. A naive average would call both the same.
    expect(isDark("#00ff00")).toBe(false);
    expect(isDark("#0000ff")).toBe(true);
  });

  it("treats a colour it cannot read as dark, which is the app's default", () => {
    expect(isDark("var(--something)")).toBe(true);
    expect(isDark("")).toBe(true);
    expect(isDark(undefined)).toBe(true);
  });
});

describe("the two ANSI sets", () => {
  it("name exactly the same colours", () => {
    expect(Object.keys(DARK_ANSI).sort()).toEqual(Object.keys(LIGHT_ANSI).sort());
  });

  it("give all sixteen, because a program writing colour 12 expects one", () => {
    expect(Object.keys(DARK_ANSI)).toHaveLength(16);
  });

  it("keeps red recognisably red in both, because it is a protocol not a palette", () => {
    for (const set of [DARK_ANSI, LIGHT_ANSI]) {
      const red = parseInt(set.red.slice(1), 16);
      const r = (red >> 16) & 255;
      const g = (red >> 8) & 255;
      const b = red & 255;
      expect(r).toBeGreaterThan(g);
      expect(r).toBeGreaterThan(b);
    }
  });
});

describe("xtermTheme", () => {
  it("answers with something usable when there is no document at all", () => {
    const theme = xtermTheme(null);
    expect(theme.background).toBeTruthy();
    expect(theme.red).toBeTruthy();
  });
});

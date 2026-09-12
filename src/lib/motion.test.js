import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { EASE, MOTION, exitDuration, prefersReducedMotion } from "./motion.js";

const CSS = fs.readFileSync(path.join(import.meta.dirname, "..", "styles", "globals.css"), "utf8");

function fakeDocument({ attr, media } = {}) {
  return {
    documentElement: { dataset: attr === undefined ? {} : { reduceMotion: attr } },
    defaultView: {
      matchMedia: (query) => ({ matches: query.includes("reduce") ? Boolean(media) : false }),
    },
  };
}

describe("the durations the stylesheet and the code agree on", () => {
  it("echoes every JavaScript duration as a custom property", () => {
    // The whole reason the file exists. A menu timed out of the tree at 150ms
    // by React and faded over 200ms by CSS is a menu that vanishes mid-fade.
    for (const [name, ms] of Object.entries(MOTION)) {
      expect(CSS, `--motion-${name}`).toMatch(new RegExp(`--motion-${name}:\\s*${ms}ms;`));
    }
  });

  it("echoes every easing curve", () => {
    for (const [name, curve] of Object.entries(EASE)) {
      const escaped = curve.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      expect(CSS, `--ease-${name}`).toMatch(new RegExp(`--ease-${name}:\\s*${escaped};`));
    }
  });

  it("leaves faster than it arrives", () => {
    // The person has already decided; there is nothing to watch on the way out.
    expect(MOTION.exit).toBeLessThan(MOTION.base);
    expect(MOTION.reduced).toBeLessThan(MOTION.exit);
  });
});

describe("knowing when to hold still", () => {
  it("reads the app's own setting off the root element", () => {
    expect(prefersReducedMotion(fakeDocument({ attr: "true" }))).toBe(true);
  });

  it("reads the operating system's", () => {
    expect(prefersReducedMotion(fakeDocument({ media: true }))).toBe(true);
  });

  it("moves when neither asks it not to", () => {
    expect(prefersReducedMotion(fakeDocument())).toBe(false);
  });

  it("does not read the string false as a request", () => {
    // `appearance.js` removes the attribute rather than writing "false", and
    // this is the other half of that contract.
    expect(prefersReducedMotion(fakeDocument({ attr: "false" }))).toBe(false);
  });

  it("answers without a document at all", () => {
    expect(prefersReducedMotion(null)).toBe(false);
  });

  it("survives a window with no matchMedia", () => {
    const doc = { documentElement: { dataset: {} }, defaultView: {} };
    expect(prefersReducedMotion(doc)).toBe(false);
  });
});

describe("how long a leaving element stays", () => {
  it("stays for the exit when motion is on", () => {
    expect(exitDuration(MOTION.exit, false)).toBe(MOTION.exit);
    expect(exitDuration(300, false)).toBe(300);
  });

  it("stays only for the short fade when motion is reduced", () => {
    expect(exitDuration(300, true)).toBe(MOTION.reduced);
  });

  it("never shortens an exit that was already shorter than the fade", () => {
    expect(exitDuration(60, true)).toBe(60);
  });

  it("is never zero", () => {
    // Removed on the same frame the fade starts is removed before the fade has
    // a first frame - and a zero timer reorders against React's own update.
    expect(exitDuration(0, false)).toBe(MOTION.exit);
    expect(exitDuration(-5, false)).toBe(MOTION.exit);
    expect(exitDuration(NaN, true)).toBe(MOTION.reduced);
    expect(exitDuration("soon", false)).toBe(MOTION.exit);
  });
});

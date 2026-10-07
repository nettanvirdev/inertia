import { describe, expect, it } from "vitest";
import { readFileSync } from "fs";
import { fileURLToPath } from "url";

/**
 * The two palettes, kept in step.
 *
 * The stylesheet has said "every token below MUST exist in both :root and
 * .dark" since it was written, and nothing has ever checked. A token defined in
 * one mode and not the other does not fail a build or throw in a console - the
 * variable simply resolves to whatever the light block left behind, so a dark
 * screen quietly paints one element in a light colour, and the only way anyone
 * finds out is by looking at that screen in that mode.
 *
 * Geometry is deliberately exempt: a rail is the same width at night.
 */

// Normalised, because the working copy is checked out with CRLF on Windows
// and LF elsewhere - and this file matches on a multi-line selector, so
// without it the suite would pass or fail depending on who cloned the repo.
const css = readFileSync(fileURLToPath(new URL("./globals.css", import.meta.url)), "utf8").replace(
  /\r\n/g,
  "\n"
);

/**
 * The declarations inside one top-level block, by name.
 *
 * `head` is the selector exactly as written, because light is `:root, .light`
 * across two lines - one rule, so that a light value is still written down once
 * whether it is the default or a preview tile asking for it.
 */
function block(head) {
  const at = css.indexOf(`${head} {`);
  expect(at, `${head} block`).toBeGreaterThan(-1);
  const body = css.slice(at, css.indexOf("\n}", at));
  const found = new Map();
  for (const [, name, value] of body.matchAll(/^\s*(--[\w-]+):\s*([^;]+);/gm)) {
    found.set(name, value.trim());
  }
  return found;
}

/** `#rrggbb` to its three channels. Every colour in this file is written that way. */
function channels(hex) {
  const match = /^#([0-9a-f]{6})$/i.exec(String(hex).trim());
  expect(match, `not a plain hex: ${hex}`).not.toBeNull();
  const n = parseInt(match[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** WCAG relative luminance and contrast, straight from the definition. */
function contrast(a, b) {
  const lum = (hex) => {
    const [r, g, bl] = channels(hex).map((v) => {
      const s = v / 255;
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [x, y] = [lum(a), lum(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

/** Sizes, not colours - the same in both modes on purpose. */
const GEOMETRY = new Set([
  "--radius",
  "--spacing",
  "--rail-collapsed",
  "--rail-expanded",
  "--app-rail-w",
  "--header-h",
  "--titlebar-h",
  "--content-max",
]);

const light = block(`:root,\n.light`);
const dark = block(".dark");

describe("the two palettes", () => {
  it("defines every themed token in both", () => {
    const missing = [...light.keys()].filter((name) => !GEOMETRY.has(name) && !dark.has(name));
    expect(missing, "defined in light but not in dark").toEqual([]);
  });

  it("does not define a token in dark that light has never heard of", () => {
    // This direction matters more: light is the fallback, so a dark-only token
    // is one that resolves to nothing at all in the other mode.
    expect([...dark.keys()].filter((name) => !light.has(name))).toEqual([]);
  });

  it("gives the two modes different values, or the token is not themed", () => {
    // A token that is identical in both is either geometry - which is exempt
    // above - or a colour somebody forgot to give a dark value to, and the
    // second one is a bug that looks exactly like a decision.
    const identical = [...light.entries()]
      .filter(([name]) => !GEOMETRY.has(name))
      .filter(([name, value]) => dark.get(name) === value)
      // The same alpha of --ink in both, which is not a missing dark value:
      // --ink is what differs, so the colour does too. (--control-border was
      // here as well until light needed 0.14 to weigh what dark gets from
      // 0.10 - an exemption is worth removing the moment it stops being true.)
      .filter(([name]) => name !== "--thumb-border")
      // Named one at a time rather than by prefix. `--destructive-*` would also
      // wave through --destructive-ink and --destructive-wash, which do differ
      // between the modes and are exactly what this test is for; an exemption
      // that quietly grows to cover its own subject stops being one.
      .filter(
        ([name]) =>
          ![
            // White on red at night as well.
            "--destructive-foreground",
            // The pressed and hover tints of a solid red button, which is the
            // same red in both modes because red does not have a night shift.
            "--destructive-hover",
            "--destructive-pressed",
          ].includes(name)
      )
      .map(([name]) => name);
    expect(identical).toEqual([]);
  });

  it("keeps a status ink readable on every surface it can land on", () => {
    // This is the check the old colours would have failed. Tailwind's stock
    // amber-500 as text on white is 2.15:1 and emerald-500 is 2.5:1 - fine as
    // a dot, unreadable as the word next to it, which is what they were being
    // used for. A status colour is the one place where "looks about right" and
    // "can be read" come apart, so the number is asserted rather than eyeballed.
    for (const [mode, tokens] of [
      ["light", light],
      ["dark", dark],
    ]) {
      const surfaces = ["--background", "--overlay", "--card", "--card-subtle"];
      for (const name of ["--destructive-ink", "--success-ink", "--warning-ink", "--info-ink"]) {
        for (const surface of surfaces) {
          const got = contrast(tokens.get(name), tokens.get(surface));
          expect(got, `${mode} ${name} on ${surface}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  });

  it("keeps the ink as bare channels, because every fill mixes it", () => {
    // `rgb(var(--ink) / 0.06)` only works if --ink is "51 53 55" and not
    // "rgb(51,53,55)". Getting this wrong makes every hover in the app vanish
    // at once, which is a spectacular failure and an easy typo.
    for (const [mode, tokens] of [
      ["light", light],
      ["dark", dark],
    ]) {
      expect(tokens.get("--ink"), mode).toMatch(/^\d+ \d+ \d+$/);
    }
  });
});

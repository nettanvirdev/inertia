import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  ACCENTS,
  APPEARANCE_DEFAULTS,
  CONTRASTS,
  DARK_PALETTES,
  DENSITIES,
  FONT_FAMILIES,
  LIGHT_PALETTES,
  LINE_HEIGHTS,
  MESSAGE_WIDTHS,
  MONO_FAMILIES,
  PREFERENCE_DEFAULTS,
  RADII,
  TRANSCRIPTS,
  appearanceOf,
  applyAppearance,
  isBehindMore,
  shownFirst,
} from "./appearance";
import { readFileSync } from "node:fs";

/**
 * Appearance, checked at the only place it is observable.
 *
 * Every one of these settings is a promise that something on screen changes,
 * and the way that promise gets broken is never dramatic: an option is added to
 * the list, the pane renders it, and the line that stamps it on the document is
 * forgotten. The value is saved, the screen does not move, and nothing fails.
 *
 * So the test stands a fake document up and reads back what was written to it -
 * the same handful of custom properties the stylesheet consumes. There is no
 * DOM in this test environment and none is needed: `applyAppearance` touches
 * exactly two things, `documentElement.style` and `documentElement.dataset`.
 */
function fakeDocument() {
  const props = new Map();
  return {
    props,
    documentElement: {
      dataset: {},
      style: {
        setProperty: (key, value) => props.set(key, value),
        removeProperty: (key) => props.delete(key),
      },
    },
  };
}

let doc;

beforeEach(() => {
  doc = fakeDocument();
  globalThis.document = doc;
});

afterEach(() => {
  delete globalThis.document;
});

const apply = (prefs, isDark = false) => {
  applyAppearance(prefs, isDark);
  return { css: doc.props, data: doc.documentElement.dataset };
};

describe("the shipped defaults", () => {
  it("gives every appearance option a default", () => {
    // A preference with no default is one "Reset all settings" silently leaves
    // alone, which is the kind of bug nobody reports and everybody hits once.
    for (const [key, value] of Object.entries(APPEARANCE_DEFAULTS)) {
      expect(value, key).not.toBeUndefined();
      expect(PREFERENCE_DEFAULTS[key], key).toEqual(value);
    }
  });

  it("defaults to a value that exists in its own list", () => {
    const lists = {
      accent: ACCENTS,
      fontFamily: FONT_FAMILIES,
      monoFamily: MONO_FAMILIES,
      radius: RADII,
      density: DENSITIES,
      messageWidth: MESSAGE_WIDTHS,
      lineHeight: LINE_HEIGHTS,
      transcript: TRANSCRIPTS,
      contrast: CONTRASTS,
    };
    for (const [key, list] of Object.entries(lists)) {
      expect(
        list.map((entry) => entry.value),
        key
      ).toContain(APPEARANCE_DEFAULTS[key]);
    }
  });

  it("fills in what a hand-edited identity file left out", () => {
    expect(appearanceOf({ accent: "rose" })).toEqual({
      ...APPEARANCE_DEFAULTS,
      accent: "rose",
    });
    expect(appearanceOf(null)).toEqual(APPEARANCE_DEFAULTS);
  });
});

describe("stamping the choices on the document", () => {
  it("writes a token for every option that has one", () => {
    const { css } = apply(APPEARANCE_DEFAULTS);
    for (const token of [
      "--font-sans",
      "--font-mono",
      "--radius",
      "--chat-font-size",
      "--code-font-size",
      "--chat-line-height",
      "--chat-measure",
      "--spacing",
    ]) {
      expect(css.has(token), token).toBe(true);
    }
  });

  it("resolves the accent against the theme that is actually painted", () => {
    const blue = ACCENTS.find((a) => a.value === "blue");
    expect(apply({ accent: "blue" }, false).css.get("--accent-solid")).toBe(blue.light);
    expect(apply({ accent: "blue" }, true).css.get("--accent-solid")).toBe(blue.dark);
  });

  it("removes the accent entirely for Ink, rather than painting a grey", () => {
    apply({ accent: "blue" });
    const { css } = apply({ accent: "default" });
    expect(css.has("--accent-solid")).toBe(false);
    expect(css.has("--accent-on")).toBe(false);
  });

  it("keeps the interface typeface and the code typeface apart", () => {
    const { css } = apply({ fontFamily: "serif", monoFamily: "courier" });
    expect(css.get("--font-sans")).toContain("Georgia");
    expect(css.get("--font-mono")).toContain("Courier");
  });

  it("clamps a size a hand-edited file put out of range", () => {
    expect(apply({ fontSize: 400 }).css.get("--chat-font-size")).toBe("20px");
    expect(apply({ codeSize: 2 }).css.get("--code-font-size")).toBe("11px");
    // Not a number at all falls to the bottom of the range rather than writing
    // `NaNpx`, which the stylesheet would drop and nobody would ever see.
    expect(apply({ fontSize: "large" }).css.get("--chat-font-size")).toBe("12px");
  });

  it("falls back to the shipped value when a name matches nothing", () => {
    // A workspace folder written by a newer build, opened by an older one.
    const { css, data } = apply({ monoFamily: "comic", transcript: "carousel" });
    expect(css.get("--font-mono")).toBe(MONO_FAMILIES[0].stack);
    expect(data.transcript).toBe("bubbles");
  });

  it("puts the attribute-driven options on the element as attributes", () => {
    const { data } = apply({ transcript: "flat", contrast: "high", density: "compact" });
    expect(data.transcript).toBe("flat");
    expect(data.contrast).toBe("high");
    expect(data.density).toBe("compact");
  });

  it("stamps both grounds, whichever theme is on", () => {
    // Both, always. Stamping only the active one leaves the other attribute
    // behind from the last time the theme flipped, and a stale attribute is a
    // palette nobody chose.
    const { data } = apply({ lightPalette: "sepia", darkPalette: "ink" });
    expect(data.lightPalette).toBe("sepia");
    expect(data.darkPalette).toBe("ink");
  });

  it("takes the attribute off for the default ground rather than naming it", () => {
    // There is no [data-light-palette="default"] block in the stylesheet, and
    // there must not be: the default palette is :root itself.
    apply({ lightPalette: "mist", darkPalette: "graphite" });
    const { data } = apply({ lightPalette: "default", darkPalette: "default" });
    expect(data.lightPalette).toBeUndefined();
    expect(data.darkPalette).toBeUndefined();
  });

  it("falls back to the default ground when a palette name matches nothing", () => {
    const { data } = apply({ lightPalette: "neon", darkPalette: "vantablack" });
    expect(data.lightPalette).toBeUndefined();
    expect(data.darkPalette).toBeUndefined();
  });

  it("ships a default entry in each ground list, and only one", () => {
    // The lists drive the settings tiles, and a list with no default would
    // offer no way back to the app's own palette.
    expect(LIGHT_PALETTES.filter((p) => p.value === "default")).toHaveLength(1);
    expect(DARK_PALETTES.filter((p) => p.value === "default")).toHaveLength(1);
  });

  /**
   * The failure this whole file exists to prevent: an option that saves a
   * value nothing reads. A ground is only real if the stylesheet has a block
   * keyed on the attribute `applyAppearance` stamps - and "default" is real
   * precisely by NOT having one, because the default palette is `:root`.
   */
  it("has a stylesheet block behind every ground it offers", () => {
    const css = readFileSync(new URL("../styles/globals.css", import.meta.url), "utf8");
    for (const [kind, list] of [
      ["light", LIGHT_PALETTES],
      ["dark", DARK_PALETTES],
    ]) {
      for (const palette of list) {
        const selector = `[data-${kind}-palette="${palette.value}"]`;
        if (palette.value === "default") {
          expect(css).not.toContain(selector);
        } else {
          expect(css, `${kind} ${palette.value}`).toContain(selector);
        }
      }
    }
  });

  it("keeps the first row of every picker small, and can still reach the rest", () => {
    for (const list of [ACCENTS, LIGHT_PALETTES, DARK_PALETTES]) {
      // The default is never folded away: it is the way back.
      expect(shownFirst(list).some((entry) => entry.value === "default")).toBe(true);
      expect(shownFirst(list).length).toBeLessThan(list.length);
      for (const entry of list) {
        expect(isBehindMore(list, entry.value)).toBe(!!entry.more);
      }
    }
  });

  it("gives every accent both a light and a dark value, or neither", () => {
    for (const accent of ACCENTS) {
      if (accent.value === "default") {
        expect(accent.light).toBeNull();
        continue;
      }
      // Showing a colour the user will never see - the light chip while the
      // app is dark - is the one thing an accent swatch must not do.
      expect(accent.light, accent.value).toMatch(/^#[0-9a-f]{6}$/i);
      expect(accent.dark, accent.value).toMatch(/^#[0-9a-f]{6}$/i);
      expect(accent.on.light, accent.value).toMatch(/^#[0-9a-f]{6}$/i);
      expect(accent.on.dark, accent.value).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  it("gives every ground a swatch the settings tile can paint itself with", () => {
    for (const palette of [...LIGHT_PALETTES, ...DARK_PALETTES]) {
      expect(palette.swatch.ground).toMatch(/^#[0-9a-f]{6}$/i);
      expect(palette.swatch.surface).toMatch(/^#[0-9a-f]{6}$/i);
      expect(palette.swatch.ink).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  /**
   * A font stack is consulted per character, so a Latin face with no Bengali
   * glyphs does not fail - it falls through to whatever the platform has, and
   * the platform's answer changes by machine. That is why the same Bengali
   * sentence used to be drawn in a different face depending on which *Latin*
   * font was selected.
   */
  it("names a Bengali face in every stack it offers", () => {
    for (const font of [...FONT_FAMILIES, ...MONO_FAMILIES]) {
      expect(font.stack).toMatch(/Bengali|Nirmala UI/);
    }
  });

  it("removes reduce-motion rather than setting it to the string false", () => {
    // `dataset` stringifies, and "false" is truthy to an attribute selector, so
    // the attribute has to be absent and not merely off.
    apply({ reduceMotion: true });
    expect(doc.documentElement.dataset.reduceMotion).toBe("true");
    apply({ reduceMotion: false });
    expect(doc.documentElement.dataset.reduceMotion).toBeUndefined();
  });
});

import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "fs";
import { fileURLToPath } from "url";
import { join } from "path";

/**
 * Every class from this app's own utility families has to exist.
 *
 * A misspelled Tailwind class is not an error. It is not a warning either - it
 * compiles to nothing at all, silently, and the element renders without the
 * thing you asked for. That is survivable for a stock utility, where a missing
 * `p-4` is visible the moment anyone looks. It is not survivable for the
 * families defined in `globals.css`, because those are exactly the ones nobody
 * can check from memory.
 *
 * Three of them were in the tree when this was written. `fill-danger`,
 * `fill-warning` and `fill-success` had never existed - somebody reasonably
 * assumed the `fill-*` ladder had a status arm, and the error box in the
 * cookie-import dialog, the warning in the plan card and the success note in
 * the import dialog had all been rendering on no background whatsoever, since
 * the day each was written. Nothing failed. Nothing logged. They just looked
 * slightly plainer than intended, which is not a thing anyone reports.
 */

const ROOT = fileURLToPath(new URL("../", import.meta.url));

/** The families this file owns. Anything matching these must be declared. */
const FAMILIES = /^(fill|overlay|card-surface|icon-surface|accent|scrim|chat)(-|$)/;

/**
 * Tailwind's own utilities that happen to start with one of our prefixes.
 *
 * `fill-current` and `fill-none` are SVG paint; `accent-*` is `accent-color`.
 * They are real, they are just not ours, so they are not in globals.css.
 */
const NOT_OURS = new Set(["fill-current", "fill-none", "accent-current", "accent-auto"]);

function jsxFiles(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...jsxFiles(path));
    else if (name.endsWith(".jsx")) out.push(path);
  }
  return out;
}

/** Everything `@utility` declares, which is the whole set we are allowed to use. */
function declared() {
  const css = readFileSync(join(ROOT, "styles/globals.css"), "utf8");
  return new Set([...css.matchAll(/@utility\s+([a-z0-9-]+)/g)].map((m) => m[1]));
}

/**
 * Class names out of a file's string literals.
 *
 * Only strings that are class LISTS - more than one word, at least one of them
 * recognisably Tailwind. A one-word string is too ambiguous to judge, and the
 * ambiguity is not theoretical: `"chat"` is a view name, `"accent"` is a
 * preference key and `"chat-completions"` is a provider protocol. The first
 * draft of this test reported all three as missing utilities.
 *
 * The cost is a phantom class standing entirely alone, with nothing beside it,
 * which this will not catch. That is the right trade for a test whose failures
 * are all real: one that cries wolf gets an allowlist, and an allowlist that
 * grows with every new API string is how a test stops being read.
 */
const LOOKS_LIKE_TAILWIND =
  /(^|\s)(flex|grid|block|hidden|absolute|relative|inline-flex|truncate|border|rounded|(?:p|m|w|h|gap|text|bg|size|px|py|mt|mb|ml|mr|min|max)-)/;

function classesIn(source) {
  const found = new Set();
  for (const [, dq, sq, tl] of source.matchAll(/"([^"\n]*)"|'([^'\n]*)'|`([^`]*)`/g)) {
    const text = dq ?? sq ?? tl ?? "";
    if (!LOOKS_LIKE_TAILWIND.test(text)) continue;
    for (const raw of text.split(/\s+/)) {
      // `hover:`, `dark:`, `group-hover:`, `focus-visible:` - strip them all.
      const name = raw.replace(/^(?:[a-z-]+:)+/, "");
      if (/^[a-z][a-z0-9-]*$/.test(name)) found.add(name);
    }
  }
  return found;
}

describe("the app's own utilities", () => {
  const defined = declared();
  const files = jsxFiles(ROOT);

  it("finds the stylesheet and the components", () => {
    // If either half of this comes back empty the test passes vacuously, which
    // is the one way a test like this fails silently.
    expect(defined.size).toBeGreaterThan(10);
    expect(files.length).toBeGreaterThan(50);
  });

  it("never uses one that does not exist", () => {
    const missing = [];
    for (const file of files) {
      for (const name of classesIn(readFileSync(file, "utf8"))) {
        if (!FAMILIES.test(name) || NOT_OURS.has(name) || defined.has(name)) continue;
        missing.push(`${name}  (${file.slice(ROOT.length).replace(/\\/g, "/")})`);
      }
    }
    expect(missing).toEqual([]);
  });
});

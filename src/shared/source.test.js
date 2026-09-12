import { describe, it, expect } from "vitest";
import fs from "node:fs";
import path from "node:path";

/**
 * Things that are wrong with a source file and invisible in a diff.
 *
 * A control character costs an afternoon in a way a syntax error never does. A
 * regex meant to read `/\bimage\b/` and saved with a literal backspace instead
 * of the two characters `\` and `b` still parses, still runs, and silently
 * matches nothing. Every tool that might have shown it renders the backspace
 * back as `\b` - the terminal, the diff, JSON.stringify - so reading the file
 * to check confirms the thing that is wrong. It happened here, to the pattern
 * that decides whether a turn survives being handed a screenshot.
 *
 * Cheap to check and impossible to eyeball, which is the whole argument.
 */

const ROOT = path.join(import.meta.dirname, "..");

/** Every source file we wrote, skipping anything generated or vendored. */
function sources(dir = ROOT, found = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === "node_modules" || entry.name === "dist" || entry.name.startsWith(".")) {
      continue;
    }
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) sources(full, found);
    else if (/\.(js|jsx|cjs|mjs)$/.test(entry.name)) found.push(full);
  }
  return found;
}

/** Every code point below space that is not tab, newline or carriage return. */
const BANNED = [...Array(32).keys()].filter((code) => code !== 9 && code !== 10 && code !== 13);

/**
 * Built from those numbers rather than written as a literal class.
 *
 * A literal `/[\x00-\x08]/` would need the very characters this test forbids,
 * so the test would be the first thing it caught - and exempting a check from
 * itself is how a check stops meaning anything.
 */
const CONTROL = new RegExp(`[${BANNED.map((code) => String.fromCharCode(code)).join("")}]`, "g");

describe("source hygiene", () => {
  it("has no control characters in any source file", () => {
    const offenders = [];

    for (const file of sources()) {
      const text = fs.readFileSync(file, "utf8");
      for (const hit of text.matchAll(CONTROL)) {
        const line = text.slice(0, hit.index).split("\n").length;
        const code = `U+${hit[0].charCodeAt(0).toString(16).padStart(4, "0")}`;
        offenders.push(`${path.relative(ROOT, file).replace(/\\/g, "/")}:${line} ${code}`);
      }
    }

    expect(offenders).toEqual([]);
  });
});

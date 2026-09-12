import { describe, it, expect } from "vitest";
import fs from "node:fs";
import path from "node:path";

/**
 * Props that are handed to a component which does not read them.
 *
 * React says nothing about this. A `<Select items={RATES}>` where the component
 * destructures `options` renders perfectly: a control with an empty list, no
 * warning in the console, no error anywhere. It shipped, and the frame-rate
 * control on the Desktop pane simply could not be changed - the panel said
 * "Nothing to choose from yet" about a list of four hard-coded values sitting
 * in the same file.
 *
 * There is no type checker here to catch it, so this reads the source. Crude,
 * and it is the difference between finding this class of bug in a test and
 * finding it in a screenshot the user sends.
 *
 * Only components whose whole job is to render a list are covered. The rule is
 * cheap to extend and expensive to over-apply: a component that spreads `...rest`
 * onto a DOM node legitimately accepts anything.
 */

const UI = path.join(import.meta.dirname, "..", "..");

/** Every .jsx under src/renderer. */
function sources(dir = UI, found = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) sources(full, found);
    else if (entry.name.endsWith(".jsx")) found.push(full);
  }
  return found;
}

/** Comments removed, so a doc comment in a destructure is not read as a prop. */
function bare(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "");
}

/** The props a component destructures from its first argument. */
function accepted(componentFile, componentName) {
  const src = fs.readFileSync(componentFile, "utf8");
  const at = src.indexOf(`export function ${componentName}(`);
  if (at === -1) throw new Error(`no ${componentName} in ${componentFile}`);
  const open = src.indexOf("{", at);
  const close = src.indexOf("}", open);
  const names = new Set(
    bare(src.slice(open + 1, close))
      .split(",")
      .map((part) => part.split(/[=:]/)[0].trim())
      .filter(Boolean)
  );
  // `key` is React's, never passed through to the component.
  names.add("key");
  return names;
}

/**
 * Props written on every `<Name ...>` in a file.
 *
 * Read at brace depth zero only. An `options={rows.map(r => ({ value, label }))}`
 * has words followed by colons and commas inside it, and a scanner that reads
 * the whole tag body reports the object's own keys as props of the component -
 * which is a false alarm, and a test that cries wolf gets deleted.
 */
function used(src, name) {
  const found = [];
  const re = new RegExp(`<${name}\\s`, "g");
  let match;
  while ((match = re.exec(src))) {
    let i = match.index + match[0].length;
    let depth = 0;
    let top = "";
    while (i < src.length) {
      const ch = src[i];
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
      else if (ch === ">" && depth === 0) break;
      // A placeholder keeps `prop={...}` looking like an assignment without
      // letting what is inside the braces be read.
      else if (depth === 0) top += ch;
      if (depth > 0 && src[i] === "{" && depth === 1) top += "{";
      i += 1;
    }
    const line = src.slice(0, match.index).split("\n").length;
    for (const prop of bare(top).matchAll(/(?:^|\s)([a-zA-Z][a-zA-Z0-9-]*)\s*=/g)) {
      found.push({ prop: prop[1], at: line });
    }
  }
  return found;
}

const LIST_COMPONENTS = [{ name: "Select", file: path.join(UI, "components", "ui", "select.jsx") }];

describe("props a component would silently ignore", () => {
  for (const { name, file } of LIST_COMPONENTS) {
    it(`${name} is never handed a prop it does not read`, () => {
      const known = accepted(file, name);
      const wrong = [];

      for (const source of sources()) {
        const src = fs.readFileSync(source, "utf8");
        // Any whitespace, not a literal space: a multi-line tag opens with a
        // newline, and requiring a space is how this test silently skipped the
        // one file that had the bug it was written for.
        if (!new RegExp(`<${name}\\s`).test(src)) continue;
        for (const { prop, at } of used(src, name)) {
          if (known.has(prop)) continue;
          wrong.push(`${path.relative(UI, source).replace(/\\/g, "/")}:${at} ${name} ${prop}=`);
        }
      }

      expect(wrong).toEqual([]);
    });
  }
});

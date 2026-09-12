import { describe, it, expect } from "vitest";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

/**
 * A module calling a name that does not exist.
 *
 * This is the one mistake a bundler will not tell you about. `getShortcuts(x)`
 * where nothing declares `getShortcuts` is, as far as Rollup is concerned, a
 * reference to a global that will be there at runtime - so `vite build`
 * succeeds, every test passes, and the failure is a `ReferenceError` thrown the
 * first time that line runs. In a React tree that unmounts the whole window:
 * the Shortcuts tab in Settings went to a blank page and the only way back was
 * a reload.
 *
 * It got there by deleting an export that had no importer, without noticing its
 * one caller was in the same file. Nothing else here can catch that - no test
 * renders a component, and a static scan for it means lexing JavaScript, where
 * `/(^|\/)anthropic(\/|$)/` is indistinguishable from a call to `anthropic`
 * unless you are a real parser.
 *
 * So: call them. Every exported function in the two folders that hold pure
 * logic is invoked with no arguments, and only `ReferenceError` counts as a
 * failure. Wrong arguments throw `TypeError`, which is expected and ignored -
 * the question is not whether the function works, it is whether every name it
 * reaches can be resolved at all.
 *
 * It catches what a no-argument call reaches, which is not everything. It is
 * the part that costs nothing and would have caught this.
 */

const ROOT = path.join(import.meta.dirname, "..");

/** The folders that are pure: data and vocabulary, and the logic both halves share. */
const FOLDERS = ["data", "shared"];

function modules(dir, found = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) modules(full, found);
    else if (entry.name.endsWith(".js") && !entry.name.includes(".test.")) found.push(full);
  }
  return found;
}

/** The names whose bodies reach something that does not exist. */
async function unresolvable(file) {
  const module = await import(pathToFileURL(file).href);
  const broken = [];

  for (const [name, value] of Object.entries(module)) {
    if (typeof value !== "function") continue;
    try {
      value();
    } catch (error) {
      if (error instanceof ReferenceError) broken.push(`${name}: ${error.message}`);
    }
  }
  return broken;
}

describe("every module can resolve what it calls", () => {
  const files = FOLDERS.flatMap((folder) => modules(path.join(ROOT, folder)));

  it("finds the modules to check", () => {
    expect(files.length).toBeGreaterThan(15);
  });

  it.each(files.map((file) => [path.relative(ROOT, file), file]))("%s", async (_name, file) => {
    expect(await unresolvable(file)).toEqual([]);
  });
});

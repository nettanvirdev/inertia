import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "fs";
import { fileURLToPath } from "url";
import { join } from "path";

/**
 * No view writes its own page gutter.
 *
 * This is the check that would have caught the thing it was written for. Five
 * views sat at `px-4 sm:px-6` and three at `px-4`, so the title jumped eight
 * pixels left when you opened Computers, Integrations or a chat and eight back
 * when you left. Every one of those files was individually reasonable; the bug
 * only existed between them, which is why reading any one of them never finds
 * it and why it survived several passes over this code.
 *
 * The rule is deliberately narrow. A `px-4` that widens to `sm:px-6` is always
 * the gutter - nothing else in the app is responsive that way - and a plain
 * `px-4` only counts on the 56px header bar, which is the other shape that
 * drifted. Cards, buttons and menu rows keep their own padding; none of that is
 * the page.
 */

const FEATURES = fileURLToPath(new URL("../../features/", import.meta.url));

const RESPONSIVE = /\bsm:px-6\b/;
const HEADER_BAR = /\bh-14\b/;
const BARE = /\bpx-4\b/;

/**
 * A view is the top-level screen a rail item opens - `AgentsView`, `ChatView` -
 * and not the rows, cards and dialogs underneath it.
 */
function views(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...views(path));
    else if (/View\.jsx$/.test(name)) out.push(path);
  }
  return out;
}

/**
 * Lines, not quoted strings. Pairing quotes across a file goes wrong the
 * moment one appears in a comment or a character class, and it fails by
 * quietly reading the gaps between the real strings - which is exactly how the
 * first version of this test passed while a hand-written gutter sat in
 * MemoryView. A line is coarser and cannot slip.
 */
function lines(source) {
  return source.split(/\r?\n/);
}

describe("the page gutter", () => {
  const files = views(FEATURES);

  it("finds the views", () => {
    // Vacuously passing is the one failure mode a test like this has.
    expect(files.length).toBeGreaterThanOrEqual(7);
  });

  it("is imported, never written out", () => {
    const offenders = [];
    for (const file of files) {
      const where = file.slice(FEATURES.length).replace(/\\/g, "/");
      for (const list of lines(readFileSync(file, "utf8"))) {
        if (BARE.test(list) && (RESPONSIVE.test(list) || HEADER_BAR.test(list))) {
          offenders.push(`${where}: ${list}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it("comes from one place for every view that uses it", () => {
    for (const file of files) {
      const source = readFileSync(file, "utf8");
      if (!source.includes("GUTTER")) continue;
      expect(source, file).toMatch(/GUTTER[^;]*\} from "@\/components\/layout\/View"/);
    }
  });
});

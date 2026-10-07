import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import url from "node:url";

/**
 * The client has to carry everything the bridge offers.
 *
 * `chooseFolder` was exposed by the bridge, handled by the backend, called by
 * the composer - and simply not listed in `bridgeClient`. So it was `undefined` in
 * the real desktop app, the composer's `typeof chooseFolder !== "function"`
 * guard concluded it was not running in a desktop app, and picking a working
 * folder told the user to go and use the desktop app they were already using.
 *
 * Nothing in the type system or at runtime notices a forgotten line in an
 * object literal, so this compares the two lists as text. It is crude, and it
 * is the only thing that would have caught it.
 */

const here = path.dirname(url.fileURLToPath(import.meta.url));
const root = path.resolve(here, "..", "..");

const read = (relative) => fs.readFileSync(path.join(root, relative), "utf8");

/**
 * The property names of the object literal that CONTAINS `marker`, at that
 * literal's own top level only.
 *
 * The marker points at a line inside the literal and the opening brace is
 * found by walking back from it, because the literals are not all introduced
 * the same way, and anchoring on the introduction meant a special case for
 * each.
 *
 * Both spellings count, because both adapters use both: `name: value` in the
 * bridge, and `async name()` method shorthand in the browser stand-in.
 */
function keysOfLiteral(source, marker) {
  const inside = source.indexOf(marker);
  if (inside === -1) throw new Error(`Could not find ${marker}`);

  const open = source.lastIndexOf("{", inside);
  if (open === -1) throw new Error(`No object literal around ${marker}`);

  let depth = 0;
  let close = -1;
  for (let i = open; i < source.length; i += 1) {
    if (source[i] === "{") depth += 1;
    else if (source[i] === "}") {
      depth -= 1;
      if (depth === 0) {
        close = i;
        break;
      }
    }
  }
  if (close === -1) throw new Error(`Unterminated object literal around ${marker}`);

  const keys = new Set();
  let nesting = 0;
  for (const line of source.slice(open + 1, close).split("\n")) {
    // Only lines that start at the literal's own top level. Anything deeper is
    // a method body or a nested object such as `secrets`, whose own property
    // names are not part of this surface.
    if (nesting === 0) {
      const match = /^(?:async\s+)?([A-Za-z_$][\w$]*)\s*[:(]/.exec(line.trim());
      // `call(...)` wrapped onto its own line is the body of the property
      // above it, not a property of this literal. Counting it would make the
      // bridge look like it exposed a method nobody can call.
      if (match && match[1] !== "call") keys.add(match[1]);
    }
    nesting += (line.match(/\{/g)?.length ?? 0) - (line.match(/\}/g)?.length ?? 0);
  }
  return keys;
}

describe("the workspace bridge adapter", () => {
  // The bridge module is the one place that says what the window may call.
  // The names have to match there and in both adapters,
  // and nothing at runtime notices a line missing from an object literal.
  const bridge = read("src/bridge/workspace.js");
  const client = read("src/lib/workspace-client.js");

  const exposed = keysOfLiteral(bridge, 'status: () => call("ws_status")');
  const forwarded = keysOfLiteral(client, 'kind: "bridge"');
  const standIn = keysOfLiteral(client, 'kind: "memory"');

  it("reads a real surface, not an empty set", () => {
    // A parser that silently found nothing would make every check below pass.
    expect(exposed.size).toBeGreaterThan(15);
    expect(forwarded.size).toBeGreaterThan(15);
    expect(standIn.size).toBeGreaterThan(15);
  });

  it("forwards every key the bridge exposes", () => {
    const missing = [...exposed].filter((key) => !forwarded.has(key));
    expect(missing, `workspaceAPI keys the renderer cannot reach: ${missing.join(", ")}`).toEqual(
      []
    );
  });

  it("offers the same surface in the browser stand-in", () => {
    // Otherwise a feature works in the desktop app and throws in a preview,
    // which is a worse failure than not offering it at all.
    const missing = [...exposed].filter((key) => !standIn.has(key));
    expect(missing, `keys the browser stand-in does not answer: ${missing.join(", ")}`).toEqual([]);
  });

  it("knows chooseFolder in particular, which is the one that was lost", () => {
    expect(exposed.has("chooseFolder")).toBe(true);
    expect(forwarded.has("chooseFolder")).toBe(true);
    expect(standIn.has("chooseFolder")).toBe(true);
  });
});

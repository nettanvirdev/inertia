import { describe, expect, it } from "vitest";
import fs from "node:fs";
import path from "node:path";
import url from "node:url";

/**
 * Every icon name the app spends must be a name the set can pay.
 *
 * Half the icons in Inertia are not written as components. They are strings -
 * `icon: "FolderOpen"` in a provider preset, `"FileJson"` in a file-kind map,
 * a name persisted with a routine and read back through `<Icon name>`. A
 * string cannot be checked by the module system: `getIcon` shrugged and
 * returned `Circle`, so a misspelled name drew a small dot and no error.
 *
 * That is exactly how the routine icon picker came to offer "GitPullRequest",
 * a glyph the set has never had. Seven choices drew a glyph and the eighth
 * drew a dot, and because a dot is a real glyph nothing looked broken enough
 * to chase. This test reads the geometry tables and the call sites as text and
 * compares the two lists, which is the only thing that would have caught it.
 *
 * It is deliberately textual rather than a render test. Rendering proves a
 * component mounts; it cannot prove that the string on line 50 of a dialog you
 * did not render names a glyph that exists.
 */

const here = path.dirname(url.fileURLToPath(import.meta.url));
const root = path.resolve(here, "..", "..", "..");
const iconsDir = path.join(root, "src", "components", "icons");

const GLYPH_TABLES = ["core", "files", "people", "science", "status", "system", "tools"];

/**
 * The glyph names, parsed out of the geometry tables.
 *
 * Read as text rather than imported so the test sees what a developer sees. An
 * import would also work today, but the tables are plain data whose only shape
 * is two-space indentation inside one exported object, and matching that shape
 * means a name that is somehow unreachable at runtime still shows up here as a
 * mismatch rather than quietly agreeing with itself.
 */
function definedGlyphs() {
  const names = new Set();
  for (const table of GLYPH_TABLES) {
    const source = fs.readFileSync(path.join(iconsDir, `glyphs.${table}.js`), "utf8");
    for (const match of source.matchAll(/^ {2}([A-Z][A-Za-z0-9]*):\s*\[/gm)) names.add(match[1]);
  }
  return names;
}

/** Every .js/.jsx file under src, minus the icon set itself. */
function sourceFiles(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === "node_modules") continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) sourceFiles(full, out);
    else if (/\.jsx?$/.test(entry.name)) out.push(full);
  }
  return out;
}

/**
 * The shapes a glyph name is written in.
 *
 * Each one is a position where the string is *known* to be an icon name, which
 * is what makes a miss a real failure rather than a coincidence. A blanket
 * "any capitalised string" rule was tried first and drowned the result in
 * labels - "Assistant", "Workspace", "Unknown" - none of which are icons.
 */
const NAME_SITES = [
  // <Icon name="Globe" />
  /<Icon\s+name="([^"]+)"/g,
  // { icon: "Globe" } in a preset, a catalog entry, a nav item
  /\bicon:\s*"([A-Za-z0-9]+)"/g,
  // the same key inside JSON-shaped data
  /"icon":\s*"([^"]+)"/g,
  // getIcon("Wrench")
  /\bgetIcon\(\s*"([^"]+)"/g,
  // the fallback half of `tool.icon ?? "Wrench"`
  /\bicon\s*\?\?\s*"([A-Za-z0-9]+)"/g,
];

/**
 * Constants whose whole job is holding glyph names - `FILE_KIND_ICON`,
 * `ICON_BY_TOOL`, the routine picker's `ICONS`. Their values sit in no `icon:`
 * position, so the patterns above cannot see them, and the picker's list was
 * where the real bug lived. Any const with ICON in its name counts, and every
 * double-quoted capitalised word inside it is treated as a glyph name.
 */
const ICON_TABLE = /\bconst\s+[\w$]*ICON[\w$]*\s*=\s*(\[[\s\S]*?\n\]|\{[\s\S]*?\n\})/g;

/** Named imports off the barrel: `import { Sparkles } from "@/components/icons"`. */
const BARREL_IMPORT = /import\s*\{([^}]*)\}\s*from\s*"@\/components\/icons"/g;

/** The barrel's own exports, which are not glyphs. */
const API = new Set(["Icon", "getIcon", "GLYPHS", "ICON_NAMES", "createIcon", "createIcons"]);

function collectReferences() {
  const refs = new Map();
  const note = (name, where) => {
    if (API.has(name)) return;
    if (!refs.has(name)) refs.set(name, new Set());
    refs.get(name).add(where);
  };

  for (const file of sourceFiles(path.join(root, "src"))) {
    const rel = path.relative(root, file).split(path.sep).join("/");
    if (rel.startsWith("src/components/icons/")) continue;
    const source = fs.readFileSync(file, "utf8");

    source.split("\n").forEach((line, i) => {
      for (const pattern of NAME_SITES) {
        pattern.lastIndex = 0;
        let match;
        while ((match = pattern.exec(line))) note(match[1], `${rel}:${i + 1}`);
      }
    });

    ICON_TABLE.lastIndex = 0;
    for (const match of source.matchAll(ICON_TABLE)) {
      for (const value of match[1].matchAll(/"([A-Z][A-Za-z0-9]*)"/g)) {
        note(value[1], `${rel} (icon table)`);
      }
    }

    BARREL_IMPORT.lastIndex = 0;
    for (const match of source.matchAll(BARREL_IMPORT)) {
      for (const raw of match[1].split(",")) {
        const name = raw
          .trim()
          .split(/\s+as\s+/)[0]
          .trim();
        if (/^[A-Z][A-Za-z0-9]*$/.test(name)) note(name, `${rel} (import)`);
      }
    }
  }
  return refs;
}

describe("the icon set", () => {
  const defined = definedGlyphs();
  const referenced = collectReferences();

  it("parses a real set of glyphs, not an empty one", () => {
    // A parser that found nothing would make the check below pass forever.
    expect(defined.size).toBeGreaterThan(120);
  });

  it("finds a real set of call sites, not an empty one", () => {
    expect(referenced.size).toBeGreaterThan(80);
  });

  it("resolves every name the app asks for", () => {
    const missing = [...referenced]
      .filter(([name]) => !defined.has(name))
      .map(([name, where]) => `${name} (${[...where].join(", ")})`);
    expect(
      missing,
      `icon names nothing in the set can resolve - these render a silent fallback:\n  ${missing.join(
        "\n  "
      )}`
    ).toEqual([]);
  });

  it("exports every glyph it defines, by name", () => {
    // The barrel re-exports the whole table through one destructuring
    // statement. A glyph added to a table but forgotten there is importable
    // nowhere, and a name typed there but present in no table destructures to
    // `undefined` - which React renders as a blank and blames on the caller.
    const barrel = fs.readFileSync(path.join(iconsDir, "index.jsx"), "utf8");
    const block = /export const \{([\s\S]*?)\} = ICONS;/.exec(barrel);
    expect(block, "index.jsx no longer re-exports the set the way this test reads it").toBeTruthy();

    const exported = new Set(block[1].split(/[,\s]+/).filter(Boolean));
    const unexported = [...defined].filter((name) => !exported.has(name));
    const phantom = [...exported].filter((name) => !defined.has(name));

    expect(unexported, `glyphs defined but not exported: ${unexported.join(", ")}`).toEqual([]);
    expect(phantom, `names exported but drawn by nothing: ${phantom.join(", ")}`).toEqual([]);
  });

  it("keeps the two fallback glyphs getIcon reaches for", () => {
    // `getIcon` hands back `Circle` in production and `OctagonAlert` in
    // development when a name is unknown. If either name ever goes, the
    // fallback itself becomes `undefined` and the failure mode gets worse
    // rather than louder.
    expect(defined.has("Circle")).toBe(true);
    expect(defined.has("OctagonAlert")).toBe(true);
  });

  it("draws something for every glyph", () => {
    // An entry with an empty array is a name that resolves, passes every check
    // above, and paints an empty 24x24 box.
    const empty = [];
    for (const table of GLYPH_TABLES) {
      const source = fs.readFileSync(path.join(iconsDir, `glyphs.${table}.js`), "utf8");
      for (const match of source.matchAll(/^ {2}([A-Z][A-Za-z0-9]*):\s*\[\s*\]/gm)) {
        empty.push(`${match[1]} (glyphs.${table}.js)`);
      }
    }
    expect(empty, `glyphs with no geometry: ${empty.join(", ")}`).toEqual([]);
  });
});

/**
 * The other half of "does every icon render": the ones that are files.
 *
 * The drawn set is only most of what the app shows. The logo in the rail, the
 * one on the setup screen, the tray icon, the .ico the Tauri bundler stamps
 * into the .exe and the two BMPs NSIS paints on the installer are all files on
 * disk named by a string somewhere, and every one of those strings can be
 * wrong in a way nothing notices until a user is looking at the result.
 *
 * They live in this file rather than one of their own because they fail for
 * exactly the reason the glyph names do: a name in a string position that the
 * module system cannot check.
 */

/** The first bytes each format must start with, so an extension cannot lie. */
const MAGIC = {
  ".png": [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a],
  ".ico": [0x00, 0x00, 0x01, 0x00],
  ".bmp": [0x42, 0x4d],
};

function inspect(relative) {
  const full = path.join(root, relative);
  if (!fs.existsSync(full)) return { relative, missing: true };
  const bytes = fs.readFileSync(full);
  const want = MAGIC[path.extname(relative).toLowerCase()];
  return {
    relative,
    missing: false,
    size: bytes.length,
    typed: !want || want.every((byte, i) => bytes[i] === byte),
  };
}

describe("the image files the app ships", () => {
  it("has every icon the bundle config names", () => {
    // Tauri resolves these at package time, on a machine that is usually not
    // the one that renamed the file, and a missing .ico is a failed release
    // rather than a missing image. The paths are relative to `src-tauri`.
    const config = JSON.parse(
      fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8")
    );
    const named = config.bundle?.icon ?? [];
    expect(named.length, "the bundle config stopped naming its icons").toBeGreaterThan(0);

    const results = named.map((relative) => inspect(path.join("src-tauri", relative)));
    const broken = results.filter((r) => r.missing || r.size === 0 || r.typed === false);
    expect(
      broken.map((r) => `${r.relative} (${r.missing ? "missing" : `${r.size} bytes`})`),
      "icons the bundle config names but cannot use"
    ).toEqual([]);
  });

  it("has every logo the renderer asks for", () => {
    const results = [
      "public/assets/logo-64.png",
      "public/assets/logo-256.png",
      "public/assets/logo-512.png",
    ].map(inspect);
    const broken = results.filter((r) => r.missing || r.size === 0 || r.typed === false);
    expect(
      broken.map((r) => r.relative),
      "logo sizes that are missing or not PNGs"
    ).toEqual([]);
  });

  it("points every <img> at a file that is really there", () => {
    const missing = [];
    for (const file of sourceFiles(path.join(root, "src"))) {
      const source = fs.readFileSync(file, "utf8");
      const rel = path.relative(root, file).split(path.sep).join("/");
      source.split("\n").forEach((line, i) => {
        for (const match of line.matchAll(/src="\.?\/(assets\/[^"]+)"/g)) {
          if (!fs.existsSync(path.join(root, "public", match[1]))) {
            missing.push(`${rel}:${i + 1} -> public/${match[1]}`);
          }
        }
      });
    }
    expect(missing, `<img> sources with nothing behind them: ${missing.join(", ")}`).toEqual([]);
  });

  it("keeps those <img> sources relative, so they survive file://", () => {
    // A relative source resolves under any origin the page is served from. The
    // earlier Electron build loaded `dist/index.html` as a file:// document,
    // where `src="/assets/logo-256.png"` is the root of the drive: it resolved
    // to file:///D:/assets/logo-256.png and ERR_FILE_NOT_FOUND, so the setup
    // screen, the rail and the settings header all opened with a broken-image
    // box in every build that was packaged. Tauri's asset protocol would
    // forgive the absolute form; this keeps the pages from depending on that.
    const absolute = [];
    for (const file of sourceFiles(path.join(root, "src"))) {
      const rel = path.relative(root, file).split(path.sep).join("/");
      // This file quotes the broken form in a comment, three lines up.
      if (rel.startsWith("src/components/icons/")) continue;
      fs.readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, i) => {
          if (/src="\/(?!\/)/.test(line)) absolute.push(`${rel}:${i + 1}`);
        });
    }
    expect(
      absolute,
      `absolute asset paths that break under file://: ${absolute.join(", ")}`
    ).toEqual([]);
  });
});

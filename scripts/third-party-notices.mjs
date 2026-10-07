// Writes THIRD_PARTY_NOTICES.md: every license we owe a copy of, once.
//
//   1. Rust: cargo-about over the three Cargo workspaces (app, installer,
//      uninstaller), configured by about.toml at the repo root. That file is
//      also the license allowlist - a dependency under anything new fails here.
//   2. JS: the production closure of package.json "dependencies", walked
//      through node_modules (devDependencies never ship, so they never start a
//      walk). License text comes from each package's own LICENSE/COPYING file,
//      plus NOTICE where an Apache package ships one.
//   3. Fonts: the @fontsource-variable packages are OFL-1.1. The OFL wants
//      each font's copyright line plus the license text, so they get their own
//      section instead of six copies of the same license.
//
// Identical texts are merged across all three sources and everything is
// sorted, so re-running on an unchanged tree produces no diff.
//
// Needs cargo-about: `cargo install cargo-about --locked --features cli`.

import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifests = ["src-tauri", "installer/src-tauri", "uninstaller/src-tauri"];
const FONT_SCOPE = "@fontsource-variable/";

// The fontsource LICENSE for Source Serif 4 only says "Google Inc.". Upstream
// (adobe-fonts/source-serif) is Adobe's, and reserves the font name.
const FONT_COPYRIGHT_OVERRIDES = {
  "source-serif-4":
    "Copyright 2014-2023 Adobe (http://www.adobe.com/), with Reserved Font Name 'Source'. " +
    "Source is a trademark of Adobe in the United States and/or other countries.",
};

// The Tauri plugin npm packages ship only a LICENSE.spdx stub. They are cut
// from the same repo as their Rust crate, so borrow the crate's license text.
const NPM_TO_CRATE = {
  "@tauri-apps/plugin-dialog": "tauri-plugin-dialog",
  "@tauri-apps/plugin-opener": "tauri-plugin-opener",
};

const tidy =(text) => text.replace(/\r\n?/g, "\n").replace(/[ \t]+$/gm, "").trim();
const byText = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

// license id -> normalized text -> { packages: Set<string>, notices: Map<label, text> }
const groups = new Map();
function record(license, text, label, notice) {
  if (!groups.has(license)) groups.set(license, new Map());
  const texts = groups.get(license);
  const key = tidy(text);
  if (!texts.has(key)) texts.set(key, { packages: new Set(), notices: new Map() });
  texts.get(key).packages.add(label);
  if (notice) texts.get(key).notices.set(label.split(" ").slice(0, 2).join(" "), tidy(notice));
}

function noticeIn(dir) {
  const file = existsSync(dir) && readdirSync(dir).find((f) => /^notice(\.(md|txt))?$/i.test(f));
  return file ? readFileSync(join(dir, file), "utf8") : null;
}

function repoUrl(repository) {
  let url = typeof repository === "string" ? repository : repository?.url;
  if (!url) return "";
  url = url.replace(/^git\+/, "").replace(/\.git$/, "").replace(/^git:\/\//, "https://");
  url = url.replace(/^git@github\.com:/, "https://github.com/").replace(/^github:/, "https://github.com/");
  return /^[\w.-]+\/[\w.-]+$/.test(url) ? `https://github.com/${url}` : url;
}

// 1. Rust
const crateTexts = new Map(); // "crate license" -> text, for npm packages that ship none
for (const manifest of manifests) {
  const args = ["about", "generate", "--format", "json", "--locked", "-c", join(root, "about.toml")];
  args.push("-m", join(root, manifest, "Cargo.toml"));
  console.log(`> cargo ${args.join(" ")}`);
  const result = spawnSync("cargo", args, { encoding: "utf8", maxBuffer: 1 << 28, stdio: ["ignore", "pipe", "inherit"] });
  if (result.status !== 0) {
    console.error(`cargo-about failed for ${manifest} (exit ${result.status}).`);
    process.exit(result.status ?? 1);
  }
  for (const license of JSON.parse(result.stdout).licenses) {
    for (const { crate } of license.used_by) {
      const label = `${crate.name} ${crate.version} ${repoUrl(crate.repository ?? crate.homepage ?? "")}`.trim();
      record(license.id, license.text, label, noticeIn(dirname(crate.manifest_path)));
      crateTexts.set(`${crate.name} ${license.id}`, license.text);
    }
  }
}

// 2. JS - node resolution: look in ./node_modules, then each parent's.
function resolvePackage(name, fromDir) {
  for (let dir = fromDir; ; dir = dirname(dir)) {
    const candidate = join(dir, "node_modules", name);
    if (existsSync(join(candidate, "package.json"))) return candidate;
    if (dir === root || dirname(dir) === dir) return null;
  }
}

const fonts = [];
const seen = new Set();
const rootPkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const queue = Object.keys(rootPkg.dependencies ?? {}).map((name) => [name, root]);
while (queue.length) {
  const [name, from] = queue.shift();
  const dir = resolvePackage(name, from);
  if (!dir) continue; // an optional dependency for another platform
  if (seen.has(dir)) continue;
  seen.add(dir);
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  for (const dep of Object.keys({ ...pkg.dependencies, ...pkg.optionalDependencies })) queue.push([dep, dir]);

  // Same rule as about.toml: of "MIT OR Apache-2.0", we take MIT.
  let license = typeof pkg.license === "string" ? pkg.license : pkg.license?.type ?? "UNKNOWN";
  if (/\bOR\b/.test(license) && /\bMIT\b/.test(license)) license = "MIT";

  const files = readdirSync(dir)
    .filter((f) => /^(licen[cs]e|copying)([._-].*)?$/i.test(f) && !f.endsWith(".spdx"))
    .sort((a, b) => /mit/i.test(b) - /mit/i.test(a) || byText(a, b));
  let text = files.length ? readFileSync(join(dir, files[0]), "utf8") : null;
  text ??= crateTexts.get(`${NPM_TO_CRATE[name]} ${license}`) ?? null;
  if (name.startsWith(FONT_SCOPE)) {
    fonts.push({ name, version: pkg.version, text });
    continue;
  }
  if (!text) {
    console.error(`${name}@${pkg.version} ships no license file - map it in NPM_TO_CRATE or add its text.`);
    process.exit(1);
  }
  record(license, text, `${pkg.name} ${pkg.version} ${repoUrl(pkg.repository)}`.trim(), noticeIn(dir));
}

// 3. Render
const fence = (text) => `\`\`\`\`text\n${text}\n\`\`\`\``;
const out = [
  "# Third-party notices",
  "",
  "Inertia is MIT licensed (see LICENSE). It includes the following third-party software, " +
    "listed under the license each is used under. Where a package offers a choice of licenses, " +
    "the first acceptable one in about.toml is shown. Fonts are listed separately at the end.",
  "",
  "Generated by `bun run notices` (scripts/third-party-notices.mjs). Do not edit by hand.",
  "",
  "| License | Packages |",
  "| --- | --- |",
];
const licenses = [...groups.keys()].sort(byText);
for (const id of licenses) {
  const count = new Set([...groups.get(id).values()].flatMap((g) => [...g.packages])).size;
  out.push(`| ${id} | ${count} |`);
}
out.push(`| OFL-1.1 (fonts) | ${fonts.length} |`, "");

for (const id of licenses) {
  out.push(`## ${id}`, "");
  const entries = [...groups.get(id)].map(([text, g]) => [text, [...g.packages].sort(byText), g.notices]);
  entries.sort((a, b) => byText(a[1][0], b[1][0]));
  for (const [text, packages, notices] of entries) {
    out.push("Used by:", "", ...packages.map((p) => `- ${p}`), "", fence(text), "");
    for (const [pkg, notice] of [...notices].sort((a, b) => byText(a[0], b[0]))) {
      out.push(`NOTICE (${pkg}):`, "", fence(notice), "");
    }
  }
}

fonts.sort((a, b) => byText(a.name, b.name));
out.push("## Fonts (OFL-1.1)", "");
for (const font of fonts) {
  const slug = font.name.slice(FONT_SCOPE.length);
  const copyright = FONT_COPYRIGHT_OVERRIDES[slug] ?? tidy(font.text.split("This Font Software")[0]);
  out.push(`- ${font.name} ${font.version}: ${copyright.replace(/\s+/g, " ")}`);
}
const ofl = tidy(fonts[0].text.slice(fonts[0].text.indexOf("-----")));
out.push("", fence(ofl), "");

const target = join(root, "THIRD_PARTY_NOTICES.md");
writeFileSync(target, out.join("\n"));
console.log(`Wrote ${target}: ${licenses.length + 1} licenses, ${seen.size} npm packages.`);

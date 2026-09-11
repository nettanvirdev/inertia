// Builds the Windows setup experience end to end.
//
//   1. build the themed uninstaller and stage it as an app resource, so the
//      app's own bundle carries it into the install directory
//   2. bundle the app, which produces the real NSIS installer
//   3. embed that installer into the bootstrapper as its payload
//   4. build the bootstrapper as a plain exe (no bundling - it IS the thing
//      people download)
//   5. drop the result in dist-setup/ under a name worth handing to someone
//
// Those are three separate Cargo projects with three separate target
// directories, so this is the only place that knows they are one deliverable.
// The order matters: the uninstaller must exist before the app is bundled, or
// NSIS packages a missing resource and Windows' uninstall entry stays pointed
// at the stock grey wizard.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, copyFileSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const appTauri = join(root, "src-tauri");
const setupTauri = join(root, "installer", "src-tauri");
const uninstallTauri = join(root, "uninstaller", "src-tauri");

if (process.platform !== "win32") {
  console.error(
    "The setup bootstrapper is Windows-only. On macOS, `bun run tauri build` already\n" +
      "produces the .dmg, which is the platform's normal install experience.",
  );
  process.exit(1);
}

function run(command, args, cwd) {
  console.log(`\n> ${command} ${args.join(" ")}  (in ${cwd})`);
  // `shell: true` because on Windows `bun`/`bunx` are .cmd shims, which
  // CreateProcess cannot execute directly.
  const result = spawnSync(command, args, { cwd, stdio: "inherit", shell: true });
  if (result.status !== 0) {
    console.error(`\n${command} failed with exit code ${result.status}.`);
    process.exit(result.status ?? 1);
  }
}

function newestNsisInstaller() {
  const dir = join(appTauri, "target", "release", "bundle", "nsis");
  if (!existsSync(dir)) {
    console.error(`No NSIS output at ${dir}. Did the app bundle step run?`);
    process.exit(1);
  }
  // Old versions linger in this directory, so pick by mtime rather than
  // assuming the only .exe present is the one just built.
  const candidates = readdirSync(dir)
    .filter((name) => name.endsWith(".exe"))
    .map((name) => join(dir, name))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs);

  if (candidates.length === 0) {
    console.error(`No .exe in ${dir}.`);
    process.exit(1);
  }
  return candidates[0];
}

/**
 * Windows locks the destination while anything holds a handle to it - the
 * previously built installer still open, Explorer generating a thumbnail, or
 * Defender mid-scan of a file written seconds ago. All of those clear on their
 * own, so retry rather than failing a ten-minute build on the last line.
 */
function copyWithRetry(from, to, attempts = 6) {
  for (let i = 1; i <= attempts; i++) {
    try {
      copyFileSync(from, to);
      return;
    } catch (error) {
      const locked = error.code === "EBUSY" || error.code === "EPERM" || error.code === "EACCES";
      if (!locked || i === attempts) {
        if (locked) {
          console.error(
            `\nCannot write ${to} - it is locked by another process.\n` +
              `Close the running installer (or wait for Defender to finish scanning it) and re-run.`,
          );
          process.exit(1);
        }
        throw error;
      }
      // Synchronous sleep: this script is a linear pipeline, and going async
      // here would mean threading promises through every step for one retry.
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 500 * i);
    }
  }
}

const { productName, version } = JSON.parse(
  readFileSync(join(appTauri, "tauri.conf.json"), "utf8"),
);

// 1. The themed uninstaller, staged where the app's bundle.resources expects it.
run("bunx", ["tauri", "build", "--no-bundle"], uninstallTauri);
const uninstallBuilt = join(uninstallTauri, "target", "release", "inertia-uninstall.exe");
if (!existsSync(uninstallBuilt)) {
  console.error(`Expected the uninstaller at ${uninstallBuilt}, but it is not there.`);
  process.exit(1);
}
const uninstallStaged = join(appTauri, "resources", "inertia-uninstall.exe");
mkdirSync(dirname(uninstallStaged), { recursive: true });
copyWithRetry(uninstallBuilt, uninstallStaged);
console.log(
  `
Staging uninstaller (${(statSync(uninstallStaged).size / 1024 / 1024).toFixed(1)} MB)`,
);

// 2. The app.
run("bun", ["run", "tauri", "build"], root);

// 3. The payload.
const payload = newestNsisInstaller();
const target = join(setupTauri, "resources", "payload.exe");
mkdirSync(dirname(target), { recursive: true });
copyFileSync(payload, target);
console.log(
  `\nEmbedding ${payload}\n       as ${target}  (${(statSync(target).size / 1024 / 1024).toFixed(1)} MB)`,
);

// 4. The bootstrapper. Run from its own crate directory: the Tauri CLI finds
// the project by walking up from the working directory, and `--config` alone
// would not move it off the app's.
run("bunx", ["tauri", "build", "--no-bundle"], setupTauri);

// 5. The deliverable.
const built = join(setupTauri, "target", "release", "inertia-setup.exe");
if (!existsSync(built)) {
  console.error(`Expected the bootstrapper at ${built}, but it is not there.`);
  process.exit(1);
}

const outDir = join(root, "dist-setup");
mkdirSync(outDir, { recursive: true });
const out = join(outDir, `${productName}-Setup-${version}.exe`);
copyWithRetry(built, out);

console.log(`\nDone.\n  ${out}  (${(statSync(out).size / 1024 / 1024).toFixed(1)} MB)\n`);

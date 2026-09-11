# tauri-rust-starter-project

A Tauri v2 + React + TypeScript starter with a custom frameless window, a themed
design system, and dependency updates wired up.

## What's in it

- **Tauri 2 + React 19 + TypeScript + Vite**, package-managed with [Bun](https://bun.sh).
- **Frameless window with custom chrome.** Native decorations are off; the
  titlebar in `src/components/Titlebar.tsx` draws its own minimize / maximize /
  close controls from a small hand-drawn icon set (`src/lib/icons.tsx`).
- **Animated maximize and restore.** `src/hooks/useWindowChrome.ts` tweens the
  window's bounds instead of letting it snap, so the window grows and shrinks
  smoothly. It targets the monitor's real work area via a small Win32 call
  (`get_monitor_work_area` in `src-tauri/src/lib.rs`), so a maximized window
  doesn't sit under the taskbar.
- **Rounded corners that stay correct.** The compositor owns the window corner:
  `apply_rounded_corners` sets `DWMWA_WINDOW_CORNER_PREFERENCE` on every window
  the app opens, so rounding holds even where the Windows default is disabled,
  and it tracks DPI on its own. Nothing rounds the shell in CSS - a second,
  tighter curve there just cuts the content away from the frame and leaves a
  gap of the desktop showing through.
- **A full theme.** `src/styles/globals.css` carries a two-palette design system
  (light and dark, tokens for surfaces, fills, status colours, motion curves and
  a radius ramp) built on Tailwind v4. Retheming the whole app happens in that
  one file.
- **A themed Windows installer.** The stock NSIS wizard is never shown: a
  second tiny Tauri app in `installer/` wears the app's own chrome and
  palette and drives the real installer silently underneath. See
  [The setup experience](#the-setup-experience). macOS gets the normal
  drag-to-Applications `.dmg`.
- **A first-run walkthrough.** `src/onboarding/` runs once, picks a theme,
  and persists it through a small JSON file the Rust side owns.
- **Dependabot.** Weekly npm and cargo updates, grouped into one pull request
  per ecosystem. There is deliberately no CI workflow: compiling Tauri on a
  hosted Windows runner is slow and would run on every dependency bump, so
  updates are verified by building locally instead.

## Getting started

```bash
bun install
bun run tauri dev
```

To produce a release build:

```bash
bun run tauri build
```

## The setup experience

### Windows: a bootstrapper, not a wizard

`bun run tauri build` produces a stock NSIS `-setup.exe` - grey, Win95-boned,
and unthemable past a header bitmap. So it is never the thing anyone sees.
Instead, `installer/` is a second, tiny Tauri app whose window is the app's own
frameless chrome and the app's own palette, and the NSIS installer is embedded
inside it as a payload. The bootstrapper runs that payload silently
(`/S /D=<dir>`, plus `/NS` when shortcuts are declined) and animates a real
setup screen over the top of it.

```bash
bun run dev:setup     # the setup screen, hot-reloading, no install performed
bun run build:setup   # the whole chain, ending in dist-setup/
```

`scripts/build-setup.mjs` is the only place that knows those are one
deliverable: it bundles the app, copies the freshest NSIS output to
`installer/src-tauri/resources/payload.exe`, builds the bootstrapper with
`--no-bundle`, and writes `dist-setup/<product>-Setup-<version>.exe`. That
single file is what you hand to a machine.

Four things about it are worth knowing before changing any of them:

- **The payload is `include_bytes!`d, not a Tauri resource.** What people
  download has to be one file; a resource sitting next to the exe is one more
  thing to lose. `installer/src-tauri/build.rs` writes an empty placeholder so
  a clean clone still passes `cargo check`, and `run_install` refuses to run
  against an empty one.
- **`/D=` is passed with `raw_arg`, not `arg`.** NSIS reads the rest of the
  command line literally, so the path must be unquoted and last.
  `Command::arg` would quote any path containing a space, and NSIS would then
  install to its own default location rather than fail visibly.
- **The install is per-user** (`installMode: "currentUser"` in the app's
  `tauri.conf.json`), so nothing has to elevate and the bootstrapper can drive
  the whole thing without a UAC prompt in the middle of the animation.
- **The progress bar is phase-based and says so.** A silent NSIS install
  reports no byte count, so `useInstallProgress` gives each phase a ceiling and
  eases toward it - approaching asymptotically during the long copy rather
  than claiming a number it does not have.

The product name, version and binary name are read out of the app's own
`tauri.conf.json` at compile time by the installer's `build.rs`, so renaming
the app cannot leave the installer writing to a stale folder - it fails the
build instead.

### Deliberately absent: "start when Windows starts"

The installer does not offer it, and nothing here writes a Run key or a
Startup shortcut. That is on purpose.

The failure it avoids is the one everybody hits: an installer checkbox writes
autostart one way, and the app's own settings toggle writes it another, so
turning the setting off deletes the key the app owns and leaves the one the
installer created. The app then keeps launching at login while its own
settings screen truthfully reports that it will not, and no amount of
re-toggling fixes it - the toggle is reading the wrong thing.

If autostart is wanted later, the rules that keep it honest are:

- **One owner: the app.** Never the installer, never a shortcut dropped in
  `shell:startup` by a build script.
- **One mechanism.** `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
  is the whole of it. Not that *and* a Startup folder shortcut, and not a
  scheduled task as well.
- **The registry is the state, not a JSON flag.** The settings toggle should
  read the actual key every time it is shown, so the UI cannot claim one thing
  while Windows does another. A cached `autostart: false` in
  `preferences.json` is exactly how the two drift apart.
- **The uninstaller removes it.** Otherwise an uninstalled app leaves a Run
  entry pointing at a path that no longer exists.

### macOS: the normal thing

None of the above applies. `bun run tauri build` produces a `.dmg`, drag-to-
Applications is what the platform expects, and the bootstrapper crate
`compile_error!`s if you point it at a non-Windows target on purpose.

### Neither is signed

`dist-setup/` output and the `.dmg` are both unsigned, which is fine for
personal use and not fine for handing to strangers. Expect Windows SmartScreen
to show "Windows protected your PC" (More info -> Run anyway), and macOS
Gatekeeper to refuse a double-click (right-click -> Open, once, or
`xattr -dr com.apple.quarantine /Applications/inertia.app`).

## The first-run walkthrough

Onboarding lives in the app rather than the installer, so it is one flow on
both platforms and shows for a copy that arrived by any route. `src/App.tsx`
reads `preferences.json` from the OS config dir through two small Rust
commands (`load_preferences` / `save_preferences` in `src-tauri/src/lib.rs`)
and renders `src/onboarding/` instead of the workspace while
`onboardingCompleted` is false. Deleting that file brings the walkthrough
back, which is also how you iterate on it.

The theme step writes through immediately rather than at the end, so quitting
mid-walkthrough keeps the choice. `"system"` is resolved live rather than
frozen at the moment of choosing - `watchSystemTheme` in `src/lib/prefs.ts`
repaints when the OS flips.

Deliberately not the store plugin: two scalars and a first-run flag do not
need a keyed database, a migration story, or another permission set in
`capabilities/`, and the flag has to be readable before the first frame
paints.

## Notes

`src-tauri/.cargo/config.toml` caps Cargo at 4 parallel jobs and the dev profile
trims debug info. Both are there because a machine with many cores will happily
launch one `rustc` per core, and compiling Tauri's larger crates that way can
exhaust memory and fail the build with `rustc-LLVM ERROR: out of memory`. Raise
`jobs` if your machine has headroom.

Window commands used by the custom titlebar (`set-size`, `set-position`,
`start-dragging`, and so on) are granted in `src-tauri/capabilities/default.json`.
Add any further permissions there.

## Recommended IDE setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

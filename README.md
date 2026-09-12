# Inertia

A desktop app for working with agents: conversations, the tools they run, the
machines they run them on, and everything they write down. Tauri 2 on the
outside, Rust underneath, React 19 in the window, package-managed with
[Bun](https://bun.sh).

## What's in it

- **One workspace folder.** Everything the app knows - conversations, agents,
  routines, memories, permissions, secrets - is files in a folder you choose on
  first run, as JSON and Markdown you can read. Pointing a fresh install at an
  existing folder is the whole of the restore path.
- **A turn is streamed, never awaited.** `agent_run` returns a turn id and the
  turn reports on one event channel: prose, reasoning, tool calls starting and
  finishing, and the two things that suspend a tool mid-call while a person
  decides - an approval, and a question the model asked. See
  [docs/BACKEND_API.md](docs/BACKEND_API.md).
- **The tools are Rust.** Shell, files, edits, search, a real pty per terminal
  tab, a browser pane that is a child webview, sub-agents, memory, and
  scheduled routines that keep time whether a window is open or not. Each lives
  in its own crate under `src-tauri/crates/`, testable without a window.
- **Frameless window with custom chrome.** Native decorations are off;
  `src/components/layout/Titlebar.jsx` draws the controls, `bridge/app.js`
  drives the window, and `platform/window.rs` sets
  `DWMWA_WINDOW_CORNER_PREFERENCE` so rounding holds even where the Windows
  default is disabled. Nothing rounds the shell in CSS - a second, tighter
  curve there cuts the content away from the frame and leaves a gap of the
  desktop showing through.
- **A full theme.** `src/styles/globals.css` carries a two-palette design
  system (light and dark, tokens for surfaces, fills, status colours, motion
  curves and a radius ramp) built on Tailwind v4. Retheming happens in that one
  file.
- **A themed Windows installer.** The stock NSIS wizard is never shown: a
  second tiny Tauri app in `installer/` wears the app's own chrome and palette
  and drives the real installer silently underneath. See
  [The setup experience](#the-setup-experience). macOS gets the normal
  drag-to-Applications `.dmg`.
- **Dependabot.** Weekly npm and cargo updates, grouped into one pull request
  per ecosystem. There is deliberately no CI workflow: compiling Tauri on a
  hosted Windows runner is slow and would run on every dependency bump, so
  updates are verified by building locally instead.

## Layout

| Path | What it is |
|---|---|
| `src/` | the window. `features/` per screen, `bridge/` the whole of the boundary, `shared/` the logic both halves agree on |
| `src-tauri/src/` | the commands, one module per domain, thin over the crates |
| `src-tauri/crates/` | the tools, the store, the providers - no Tauri types in any of them |
| `installer/`, `uninstaller/` | two more Tauri apps: the setup bootstrapper and the themed uninstall screen |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md) and [BACKEND_API.md](docs/BACKEND_API.md) |

## Getting started

```bash
bun install
bun run tauri dev
```

Tests and checks, all four of which have to be clean before anything ships:

```bash
bun run test                                      # the window
bun run build                                     # the window, built
cd src-tauri && cargo test --workspace            # the backend
cd src-tauri && cargo clippy --workspace --all-targets
```

To produce a release build:

```bash
bun run tauri build     # the app, plus a stock NSIS installer nobody should see
bun run build:setup     # what you actually hand to a machine - see below
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

### "Start when Windows starts"

The app owns this and the installer does not go near it. That split is the
whole point.

The failure it avoids is the one everybody hits: an installer checkbox writes
autostart one way, and the app's own settings toggle writes it another, so
turning the setting off deletes the key the app owns and leaves the one the
installer created. The app then keeps launching at login while its own
settings screen truthfully reports that it will not, and no amount of
re-toggling fixes it - the toggle is reading the wrong thing.

So there are four rules, and all four hold:

- **One owner: the app.** Never the installer, never a shortcut dropped in
  `shell:startup` by a build script. The switch in Settings is the only thing
  that writes it.
- **One mechanism.** `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
  is the whole of it, through `tauri-plugin-autostart`. Not that *and* a
  Startup folder shortcut, and not a scheduled task as well.
- **The registry is the state, not a JSON flag.** The toggle asks the OS every
  time it is drawn and nothing is cached, so the UI cannot claim one thing
  while Windows does another. A stored `launchAtLogin: false` is exactly how
  the two drift apart, which is why nothing stores one.
- **The uninstaller removes it**, and only when the entry points inside the
  folder it is removing - something else on the machine may own an entry by
  the same name, and deleting that because the strings matched would be this
  uninstaller breaking an application it has nothing to do with.

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

## First run

There is no separate onboarding flow and no preferences file beside the app.
The app opens on one question - where the workspace folder goes - and that
screen (`src/features/onboarding/SetupView.jsx`) is the only thing that exists
before there is a folder. Every preference after that, the theme included,
lives in the workspace with everything else, because a setting stored outside
it is a setting that does not come back when the folder does.

The verdict under the field is the load-bearing part: creating a folder, adding
to a folder someone already uses, and adopting a workspace written by another
install are three different answers, and the screen says which one it is about
to give before the button is pressed.

## Notes

`src-tauri/.cargo/config.toml` caps Cargo at 4 parallel jobs and the dev profile
trims debug info. Both are there because a machine with many cores will happily
launch one `rustc` per core, and compiling Tauri's larger crates that way can
exhaust memory and fail the build with `rustc-LLVM ERROR: out of memory`. Raise
`jobs` if your machine has headroom.

Window commands used by the custom titlebar (`set-size`, `set-position`,
`start-dragging`, and so on) are granted in `src-tauri/capabilities/default.json`.
Add any further permissions there.

A command that no screen calls is a command that gets deleted. The two
authoritative lists are the `generate_handler![...]` block in
`src-tauri/src/lib.rs` and `src/bridge/contract.js`; if a name is in the first
and in no bridge module, it is dead, and dead IPC is surface nobody is
maintaining.

## Recommended IDE setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

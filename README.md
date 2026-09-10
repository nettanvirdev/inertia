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

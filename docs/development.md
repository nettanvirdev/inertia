# Development

How to build Inertia from source, run it in development, test it, and produce
a release build.

## Platform status

| Platform | Status |
|---|---|
| Windows 10/11 | Primary platform. Developed, built and tested here; CI runs on `windows-latest`. |
| macOS | The code is written to be cross-platform and `tauri build` produces a `.dmg`, but it is untested. |
| Linux | As macOS: expected to build, untested. |
| Setup bootstrapper and uninstaller (`installer/`, `uninstaller/`) | Windows only. |

Reports and fixes for macOS and Linux are welcome.

## Prerequisites

- **Rust**, stable toolchain via [rustup](https://rustup.rs). The minimum
  supported version is **1.85** (`rust-version` in `src-tauri/Cargo.toml`).
  Add clippy with `rustup component add clippy`.
- **[Bun](https://bun.sh)** for installing JavaScript dependencies and running
  scripts.
- **Node.js** (any current LTS) for the scripts in `scripts/`, which are run
  with `node`.
- Platform build dependencies for Tauri 2, as described in
  [Tauri's prerequisites](https://v2.tauri.app/start/prerequisites/):
  - **Windows**: Microsoft C++ Build Tools ("Desktop development with C++")
    and the WebView2 runtime (preinstalled on Windows 11).
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`).
  - **Linux**: the WebKitGTK 4.1 development packages and friends
    (`libwebkit2gtk-4.1-dev`, `build-essential`, `libssl-dev`,
    `libayatana-appindicator3-dev`, `librsvg2-dev` on Debian/Ubuntu).
- **Docker** (optional), to work on Docker computers and the sandbox image.

Optional, for the `lsp` tool while developing: `rust-analyzer`,
`typescript-language-server`, `pyright`, `gopls` on your `PATH`.

## Repository layout

| Path | What it is |
|---|---|
| `src/` | The window: React 19, plain JSX, Tailwind CSS v4. `features/` per screen, `bridge/` the boundary to Rust, `shared/` pure logic, `components/` UI building blocks, `styles/globals.css` the design tokens. |
| `src-tauri/` | Cargo workspace root and the `inertia` app crate (`src/`). `capabilities/` holds Tauri permissions, `icons/` the app icons, `windows/hooks.nsh` NSIS hooks. |
| `src-tauri/crates/` | The library crates. See [architecture.md](architecture.md#crates). |
| `src-tauri/crates/inertia-computers/sandbox/` | The sandbox image: Dockerfile and the scripts baked into it. |
| `installer/` | The Windows setup bootstrapper: its own Vite UI (`installer/src`) and Tauri project (`installer/src-tauri`). |
| `uninstaller/` | The Windows uninstaller: a Tauri project that reuses the installer's UI. |
| `scripts/` | `build-setup.mjs` (Windows setup pipeline) and `third-party-notices.mjs`. |
| `docs/` | This documentation. |

The installer and uninstaller are separate Cargo projects, not members of the
`src-tauri` workspace.

## Run it

```bash
bun install
bun run tauri dev
```

`tauri dev` starts Vite on `http://localhost:1420` and launches the app
against it, with hot reload for the window and a rebuild on Rust changes. The
first Rust build takes a while.

Logging uses `tracing`; set `RUST_LOG` to change the filter (the default is
`inertia=info`), for example `RUST_LOG=inertia=debug bun run tauri dev`.

### The rendering preview page

`src/preview.html` is a development-only page for looking at how messages
render (markdown, code blocks, tables, maths, diagrams, tool-call cards,
diffs, memory cards) without a model or the Rust side. Run the Vite dev server
on its own and open it:

```bash
bun run dev
# then open http://localhost:1420/src/preview.html
```

It runs under the same content-security policy as the app. It is not part of
the build.

### The installer UI

```bash
bun run dev:setup-ui      # the setup/uninstall UI in a browser, port 1430
bun run dev:setup         # the setup bootstrapper as a Tauri app (no install is performed)
bun run dev:uninstall     # the uninstaller as a Tauri app
```

## Tests and checks

All of these must pass before a change is merged; CI
(`.github/workflows/ci.yml`) runs them.

```bash
# Window
bun run test              # Vitest: src/**/*.test.{js,jsx}
bun run build             # tsc + vite build into dist/

# Rust (run `bun run build` once first: the app crate embeds dist/ at compile time)
cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --workspace --manifest-path src-tauri/Cargo.toml

# Installer and uninstaller (Windows; need `bun run build:setup-ui` first)
cargo check --manifest-path installer/src-tauri/Cargo.toml
cargo check --manifest-path uninstaller/src-tauri/Cargo.toml
```

To test one crate: `cargo test -p inertia-tools --manifest-path src-tauri/Cargo.toml`.

**One rule for tests in the app crate**: never construct an `AppState` (or
anything else that pulls in Tauri's window machinery) inside a
`#[cfg(test)]` in `src-tauri/src`. On Windows the test binary then fails to
start with `STATUS_ENTRYPOINT_NOT_FOUND` before any test runs. Build what the
test needs directly (`Workspace::open`, a recording emitter, a small trait)
instead.

## The headless devkit

`inertia-devkit` runs the real agent loop, the same code the app runs, against
a mock provider, mock tools and a scripted permission gate. No API key, no
network, no window. It prints the whole event stream, which makes it the
fastest way to work on the turn loop, the registry or the permission engine.

```bash
cargo run -p inertia-devkit --manifest-path src-tauri/Cargo.toml            # list scenarios
cargo run -p inertia-devkit --manifest-path src-tauri/Cargo.toml -- tools   # run one
```

Scenarios: `chat` (a plain reply), `tools` (a tool round trip), `denied` (a
refused call), `invalid` (a call the schema rejects), `loop` (the repeated-call
guard), `failure` (the provider failing mid-stream).

Two other helpers:

```bash
# Read a real workspace folder and report which records parse
cargo run -p inertia-store --example scan --manifest-path src-tauri/Cargo.toml -- <workspace>
```

`inertia-mcp` also builds a `fake-mcp-server` binary used by its integration
tests.

## Release builds

```bash
bun run tauri build
```

This builds the window, compiles the app in release mode (fat LTO, stripped)
and bundles it: an NSIS installer on Windows (`src-tauri/tauri.windows.conf.json`
limits Windows to NSIS), a `.dmg`/`.app` on macOS, and Linux packages, under
`src-tauri/target/release/bundle/`. The release build bundles `LICENSE` and
`THIRD_PARTY_NOTICES.md`.

If `rustc` runs out of memory during the release link, `lto = "thin"` in
`[profile.release]` is a cheap fallback; you can also limit parallel jobs with
`CARGO_BUILD_JOBS`.

### The Windows setup bootstrapper

What Windows users download is not the stock NSIS installer but a small
themed Tauri app that embeds it and runs it silently.

```bash
bun run build:setup       # Windows only; output in dist-setup/
```

`scripts/build-setup.mjs`:

1. builds the uninstaller (`uninstaller/src-tauri`, `--no-bundle`) and stages
   it as `src-tauri/resources/inertia-uninstall.exe`;
2. builds and bundles the app with `--config src-tauri/tauri.setup.conf.json`,
   which adds the uninstaller as a resource (a plain `tauri build` does not
   need it);
3. copies the newest NSIS installer to
   `installer/src-tauri/resources/payload.exe`;
4. builds the bootstrapper (`installer/src-tauri`, `--no-bundle`), which embeds
   the payload with `include_bytes!`;
5. writes `dist-setup/<productName>-Setup-<version>.exe`.

Details worth knowing before changing it:

- The install is per user (`installMode: "currentUser"`), so nothing needs to
  elevate.
- The installer's `build.rs` reads the product name, version and binary name
  from `src-tauri/tauri.conf.json`, so they cannot drift.
- `installer/src-tauri/build.rs` writes an empty placeholder payload so a
  fresh clone still passes `cargo check`; the bootstrapper refuses to run with
  an empty payload.
- The NSIS `/D=<dir>` argument is passed raw, unquoted and last, because NSIS
  reads the rest of the command line literally.
- "Start when Windows starts" is owned by the app (Settings, through
  `tauri-plugin-autostart`), never by the installer. The uninstaller removes
  the entry only if it points into the folder being removed.

See [releasing.md](releasing.md) for the release checklist.

## Third-party notices

`THIRD_PARTY_NOTICES.md` at the repository root collects the license texts of
everything that ships in the app. Regenerate it after changing dependencies:

```bash
cargo install cargo-about --locked --features cli   # once
bun install                                         # node_modules must be present
bun run notices
```

This runs `scripts/third-party-notices.mjs`, which runs `cargo-about` over the
three Cargo projects (configured by `about.toml`, which is also the license
allowlist: a dependency under a license not on it fails the run), walks the
production npm dependencies in `node_modules`, and adds the bundled fonts.
Commit the result. See [third-party-licenses.md](third-party-licenses.md).

## Code style

- **Rust**: `cargo fmt` (`src-tauri/rustfmt.toml`, width 100). Keep clippy
  clean with the workspace lints in `src-tauri/Cargo.toml`; avoid `unwrap` and
  `expect` outside tests. `thiserror` in library crates, `anyhow` only in
  binaries. Nothing under `crates/` may depend on Tauri.
- **JavaScript**: plain JSX and ES modules, no TypeScript in `src/` (the
  installer UI is TypeScript). Logic that both sides must agree on goes in
  `src/shared/` as pure functions with tests next to them.
- **Comments** explain why, not what: the constraint, the failure a line
  prevents, the alternative that was rejected.
- **Errors shown to people** are full sentences that say what to do next.
- **Dead code goes.** A command no screen calls, or a bridge method nothing
  uses, is removed rather than kept "just in case".
- Window permissions (Tauri capabilities) are granted in
  `src-tauri/capabilities/default.json`; add new ones there.

## Recommended editor setup

[VS Code](https://code.visualstudio.com/) with the
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode)
and [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
extensions.

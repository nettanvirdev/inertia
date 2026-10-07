# Contributing to Inertia

Thanks for your interest. Bug reports, fixes, documentation and features are
all welcome.

## Before you start

- **Bugs**: open an issue with the bug report template. Include your OS,
  Inertia version, steps to reproduce, and what you expected.
- **Features and larger changes**: open an issue (or a discussion) first, so
  we can agree on the approach before you spend time on it.
- **Security issues**: do not open a public issue. See
  [SECURITY.md](SECURITY.md).

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Setting up

Follow [docs/development.md](docs/development.md) for prerequisites, running
the app, and the test commands. [docs/architecture.md](docs/architecture.md)
explains how the code is organised.

Windows is the primary platform. If you work on macOS or Linux, fixes that
make those platforms work well are especially welcome; say in your pull
request what you tested on.

## Making a change

1. Fork the repository and create a branch from `master`
   (`fix-terminal-resize`, `docs-mcp-examples`, ...).
2. Keep the change focused. One pull request per fix or feature; unrelated
   clean-ups go in their own.
3. Add or update tests for behaviour you change. Rust logic belongs in a crate
   under `src-tauri/crates/` where it can be tested without a window; shared
   JavaScript logic belongs in `src/shared/` with a `*.test.js` beside it.
4. Update the documentation in `docs/` if you change behaviour, a command, a
   file format or a permission.
5. Make sure everything passes:
   ```bash
   bun run test
   bun run build
   cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
   cargo test --workspace --manifest-path src-tauri/Cargo.toml
   ```
   CI runs the same checks on every pull request.
6. If you added or changed dependencies, run `bun run notices` and commit the
   updated `THIRD_PARTY_NOTICES.md`. New dependencies must be under a license
   already accepted in `about.toml` (no GPL/LGPL/AGPL).
7. Open a pull request and fill in the template.

## Commit messages

The history uses short, plain-English sentences in the imperative, without
prefixes or trailing periods, describing the effect of the change:

```
Wait out a bad minute, and let an agent hand the chat on
Put back the function the cleanup took out from under its caller
Add a themed Windows installer and uninstaller
```

The body (optional for small changes) explains why: what was wrong, what the
change does about it, and anything a reviewer should know. Wrap at about 72
characters.

## Code style

- **Rust**: run `cargo fmt`. Keep clippy clean under the workspace lints in
  `src-tauri/Cargo.toml`; avoid `unwrap`/`expect` outside tests. Library crates
  use `thiserror`; nothing under `src-tauri/crates/` may depend on Tauri.
- **JavaScript**: plain JSX and ES modules in `src/`. Match the surrounding
  code.
- **Comments** explain why, not what.
- **Errors shown to people** are complete sentences that say what to do.
- **Remove what nothing uses.** A command no screen calls, or a bridge method
  nothing uses, is deleted rather than kept.

See [docs/development.md](docs/development.md#code-style) for more.

## Licensing

Inertia is MIT licensed. By submitting a contribution you agree that it is
licensed under the [MIT License](LICENSE).

# Releasing

How to cut a release. Releases are built on a Windows machine; there is no
release workflow in `.github/workflows/` yet, so building and uploading are
manual.

## Versioning

Inertia uses [semantic versioning](https://semver.org). The version appears in
several manifests that must all agree:

| File | Field |
|---|---|
| `package.json` | `version` |
| `src-tauri/tauri.conf.json` | `version` (this is what the installer, the About pane and the setup file name use) |
| `src-tauri/Cargo.toml` | `version` under `[workspace.package]` (every crate inherits it) |
| `installer/src-tauri/tauri.conf.json` | `version` |
| `installer/src-tauri/Cargo.toml` | `version` under `[package]` |
| `uninstaller/src-tauri/tauri.conf.json` | `version` |
| `uninstaller/src-tauri/Cargo.toml` | `version` under `[package]` |

The setup bootstrapper reads the product name and version from
`src-tauri/tauri.conf.json` at compile time, so it always matches the app.

The sandbox image has its own version (`tag` and `daytonaSnapshot` in
`src-tauri/crates/inertia-computers/sandbox/image.json`). Bump it only when the
image changes, and say so in the release notes, since Daytona users need a new
snapshot (see [computers.md](computers.md#daytona)).

## Checklist

1. Start from an up-to-date `master` with CI green.
2. Bump the version in every file listed above. Then refresh the lockfiles:
   ```bash
   cargo update --workspace --manifest-path src-tauri/Cargo.toml
   cargo update --workspace --manifest-path installer/src-tauri/Cargo.toml
   cargo update --workspace --manifest-path uninstaller/src-tauri/Cargo.toml
   ```
   (`--workspace` updates only the local packages' entries.)
3. If dependencies changed since the last release, regenerate the notices
   with `bun run notices` and commit `THIRD_PARTY_NOTICES.md` (see
   [third-party-licenses.md](third-party-licenses.md)).
4. Run every check from [development.md](development.md#tests-and-checks).
5. Build the Windows setup:
   ```bash
   bun run build:setup
   ```
   The result is `dist-setup/Inertia-Setup-<version>.exe`.
6. Smoke-test it on a clean Windows user account: install, first-run folder
   selection, add a provider, a chat with a tool call, uninstall. Check that
   **Settings > About** shows the new version.
7. Commit the version bump, tag it and push:
   ```bash
   git tag v<version>
   git push origin master v<version>
   ```
8. Create a GitHub release for the tag, attach
   `Inertia-Setup-<version>.exe`, and write release notes: user-facing
   changes, anything that changes the workspace folder, sandbox image changes,
   and known issues.

Optionally attach the plain NSIS installer from
`src-tauri/target/release/bundle/nsis/` for people who prefer a standard
installer. macOS and Linux bundles from `bun run tauri build` are untested; if
you publish them, label them as such.

## Unsigned builds

Neither the Windows installer nor the macOS bundle is code-signed. Expect:

- **Windows SmartScreen**: "Windows protected your PC". Users choose **More
  info > Run anyway**. Reputation builds slowly per file, so every release
  starts over.
- **macOS Gatekeeper**: refuses a double-click on an unsigned app. Users can
  right-click the app and choose **Open** once, or remove the quarantine flag
  with `xattr -dr com.apple.quarantine /Applications/Inertia.app`.

Say this in the release notes. Signing material (`.pfx`, `.p12`, `.key`,
`.pem`) must never be committed; `.gitignore` excludes those extensions.

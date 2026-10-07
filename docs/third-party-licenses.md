# Third-party licenses

## Inertia's license

Inertia is released under the [MIT License](../LICENSE), copyright Tanvir
Ahamed. That covers all the code in this repository, including the crates under
`src-tauri/crates/`, the installer and the uninstaller.

## Dependencies

Inertia is built on open-source Rust crates and npm packages. Everything that
ships in the app is under a permissive license. Where a dependency offers a
choice (commonly MIT or Apache-2.0), the notices list the first accepted one:

| License | Used by |
|---|---|
| MIT | the large majority of crates and packages |
| Apache-2.0 | a few crates (such as `tao`, `ring`, `dunce`) and npm packages |
| BSD-3-Clause | a handful of crates |
| ISC | a handful of crates and packages |
| Zlib | a couple of crates |
| BSL-1.0 | `clipboard-win`, `error-code` |
| Unicode-3.0 | the ICU4X crates and related (`zerovec`, `unicode-ident`, ...) |
| MPL-2.0 | a few unmodified crates: `cssparser`, `cssparser-macros`, `selectors`, `dtoa-short`, `option-ext` |
| CDLA-Permissive-2.0 | `webpki-roots` (the Mozilla CA certificate list used by rustls) |

The MPL-2.0 crates are used unmodified. MPL-2.0 is a file-level copyleft
license: it applies to those crates' own source files, not to Inertia, and
their source is available from their upstream repositories (linked in the
notices file). Some build tools (for example `lightningcss`, used by Tailwind
CSS during the frontend build) are also MPL-2.0; they run at build time only
and are not shipped.

No dependency is under the GPL, LGPL or AGPL. `about.toml` at the repository
root lists the accepted licenses, and generating the notices fails if a new
dependency is under anything else.

## Bundled fonts

The window bundles these typefaces (via the `@fontsource-variable` npm
packages), all under the [SIL Open Font License 1.1](https://openfontlicense.org):

| Font | Copyright |
|---|---|
| Inter | The Inter Project Authors |
| JetBrains Mono | The JetBrains Mono Project Authors |
| Source Serif 4 | Adobe, with Reserved Font Name "Source" |
| Noto Sans Bengali | The Noto Project Authors |
| Noto Serif Bengali | The Noto Project Authors |
| Anek Bangla | The Anek Project Authors |

The OFL allows the fonts to be bundled with software. Because Source Serif 4
has a Reserved Font Name, a modified version of that font must not be
distributed under the name "Source".

## The full texts

`THIRD_PARTY_NOTICES.md` at the repository root lists every third-party crate,
package and font, grouped by license, with the full license texts and
copyright notices. It is also shipped inside the app's install folder next to
`LICENSE.txt`.

## Regenerating the notices

Regenerate `THIRD_PARTY_NOTICES.md` whenever dependencies change, and before
every release:

```bash
cargo install cargo-about --locked --features cli   # once
bun install
bun run notices
```

`bun run notices` runs `scripts/third-party-notices.mjs`, which:

1. runs `cargo-about` over the three Cargo projects (`src-tauri`,
   `installer/src-tauri`, `uninstaller/src-tauri`), counting only what ends up
   in the binaries (build and dev dependencies are ignored) for the Windows,
   macOS and Linux targets;
2. walks the production npm dependencies (`dependencies` in `package.json`,
   not `devDependencies`) in `node_modules` and collects each package's license
   and notice files;
3. adds the bundled fonts with their copyright lines;
4. merges identical license texts and sorts everything, so re-running on an
   unchanged tree produces no diff.

Commit the regenerated file. Do not edit it by hand.

## What this changes

<!-- What the change does and why. Link the issue it addresses, e.g. "Fixes #123". -->

## How it was tested

<!-- Which platform(s) you ran it on, and what you checked by hand. -->

## Checklist

- [ ] `bun run test` and `bun run build` pass
- [ ] `cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings` is clean
- [ ] `cargo test --workspace --manifest-path src-tauri/Cargo.toml` passes
- [ ] Tests added or updated for changed behaviour
- [ ] Docs in `docs/` updated if behaviour, commands, file formats or permissions changed
- [ ] `bun run notices` run and `THIRD_PARTY_NOTICES.md` committed, if dependencies changed
- [ ] No secrets, personal paths or private data in the diff, screenshots or logs

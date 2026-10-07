//! The sandbox image every machine is made from, and the build context it is
//! made from.
//!
//! One manifest, so Docker builds the same thing a cloud provider is handed a
//! reference to. The values are pinned rather than configurable because they
//! are a contract between the image and the code that drives it: the workdir is
//! where a volume is mounted, and the shell is what `exec` invokes.
//!
//! # Why the context is compiled in rather than bundled as a resource
//!
//! Tauri can ship a folder next to the binary through `bundle.resources`, and
//! that was the other candidate. It loses on the one requirement that matters:
//! the same code path has to work under `bunx tauri dev` and in an installed
//! build. `bundle.resources` only exists after a bundle is produced, so the dev
//! run would need a second path - "look beside the exe, then look up the source
//! tree" - and a two-candidate search is exactly the shape that was already
//! wrong in the Electron app, where the third candidate (`process.cwd()`) was
//! there to paper over the first two. It also puts the Dockerfile outside the
//! binary, so a user who moves the exe alone gets a build that fails with "no
//! Dockerfile" for a reason nobody can see.
//!
//! `include_str!` has neither problem. The bytes are in the binary, so
//! `buildable` is a compile-time fact rather than a filesystem question, an
//! edit to the Dockerfile is picked up by `cargo build` rather than by
//! remembering to re-sync a resource folder, and dev and release are the same
//! code with no branches. `include_str!` over `include_dir` because it needs no
//! dependency added to the workspace table and because the context is a fixed,
//! small list of files that the Dockerfile's own `COPY` lines already name: a
//! file added to `sandbox/` without a line here would be a file the Dockerfile
//! does not copy either.
//!
//! Docker needs a directory to build from, so the embedded bytes are written to
//! a fresh, private temp directory for each build and deleted after it. Not a
//! stable, predictable path: another account on the machine could create
//! `<temp>/<known name>` first, or swap a file in it between the write and the
//! build, and whatever it put there would be built into the image every
//! machine runs. Docker's layer cache keys off the content rather than the
//! path, so a new directory each time costs no rebuild.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The manifest, verbatim. Parsed at runtime rather than at build time so the
/// file stays the one place a version is bumped.
const IMAGE_JSON: &str = include_str!("../sandbox/image.json");

/// Every file the Dockerfile needs to build, as `(file name, bytes)`. Flat on
/// purpose: a name with a slash in it would need splitting into components to
/// be written correctly on Windows.
const CONTEXT: &[(&str, &str)] = &[
    ("Dockerfile", include_str!("../sandbox/Dockerfile")),
    ("image.json", IMAGE_JSON),
    ("start.sh", include_str!("../sandbox/start.sh")),
    (
        "inertia-browser",
        include_str!("../sandbox/inertia-browser"),
    ),
    (
        "inertia-browser.desktop",
        include_str!("../sandbox/inertia-browser.desktop"),
    ),
    ("fluxbox.init", include_str!("../sandbox/fluxbox.init")),
    ("fluxbox.apps", include_str!("../sandbox/fluxbox.apps")),
    ("fluxbox.menu", include_str!("../sandbox/fluxbox.menu")),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: String,
    pub tag: String,
    /// `name:tag`, which is what Docker and a cloud provider both want.
    pub reference: String,
    pub workdir: String,
    /// The account commands run as inside the machine.
    pub user: String,
    /// The program and flag that turn a command line into a process. `-lc`
    /// rather than `-c` so a login shell's PATH is in place - without it a tool
    /// installed by the image's own setup is not on the PATH of the command
    /// that needs it.
    pub shell: Vec<String>,
    /// The file inside the context that Docker is pointed at.
    pub dockerfile: String,
    /// The snapshot a Daytona sandbox is made from when nobody picked one. It
    /// exists only on an account where someone pushed this image under that
    /// name; Daytona's `create` says so plainly when it does not.
    pub daytona_snapshot: String,
}

/// The values a broken or missing `image.json` falls back to.
///
/// A manifest someone broke while editing it must not stop the app from
/// provisioning; these are the shipped values anyway.
const NAME: &str = "inertia-sandbox";
const TAG: &str = "1.1.0";

fn string(parsed: &Value, key: &str, fallback: &str) -> String {
    parsed
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

pub fn manifest() -> Manifest {
    let parsed: Value = serde_json::from_str(IMAGE_JSON).unwrap_or(Value::Null);

    let name = string(&parsed, "name", NAME);
    let tag = string(&parsed, "tag", TAG);
    let shell = parsed
        .get("shell")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|parts| !parts.is_empty())
        .unwrap_or_else(|| vec!["/bin/bash".into(), "-lc".into()]);

    Manifest {
        reference: format!("{name}:{tag}"),
        name,
        tag,
        workdir: string(&parsed, "workdir", "/workspace"),
        user: string(&parsed, "user", "agent"),
        shell,
        dockerfile: string(&parsed, "dockerfile", "Dockerfile"),
        daytona_snapshot: string(&parsed, "daytonaSnapshot", ""),
    }
}

/// Writes the embedded context into a new private temp directory and answers
/// it. The directory is deleted when the answer is dropped, so the caller holds
/// it for exactly as long as the build runs.
pub fn write_context() -> io::Result<tempfile::TempDir> {
    let dir = tempfile::Builder::new()
        .prefix("inertia-sandbox-context-")
        .tempdir()?;
    write_context_into(dir.path())?;
    Ok(dir)
}

/// The same, into a directory the caller names. Separate so a test can write
/// somewhere it owns rather than into the real temp folder.
pub fn write_context_into(dir: &Path) -> io::Result<()> {
    for (name, body) in CONTEXT {
        std::fs::write(dir.join(name), body.as_bytes())?;
    }
    Ok(())
}

/// What the settings screen shows about the image.
///
/// `buildable` is unconditionally true, and that is the point of compiling the
/// context in: there is no filesystem state that can make it false, so the
/// screen cannot show "No Dockerfile found" on a correctly installed app.
pub fn info() -> Value {
    let manifest = manifest();
    serde_json::json!({
        "ref": manifest.reference,
        "workdir": manifest.workdir,
        "buildable": true,
        "daytonaSnapshot": manifest.daytona_snapshot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference is the tag a new machine is made from, and a bump is what
    /// makes every install build the changed image instead of reusing the old
    /// one. Machines that already exist keep the image they were made from.
    #[test]
    fn the_reference_is_the_shipped_tag() {
        assert_eq!(manifest().reference, "inertia-sandbox:1.1.0");
        assert_eq!(manifest().workdir, "/workspace");
    }

    #[test]
    fn the_shell_is_a_login_shell() {
        assert_eq!(manifest().shell, ["/bin/bash", "-lc"]);
    }

    /// The manifest is read from the shipped `image.json` rather than from
    /// constants beside it, so bumping a version is one line in one file.
    #[test]
    fn the_manifest_comes_from_the_shipped_json() {
        let manifest = manifest();
        assert_eq!(manifest.user, "agent");
        assert_eq!(manifest.dockerfile, "Dockerfile");
        assert_eq!(manifest.daytona_snapshot, "inertia-sandbox-1.1.0");
    }

    /// The whole reason for embedding: the app can always answer "yes, there is
    /// something to build from", with no folder to find.
    #[test]
    fn the_image_is_always_buildable() {
        let info = info();
        assert_eq!(info["buildable"], serde_json::json!(true));
        assert_eq!(info["ref"], serde_json::json!("inertia-sandbox:1.1.0"));
    }

    #[test]
    fn the_dockerfile_and_everything_it_copies_are_embedded() {
        let names: Vec<&str> = CONTEXT.iter().map(|(path, _)| *path).collect();
        for needed in [
            "Dockerfile",
            "start.sh",
            "inertia-browser",
            "inertia-browser.desktop",
            "fluxbox.init",
            "fluxbox.apps",
            "fluxbox.menu",
        ] {
            assert!(names.contains(&needed), "{needed} is not in the context");
        }
        assert!(CONTEXT.iter().all(|(_, body)| !body.is_empty()));
    }

    /// The base is pinned by digest, so a re-pointed tag upstream cannot change
    /// what is built; and nothing is piped from the network into a shell.
    #[test]
    fn the_dockerfile_builds_only_from_pinned_bytes() {
        let dockerfile = CONTEXT[0].1;
        assert!(dockerfile.contains("FROM debian:bookworm-slim@sha256:"));
        assert!(dockerfile.contains("sha256sum -c"));
        assert!(!dockerfile.contains("| bash"));
    }

    /// Every build gets a new directory of its own, and it is gone afterwards.
    /// A shared, predictable path in temp is one another account can plant a
    /// Dockerfile in before the build reads it.
    #[test]
    fn each_build_context_is_private_and_removed_after() {
        let first = write_context().unwrap();
        let second = write_context().unwrap();
        assert_ne!(first.path(), second.path());

        let text = std::fs::read_to_string(first.path().join("Dockerfile")).unwrap();
        assert!(text.contains("FROM debian:bookworm-slim"));
        assert!(first.path().join("start.sh").is_file());

        let gone = first.path().to_path_buf();
        drop(first);
        assert!(!gone.exists());
    }
}

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
//! one under the system temp folder, keyed by the image tag. A stable path
//! rather than a fresh temp dir per build: the build context is the same bytes
//! every time, Docker's layer cache keys off the content and not the path, and
//! a stable path is one a person reading "Buildable" on the settings screen can
//! actually go and look at.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The manifest, verbatim. Parsed at runtime rather than at build time so the
/// file stays the one place a version is bumped.
const IMAGE_JSON: &str = include_str!("../sandbox/image.json");

/// Every file the Dockerfile needs to build, as `(relative path, bytes)`.
///
/// The relative path uses forward slashes and is split component-wise when it
/// is written, because a `PathBuf` built from a string with an embedded slash
/// compares unequal to the same path built properly on Windows.
const CONTEXT: &[(&str, &str)] = &[
    ("Dockerfile", include_str!("../sandbox/Dockerfile")),
    ("image.json", IMAGE_JSON),
    ("start.sh", include_str!("../sandbox/start.sh")),
    ("inertia-browser", include_str!("../sandbox/inertia-browser")),
    (
        "inertia-browser.desktop",
        include_str!("../sandbox/inertia-browser.desktop"),
    ),
    ("fluxbox.init", include_str!("../sandbox/fluxbox.init")),
    ("fluxbox.apps", include_str!("../sandbox/fluxbox.apps")),
    ("fluxbox.menu", include_str!("../sandbox/fluxbox.menu")),
    (
        "bootstrap/setup.sh",
        include_str!("../sandbox/bootstrap/setup.sh"),
    ),
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
    /// What a cloud provider that cannot see this laptop falls back to.
    pub daytona_fallback_image: String,
    /// Where the built image is published, for a service that cannot see this
    /// laptop. Empty when nobody has pushed it.
    pub registry: String,
    /// The name that image is registered under in Daytona, which is not the
    /// same string as the registry reference.
    pub daytona_snapshot: String,
}

/// The values a broken or missing `image.json` falls back to.
///
/// A manifest someone broke while editing it must not stop the app from
/// provisioning; these are the shipped values anyway.
const NAME: &str = "inertia-sandbox";
const TAG: &str = "1.0.0";

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
        daytona_fallback_image: string(&parsed, "daytonaFallbackImage", "debian:12-slim"),
        registry: string(&parsed, "registry", ""),
        daytona_snapshot: string(&parsed, "daytonaSnapshot", ""),
    }
}

/// The bootstrap script's text, for a provider handed a bare base image.
pub fn bootstrap_script() -> Option<&'static str> {
    CONTEXT
        .iter()
        .find(|(path, _)| *path == "bootstrap/setup.sh")
        .map(|(_, body)| *body)
}

/// Where the build context would be written.
///
/// Keyed by the reference, so bumping the tag cannot reuse a folder holding the
/// previous version's Dockerfile - which would build the new tag from the old
/// recipe and be invisible until something inside the machine was missing.
pub fn context_dir() -> PathBuf {
    let manifest = manifest();
    let mut dir = std::env::temp_dir();
    dir.push("inertia-sandbox-context");
    dir.push(format!("{}-{}", manifest.name, manifest.tag));
    dir
}

/// Writes the embedded context to disk and answers where it is.
///
/// Rewritten every call rather than written once and trusted. It is nine small
/// files, and the alternative is a stale context surviving in temp after an
/// edit to the Dockerfile - a build that silently produces last week's image is
/// far more expensive than the milliseconds this costs.
pub fn write_context() -> io::Result<PathBuf> {
    let dir = context_dir();
    write_context_into(&dir)?;
    Ok(dir)
}

/// The same, into a directory the caller names. Separate so a test can write
/// somewhere it owns rather than into the real temp folder.
pub fn write_context_into(dir: &Path) -> io::Result<()> {
    for (relative, body) in CONTEXT {
        let mut file = dir.to_path_buf();
        // One component at a time. A `PathBuf` built by joining a string with
        // an embedded slash keeps the slash inside the component on Windows,
        // and then compares unequal to the same path built properly.
        for part in relative.split('/') {
            file.push(part);
        }
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file, body.as_bytes())?;
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
        "dir": context_dir().display().to_string(),
        "fallback": manifest.daytona_fallback_image,
        "registry": manifest.registry,
        "daytonaSnapshot": manifest.daytona_snapshot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference is what an existing workspace's records already carry -
    /// the machine in a real workspace was built from `inertia-sandbox:1.0.0`,
    /// and changing this orphans it.
    #[test]
    fn the_reference_is_stable() {
        assert_eq!(manifest().reference, "inertia-sandbox:1.0.0");
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
        assert_eq!(manifest.daytona_fallback_image, "debian:12-slim");
        assert!(manifest.registry.contains("inertia-sandbox"));
    }

    /// The whole reason for embedding: the app can always answer "yes, there is
    /// something to build from", with no folder to find.
    #[test]
    fn the_image_is_always_buildable() {
        let info = info();
        assert_eq!(info["buildable"], serde_json::json!(true));
        assert_eq!(info["ref"], serde_json::json!("inertia-sandbox:1.0.0"));
        assert!(info["dir"].as_str().is_some_and(|dir| !dir.is_empty()));
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

    #[test]
    fn writing_the_context_produces_a_directory_docker_can_build() {
        let dir = tempfile::tempdir().unwrap();
        write_context_into(dir.path()).unwrap();

        let mut dockerfile = dir.path().to_path_buf();
        dockerfile.push("Dockerfile");
        let text = std::fs::read_to_string(&dockerfile).unwrap();
        assert!(text.contains("FROM debian:bookworm-slim"));

        // The nested path is written as a real subdirectory, not as a file
        // called "bootstrap/setup.sh".
        let mut setup = dir.path().to_path_buf();
        setup.push("bootstrap");
        setup.push("setup.sh");
        assert!(setup.is_file());
    }

    #[test]
    fn the_bootstrap_script_is_there_for_a_bare_base_image() {
        assert!(bootstrap_script().is_some_and(|text| !text.trim().is_empty()));
    }

    /// A context folder per tag. Sharing one would build a new tag from the
    /// previous version's Dockerfile, and nothing would say so.
    #[test]
    fn the_context_folder_is_keyed_by_the_tag() {
        let dir = context_dir().display().to_string();
        assert!(dir.contains("inertia-sandbox-1.0.0"));
    }
}

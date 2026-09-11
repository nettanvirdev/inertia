use std::path::{Path, PathBuf};

/// The app's own config is the single source of truth for the product name,
/// version and binary name. Reading them here rather than restating them in
/// the installer means a rename in `src-tauri/tauri.conf.json` cannot leave
/// the installer writing to the wrong folder or launching a binary that no
/// longer exists - it fails the build instead.
fn main() {
    tauri_build::build();

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let app = manifest.join("../../src-tauri");

    let conf_path = app.join("tauri.conf.json");
    println!("cargo:rerun-if-changed={}", conf_path.display());
    let conf: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&conf_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", conf_path.display())),
    )
    .expect("app tauri.conf.json is not valid JSON");

    let product = conf["productName"].as_str().expect("productName missing");
    let version = conf["version"].as_str().expect("version missing");

    // `mainBinaryName` is optional; Tauri falls back to the Cargo package name.
    let binary = match conf["mainBinaryName"].as_str() {
        Some(name) => name.to_string(),
        None => app_package_name(&app.join("Cargo.toml")),
    };

    println!("cargo:rustc-env=APP_PRODUCT_NAME={product}");
    println!("cargo:rustc-env=APP_VERSION={version}");
    println!("cargo:rustc-env=APP_BINARY_NAME={binary}");

    // `include_bytes!` needs the file to exist at compile time, and it will
    // not until the app has been bundled. An empty placeholder keeps `cargo
    // check` working on a clean clone; `run_install` refuses to run on one.
    let payload = manifest.join("resources/payload.exe");
    println!("cargo:rerun-if-changed={}", payload.display());
    if !payload.exists() {
        std::fs::create_dir_all(payload.parent().unwrap()).unwrap();
        std::fs::write(&payload, b"").unwrap();
    }
}

/// A deliberately small scan rather than a TOML dependency: the one field
/// needed is the first `name =` after `[package]`.
fn app_package_name(cargo_toml: &Path) -> String {
    let text = std::fs::read_to_string(cargo_toml).expect("cannot read the app's Cargo.toml");
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package {
            if let Some(rest) = line.strip_prefix("name") {
                if let Some(value) = rest.trim_start().strip_prefix('=') {
                    return value.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    panic!("no [package] name in the app's Cargo.toml");
}

use std::path::{Path, PathBuf};

/// Same contract as the installer's build script: the app's own config is the
/// single source of truth, so a rename cannot leave the uninstaller deleting
/// the wrong folder - it fails the build instead.
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
    let identifier = conf["identifier"].as_str().expect("identifier missing");

    let binary = match conf["mainBinaryName"].as_str() {
        Some(name) => name.to_string(),
        None => app_package_name(&app.join("Cargo.toml")),
    };

    println!("cargo:rustc-env=APP_PRODUCT_NAME={product}");
    println!("cargo:rustc-env=APP_VERSION={version}");
    println!("cargo:rustc-env=APP_IDENTIFIER={identifier}");
    println!("cargo:rustc-env=APP_BINARY_NAME={binary}");
}

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

fn main() {
    // The compiler that built this binary, asked at the moment it is built.
    //
    // There is no runtime API for it - a Rust binary does not carry its
    // toolchain - so the About pane would otherwise have to either leave the row
    // empty or print the version this crate was developed against, which is a
    // different thing and would go stale without anyone noticing. `rustc -V`
    // here is the only answer that is true of the binary a person is actually
    // running. If it cannot be run, the variable is empty and the pane draws a
    // dash, which is what it does for every value it does not have.
    let rustc = std::process::Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
        .arg("-V")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=INERTIA_RUSTC={rustc}");

    tauri_build::build()
}

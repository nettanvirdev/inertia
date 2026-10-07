// No console window behind the installer in release, same as the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!(
    "The setup bootstrapper is Windows-only: it drives the NSIS payload. \
     macOS ships the .dmg that `bun run tauri build` already produces."
);

use serde::Serialize;
use tauri::{Emitter, Manager};

/// The app's real installer, baked into this binary at compile time. Embedding
/// beats shipping it as a Tauri resource next to the exe: what people download
/// has to be one self-contained file, and a resource sitting beside it is one
/// more thing to lose between here and a machine.
const PAYLOAD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/payload.exe"
));

const PRODUCT_NAME: &str = env!("APP_PRODUCT_NAME");
const APP_VERSION: &str = env!("APP_VERSION");
const BINARY_NAME: &str = env!("APP_BINARY_NAME");

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SetupInfo {
    product_name: String,
    version: String,
    default_dir: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    /// "preparing" | "installing" | "finishing" | "done"
    phase: String,
    message: String,
}

fn emit(app: &tauri::AppHandle, phase: &str, message: &str) {
    let _ = app.emit(
        "install://progress",
        Progress {
            phase: phase.to_string(),
            message: message.to_string(),
        },
    );
}

/// Matches what the NSIS payload picks on its own in `currentUser` mode:
/// `$LOCALAPPDATA\<PRODUCTNAME>`. Passing it explicitly anyway keeps the path
/// shown on screen and the path installed to the same string.
fn default_install_dir() -> String {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
        std::env::var("USERPROFILE")
            .map_or_else(|_| "C:\\".to_string(), |p| format!("{p}\\AppData\\Local"))
    });
    format!("{base}\\{PRODUCT_NAME}")
}

#[tauri::command]
fn setup_info() -> SetupInfo {
    SetupInfo {
        product_name: PRODUCT_NAME.to_string(),
        version: APP_VERSION.to_string(),
        default_dir: default_install_dir(),
    }
}

/// A folder picker returns a container, not an install directory - nobody
/// means "put the binaries loose in Documents". Append the product name
/// unless the chosen folder is already named for it, which is what happens
/// when someone navigates to a previous install.
#[tauri::command]
fn resolve_install_dir(picked: String) -> String {
    let path = std::path::Path::new(picked.trim_end_matches(['\\', '/']));
    let already_named = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(PRODUCT_NAME));

    if already_named {
        path.display().to_string()
    } else {
        path.join(PRODUCT_NAME).display().to_string()
    }
}

#[tauri::command]
async fn run_install(app: tauri::AppHandle, dir: String, shortcuts: bool) -> Result<(), String> {
    // The whole install is blocking process work. Run it off the async runtime
    // so the webview keeps painting the progress it is being sent.
    tauri::async_runtime::spawn_blocking(move || install(&app, &dir, shortcuts))
        .await
        .map_err(|e| format!("the installer task stopped unexpectedly: {e}"))?
}

fn install(app: &tauri::AppHandle, dir: &str, shortcuts: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    if PAYLOAD.is_empty() {
        return Err(
            "This build has no installer payload. Run `bun run build:setup`, which bundles the \
             app first and embeds the result here."
                .to_string(),
        );
    }

    emit(app, "preparing", "Unpacking");
    let payload_path =
        std::env::temp_dir().join(format!("{PRODUCT_NAME}-payload-{}.exe", std::process::id()));
    std::fs::write(&payload_path, PAYLOAD)
        .map_err(|e| format!("could not write the payload to a temporary file: {e}"))?;

    emit(app, "installing", "Installing files");

    // `/D=` is not a normal argument: NSIS takes the rest of the command line
    // literally, so the path must be unquoted, last, and without a trailing
    // separator. `Command::arg` would quote any path containing a space - and
    // NSIS would then install to its own default location rather than fail
    // visibly. `raw_arg` appends the text verbatim, which is what it needs.
    let mut command = std::process::Command::new(&payload_path);
    command.arg("/S");
    if !shortcuts {
        command.arg("/NS");
    }
    command.raw_arg(format!("/D={}", dir.trim_end_matches(['\\', '/'])));

    let status = command
        .status()
        .map_err(|e| format!("could not start the installer: {e}"))?;

    emit(app, "finishing", "Cleaning up");
    let _ = std::fs::remove_file(&payload_path);

    if !status.success() {
        // NSIS uses 1 for a cancelled install and 2 for an error.
        return Err(match status.code() {
            Some(1) => "The installation was cancelled.".to_string(),
            Some(code) => format!("The installer exited with code {code}."),
            None => "The installer was terminated.".to_string(),
        });
    }

    emit(app, "done", "Installed");
    Ok(())
}

#[tauri::command]
fn launch_app(app: tauri::AppHandle, dir: String) -> Result<(), String> {
    let exe = std::path::Path::new(&dir).join(format!("{BINARY_NAME}.exe"));
    std::process::Command::new(&exe)
        .current_dir(&dir)
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))?;
    app.exit(0);
    Ok(())
}

// Same compositor call the app makes, for the same reason: this window is
// frameless, and a frameless window does not always get the system's rounded
// corner. Copied rather than shared - linking the app's library into the
// installer to reuse twenty lines would pull the app in with it.
fn apply_rounded_corners(window: &tauri::WebviewWindow) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };

    let Ok(handle) = window.hwnd() else { return };
    let hwnd = windows::Win32::Foundation::HWND(handle.0);
    let preference = DWMWCP_ROUND;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const _ as *const _,
            std::mem::size_of_val(&preference) as u32,
        );
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            setup_info,
            resolve_install_dir,
            run_install,
            launch_app
        ])
        .setup(|app| {
            for (_, window) in app.webview_windows() {
                apply_rounded_corners(&window);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the installer");
}

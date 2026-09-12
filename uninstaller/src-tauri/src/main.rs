// No console window behind the uninstaller in release, same as the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("The themed uninstaller is Windows-only; macOS uninstalls by dragging to Trash.");

use serde::Serialize;
use tauri::{Emitter, Manager};

const PRODUCT_NAME: &str = env!("APP_PRODUCT_NAME");
const APP_VERSION: &str = env!("APP_VERSION");
const APP_IDENTIFIER: &str = env!("APP_IDENTIFIER");

/// Set on the relaunched copy, carrying the directory to remove. Its presence
/// is also how the process knows it is already running from the temp copy.
const RELAUNCH_FLAG: &str = "--uninstall-from";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UninstallInfo {
    product_name: String,
    version: String,
    install_dir: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
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

/// The directory being removed. On the relaunched copy this comes from the
/// command line, because by then the executable lives in %TEMP% and its own
/// path says nothing about where the app was installed.
fn install_dir() -> std::path::PathBuf {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == RELAUNCH_FLAG {
            if let Some(dir) = args.next() {
                return std::path::PathBuf::from(dir);
            }
        }
    }
    // Located by looking for the NSIS uninstaller rather than by assuming a
    // fixed depth. This binary is mapped to the install root today, but it
    // shipped under a resources/ subdirectory first - and that one level of
    // difference silently pointed the whole uninstall at the wrong folder.
    // Walking up until uninstall.exe turns up is immune to that moving again.
    let exe = std::env::current_exe().unwrap_or_default();
    let mut dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for _ in 0..3 {
        if dir.join("uninstall.exe").exists() {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => break,
        }
    }
    exe.parent().map(|p| p.to_path_buf()).unwrap_or_default()
}

fn running_from_temp() -> bool {
    std::env::args().any(|a| a == RELAUNCH_FLAG)
}

/// Windows will not delete a running executable, and this one sits inside the
/// directory it has to remove. So the first thing it does is copy itself to
/// %TEMP%, hand the install path to the copy, and exit - leaving nothing of
/// itself behind to block `RMDir /r`. Returns true if a relaunch happened and
/// this process should quit.
fn relaunch_from_temp() -> bool {
    if running_from_temp() {
        return false;
    }

    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    // The resolved install root, not this executable's own directory - those
    // are only the same while the binary sits in the install root, and handing
    // the temp copy the wrong path points the entire uninstall at it.
    let dir = install_dir();

    let temp = std::env::temp_dir().join(format!("{PRODUCT_NAME}-uninstall-{}.exe", std::process::id()));
    if std::fs::copy(&exe, &temp).is_err() {
        // Could not stage a copy - carry on in place rather than refusing to
        // uninstall at all. The directory removal will leave this one file
        // behind, which is better than leaving all of them.
        return false;
    }

    match std::process::Command::new(&temp)
        .arg(RELAUNCH_FLAG)
        .arg(dir)
        .spawn()
    {
        Ok(_) => true,
        Err(_) => false,
    }
}

/// Take the app out of `HKCU\...\Run`, if it put itself there.
///
/// Only when the value points inside the folder being removed. The name is the
/// product's, and something else on this machine may legitimately own an entry
/// by that name; deleting it because the strings matched would be this
/// uninstaller breaking an application it has nothing to do with.
#[cfg(windows)]
fn forget_autostart(dir: &std::path::Path) {
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    let Ok(query) = std::process::Command::new("reg")
        .args(["query", RUN_KEY, "/v", PRODUCT_NAME])
        .output()
    else {
        return;
    };
    if !query.status.success() {
        // No entry, which is the normal case: the setting is off by default.
        return;
    }

    // `reg query` prints the value's data on the same line as its name. A
    // case-insensitive compare because the registry stores whatever spelling
    // was written and Windows paths do not care.
    let printed = String::from_utf8_lossy(&query.stdout).to_lowercase();
    let ours = dir.display().to_string().to_lowercase();
    if !printed.contains(&ours) {
        return;
    }

    let _ = std::process::Command::new("reg")
        .args(["delete", RUN_KEY, "/v", PRODUCT_NAME, "/f"])
        .output();
}

/// Nothing to forget anywhere else: the Run key is Windows' own idea.
#[cfg(not(windows))]
fn forget_autostart(_dir: &std::path::Path) {}

#[tauri::command]
fn app_mode() -> &'static str {
    "uninstall"
}

#[tauri::command]
fn uninstall_info() -> UninstallInfo {
    UninstallInfo {
        product_name: PRODUCT_NAME.to_string(),
        version: APP_VERSION.to_string(),
        install_dir: install_dir().display().to_string(),
    }
}

#[tauri::command]
async fn run_uninstall(app: tauri::AppHandle, wipe_settings: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || uninstall(&app, wipe_settings))
        .await
        .map_err(|e| format!("the uninstall task stopped unexpectedly: {e}"))?
}

fn uninstall(app: &tauri::AppHandle, wipe_settings: bool) -> Result<(), String> {
    let dir = install_dir();
    let uninstaller = dir.join("uninstall.exe");

    if !uninstaller.exists() {
        return Err(format!(
            "Could not find the uninstaller at {}. {PRODUCT_NAME} may already be removed.",
            uninstaller.display()
        ));
    }

    emit(app, "preparing", "Stopping the app");
    // The app cannot be holding its own files open while they are deleted.
    // Failure here is not fatal: it usually just means it was not running.
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/IM", &format!("{}.exe", env!("APP_BINARY_NAME"))])
        .output();

    emit(app, "installing", "Removing files");
    // `_?=` tells the NSIS uninstaller not to copy itself to temp and detach,
    // which is what makes `.status()` actually wait for it to finish rather
    // than returning the moment the copy is spawned.
    let status = std::process::Command::new(&uninstaller)
        .arg("/S")
        .arg(format!("_?={}", dir.display()))
        .status()
        .map_err(|e| format!("could not start the uninstaller: {e}"))?;

    if !status.success() {
        return Err(match status.code() {
            Some(code) => format!("The uninstaller exited with code {code}."),
            None => "The uninstaller was terminated.".to_string(),
        });
    }

    emit(app, "finishing", "Cleaning up");

    // With `_?=` the NSIS uninstaller leaves its own exe and the directory
    // behind on purpose, so finish that off here.
    let _ = std::fs::remove_file(dir.join("uninstall.exe"));
    let _ = std::fs::remove_dir_all(&dir);

    // The Run key, if the app was ever set to start with Windows.
    //
    // The fourth of the rules in the README: one owner, one mechanism, the
    // registry is the state, and the uninstaller takes it away. Without this
    // last one an uninstalled app leaves an entry pointing at a path that no
    // longer exists, and Windows tries to launch it at every login.
    forget_autostart(&dir);

    if wipe_settings {
        // Where `app_config_dir()` resolves for the installed app: Tauri uses
        // the bundle identifier under %APPDATA% on Windows.
        if let Ok(roaming) = std::env::var("APPDATA") {
            let config = std::path::Path::new(&roaming).join(APP_IDENTIFIER);
            let _ = std::fs::remove_dir_all(config);
        }
    }

    emit(app, "done", "Removed");
    Ok(())
}

#[tauri::command]
fn close_uninstaller(app: tauri::AppHandle) {
    app.exit(0);
}

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
    // Before any window exists: get out of the directory we are about to
    // delete. The relaunched copy falls through and runs normally.
    if relaunch_from_temp() {
        return;
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            app_mode,
            uninstall_info,
            run_uninstall,
            close_uninstaller
        ])
        .setup(|app| {
            for (_, window) in app.webview_windows() {
                apply_rounded_corners(&window);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the uninstaller");
}

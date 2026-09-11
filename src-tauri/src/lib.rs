// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// Tauri returns an HWND from whatever version of the `windows` crate IT
// depends on, which need not be the version this crate depends on - and two
// copies of that crate in one dependency graph make those two HWNDs distinct
// types that will not substitute for each other. A window handle is a bare
// pointer in every version, so rebuild it in ours. Matching the two versions
// by hand instead would only move the breakage to whichever side bumps first.
#[cfg(target_os = "windows")]
fn hwnd_of(window: &tauri::WebviewWindow) -> Option<windows::Win32::Foundation::HWND> {
    Some(windows::Win32::Foundation::HWND(window.hwnd().ok()?.0))
}

// Windows 11 rounds top-level windows automatically, but that default can be
// disabled by group policy or a non-standard window setup (e.g. borderless
// windows). Setting DWMWA_WINDOW_CORNER_PREFERENCE explicitly guarantees
// rounded corners regardless. Call this for every window this app creates,
// including future popups/dialogs, right after they're built.
#[cfg(target_os = "windows")]
pub fn apply_rounded_corners(window: &tauri::WebviewWindow) {
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };

    if let Some(hwnd) = hwnd_of(window) {
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
}

#[cfg(not(target_os = "windows"))]
pub fn apply_rounded_corners(_window: &tauri::WebviewWindow) {}

// The window is frameless, so maximize/restore is animated entirely from the
// frontend by tweening setSize/setPosition rather than calling the native
// maximize (which would snap instantly and can't be interrupted mid-tween).
// That animation needs the true usable area of the monitor - excluding the
// taskbar - which Tauri's own JS API does not expose, so this reaches for it
// directly via Win32.
#[cfg(target_os = "windows")]
#[tauri::command]
fn get_monitor_work_area(window: tauri::WebviewWindow) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };

    let hwnd = hwnd_of(&window)?;
    unsafe {
        let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmonitor, &mut info).as_bool() {
            let rc = info.rcWork;
            Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
        } else {
            None
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
fn get_monitor_work_area(_window: tauri::WebviewWindow) -> Option<(i32, i32, i32, i32)> {
    None
}


// ── preferences ─────────────────────────────────────────────────────────────
// A single small JSON file in the OS config dir, written whole on every
// change. Deliberately not the store plugin: two scalars and a first-run flag
// do not need a keyed database, a migration story, or another permission set
// in `capabilities/`, and the onboarding flow has to be able to read this
// before the first frame paints.

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Preferences {
    /// False until the walkthrough is finished or skipped; `App` renders the
    /// walkthrough instead of the app while it is.
    pub onboarding_completed: bool,
    /// "system" | "light" | "dark". "system" follows the OS preference live;
    /// the default is "dark" rather than "system" because the app is designed
    /// dark-first and the installer that precedes it is dark unconditionally.
    pub theme: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            onboarding_completed: false,
            theme: "dark".to_string(),
        }
    }
}

fn preferences_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("preferences.json"))
}

// Any failure here - no config dir, unreadable file, JSON written by an older
// build with a field this one no longer understands - resolves to defaults
// rather than an error. The cost of being wrong is showing the walkthrough a
// second time; the cost of propagating the error is an app that will not open.
#[tauri::command]
fn load_preferences(app: tauri::AppHandle) -> Preferences {
    preferences_path(&app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn save_preferences(app: tauri::AppHandle, preferences: Preferences) -> Result<(), String> {
    let path = preferences_path(&app)?;
    let json = serde_json::to_string_pretty(&preferences).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            get_monitor_work_area,
            load_preferences,
            save_preferences
        ])
        .setup(|app| {
            for (_, window) in tauri::Manager::webview_windows(app) {
                apply_rounded_corners(&window);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

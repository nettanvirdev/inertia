//! Native window chrome: rounded corners.
//!
//! The window is frameless (`decorations: false`), which means the app draws
//! its own title bar and therefore owns behaviour the OS would normally
//! provide. This is the cross-platform seam for the one such behaviour Tauri
//! does not cover: the Windows implementation reaches for Win32 and every
//! other platform gets a defensible fallback.
//!
//! Moving, resizing and maximising are Tauri's own window API, driven from
//! `bridge/app.js`. They were commands here once, tweening the window rect a
//! frame at a time, back when the title bar was Electron chrome that could not
//! reach the window itself.

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

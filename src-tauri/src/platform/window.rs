//! Native window chrome: rounded corners, the monitor work area, and the
//! maximize/restore animation.
//!
//! The window is frameless (`decorations: false`), which means the app draws
//! its own title bar and therefore owns behaviour the OS would normally
//! provide. Each function here is the cross-platform seam for one such
//! behaviour; the Windows implementation reaches for Win32 and every other
//! platform gets a defensible fallback.

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

// The window is frameless, so maximize/restore is animated by tweening the
// window rect rather than calling the native maximize (which would snap
// instantly and can't be interrupted mid-tween). That animation needs the true
// usable area of the monitor - excluding the taskbar - which Tauri's own JS
// API does not expose, so this reaches for it directly via Win32.
#[cfg(target_os = "windows")]
#[tauri::command]
pub fn get_monitor_work_area(window: tauri::WebviewWindow) -> Option<(i32, i32, i32, i32)> {
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
pub fn get_monitor_work_area(_window: tauri::WebviewWindow) -> Option<(i32, i32, i32, i32)> {
    None
}

// ── the maximize / restore animation ────────────────────────────────────────
// This used to live in the frontend, tweening with requestAnimationFrame and
// calling setPosition + setSize over IPC every frame. Three things made that
// visibly rough, and all three are gone by doing it here:
//
//   - setPosition and setSize are two separate SetWindowPos calls, so every
//     single frame the window existed for an instant at its new origin with
//     its old size. That shear is the flicker.
//   - sixty IPC round-trips a second, each awaited, means the frame interval
//     is however long the round-trip took rather than a steady 8ms.
//   - the tween finished by calling the native maximize(), which re-snapped
//     the window to the OS's own maximized rect - a few pixels off the work
//     area we had just animated to. That is the jump right at the end.
//
// So: one atomic SetWindowPos per frame, from a thread, and no native
// maximize at all. "Maximized" here means "occupying the work area", which is
// what it looked like anyway, and nothing re-snaps when the tween lands.
#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn animate_window_to(
    window: tauri::WebviewWindow,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    duration_ms: u64,
) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER};

    let hwnd = hwnd_of(&window).ok_or("no window handle")?;
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;

    let (x0, y0) = (position.x, position.y);
    let (w0, h0) = (size.width as i32, size.height as i32);

    // HWND is a raw pointer and therefore not Send, so carry it across the
    // thread boundary as an integer and rebuild it there.
    let raw = hwnd.0 as isize;

    tauri::async_runtime::spawn_blocking(move || {
        let hwnd = windows::Win32::Foundation::HWND(raw as *mut std::ffi::c_void);
        let started = std::time::Instant::now();
        let duration = duration_ms.max(1) as f32;

        loop {
            let t = (started.elapsed().as_millis() as f32 / duration).min(1.0);
            // Matches --ease-out in globals.css: fast off the trigger, settles
            // without overshoot. Kept identical so window motion and in-app
            // motion read as the same system.
            let e = 1.0 - (1.0 - t).powi(3);

            let cx = x0 + ((x - x0) as f32 * e).round() as i32;
            let cy = y0 + ((y - y0) as f32 * e).round() as i32;
            let cw = w0 + ((width - w0) as f32 * e).round() as i32;
            let ch = h0 + ((height - h0) as f32 * e).round() as i32;

            unsafe {
                let _ = SetWindowPos(hwnd, None, cx, cy, cw, ch, SWP_NOZORDER | SWP_NOACTIVATE);
            }

            if t >= 1.0 {
                break;
            }
            // ~120Hz. Finer than the compositor needs, but the cost is a few
            // extra SetWindowPos calls rather than a few extra IPC hops.
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(not(target_os = "windows"))]
#[tauri::command]
pub async fn animate_window_to(
    window: tauri::WebviewWindow,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    _duration_ms: u64,
) -> Result<(), String> {
    // No smooth path off Windows; land on the target in one step.
    use tauri::{PhysicalPosition, PhysicalSize};
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    window
        .set_size(PhysicalSize::new(width as u32, height as u32))
        .map_err(|e| e.to_string())
}

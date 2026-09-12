//! First-run and appearance preferences.
//!
//! A single small JSON file in the OS config dir, written whole on every
//! change. Deliberately not the store plugin: two scalars and a first-run flag
//! do not need a keyed database, a migration story, or another permission set
//! in `capabilities/`, and the onboarding flow has to be able to read this
//! before the first frame paints.
//!
//! This is distinct from workspace settings, which live in the workspace
//! directory and are owned by `inertia-store`. The split is deliberate: these
//! are the few facts the app needs before it knows which workspace to open.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
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
pub fn load_preferences(app: tauri::AppHandle) -> Preferences {
    preferences_path(&app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn save_preferences(app: tauri::AppHandle, preferences: Preferences) -> Result<(), String> {
    let path = preferences_path(&app)?;
    let json = serde_json::to_string_pretty(&preferences).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

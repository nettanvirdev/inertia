//! Where the browsers on this computer keep their cookies.
//!
//! A browser is "installed" here if its folder is on disk with a cookie file in
//! it. Not if it is registered, not if it is the default: a browser that was
//! uninstalled leaves its registration behind more often than it leaves its
//! profile, and what this needs is the file.

use std::path::{Path, PathBuf};

use crate::Source;

/// A Chromium-family browser, by where its user-data folder is per platform.
struct Chromium {
    id: &'static str,
    label: &'static str,
    /// Under `%LOCALAPPDATA%`.
    win: &'static str,
    /// Under `%APPDATA%` instead, for the ones that live there.
    win_roaming: &'static str,
    /// Under `~/Library/Application Support`.
    mac: &'static str,
    /// Under `$XDG_CONFIG_HOME`.
    linux: &'static str,
}

/// Opera is the odd one: its folder is a profile rather than a folder of
/// profiles, which the scan handles by treating the root as a candidate too.
const CHROMIUM: &[Chromium] = &[
    Chromium { id: "chrome", label: "Google Chrome", win: "Google/Chrome/User Data", win_roaming: "", mac: "Google/Chrome", linux: "google-chrome" },
    Chromium { id: "chrome-beta", label: "Chrome Beta", win: "Google/Chrome Beta/User Data", win_roaming: "", mac: "Google/Chrome Beta", linux: "google-chrome-beta" },
    Chromium { id: "edge", label: "Microsoft Edge", win: "Microsoft/Edge/User Data", win_roaming: "", mac: "Microsoft Edge", linux: "microsoft-edge" },
    Chromium { id: "brave", label: "Brave", win: "BraveSoftware/Brave-Browser/User Data", win_roaming: "", mac: "BraveSoftware/Brave-Browser", linux: "BraveSoftware/Brave-Browser" },
    Chromium { id: "vivaldi", label: "Vivaldi", win: "Vivaldi/User Data", win_roaming: "", mac: "Vivaldi", linux: "vivaldi" },
    Chromium { id: "chromium", label: "Chromium", win: "Chromium/User Data", win_roaming: "", mac: "Chromium", linux: "chromium" },
    Chromium { id: "yandex", label: "Yandex", win: "Yandex/YandexBrowser/User Data", win_roaming: "", mac: "Yandex/YandexBrowser", linux: "yandex-browser" },
    Chromium { id: "opera", label: "Opera", win: "", win_roaming: "Opera Software/Opera Stable", mac: "com.operasoftware.Opera", linux: "opera" },
    Chromium { id: "opera-gx", label: "Opera GX", win: "", win_roaming: "Opera Software/Opera GX Stable", mac: "com.operasoftware.OperaGX", linux: "" },
];

/// A Firefox-family browser, by where its folder of profiles is.
struct Firefox {
    id: &'static str,
    label: &'static str,
    win_roaming: &'static str,
    mac: &'static str,
    /// Relative to the home directory, on Linux.
    home: &'static str,
}

const FIREFOX: &[Firefox] = &[
    Firefox { id: "firefox", label: "Firefox", win_roaming: "Mozilla/Firefox/Profiles", mac: "Firefox/Profiles", home: ".mozilla/firefox" },
    Firefox { id: "librewolf", label: "LibreWolf", win_roaming: "librewolf/Profiles", mac: "LibreWolf/Profiles", home: ".librewolf" },
    Firefox { id: "waterfox", label: "Waterfox", win_roaming: "Waterfox/Profiles", mac: "Waterfox/Profiles", home: ".waterfox" },
    Firefox { id: "zen", label: "Zen Browser", win_roaming: "zen/Profiles", mac: "zen/Profiles", home: ".zen" },
];

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData").join("Local"))
}

fn roaming_app_data() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData").join("Roaming"))
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
}

/// Joins a `/`-separated relative path onto a base, whatever the platform's
/// own separator is.
fn under(base: PathBuf, relative: &str) -> PathBuf {
    relative.split('/').fold(base, |path, part| path.join(part))
}

fn chromium_folder(browser: &Chromium) -> Option<PathBuf> {
    if cfg!(windows) {
        if !browser.win_roaming.is_empty() {
            return Some(under(roaming_app_data(), browser.win_roaming));
        }
        return (!browser.win.is_empty()).then(|| under(local_app_data(), browser.win));
    }
    if cfg!(target_os = "macos") {
        return (!browser.mac.is_empty())
            .then(|| under(home().join("Library").join("Application Support"), browser.mac));
    }
    (!browser.linux.is_empty()).then(|| under(config_home(), browser.linux))
}

fn firefox_folder(browser: &Firefox) -> Option<PathBuf> {
    if cfg!(windows) {
        return (!browser.win_roaming.is_empty())
            .then(|| under(roaming_app_data(), browser.win_roaming));
    }
    if cfg!(target_os = "macos") {
        return (!browser.mac.is_empty())
            .then(|| under(home().join("Library").join("Application Support"), browser.mac));
    }
    (!browser.home.is_empty()).then(|| under(home(), browser.home))
}

/// A file's size and modified time, or `None` if it is not a file with bytes
/// in it. An empty cookie store is a browser nobody has browsed on.
fn stat_of(file: &Path) -> Option<(u64, Option<String>)> {
    let meta = std::fs::metadata(file).ok()?;
    if !meta.is_file() || meta.len() == 0 {
        return None;
    }
    let modified = meta.modified().ok().map(|when| {
        let secs = when
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        // ISO, so the sort below is a string comparison and the screen can
        // print it without a second format.
        iso_of(secs)
    });
    Some((meta.len(), modified))
}

/// Milliseconds since the epoch as an ISO timestamp.
///
/// Written out rather than pulled from a date crate: this is the only place in
/// the crate that needs one, and it needs one format.
fn iso_of(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Howard Hinnant's days-from-civil, inverted. Exact for every date this can
/// see, which a month-length table is not.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn directories(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect()
}

/// The cookie file inside a Chromium profile.
///
/// `Network/Cookies` since Chrome 96, `Cookies` before it. Both are checked,
/// because a profile that has not been opened since the move still has the old
/// one and a profile carried over from an old machine has both - in which case
/// the newer path is the live one.
pub fn chromium_cookie_file(profile_dir: &Path) -> Option<PathBuf> {
    let modern = profile_dir.join("Network").join("Cookies");
    if stat_of(&modern).is_some() {
        return Some(modern);
    }
    let legacy = profile_dir.join("Cookies");
    stat_of(&legacy).is_some().then_some(legacy)
}

/// The names the browser shows in its own profile switcher, by folder.
fn chromium_profile_names(user_data: &Path) -> serde_json::Map<String, serde_json::Value> {
    let Ok(raw) = std::fs::read_to_string(user_data.join("Local State")) else {
        // A profile with no friendly name is still a profile; it just gets
        // called by its folder.
        return Default::default();
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|state| state.pointer("/profile/info_cache").cloned())
        .and_then(|cache| cache.as_object().cloned())
        .unwrap_or_default()
}

fn chromium_profiles(browser: &Chromium) -> Vec<Source> {
    let Some(dir) = chromium_folder(browser) else {
        return Vec::new();
    };
    let names = chromium_profile_names(&dir);

    // The empty string is the root itself, for Opera, whose folder is one
    // profile rather than a folder of them.
    let mut candidates = vec![String::new()];
    candidates.extend(directories(&dir));

    candidates
        .into_iter()
        .filter_map(|folder| {
            let profile_dir = if folder.is_empty() { dir.clone() } else { dir.join(&folder) };
            let file = chromium_cookie_file(&profile_dir)?;
            let (bytes, updated_at) = stat_of(&file)?;
            let named = names
                .get(&folder)
                .and_then(|info| info.get("name"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            Some(Source {
                id: format!("{}:{}", browser.id, if folder.is_empty() { "." } else { &folder }),
                browser_id: browser.id.to_string(),
                browser: browser.label.to_string(),
                family: "chromium".into(),
                profile: named
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| {
                        if folder.is_empty() { browser.label.to_string() } else { folder.clone() }
                    }),
                file,
                // The key lives one level up, beside every profile that shares
                // it.
                local_state: dir.join("Local State"),
                bytes,
                updated_at,
                cookies: None,
            })
        })
        .collect()
}

fn firefox_profiles(browser: &Firefox) -> Vec<Source> {
    let Some(dir) = firefox_folder(browser) else {
        return Vec::new();
    };
    // On Linux the folder is the browser's root and the profiles are either
    // inside a `Profiles` folder or directly in it, depending on the build.
    let roots = [dir.clone(), dir.join("Profiles")];

    let mut found = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for root in roots {
        for folder in directories(&root) {
            let file = root.join(&folder).join("cookies.sqlite");
            let Some((bytes, updated_at)) = stat_of(&file) else {
                continue;
            };
            if !seen.insert(file.clone()) {
                continue;
            }
            found.push(Source {
                id: format!("{}:{folder}", browser.id),
                browser_id: browser.id.to_string(),
                browser: browser.label.to_string(),
                family: "firefox".into(),
                // `vhnlmsxe.default-release` is a folder name and a hash; the
                // half after the dot is the part a person named.
                profile: match folder.find('.') {
                    Some(at) => folder[at + 1..].to_string(),
                    None => folder.clone(),
                },
                file,
                local_state: PathBuf::new(),
                bytes,
                updated_at,
                cookies: None,
            });
        }
    }
    found
}

/// Every browser profile on this computer with cookies in it.
///
/// Sorted by how recently the file was written, because the browser somebody
/// actually uses is the one whose cookies changed this morning, and it should
/// not be third in a list under two they installed once.
pub fn profiles() -> Vec<Source> {
    let mut found: Vec<Source> = CHROMIUM
        .iter()
        .flat_map(chromium_profiles)
        .chain(FIREFOX.iter().flat_map(firefox_profiles))
        .collect();
    found.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_joins_with_this_platforms_separator() {
        let joined = under(PathBuf::from("base"), "Google/Chrome/User Data");
        assert_eq!(joined, PathBuf::from("base").join("Google").join("Chrome").join("User Data"));
    }

    /// The sort is a string comparison, so the format has to be the one where
    /// later is larger.
    #[test]
    fn timestamps_are_iso_and_sort_as_text() {
        assert_eq!(iso_of(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_of(1_788_000_000_000), "2026-08-29T10:40:00Z");
        assert!(iso_of(1_788_000_000_000) > iso_of(0));
    }

    /// A leap day, which a table of month lengths gets wrong.
    #[test]
    fn a_leap_day_is_the_day_it_says() {
        // 2024-02-29T12:00:00Z
        assert_eq!(iso_of(1_709_208_000_000), "2024-02-29T12:00:00Z");
    }

    #[test]
    fn the_modern_cookie_path_wins_over_the_legacy_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let profile = dir.path();
        std::fs::create_dir_all(profile.join("Network")).expect("the network folder");
        std::fs::write(profile.join("Cookies"), b"old").expect("the legacy file");
        assert_eq!(chromium_cookie_file(profile), Some(profile.join("Cookies")));

        std::fs::write(profile.join("Network").join("Cookies"), b"new").expect("the modern file");
        assert_eq!(
            chromium_cookie_file(profile),
            Some(profile.join("Network").join("Cookies"))
        );
    }

    /// An empty file is a browser nobody has browsed on, not a profile.
    #[test]
    fn an_empty_cookie_file_is_not_a_profile() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("Cookies"), b"").expect("an empty file");
        assert_eq!(chromium_cookie_file(dir.path()), None);
    }
}

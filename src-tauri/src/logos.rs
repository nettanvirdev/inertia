//! App logos, fetched here and handed to the window as data URIs.
//!
//! The window cannot fetch them itself. Its content policy allows `self`, `data:`
//! and `blob:` images and nothing else, deliberately: it renders model-authored
//! markdown, and a policy that allows arbitrary remote images is a policy that
//! lets a model exfiltrate a conversation one pixel URL at a time. Widening it
//! so some logos load would trade a real defence for decoration.
//!
//! So the fetch happens on this side, where there is no document to leak, and
//! the bytes cross already encoded. That also puts one cache in front of a CDN
//! the catalogue screen would otherwise hit a few hundred times per scroll.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use tauri::{AppHandle, Manager};

/// A logo is a small square. Anything past this is not one.
const MAX_BYTES: usize = 512 * 1024;

/// Long enough for a cold CDN, short enough not to hold a row's render.
const TIMEOUT: Duration = Duration::from_secs(8);

/// Bumped whenever what counts as a usable logo changes. A cached `""` from a
/// build that refused SVG would otherwise outlive the build that accepts it,
/// and the user would have to find and delete a folder to see their logos.
const CACHE_VERSION: u32 = 1;

/// What counts as a logo.
///
/// SVG is on the list, and it is worth saying why: Composio serves every logo
/// in its catalogue as `image/svg+xml`, so refusing it means refusing all of
/// them. An SVG loaded through an `img` element - the only way one ever reaches
/// this screen - renders in a context that runs no script and fetches nothing
/// external, so the usual reason to distrust one does not apply. The `script`
/// and `foreignObject` check below is belt to that braces, and costs a
/// substring scan of a file already in hand.
const TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/svg+xml",
    "image/x-icon",
    "image/vnd.microsoft.icon",
];

/// Held for the process's life: the catalogue re-renders far more than it
/// changes. The window de-duplicates in-flight requests per URL of its own
/// accord, so there is no second map for those here.
fn memory() -> &'static Mutex<HashMap<String, String>> {
    static MEMORY: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MEMORY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn usable(content_type: &str) -> Option<&'static str> {
    let declared = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();
    TYPES.iter().copied().find(|known| *known == declared)
}

fn suspicious(kind: &str, bytes: &[u8]) -> bool {
    if kind != "image/svg+xml" {
        return false;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(64 * 1024)]).to_ascii_lowercase();
    head.contains("<script") || head.contains("<foreignobject")
}

/// A file name for a URL: FNV-1a, written out rather than pulled in.
///
/// A hash function from a crate would be a dependency added to name files in a
/// throwaway cache. The only thing asked of it is that the same URL names the
/// same file across runs and builds, which rules out the standard library's
/// default hasher - it is explicitly allowed to change between releases, and a
/// cache that silently empties itself on an upgrade is not one.
fn key_of(url: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{CACHE_VERSION}:{url}").as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

fn cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_cache_dir()
        .ok()
        .map(|dir| dir.join("logo-cache"))
}

fn from_disk(app: &AppHandle, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(cache_dir(app)?.join(key)).ok()?;
    Some(if text.starts_with("data:") {
        text
    } else {
        String::new()
    })
}

fn to_disk(app: &AppHandle, key: &str, value: &str) {
    let Some(dir) = cache_dir(app) else { return };
    if std::fs::create_dir_all(&dir).is_ok() {
        // A cache that cannot be written is still a working cache.
        let _ = std::fs::write(dir.join(key), value);
    }
}

async fn download(url: &str) -> String {
    let Ok(client) = reqwest::Client::builder().timeout(TIMEOUT).build() else {
        return String::new();
    };
    let Ok(response) = client.get(url).header("Accept", "image/*").send().await else {
        return String::new();
    };
    if !response.status().is_success() {
        return String::new();
    }

    let Some(kind) = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(usable)
    else {
        return String::new();
    };

    // Trust the header enough to skip an oversized body, but check the bytes
    // too: a CDN that omits the length would otherwise be unbounded.
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return String::new();
    }
    let Ok(bytes) = response.bytes().await else {
        return String::new();
    };
    if bytes.is_empty() || bytes.len() > MAX_BYTES || suspicious(kind, &bytes) {
        return String::new();
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    format!("data:{kind};base64,{encoded}")
}

/// A logo URL as a data URI, or `""` when there is not one to be had.
///
/// The empty string is cached as hard as a hit is. A vendor with no logo, or
/// one behind a dead URL, would otherwise be re-fetched on every scroll for the
/// life of the window - and the screen already has a good answer for it, which
/// is the first letter of the app's name.
pub async fn logo(app: &AppHandle, raw: &str) -> String {
    let url = raw.trim();
    if url.is_empty() {
        return String::new();
    }
    if url.starts_with("data:") {
        return url.to_string();
    }
    if !url.to_ascii_lowercase().starts_with("https://") {
        return String::new();
    }

    if let Some(held) = memory().lock().ok().and_then(|map| map.get(url).cloned()) {
        return held;
    }

    let key = key_of(url);
    let value = match from_disk(app, &key) {
        Some(cached) => cached,
        None => {
            let fetched = download(url).await;
            to_disk(app, &key, &fetched);
            fetched
        }
    };

    if let Ok(mut map) = memory().lock() {
        map.insert(url.to_string(), value.clone());
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_types_a_vendor_actually_serves() {
        assert_eq!(usable("image/svg+xml"), Some("image/svg+xml"));
        assert_eq!(usable("image/PNG; charset=binary"), Some("image/png"));
        assert_eq!(usable("text/html"), None);
    }

    #[test]
    fn refuses_an_svg_carrying_script() {
        assert!(suspicious(
            "image/svg+xml",
            b"<svg><script>x</script></svg>"
        ));
        assert!(suspicious("image/svg+xml", b"<svg><foreignObject/></svg>"));
        assert!(!suspicious("image/svg+xml", b"<svg><path/></svg>"));
        // Only SVG is scanned: the same bytes in a PNG are pixels.
        assert!(!suspicious("image/png", b"<script>"));
    }

    #[test]
    fn the_cache_key_follows_the_version() {
        assert_ne!(key_of("https://a/b.svg"), key_of("https://a/c.svg"));
        assert_eq!(key_of("https://a/b.svg"), key_of("https://a/b.svg"));
    }
}

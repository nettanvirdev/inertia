//! Getting a cookie's value back out of a browser's store.
//!
//! Chromium encrypts cookie values at rest with a key that belongs to the
//! logged-in user, so reading them is only possible as that user - which is
//! exactly the property that makes this safe to offer and impossible to abuse
//! from anywhere else. Firefox does not encrypt them at all.
//!
//! ## The three envelopes
//!
//! A stored value begins with a version tag, and three of them matter.
//!
//! **v10 on Windows** is AES-256-GCM under a key that lives in `Local State`,
//! itself wrapped by DPAPI. Unwrapping it is one call to the operating system
//! as the current user. This is the common case and the one that works.
//!
//! **v10 on Linux** is AES-128-CBC under a key derived from the password
//! `peanuts` when there is no keyring - which is why a container browser
//! started with `--password-store=basic` is readable by anything that can read
//! its disk. `v11` is the same shape with the key in the desktop keyring, which
//! is not reachable without unlocking it, so those are reported rather than
//! guessed at.
//!
//! **v20 is a wall.** From Chrome 127 cookies are sealed with an app-bound key
//! that only Chrome itself, running as Chrome, can unwrap: the blob in
//! `Local State` is protected at SYSTEM level and handed back only to the
//! signed Chrome binary. Nothing this app can do as the user will open it, and
//! the honest answer is a count and a sentence, not an attempt. Edge does the
//! same. Brave and Vivaldi, at the time of writing, do not.
//!
//! ## The hash in front of the value
//!
//! Since Chrome 124 the plaintext under the envelope is not the cookie value.
//! It is the SHA-256 of the cookie's own domain followed by the value, so a
//! cookie copied from one host's row to another's fails to decrypt into
//! anything usable. Stripping it is not optional and it is not a guess: the
//! prefix is only removed when it matches the hash of the row's own host.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::Source;

/// The key this profile's values are encrypted with, or what it cannot open.
///
/// A value rather than an error, because "Chrome is here and its key is sealed"
/// is something the screen needs to say, and an exception cannot be put in a
/// list next to the profiles that worked.
#[derive(Debug, Clone)]
pub enum Key {
    /// Firefox. Nothing to open.
    None,
    /// AES-256-GCM, Windows.
    Gcm(Vec<u8>),
    /// AES-128-CBC under a PBKDF2 key, macOS and Linux.
    Cbc(Vec<u8>),
    /// Here, and shut. The sentence is shown to the person.
    Unavailable(String),
}

/// The sentence for a value sealed by Chrome's app-bound key.
pub const SEALED: &str =
    "sealed with an app-bound key that only the browser itself can open (Chrome and Edge 127 and later)";

const KEYRING: &str = "encrypted with a key held in the desktop keyring";

/// The tag Chromium writes in front of an encrypted value.
pub fn envelope_of(blob: &[u8]) -> Option<&'static str> {
    match blob.get(..3) {
        Some(b"v10") => Some("v10"),
        Some(b"v11") => Some("v11"),
        Some(b"v20") => Some("v20"),
        _ => None,
    }
}

/// Chromium's own derivation, on the platforms that use a password. Windows
/// uses a key from `Local State` instead and never reaches this.
#[cfg(not(windows))]
fn from_password(password: &[u8], iterations: u32) -> Vec<u8> {
    let mut key = [0u8; 16];
    // `saltysalt` is Chromium's, not ours; it is a constant in its source.
    let _ = pbkdf2::pbkdf2::<hmac::Hmac<sha1::Sha1>>(password, b"saltysalt", iterations, &mut key);
    key.to_vec()
}

/// DPAPI, through PowerShell.
///
/// There is no way to call `CryptUnprotectData` without binding to the Windows
/// API, and this is called once per profile - for the key, never for a value -
/// so the cost of a process is a few milliseconds against a dependency and a
/// block of `unsafe` that no test in this workspace could exercise.
///
/// The blob goes in on stdin rather than in the command line: a key is 32 bytes
/// but an argument list has a ceiling and a legacy value can be any length.
#[cfg(windows)]
fn dpapi_unprotect(blob: &[u8]) -> std::result::Result<Vec<u8>, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let script = [
        "$ErrorActionPreference='Stop'",
        "Add-Type -AssemblyName System.Security",
        "$in=[Console]::In.ReadToEnd().Trim()",
        "$bytes=[Convert]::FromBase64String($in)",
        "$out=[System.Security.Cryptography.ProtectedData]::Unprotect($bytes,$null,[System.Security.Cryptography.DataProtectionScope]::CurrentUser)",
        "[Console]::Out.Write([Convert]::ToBase64String($out))",
    ]
    .join("; ");

    // UTF-16LE then base64 is what `-EncodedCommand` takes, and it is what
    // keeps the quoting in the script above from meeting a shell at all.
    let wide: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(wide);

    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(mut stdin) = child.stdin.take() {
        let payload = base64::engine::general_purpose::STANDARD.encode(blob);
        stdin
            .write_all(payload.as_bytes())
            .map_err(|e| e.to_string())?;
    }

    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    base64::engine::general_purpose::STANDARD
        .decode(String::from_utf8_lossy(&output.stdout).trim())
        .map_err(|e| e.to_string())
}

/// The service name each Chromium-family browser stores its key under.
#[cfg(target_os = "macos")]
fn keychain_service(browser_id: &str) -> Option<&'static str> {
    Some(match browser_id {
        "chrome" | "chrome-beta" => "Chrome Safe Storage",
        "chromium" => "Chromium Safe Storage",
        "edge" => "Microsoft Edge Safe Storage",
        "brave" => "Brave Safe Storage",
        "vivaldi" => "Vivaldi Safe Storage",
        "opera" | "opera-gx" => "Opera Safe Storage",
        "yandex" => "Yandex Safe Storage",
        _ => return None,
    })
}

/// The key for one profile.
pub fn key_for(source: &Source) -> Key {
    if source.family == "firefox" {
        return Key::None;
    }

    #[cfg(target_os = "linux")]
    {
        let _ = source;
        return Key::Cbc(from_password(b"peanuts", 1));
    }

    #[cfg(target_os = "macos")]
    {
        let Some(service) = keychain_service(&source.browser_id) else {
            return Key::Unavailable(format!(
                "No keychain entry is known for {}.",
                source.browser
            ));
        };
        return match std::process::Command::new("security")
            .args(["find-generic-password", "-w", "-s", service])
            .output()
        {
            Ok(out) if out.status.success() => Key::Cbc(from_password(
                String::from_utf8_lossy(&out.stdout).trim().as_bytes(),
                1003,
            )),
            _ => Key::Unavailable(format!(
                "macOS would not release {}'s key from the keychain.",
                source.browser
            )),
        };
    }

    #[cfg(windows)]
    {
        let Ok(raw) = std::fs::read_to_string(&source.local_state) else {
            return Key::Unavailable(format!(
                "{} has no Local State file to take a key from.",
                source.browser
            ));
        };
        let encoded = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|state| state.pointer("/os_crypt/encrypted_key").cloned())
            .and_then(|found| found.as_str().map(str::to_string))
            .unwrap_or_default();
        if encoded.is_empty() {
            return Key::Unavailable(format!(
                "{} has not written an encryption key yet.",
                source.browser
            ));
        }

        let Ok(wrapped) = base64::engine::general_purpose::STANDARD.decode(&encoded) else {
            return Key::Unavailable(format!("{}'s stored key is not readable.", source.browser));
        };
        // Five bytes of `DPAPI` in front of the blob the operating system
        // understands.
        let blob = if wrapped.starts_with(b"DPAPI") {
            &wrapped[5..]
        } else {
            &wrapped[..]
        };
        match dpapi_unprotect(blob) {
            Ok(key) => Key::Gcm(key),
            Err(why) => Key::Unavailable(format!(
                "Windows would not unwrap {}'s key: {why}",
                source.browser
            )),
        }
    }
}

/// Chromium 124 and later put the host's hash in front of the value.
pub fn strip_domain_hash<'a>(plain: &'a [u8], host: &str) -> &'a [u8] {
    if plain.len() < 32 || host.is_empty() {
        return plain;
    }
    let expected = Sha256::digest(host.as_bytes());
    if plain[..32] == expected[..] {
        &plain[32..]
    } else {
        plain
    }
}

/// One stored value, opened, or the reason it could not be.
///
/// Never an error: one unreadable cookie in eight hundred is a number in a
/// summary, not the end of an import.
pub fn decrypt_value(
    blob: &[u8],
    plain: &str,
    host: &str,
    key: &Key,
) -> std::result::Result<String, String> {
    if blob.is_empty() {
        return Ok(plain.to_string());
    }

    let Some(envelope) = envelope_of(blob) else {
        // A raw DPAPI blob, from before Chromium had version tags. Old enough
        // that the cookie is almost certainly expired anyway.
        return Err("stored in a format this browser stopped using years ago".into());
    };
    if envelope == "v20" {
        return Err(SEALED.into());
    }
    if let Key::Unavailable(why) = key {
        return Err(why.clone());
    }
    if envelope == "v11" && !matches!(key, Key::Gcm(_)) {
        return Err(KEYRING.into());
    }

    let opened = match key {
        Key::Gcm(bytes) => {
            if blob.len() < 15 + 16 {
                return Err("the stored value is too short to be what it claims".into());
            }
            let cipher = Aes256Gcm::new_from_slice(bytes)
                .map_err(|_| "the key on this machine is the wrong size".to_string())?;
            let nonce = Nonce::from_slice(&blob[3..15]);
            cipher
                .decrypt(nonce, &blob[15..])
                .map_err(|_| "the key on this machine does not open it".to_string())?
        }
        Key::Cbc(bytes) => {
            use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
            type Decryptor = cbc::Decryptor<aes::Aes128>;
            // Sixteen spaces, which is Chromium's IV on these platforms.
            let decryptor = Decryptor::new_from_slices(bytes, &[b' '; 16])
                .map_err(|_| "the key on this machine is the wrong size".to_string())?;
            // Decrypted in place in a copy, rather than into a fresh buffer:
            // the in-place form is the one that does not need the cipher
            // crate's allocating feature turned on.
            let mut buffer = blob[3..].to_vec();
            let opened = decryptor
                .decrypt_padded_mut::<Pkcs7>(&mut buffer)
                .map_err(|_| "the key on this machine does not open it".to_string())?;
            opened.to_vec()
        }
        Key::None => return Ok(plain.to_string()),
        Key::Unavailable(why) => return Err(why.clone()),
    };

    Ok(String::from_utf8_lossy(strip_domain_hash(&opened, host)).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_envelopes_are_recognised_and_nothing_else_is() {
        assert_eq!(envelope_of(b"v10abc"), Some("v10"));
        assert_eq!(envelope_of(b"v11abc"), Some("v11"));
        assert_eq!(envelope_of(b"v20abc"), Some("v20"));
        assert_eq!(envelope_of(b"v99abc"), None);
        assert_eq!(envelope_of(b"ab"), None);
    }

    /// The prefix comes off only when it is the hash of this row's own host.
    /// Stripping 32 bytes on faith would silently eat the front of every value
    /// written before Chrome 124.
    #[test]
    fn the_domain_hash_is_removed_only_when_it_is_the_right_one() {
        let host = "example.com";
        let mut sealed = Sha256::digest(host.as_bytes()).to_vec();
        sealed.extend_from_slice(b"the-value");
        assert_eq!(strip_domain_hash(&sealed, host), b"the-value");

        // Same bytes, a different row.
        assert_eq!(strip_domain_hash(&sealed, "other.com"), &sealed[..]);
        // Too short to carry one.
        assert_eq!(strip_domain_hash(b"short", host), b"short");
    }

    /// Chrome 127's app-bound key is a wall, and the answer is the sentence,
    /// not an attempt that fails in some other way.
    #[test]
    fn a_sealed_value_says_so_rather_than_failing_obscurely() {
        let key = Key::Gcm(vec![0u8; 32]);
        let why = decrypt_value(b"v20whatever", "", "example.com", &key).expect_err("a reason");
        assert_eq!(why, SEALED);
    }

    #[test]
    fn a_firefox_value_is_its_own_plain_text() {
        assert_eq!(
            decrypt_value(b"", "already-plain", "example.com", &Key::None).unwrap(),
            "already-plain"
        );
    }

    /// A key that could not be unwrapped is reported once, in the words the
    /// screen prints, rather than as eight hundred decryption failures.
    #[test]
    fn an_unavailable_key_reports_its_own_reason() {
        let key = Key::Unavailable("Windows would not unwrap it".into());
        let why = decrypt_value(b"v10....", "", "example.com", &key).expect_err("a reason");
        assert_eq!(why, "Windows would not unwrap it");
    }

    /// A round trip through the GCM path, which is the one that matters on
    /// Windows and the one nothing else here exercises.
    #[test]
    fn a_gcm_value_round_trips_with_its_domain_hash() {
        use aes_gcm::aead::Aead;

        let raw = [7u8; 32];
        let cipher = Aes256Gcm::new_from_slice(&raw).expect("a key");
        let nonce = Nonce::from_slice(b"012345678901");

        let host = "example.com";
        let mut plain = Sha256::digest(host.as_bytes()).to_vec();
        plain.extend_from_slice(b"session=yes");

        let mut blob = b"v10".to_vec();
        blob.extend_from_slice(nonce);
        blob.extend_from_slice(&cipher.encrypt(nonce, plain.as_slice()).expect("sealed"));

        let opened = decrypt_value(&blob, "", host, &Key::Gcm(raw.to_vec())).expect("opened");
        assert_eq!(opened, "session=yes");
    }

    #[test]
    fn a_value_the_key_does_not_open_is_one_reason_not_a_panic() {
        let blob = {
            let cipher = Aes256Gcm::new_from_slice(&[1u8; 32]).expect("a key");
            let nonce = Nonce::from_slice(b"012345678901");
            let mut blob = b"v10".to_vec();
            blob.extend_from_slice(nonce);
            blob.extend_from_slice(&cipher.encrypt(nonce, b"x".as_slice()).expect("sealed"));
            blob
        };
        let why = decrypt_value(&blob, "", "example.com", &Key::Gcm(vec![2u8; 32]))
            .expect_err("a reason");
        assert!(why.contains("does not open it"), "{why}");
    }
}

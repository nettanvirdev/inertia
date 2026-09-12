//! `file:` URIs, both ways.
//!
//! A separate file because both directions are lossy if done carelessly and
//! both are load-bearing: the URI is how a document is named on the wire, and
//! a path that does not round-trip means diagnostics published for a file we
//! opened are filed under a name nothing looks up.

use std::path::{Path, PathBuf};

/// Characters that survive a path unencoded. Everything else becomes `%xx`,
/// which is what matters for a path containing a space, a `#`, or a `%`.
fn safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'-' | b'.' | b'_' | b'~' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+'
                | b',' | b';' | b'=' | b':' | b'@' | b'/'
        )
}

/// An absolute path as a `file:` URI.
pub fn to_uri(file: &Path) -> String {
    let absolute = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let text = absolute.to_string_lossy().replace('\\', "/");
    // A Windows path starts at a drive letter and a URI path starts at a
    // slash; a POSIX path already has one.
    let with_root = if text.starts_with('/') {
        text
    } else {
        format!("/{text}")
    };

    let mut out = String::from("file://");
    for byte in with_root.as_bytes() {
        if safe(*byte) {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The path a `file:` URI names, or `None`.
///
/// `None` for anything that is not a file on this disk. A server that
/// publishes for an `untitled:` or `jar:` document is talking about something
/// there is nothing to report against.
pub fn from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // An authority of `localhost` or nothing is the only one that names a
    // local file. Anything else is a share we are not going to open.
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(slash) if &rest[..slash] == "localhost" => &rest[slash..],
        _ => return None,
    };

    let decoded = decode(path);
    if cfg!(windows) {
        // `/C:/x` is the URI spelling of `C:\x`.
        let trimmed = decoded.strip_prefix('/').unwrap_or(&decoded);
        let looks_like_drive = trimmed
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
            && trimmed.as_bytes().get(1) == Some(&b':');
        if looks_like_drive {
            return Some(PathBuf::from(trimmed.replace('/', "\\")));
        }
        return Some(PathBuf::from(decoded.replace('/', "\\")));
    }
    Some(PathBuf::from(decoded))
}

/// A path in the one spelling used as a map key.
///
/// On Windows a server may hand back `c:\src\a.ts` for a file we opened as
/// `C:\src\a.ts`, and a lookup keyed on the string we sent then misses its own
/// diagnostics. This is the whole bug, and it only appears on one platform.
pub fn normalize(file: &Path) -> String {
    let resolved = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    let text = resolved.to_string_lossy().into_owned();
    if cfg!(windows) {
        text.replace('\\', "/").to_lowercase()
    } else {
        text
    }
}

/// A path as the filesystem actually spells it.
///
/// Only Windows needs this, and only for a file the model never opened: the
/// server is the sole source of that path and tsserver lowercases the ones it
/// reports. A drive letter and a home directory in the wrong case are still a
/// valid path to open, but they read as a different file, and the point of
/// naming the file at all is that the model goes and looks at it.
pub fn spelling(file: &Path) -> PathBuf {
    match std::fs::canonicalize(file) {
        // Rust's canonicalize returns the extended-length form on Windows.
        // `\\?\C:\x` is a path every API accepts and no human recognises.
        Ok(real) => {
            let text = real.to_string_lossy().into_owned();
            match text.strip_prefix(r"\\?\") {
                Some(plain) => PathBuf::from(plain),
                None => real,
            }
        }
        // Deleted, or a path this process cannot stat. Its own spelling will
        // do.
        Err(_) => std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_round_trips_through_a_uri() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = dir.path().to_path_buf();
        file.push("a file with spaces.ts");
        std::fs::write(&file, "x").unwrap();

        let uri = to_uri(&file);
        assert!(uri.starts_with("file:///"), "got {uri}");
        assert!(!uri.contains(' '), "a space must be encoded: {uri}");
        assert_eq!(normalize(&from_uri(&uri).unwrap()), normalize(&file));
    }

    #[test]
    fn a_document_that_is_not_a_file_has_no_path() {
        assert!(from_uri("untitled:Untitled-1").is_none());
        assert!(from_uri("jar:file:///a.jar!/b.class").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_are_keyed_case_insensitively() {
        assert_eq!(
            normalize(Path::new(r"C:\Src\A.ts")),
            normalize(Path::new(r"c:\src\a.ts"))
        );
    }
}

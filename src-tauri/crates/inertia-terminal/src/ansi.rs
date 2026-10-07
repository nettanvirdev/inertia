//! Reading the one thing a shell volunteers about itself.
//!
//! A pty is a stream of bytes and nothing more. It does not know what a working
//! directory is, and the process handle cannot tell you either - asking a
//! process on Windows where it currently is has no portable answer at all.
//!
//! So the shell is asked to say so, the way every terminal emulator worth using
//! asks: an escape sequence emitted from the prompt, carrying the path, which
//! the emulator reads and the person never sees. Two sequences exist because
//! the ecosystem never agreed - OSC 7 with a `file://` URL is the POSIX
//! convention, OSC 9;9 with a bare path is Windows Terminal's - and both are
//! read here, because both are in the wild and the app emits whichever the
//! shell in front of it can produce (see `shell.rs`).
//!
//! Anything unparseable is `None` rather than a guess. A wrong working
//! directory in a tool result is worse than no working directory: the model
//! goes looking for a file in a folder the person has not been in for an hour.

/// The working directory a shell just reported, or `None`.
///
/// The LAST report in the chunk wins, not the first: a chunk that spans two
/// prompts contains two of these, and the newer one is where the shell is now.
/// OSC 9;9 is preferred when both appear, because a shell that emits it is the
/// one this app configured and the other is likely to be a leftover from the
/// person's own prompt framework.
pub fn read_cwd(raw: &str) -> Option<String> {
    let mut osc9: Option<String> = None;
    let mut osc7: Option<String> = None;

    for body in osc_bodies(raw) {
        if let Some(rest) = body.strip_prefix("9;9;") {
            osc9 = Some(rest.trim_matches('"').to_string());
        } else if let Some(rest) = body.strip_prefix("7;file://") {
            // `file://HOST/path`: the host is everything up to the first
            // slash, and is nothing this app can use.
            let path = match rest.find('/') {
                Some(at) => &rest[at..],
                None => continue,
            };
            osc7 = Some(percent_decode(path));
        }
    }

    normalize(&osc9.or(osc7)?)
}

/// Every OSC payload in the text, in order.
///
/// An OSC is `ESC ]` then the payload then either BEL or ST (`ESC \`). Both
/// terminators are real and both are emitted by shells in the wild, so both end
/// one here. An unterminated trailing sequence is dropped: it is the front half
/// of something that will arrive complete in the next chunk, and half a path is
/// not a path.
fn osc_bodies(raw: &str) -> Vec<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] != '\u{1b}' || chars[i + 1] != ']' {
            i += 1;
            continue;
        }
        let mut body = String::new();
        let mut j = i + 2;
        let mut closed = false;
        while j < chars.len() {
            if chars[j] == '\u{7}' {
                closed = true;
                j += 1;
                break;
            }
            if chars[j] == '\u{1b}' {
                // ST, or the start of something else entirely. Either way this
                // sequence is over.
                closed = chars.get(j + 1) == Some(&'\\');
                j += if closed { 2 } else { 1 };
                break;
            }
            body.push(chars[j]);
            j += 1;
        }
        if closed {
            out.push(body);
        }
        i = j;
    }
    out
}

/// `%20` and friends, because a `file://` URL is a URL.
///
/// Anything that is not a valid escape is left exactly as it was found. A path
/// this app rewrote is a path this app got wrong.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok());
            if let Some(byte) = hex {
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

/// `/D:/code/inertia` is what a `file://` URL calls a Windows path, and it is
/// not one. The leading slash comes off; nothing else is touched.
fn normalize(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    Some(if drive { text[1..].to_string() } else { text.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_the_windows_terminal_sequence_powershell_emits() {
        let raw = "PS D:\\work> \u{1b}]9;9;\"D:\\work\\app\"\u{7}";
        assert_eq!(read_cwd(raw).as_deref(), Some("D:\\work\\app"));
    }

    #[test]
    fn it_reads_the_posix_sequence_and_undoes_the_url() {
        let raw = "\u{1b}]7;file://host/home/me/my%20project\u{7}$ ";
        assert_eq!(read_cwd(raw).as_deref(), Some("/home/me/my project"));
    }

    #[test]
    fn a_file_url_holding_a_windows_path_loses_its_leading_slash() {
        let raw = "\u{1b}]7;file:///D:/code/inertia\u{1b}\\";
        assert_eq!(read_cwd(raw).as_deref(), Some("D:/code/inertia"));
    }

    /// A chunk spanning two prompts holds two reports, and the shell is where
    /// the newer one says.
    #[test]
    fn the_last_report_in_a_chunk_wins() {
        let raw = "\u{1b}]9;9;\"C:\\one\"\u{7}cd two\r\n\u{1b}]9;9;\"C:\\two\"\u{7}";
        assert_eq!(read_cwd(raw).as_deref(), Some("C:\\two"));
    }

    #[test]
    fn half_a_sequence_is_not_a_path() {
        // The rest of it arrives in the next chunk; answering now would report
        // a truncated folder as the truth.
        assert_eq!(read_cwd("\u{1b}]9;9;\"C:\\par"), None);
        assert_eq!(read_cwd("no escapes here at all"), None);
    }

    #[test]
    fn an_ordinary_window_title_is_not_a_working_directory() {
        assert_eq!(read_cwd("\u{1b}]0;npm run dev\u{7}"), None);
    }
}

//! The small desktop errands: what the app is, a picture off the web, a
//! picture onto the disk, and a command out of a code block.
//!
//! These answer with plain values rather than the `{ ok, data }` envelope the
//! workspace commands use, because that is what the renderer reads - `info
//! ?.version`, `result?.url`, `result?.saved`. The envelope belongs to the
//! workspace surface, where telling "empty" from "gone" actually matters.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;

/// A picture in a message can be large, and the window has to hold the whole
/// thing as a data URL. Past this it is not worth drawing.
const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
const IMAGE_TIMEOUT: Duration = Duration::from_secs(15);

/// Long enough for a build, short enough that a hung command does not hold a
/// spinner forever.
const SNIPPET_TIMEOUT: Duration = Duration::from_secs(120);
/// The tail is kept, because the end of a build is the part with the error in
/// it.
const MAX_OUTPUT: usize = 200 * 1024;

/// What the About pane shows.
///
/// `electron` and `node` are deliberately absent rather than faked: this is not
/// Electron, and a row claiming a Node version that nothing here runs would be
/// a lie in a pane whose entire job is to say what is actually installed. The
/// pane names Tauri, Rust and the webview instead, and renders a dash for
/// anything missing.
///
/// The webview is the one row whose *name* changes by platform: the same field
/// is a WebView2 version on Windows, WKWebView on macOS and WebKitGTK on Linux,
/// and a fixed "WebView2" label would be wrong on two of the three. The name
/// travels with the number so the pane does not have to work it out.
#[tauri::command]
pub fn app_info(app: AppHandle) -> Value {
    json!({
        "version": app.package_info().version.to_string(),
        "appVersion": app.package_info().version.to_string(),
        "name": app.package_info().name,
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "tauri": tauri::VERSION,
        "webview": tauri::webview_version().unwrap_or_default(),
        "webviewName": webview_name(),
        // Captured by the build script; see there for why it cannot be read at
        // runtime. "rustc 1.90.0 (abcdef 2025-01-01)" is trimmed to the version
        // itself, which is the part anybody reads.
        "rust": env!("INERTIA_RUSTC")
            .strip_prefix("rustc ")
            .unwrap_or(env!("INERTIA_RUSTC"))
            .split_whitespace()
            .next()
            .unwrap_or_default(),
        "profile": if cfg!(debug_assertions) { "development" } else { "release" },
    })
}

/// What the platform's own webview is called.
fn webview_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "WebView2",
        "macos" | "ios" => "WKWebView",
        _ => "WebKitGTK",
    }
}

/// A picture from the web, as a data URL.
///
/// Fetched here because the window's content policy will not load a remote
/// image, which is what stops a model from turning a picture in a reply into a
/// request to a host it named. The transcript only asks for one when the
/// person clicks to load it. Everything that can go wrong - a bad URL, a
/// non-image, something too big, a host on this machine or its network -
/// answers `null`, and the transcript draws a broken-image state from that. It
/// needs no reason, and inventing error shapes for each case would only give
/// it more to ignore.
///
/// Only the public internet. The URL is the model's, and without this a
/// picture in a reply was a request from this machine to anything it can
/// reach: the router's admin page, a dev server on localhost, a cloud
/// metadata endpoint. Hosts are checked by the addresses they resolve to, at
/// the moment of connecting, so a name that resolves somewhere private is
/// caught however it is spelled; and every redirect is checked the same way,
/// so a public URL cannot bounce the request inward.
#[tauri::command]
pub async fn app_fetch_image(url: String) -> Option<Value> {
    let parsed = reqwest::Url::parse(&url).ok()?;
    if !reaches_only_the_internet(&parsed) {
        return None;
    }

    let client = reqwest::Client::builder()
        .timeout(IMAGE_TIMEOUT)
        .dns_resolver(std::sync::Arc::new(PublicOnly))
        // A proxy resolves the host itself, out of reach of the check above.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                attempt.error("too many redirects")
            } else if reaches_only_the_internet(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .ok()?;
    let response = client.get(parsed).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }

    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
        .filter(|value| value.starts_with("image/"))?;

    // Checked before reading as well as after: a server that declares a
    // gigabyte should not get a gigabyte of ours before we decline it.
    if response.content_length().is_some_and(|n| n > MAX_IMAGE_BYTES as u64) {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(json!({
        "url": format!("data:{mime};base64,{encoded}"),
        "bytes": bytes.len(),
    }))
}

/// More than this and it is not a picture, it is a maze.
const MAX_REDIRECTS: usize = 5;

/// Is this a URL a picture may be fetched from, as far as can be told before
/// connecting? An address written into the URL is checked here; a name is
/// checked by [`PublicOnly`] when it is resolved.
fn reaches_only_the_internet(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    // The URL parser has already turned `2130706433` and `0x7f.1` into the
    // dotted address they spell, so what is left is an address or a name.
    match url.host_str() {
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .map_or(true, is_public),
        None => false,
    }
}

/// Resolves a host to its public addresses only, and fails a host that has
/// none - so the connection is made to exactly the addresses that were checked.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let found: Vec<std::net::SocketAddr> = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .filter(|address| is_public(address.ip()))
                .collect();
            if found.is_empty() {
                return Err(format!("{} is not on the public internet", name.as_str()).into());
            }
            let addrs: reqwest::dns::Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// Is this an address on the public internet?
///
/// Everything else is somewhere a request from this machine should not be
/// sent on a model's say-so: this machine, its network, the carrier's shared
/// space, link-local services such as cloud metadata, multicast and broadcast.
/// An IPv6 address that carries an IPv4 one - mapped, compatible, NAT64, 6to4 -
/// is judged by the address it carries.
fn is_public(ip: std::net::IpAddr) -> bool {
    use std::net::{IpAddr, Ipv4Addr};
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_unspecified()
                || a == 0
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (18..20).contains(&b))
                || v4.is_documentation()
                || v4.is_multicast()
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            let embedded = |high: u16, low: u16| {
                Ipv4Addr::new((high >> 8) as u8, high as u8, (low >> 8) as u8, low as u8)
            };
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            // `::a.b.c.d`, the deprecated compatible form. `::` and `::1` are
            // in it too, and are refused below by the same test.
            if segments[..6].iter().all(|s| *s == 0) {
                return is_public(IpAddr::V4(embedded(segments[6], segments[7])));
            }
            // NAT64, 64:ff9b::/96.
            if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return is_public(IpAddr::V4(embedded(segments[6], segments[7])));
            }
            // 6to4, 2002::/16, carries its IPv4 address in the next 32 bits.
            if segments[0] == 0x2002 {
                return is_public(IpAddr::V4(embedded(segments[1], segments[2])));
            }
            !(v6.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] & 0xffc0) == 0xfec0
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SaveImage {
    /// Every picture in a message is already a data URL by the time it is
    /// drawn, so saving is a dialog and a write with no second request.
    pub data_url: String,
    pub name: String,
}

/// Splits `data:image/png;base64,AAAA` into its type and its bytes.
fn decode_data_url(value: &str) -> Option<(String, Vec<u8>)> {
    let rest = value.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mime = meta.split(';').next()?.to_string();
    if !meta.contains("base64") {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(payload).ok()?;
    Some((mime, bytes))
}

fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        _ => "png",
    }
}

/// A picture out of a message, onto the disk, where the person chooses.
#[tauri::command]
pub async fn app_save_image(app: AppHandle, request: SaveImage) -> Value {
    use tauri_plugin_dialog::DialogExt;

    let Some((mime, bytes)) = decode_data_url(&request.data_url) else {
        return json!({ "saved": false, "error": "That picture could not be read." });
    };
    let extension = extension_for(&mime);

    let stem = {
        let trimmed = request.name.trim();
        let base = if trimmed.is_empty() { "picture" } else { trimmed };
        // The name comes out of a message, so it is the model's text, not a
        // filename. Anything a path separator could be built from goes.
        let cleaned: String = base
            .chars()
            .map(|c| if "\\/:*?\"<>|".contains(c) { '-' } else { c })
            .collect();
        let cleaned = cleaned.trim_matches(['.', ' ']).to_string();
        let cleaned = if cleaned.is_empty() { "picture".to_string() } else { cleaned };
        match cleaned.rsplit_once('.') {
            Some((head, tail)) if tail.eq_ignore_ascii_case(extension) => head.to_string(),
            _ => cleaned,
        }
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Save this picture")
        .set_file_name(format!("{stem}.{extension}"))
        .add_filter("Image", &[extension])
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });

    let Some(path) = rx.await.ok().flatten().and_then(|p| p.into_path().ok()) else {
        // Cancelling is not a failure, and must not raise a toast saying it is.
        return json!({ "saved": false });
    };

    match std::fs::write(&path, bytes) {
        Ok(()) => json!({ "saved": true, "path": path.to_string_lossy() }),
        Err(error) => json!({ "saved": false, "error": error.to_string() }),
    }
}

/* -- running a snippet ---------------------------------------------------- */

/// The languages a code block can be run in, and what runs them.
///
/// A block of Python in a reply is as runnable as a shell command and stops
/// being useful for exactly the same reason: the reader has to save it
/// somewhere, remember which interpreter, and find the folder. So the snippet
/// is written to a temporary file and the interpreter is started on it, in the
/// conversation's folder, so a script that opens `./data.csv` finds it.
///
/// One difference from the Electron build worth naming: there, JavaScript and
/// TypeScript ran on Electron's own bundled Node, which is always present. This
/// shell bundles no runtime, so those two need `node` on the PATH like every
/// other language here. A machine without it gets no Run button, which is the
/// honest version of a button that would always fail.
const RUNNERS: &[(&str, &str, &[&str], &[&str])] = &[
    // (key, file extension, candidate programs, leading arguments)
    ("python", "py", &["python", "python3", "py"], &[]),
    ("javascript", "js", &["node"], &[]),
    ("typescript", "ts", &["node"], &["--experimental-strip-types"]),
    ("ruby", "rb", &["ruby"], &[]),
    ("php", "php", &["php"], &[]),
    ("perl", "pl", &["perl"], &[]),
    ("lua", "lua", &["lua"], &[]),
    ("go", "go", &["go"], &["run"]),
    ("r", "R", &["Rscript"], &[]),
];

const ALIASES: &[(&str, &str)] = &[
    ("py", "python"),
    ("python3", "python"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("node", "javascript"),
    ("jsx", "javascript"),
    ("ts", "typescript"),
    ("tsx", "typescript"),
    ("rb", "ruby"),
    ("pl", "perl"),
    ("golang", "go"),
    ("rscript", "r"),
];

/// Is this program on the PATH?
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![String::new()]
    };

    std::env::split_paths(&path).any(|dir| {
        extensions
            .iter()
            .any(|extension| dir.join(format!("{name}{extension}")).is_file())
    })
}

/// How to run this language here, or nothing.
///
/// Nothing is the ordinary answer - for a language with no runner, and for one
/// whose interpreter is not installed - and the block simply has no Run button,
/// which is the honest version of a button that would always fail.
fn runner_for(lang: &str) -> Option<(&'static str, &'static [&'static str], String)> {
    let name = lang.trim().to_lowercase();
    let key = ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, target)| *target)
        .unwrap_or(name.as_str());

    let (_, extension, candidates, lead) = RUNNERS.iter().find(|(k, ..)| *k == key)?;
    let program = candidates.iter().find(|candidate| on_path(candidate))?;
    Some((extension, lead, (*program).to_string()))
}

/// Whether a block in this language could be run here at all.
#[tauri::command]
pub fn app_can_run(lang: String) -> bool {
    runner_for(&lang).is_some()
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snippet {
    pub command: String,
    pub lang: Option<String>,
    pub cwd: Option<String>,
}

fn usable_directory(candidate: Option<&str>, fallback: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(wanted) = candidate.map(str::trim).filter(|s| !s.is_empty()) {
        let path = PathBuf::from(wanted);
        if path.is_absolute() && path.is_dir() {
            return Some(path);
        }
    }
    fallback.filter(|path| path.is_dir())
}

/// A command out of a code block, run because the person pressed run.
///
/// What makes this safe is not a check on the command - there is no useful one -
/// but who starts it: nothing here is ever called by a model. The agent's own
/// commands go through the `shell` tool, with its permission card and its
/// rules. This is a person clicking a button on a command they can see, and
/// then confirming it - the code is the model's, so one click is not enough.
///
/// So the guarantees are about not surprising them: it runs in the folder the
/// conversation is in, it stops at two minutes and says so rather than holding
/// a spinner forever, output is capped with the tail kept, and nothing runs
/// detached.
#[tauri::command]
pub async fn app_run_snippet(
    app: AppHandle,
    state: State<'_, AppState>,
    request: Snippet,
) -> Result<Value, String> {
    let text = request.command.trim().to_string();
    if text.is_empty() {
        return Ok(json!({ "ok": false, "error": "There is nothing to run." }));
    }

    let fallback = state.workspace().ok().map(|w| w.layout.work_dir());
    let Some(where_) = usable_directory(request.cwd.as_deref(), fallback) else {
        return Ok(json!({
            "ok": false,
            "error": "There is no folder to run this in. Choose a working folder for this agent first.",
        }));
    };

    // A language with a runner goes to a temporary file and its interpreter;
    // anything else is a command line for the shell.
    let lang = request.lang.clone().unwrap_or_default();
    let scripted = if lang.trim().is_empty() {
        None
    } else {
        runner_for(&lang)
    };

    let started = std::time::Instant::now();
    let (mut command, scratch) = match &scripted {
        Some((extension, lead, program)) => {
            let dir = app
                .path()
                .temp_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join(format!("inertia-snippet-{}", uuid::Uuid::now_v7()));
            if let Err(error) = std::fs::create_dir_all(&dir) {
                return Ok(json!({ "ok": false, "error": error.to_string(), "cwd": where_.to_string_lossy() }));
            }
            let file = dir.join(format!("snippet.{extension}"));
            if let Err(error) = std::fs::write(&file, &text) {
                let _ = std::fs::remove_dir_all(&dir);
                return Ok(json!({ "ok": false, "error": error.to_string(), "cwd": where_.to_string_lossy() }));
            }
            let mut command = tokio::process::Command::new(program);
            command.args(lead.iter()).arg(&file);
            (command, Some(dir))
        }
        None => (shell_command(&text), None),
    };

    command
        .current_dir(&where_)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // A command that stops to ask gets an immediate EOF rather than hanging
        // until the timeout with nobody there to type.
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat");
    #[cfg(windows)]
    {
        // Without this a console window flashes up for every snippet. Tokio's
        // Command exposes it directly, so no `CommandExt` import is needed.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let outcome = command.output();
    let finished = tokio::time::timeout(SNIPPET_TIMEOUT, outcome).await;

    if let Some(dir) = scratch {
        let _ = std::fs::remove_dir_all(dir);
    }

    let cwd = where_.to_string_lossy().to_string();
    let elapsed = started.elapsed().as_millis() as u64;

    match finished {
        Err(_) => Ok(json!({
            "ok": false, "code": Value::Null, "output": "", "truncated": false,
            "timedOut": true, "durationMs": elapsed, "cwd": cwd, "command": text,
        })),
        Ok(Err(error)) => Ok(json!({
            "ok": false, "error": error.to_string(), "cwd": cwd, "command": text,
            "durationMs": elapsed,
        })),
        Ok(Ok(output)) => {
            // One buffer for both streams: a script's error interleaved with
            // what it printed is what is worth reading, and two blocks would
            // put the failure a long way from what caused it. Ordering within
            // the run is lost - the pipes are read separately - so stderr
            // follows stdout rather than interleaving exactly.
            let mut text_out = String::from_utf8_lossy(&output.stdout).into_owned();
            text_out.push_str(&String::from_utf8_lossy(&output.stderr));

            let truncated = text_out.len() > MAX_OUTPUT;
            if truncated {
                // Sliced on a character boundary, so a multi-byte character cut
                // in half does not turn the tail into replacement characters.
                let start = text_out.len() - MAX_OUTPUT;
                let start = (start..text_out.len())
                    .find(|i| text_out.is_char_boundary(*i))
                    .unwrap_or(text_out.len());
                text_out = text_out[start..].to_string();
            }

            Ok(json!({
                "ok": output.status.success(),
                "code": output.status.code(),
                "output": text_out,
                "truncated": truncated,
                "timedOut": false,
                "durationMs": elapsed,
                "cwd": cwd,
                "command": text,
            }))
        }
    }
}

/// The shell, and the flag that makes it take a command line.
///
/// PowerShell reports its own exit code rather than the command's, so a failing
/// command is made to set one explicitly.
fn shell_command(command: &str) -> tokio::process::Command {
    if cfg!(windows) {
        let wrapped = format!(
            "{command}\nif ($LASTEXITCODE -ne $null -and $LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}"
        );
        let mut shell = tokio::process::Command::new("powershell.exe");
        shell.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &wrapped]);
        shell
    } else {
        let mut shell = tokio::process::Command::new("/bin/sh");
        shell.args(["-lc", command]);
        shell
    }
}

/// What to paste.
///
/// Through the backend rather than `navigator.clipboard`, which needs a user
/// gesture the paste handler does not always have and resolves to an empty
/// string rather than an error when it is refused.
#[tauri::command]
pub fn app_clipboard_read(app: AppHandle) -> String {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().read_text().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_are_public() {
        for private in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "100.64.0.1",
            "0.0.0.0", "255.255.255.255", "224.0.0.1", "::1", "::", "fe80::1", "fc00::1", "fd12::1",
            "ff02::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1", "::127.0.0.1", "64:ff9b::a9fe:a9fe",
            "2002:c0a8:0101::1",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in ["93.184.216.34", "1.1.1.1", "2606:4700::1111", "::ffff:93.184.216.34"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn a_url_naming_a_private_address_is_refused_however_it_is_written() {
        for url in [
            "http://127.0.0.1/x.png",
            "http://2130706433/x.png",
            "http://0x7f.1/x.png",
            "http://[::1]/x.png",
            "http://[::ffff:169.254.169.254]/latest",
            "file:///C:/work/project/x.png",
            "ftp://example.com/x.png",
        ] {
            let parsed = reqwest::Url::parse(url).unwrap();
            assert!(!reaches_only_the_internet(&parsed), "{url}");
        }
        let named = reqwest::Url::parse("https://example.com/x.png").unwrap();
        assert!(reaches_only_the_internet(&named));
    }

    /// A name is judged by what it resolves to, so `localhost` never gets as
    /// far as a connection.
    #[tokio::test]
    async fn a_name_that_resolves_to_this_machine_is_not_fetched() {
        assert!(app_fetch_image("http://localhost:9/x.png".into()).await.is_none());
        assert!(app_fetch_image("http://127.0.0.1:9/x.png".into()).await.is_none());
        let resolved = reqwest::dns::Resolve::resolve(&PublicOnly, "localhost".parse().unwrap()).await;
        assert!(resolved.is_err());
    }

    #[test]
    fn a_data_url_splits_into_type_and_bytes() {
        // "hi" in base64.
        let (mime, bytes) = decode_data_url("data:image/png;base64,aGk=").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, b"hi");
    }

    #[test]
    fn a_url_that_is_not_base64_is_refused() {
        assert!(decode_data_url("data:image/png,raw").is_none());
        assert!(decode_data_url("https://example.com/x.png").is_none());
        assert!(decode_data_url("not a url").is_none());
    }

    #[test]
    fn extensions_follow_the_declared_type() {
        assert_eq!(extension_for("image/jpeg"), "jpg");
        assert_eq!(extension_for("image/svg+xml"), "svg");
        // Anything unrecognised is written as a PNG rather than refused: the
        // bytes are already decoded and a file with a wrong extension is more
        // useful than no file.
        assert_eq!(extension_for("application/octet-stream"), "png");
    }

    #[test]
    fn aliases_resolve_to_their_language() {
        // Only the mapping is asserted, not the lookup: whether `node` is on
        // this machine's PATH is not a property of the code.
        for (alias, target) in ALIASES {
            assert!(
                RUNNERS.iter().any(|(key, ..)| key == target),
                "{alias} maps to {target}, which has no runner"
            );
        }
    }

    #[test]
    fn a_language_with_no_runner_cannot_be_run() {
        assert!(runner_for("brainfuck").is_none());
        assert!(runner_for("").is_none());
    }

    #[test]
    fn a_relative_directory_is_refused() {
        assert!(usable_directory(Some("./somewhere"), None).is_none());
        assert!(usable_directory(Some(""), None).is_none());
    }

    #[test]
    fn an_absolute_directory_that_exists_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        assert_eq!(usable_directory(Some(&path), None).unwrap(), dir.path());
    }

    #[test]
    fn the_fallback_is_used_when_the_first_choice_is_not_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let answer = usable_directory(Some("/nope/not/here"), Some(dir.path().to_path_buf()));
        assert_eq!(answer.unwrap(), dir.path());
    }
}

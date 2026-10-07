//! Bringing a signed-in session from this computer into a browser the app owns.
//!
//! Two destinations, one source. The reading half is [`inertia_cookies`]; this
//! is the two places the cookies go and the commands the window calls.
//!
//! ## The browser pane
//!
//! Every pane is a child webview of this window, and they share one cookie
//! store - so the import is for the pane rather than for a tab, and any live
//! webview will do to reach it. WebView2's own cookie manager takes each row,
//! `httpOnly` and all, which is why nothing here writes a SQLite file.
//!
//! ## A machine's browser
//!
//! That one is a container with its own disk, so the cookies travel as a JSON
//! payload beside a short Python program that writes them into Chromium's own
//! store. See [`inertia_cookies::inject`] for why that is the shape, and why
//! the payload deletes itself.
//!
//! ## What this is not
//!
//! It is not a tool. An agent cannot call any of this and cannot ask for it.
//! Handing an agent the ability to move somebody's live sessions into a place
//! it controls is a different feature with a different risk, and nobody asked
//! for that one. These are buttons, in two dialogs, and nothing else.

use inertia_computers::ExecRequest;
use serde_json::{json, Value};
use tauri::State;

use crate::state::AppState;

/// The browser profiles on this computer, with how many cookies each holds.
///
/// Reading no values: the count is `SELECT COUNT(*)`, because the list is drawn
/// before anybody has agreed to anything and decrypting eight hundred cookies
/// to label a row would be reading them in order to ask whether they may be
/// read.
#[tauri::command]
pub async fn cookie_sources() -> Result<Vec<Value>, String> {
    // Off the UI thread: each profile is a file copy and a SQLite open, and on
    // a machine with six browsers installed that is long enough to notice.
    tokio::task::spawn_blocking(|| {
        inertia_cookies::sources()
            .into_iter()
            .map(|source| serde_json::to_value(source).unwrap_or(Value::Null))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// What `sourceId` and `domains` arrive as from either dialog.
fn asked(options: &Value) -> (String, Vec<String>) {
    let source_id = options
        .get("sourceId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let domains = match options.get("domains") {
        Some(Value::String(text)) => inertia_cookies::read::parse_domains(text),
        Some(Value::Array(rows)) => rows
            .iter()
            .filter_map(Value::as_str)
            .flat_map(inertia_cookies::read::parse_domains)
            .collect(),
        _ => Vec::new(),
    };
    (source_id, domains)
}

/// Read one profile, off the runtime thread.
async fn harvest(
    source_id: &str,
    domains: Vec<String>,
) -> Result<(inertia_cookies::Source, inertia_cookies::Harvest), String> {
    let source = inertia_cookies::source_by_id(source_id)
        .ok_or_else(|| "That browser profile is no longer on this computer.".to_string())?;
    let reading = source.clone();
    let found = tokio::task::spawn_blocking(move || {
        let now = jiff::Timestamp::now().as_millisecond();
        inertia_cookies::read::read_profile(&reading, &domains, now)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok((source, found))
}

/// The half of the answer both destinations share.
fn summary(source: &inertia_cookies::Source, found: &inertia_cookies::Harvest) -> Value {
    json!({
        "from": { "browser": source.browser, "profile": source.profile },
        "skipped": found.skipped,
        "reasons": found.reasons,
    })
}

/// Put a profile's cookies into the browser pane beside the chat.
///
/// One at a time, because the cookie manager refuses individually - a
/// `__Host-` cookie carrying a domain, a value it will not store - and a
/// refusal has to be a count with a reason rather than the whole import
/// failing on the first odd row.
#[tauri::command]
pub async fn preview_import_cookies(options: Value) -> Result<Value, String> {
    let (source_id, domains) = asked(&options);
    let (source, found) = harvest(&source_id, domains).await?;
    let mut answer = summary(&source, &found);

    if found.cookies.is_empty() {
        answer["ok"] = json!(false);
        answer["imported"] = json!(0);
        answer["reason"] = json!("nothing-readable");
        return Ok(answer);
    }

    // Any live pane reaches the store; they all share one. A pane has to exist,
    // though, and the dialog is only reachable from inside one.
    let panes = crate::preview::Panes::global();
    let webview = panes.any_webview().ok_or_else(|| {
        "The browser pane is not open, so there is nothing to sign in.".to_string()
    })?;

    let mut imported = 0u32;
    let mut refused: std::collections::BTreeMap<String, u32> = Default::default();

    for cookie in &found.cookies {
        match shape(cookie) {
            Some(shaped) => match webview.set_cookie(shaped) {
                Ok(()) => imported += 1,
                Err(error) => {
                    let why = error.to_string();
                    *refused.entry(why.chars().take(120).collect()).or_insert(0) += 1;
                }
            },
            None => {
                *refused
                    .entry("the store gave it no host".to_string())
                    .or_insert(0) += 1;
            }
        }
    }

    let declined: u32 = refused.values().sum();
    answer["ok"] = json!(imported > 0);
    answer["imported"] = json!(imported);
    answer["skipped"]["unreadable"] = json!(found.skipped.unreadable + declined);

    let mut reasons = found.reasons.clone();
    reasons.extend(
        refused
            .into_iter()
            .map(|(reason, count)| inertia_cookies::Reason { reason, count }),
    );
    reasons.sort_by_key(|reason| std::cmp::Reverse(reason.count));
    answer["reasons"] = serde_json::to_value(reasons).unwrap_or(Value::Null);
    if imported == 0 {
        answer["reason"] = json!("nothing-accepted");
    }

    Ok(answer)
}

/// One stored cookie as the shape the webview's cookie manager takes.
///
/// A host beginning with a dot is a domain cookie and is passed as one; a bare
/// host is host-only and is passed without a domain, because supplying one is
/// exactly what turns a host-only cookie into a domain cookie.
fn shape(cookie: &inertia_cookies::Cookie) -> Option<tauri::webview::cookie::Cookie<'static>> {
    use tauri::webview::cookie::{time::OffsetDateTime, Cookie, SameSite};

    let host = cookie.host.trim();
    let bare = host.trim_start_matches('.');
    if bare.is_empty() {
        return None;
    }

    let mut built = Cookie::new(cookie.name.clone(), cookie.value.clone());
    built.set_domain(if host.starts_with('.') {
        host.to_string()
    } else {
        bare.to_string()
    });
    built.set_path(if cookie.path.starts_with('/') {
        cookie.path.clone()
    } else {
        format!("/{}", cookie.path)
    });
    built.set_secure(cookie.secure);
    built.set_http_only(cookie.http_only);
    built.set_same_site(match cookie.same_site.as_deref() {
        Some("none") => Some(SameSite::None),
        Some("lax") => Some(SameSite::Lax),
        Some("strict") => Some(SameSite::Strict),
        _ => None,
    });
    if cookie.expires_at > 0 {
        if let Ok(when) = OffsetDateTime::from_unix_timestamp(cookie.expires_at) {
            built.set_expires(when);
        }
    }
    Some(built.into_owned())
}

/// Put a profile's cookies into a machine's browser.
#[tauri::command]
pub async fn computer_import_cookies(
    state: State<'_, AppState>,
    id: String,
    options: Value,
) -> Result<Value, String> {
    use inertia_cookies::inject;

    let (source_id, domains) = asked(&options);
    let (source, found) = harvest(&source_id, domains).await?;
    let mut answer = summary(&source, &found);
    answer["imported"] = json!(0);

    if found.cookies.is_empty() {
        answer["ok"] = json!(false);
        answer["reason"] = json!("nothing-readable");
        return Ok(answer);
    }

    let (provider, record) = crate::computers::running_machine(&state, &id)?;
    let handle = crate::computers::handle_of(&record);

    let payload =
        serde_json::to_string(&json!({ "cookies": found.cookies })).map_err(|e| e.to_string())?;

    provider
        .write_file(&handle, inject::SCRIPT, inject::PROGRAM)
        .await
        .map_err(|e| e.to_string())?;
    provider
        .write_file(&handle, inject::PAYLOAD, &payload)
        .await
        .map_err(|e| e.to_string())?;

    let result = provider
        .exec(
            &handle,
            &ExecRequest {
                // The script removes the payload itself, in a `finally`; the
                // `rm` is for the program beside it and for the run that never
                // reached Python at all.
                command: format!(
                    "python3 {script} {payload}; rm -f {script} {payload}",
                    script = inject::SCRIPT,
                    payload = inject::PAYLOAD
                ),
                timeout: Some(std::time::Duration::from_secs(180)),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    let Some(said) = inject::parse_result(&result.stdout) else {
        let spoken = [result.stdout.as_str(), result.stderr.as_str()]
            .iter()
            .filter(|text| !text.trim().is_empty())
            .map(|text| text.trim())
            .collect::<Vec<_>>()
            .join("\n");
        let tail: String = spoken.lines().rev().take(3).collect::<Vec<_>>().join(" ");
        return Err(if tail.trim().is_empty() {
            "The machine did not say what happened to the cookies.".to_string()
        } else {
            tail.chars().take(300).collect()
        });
    };

    if said.get("ok").and_then(Value::as_bool) != Some(true) {
        answer["ok"] = json!(false);
        answer["reason"] = said
            .get("reason")
            .cloned()
            .unwrap_or_else(|| json!("failed"));
        return Ok(answer);
    }

    answer["ok"] = json!(true);
    answer["imported"] = said
        .get("written")
        .cloned()
        .unwrap_or_else(|| json!(found.cookies.len()));
    answer["failed"] = said.get("failed").cloned().unwrap_or_else(|| json!(0));
    answer["profiles"] = said.get("profiles").cloned().unwrap_or_else(|| json!(1));
    answer["browserClosed"] = said
        .get("browserClosed")
        .cloned()
        .unwrap_or_else(|| json!(false));
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(host: &str, same_site: Option<&str>) -> inertia_cookies::Cookie {
        inertia_cookies::Cookie {
            host: host.into(),
            name: "sid".into(),
            value: "abc".into(),
            path: "/".into(),
            expires_at: 1_788_000_000,
            secure: true,
            http_only: true,
            same_site: same_site.map(str::to_string),
        }
    }

    /// The leading dot is the difference between a cookie for one host and a
    /// cookie for a whole domain, and it has to survive the conversion.
    #[test]
    fn a_domain_cookie_keeps_its_dot_and_a_host_cookie_does_not_gain_one() {
        let domain = shape(&cookie(".example.com", Some("lax"))).expect("a cookie");
        assert_eq!(domain.domain(), Some("example.com"));

        let host_only = shape(&cookie("example.com", None)).expect("a cookie");
        assert_eq!(host_only.domain(), Some("example.com"));
    }

    #[test]
    fn a_row_with_no_host_is_refused_rather_than_guessed_at() {
        assert!(shape(&cookie("", None)).is_none());
        assert!(shape(&cookie(".", None)).is_none());
    }

    #[test]
    fn the_flags_that_make_a_session_work_are_carried() {
        use tauri::webview::cookie::SameSite;
        let built = shape(&cookie(".example.com", Some("strict"))).expect("a cookie");
        assert_eq!(built.secure(), Some(true));
        assert_eq!(
            built.http_only(),
            Some(true),
            "an httpOnly session cookie is the whole point"
        );
        assert_eq!(built.same_site(), Some(SameSite::Strict));
        assert!(built.expires().is_some());
    }

    #[test]
    fn a_same_site_the_store_did_not_say_is_left_unstated() {
        assert_eq!(
            shape(&cookie(".example.com", None))
                .expect("a cookie")
                .same_site(),
            None
        );
    }

    /// What the dialog sends, in both of the shapes it can send it.
    #[test]
    fn the_domain_filter_is_read_from_text_or_from_a_list() {
        let (id, domains) =
            asked(&json!({ "sourceId": "chrome:.", "domains": "github.com, .example.com" }));
        assert_eq!(id, "chrome:.");
        assert_eq!(domains, vec!["github.com", "example.com"]);

        let (_, listed) = asked(&json!({ "domains": ["github.com", "example.com"] }));
        assert_eq!(listed, vec!["github.com", "example.com"]);

        let (_, none) = asked(&json!({ "sourceId": "chrome:." }));
        assert!(none.is_empty(), "no filter means everything");
    }
}

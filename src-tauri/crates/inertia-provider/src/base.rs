//! What a provider's base URL means, and what to do when it is not quite one.
//!
//! People paste three things into the field: the base the docs give
//! (`https://api.groq.com/openai/v1`), the base with the route on the end,
//! straight out of a curl example (`.../v1/messages`), and the bare mount of a
//! gateway with no version in it (`https://api.minimax.io/anthropic`). The
//! first was the only one that worked. The second sent the route twice. The
//! third sent `/anthropic/messages`, which nobody serves, and reported the
//! provider as broken when the provider was one path segment away.
//!
//! So the base is read, not used. A route on the end comes off. A path with no
//! `/v1` gets one - that is where every endpoint this app has met keeps its
//! routes - and, because a gateway is free to disagree, the bare path is kept
//! as a second candidate and tried once if the first answers 404. Which one
//! answered is remembered against the address so no later request pays for the
//! lesson again. [`forget`] is the Test button: something changed, learn
//! afresh.
//!
//! Both transports use this, which is the point. The two clients had their own
//! copies of the same six lines and the same six lines had the same bug.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The routes either protocol adds itself, so one on the base is dropped.
const ROUTE_ON_THE_END: &[&str] = &["chat/completions", "completions", "messages", "models"];

/// Which candidate an address is on, by the address as the user typed it.
///
/// Module-level rather than a field on the config, because a config is built
/// fresh for every turn and the lesson has to outlive it. Keyed by the raw
/// string so two records pointing at the same gateway share what was learned.
fn chosen() -> &'static Mutex<HashMap<String, usize>> {
    static CHOSEN: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    CHOSEN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A versioned path, which is where the routes are and needs no help.
fn is_versioned(path: &str) -> bool {
    let Some(last) = path.rsplit('/').next() else {
        return false;
    };
    let Some(digits) = last.strip_prefix('v') else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// Splits an address into its origin and the path the user meant.
///
/// Hand-parsed rather than pulled through a URL crate: the whole job is three
/// string operations, and the interesting behaviour - tolerating a missing
/// scheme, dropping a route someone pasted - is ours either way.
fn parse(raw: &str) -> Option<(String, String)> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }

    let (scheme, rest) = match text.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => ("http", rest),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => ("https", rest),
        // A scheme we do not speak is not a base URL at all.
        Some(_) => return None,
        None => ("https", text),
    };

    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, path),
        None => (rest, ""),
    };
    if host.is_empty() {
        return None;
    }

    let mut path = path.trim_end_matches('/').to_string();
    for route in ROUTE_ON_THE_END {
        // Compared case-insensitively on the last segment only, so a host
        // path that merely contains "models" keeps it.
        if path.len() >= route.len() {
            let tail = &path[path.len() - route.len()..];
            let boundary =
                path.len() == route.len() || path.as_bytes()[path.len() - route.len() - 1] == b'/';
            if boundary && tail.eq_ignore_ascii_case(route) {
                path.truncate(path.len() - route.len());
                path = path.trim_end_matches('/').to_string();
                break;
            }
        }
    }

    let path = if path.is_empty() {
        String::new()
    } else {
        format!("/{}", path.trim_start_matches('/'))
    };

    Some((format!("{scheme}://{host}"), path))
}

/// The bases worth trying for this address, best first.
///
/// A versioned path, or no path at all, has one answer. A path without a
/// version has two: with `/v1` added, then as written.
pub fn candidates(base_url: &str) -> Vec<String> {
    let Some((origin, path)) = parse(base_url) else {
        return Vec::new();
    };
    if path.is_empty() {
        return vec![format!("{origin}/v1")];
    }
    if is_versioned(&path) {
        return vec![format!("{origin}{path}")];
    }
    vec![format!("{origin}{path}/v1"), format!("{origin}{path}")]
}

/// The base in use for this address right now.
pub fn base_of(base_url: &str) -> String {
    let list = candidates(base_url);
    if list.is_empty() {
        return base_url.trim_end_matches('/').to_string();
    }
    let index = chosen()
        .lock()
        .ok()
        .and_then(|map| map.get(base_url.trim()).copied())
        .unwrap_or(0);
    list[index.min(list.len() - 1)].clone()
}

/// The full URL for one route.
pub fn endpoint(base_url: &str, route: &str) -> String {
    format!("{}/{}", base_of(base_url), route.trim_start_matches('/'))
}

/// Moves to the next candidate after a 404. Returns whether there was one.
///
/// Only ever forward: a base that answered is not given up because a later
/// request to it failed for some other reason, and the caller only asks after
/// a plain "nothing is here".
pub fn advance(base_url: &str) -> bool {
    let list = candidates(base_url);
    let Ok(mut map) = chosen().lock() else {
        return false;
    };
    let key = base_url.trim().to_string();
    let index = map.get(&key).copied().unwrap_or(0);
    if index + 1 >= list.len() {
        return false;
    }
    map.insert(key, index + 1);
    true
}

/// Whether a 404 on this address is worth another try at a different base.
pub fn can_advance(base_url: &str) -> bool {
    let list = candidates(base_url);
    let index = chosen()
        .lock()
        .ok()
        .and_then(|map| map.get(base_url.trim()).copied())
        .unwrap_or(0);
    index + 1 < list.len()
}

/// Throws away what was learned. This is the Test button: something changed.
pub fn forget(base_url: Option<&str>) {
    let Ok(mut map) = chosen().lock() else {
        return;
    };
    match base_url {
        Some(url) => {
            map.remove(url.trim());
        }
        None => map.clear(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three things people actually paste.
    #[test]
    fn a_versioned_base_is_used_as_written() {
        assert_eq!(
            candidates("https://api.groq.com/openai/v1"),
            ["https://api.groq.com/openai/v1"]
        );
    }

    #[test]
    fn a_route_pasted_from_a_curl_example_comes_off() {
        assert_eq!(
            candidates("https://api.groq.com/openai/v1/chat/completions"),
            ["https://api.groq.com/openai/v1"]
        );
        assert_eq!(
            candidates("https://api.anthropic.com/v1/messages"),
            ["https://api.anthropic.com/v1"]
        );
    }

    /// The case that was reported: a gateway mounted with no version in it.
    /// `/anthropic/messages` is served by nobody, and the provider was one
    /// path segment away from working.
    #[test]
    fn a_gateway_mount_tries_v1_first_then_the_bare_path() {
        assert_eq!(
            candidates("https://api.minimax.io/anthropic"),
            [
                "https://api.minimax.io/anthropic/v1",
                "https://api.minimax.io/anthropic"
            ]
        );
    }

    #[test]
    fn a_bare_host_gets_v1() {
        assert_eq!(
            candidates("https://api.openai.com"),
            ["https://api.openai.com/v1"]
        );
        assert_eq!(
            candidates("https://api.openai.com/"),
            ["https://api.openai.com/v1"]
        );
    }

    #[test]
    fn a_missing_scheme_is_assumed_to_be_https() {
        assert_eq!(
            candidates("api.openai.com/v1"),
            ["https://api.openai.com/v1"]
        );
    }

    #[test]
    fn a_local_runtime_keeps_its_scheme_and_port() {
        assert_eq!(
            candidates("http://localhost:11434/v1"),
            ["http://localhost:11434/v1"]
        );
    }

    #[test]
    fn a_higher_version_counts_as_versioned() {
        assert_eq!(candidates("https://x.dev/api/v2"), ["https://x.dev/api/v2"]);
    }

    /// A path segment that merely contains a route word is not a route.
    #[test]
    fn only_a_whole_trailing_segment_is_stripped() {
        assert_eq!(
            candidates("https://x.dev/mymodels"),
            ["https://x.dev/mymodels/v1", "https://x.dev/mymodels"]
        );
    }

    #[test]
    fn nonsense_has_no_candidates() {
        assert!(candidates("").is_empty());
        assert!(candidates("   ").is_empty());
        assert!(candidates("ftp://x.dev").is_empty());
    }

    #[test]
    fn the_endpoint_is_the_base_plus_the_route() {
        forget(Some("https://api.minimax.io/anthropic"));
        assert_eq!(
            endpoint("https://api.minimax.io/anthropic", "messages"),
            "https://api.minimax.io/anthropic/v1/messages"
        );
    }

    /// After a 404 the second candidate is used, and stays used.
    #[test]
    fn advancing_settles_on_the_next_candidate() {
        let url = "https://gateway.test/advancing";
        forget(Some(url));

        assert_eq!(
            endpoint(url, "messages"),
            "https://gateway.test/advancing/v1/messages"
        );
        assert!(can_advance(url));
        assert!(advance(url));
        assert_eq!(
            endpoint(url, "messages"),
            "https://gateway.test/advancing/messages"
        );

        // And there is nowhere further to go.
        assert!(!can_advance(url));
        assert!(!advance(url));
        forget(Some(url));
    }

    #[test]
    fn a_single_candidate_never_advances() {
        let url = "https://api.groq.com/openai/v1";
        forget(Some(url));
        assert!(!can_advance(url));
        assert!(!advance(url));
    }

    #[test]
    fn forgetting_returns_to_the_first_candidate() {
        let url = "https://gateway.test/forgetting";
        forget(Some(url));
        advance(url);
        assert_eq!(
            endpoint(url, "messages"),
            "https://gateway.test/forgetting/messages"
        );
        forget(Some(url));
        assert_eq!(
            endpoint(url, "messages"),
            "https://gateway.test/forgetting/v1/messages"
        );
    }
}

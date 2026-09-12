//! Reading a browser's cookie database.
//!
//! ## Always a copy
//!
//! The file is copied to a temporary folder and read there, with its
//! write-ahead log beside it. Three reasons, and each one has bitten somebody.
//! A running browser holds the database open, and on Windows that is often
//! enough to make opening it fail. A database with a WAL has its newest rows -
//! this morning's logins - in the log rather than in the file, so a reader that
//! takes only the main file gets a stale answer. And opening somebody's live
//! cookie store read-write to let SQLite replay that log would put this app's
//! name on any corruption that followed. A copy has none of those problems.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::decrypt::{self, Key};
use crate::{Cookie, CookieError, Harvest, Reason, Result, Skipped, Source};

/// Chromium's epoch is the start of 1601, in seconds before the Unix one.
pub const CHROME_EPOCH_SECONDS: i64 = 11_644_473_600;

fn failed(message: impl std::fmt::Display) -> CookieError {
    CookieError::Failed(message.to_string())
}

/// The database and everything SQLite needs to make sense of it, in a folder of
/// our own. The folder is removed when the returned value is dropped.
struct Aside {
    dir: tempfile::TempDir,
    file: PathBuf,
}

impl Aside {
    fn new(file: &Path, label: &str) -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("inertia-cookies-")
            .tempdir()
            .map_err(failed)?;
        let name = file.file_name().unwrap_or_default();
        let target = dir.path().join(name);

        std::fs::copy(file, &target).map_err(|error| {
            // Every locked-file errno, in the one sentence that can be acted
            // on. A copy of the OS error names a temp path and helps nobody.
            failed(format!(
                "{label} has its cookie store locked, so it cannot be read while it is running. \
Close it completely - on Windows that includes whatever it leaves in the notification area - \
and try again. ({error})"
            ))
        })?;

        for suffix in ["-wal", "-shm", "-journal"] {
            let from = PathBuf::from(format!("{}{suffix}", file.display()));
            let to = PathBuf::from(format!("{}{suffix}", target.display()));
            // Absent is the normal case for two of the three, and a log that
            // could not be taken is not worth failing over: the main file
            // still reads, just without whatever was written since the last
            // checkpoint.
            let _ = std::fs::copy(&from, &to);
        }

        Ok(Self { dir, file: target })
    }

    fn open(&self) -> Result<rusqlite::Connection> {
        let _ = &self.dir;
        rusqlite::Connection::open(&self.file).map_err(failed)
    }
}

/// `Www.Example.com, .example.com` - what a person types, as domains.
pub fn parse_domains(text: &str) -> Vec<String> {
    text.split([' ', '\t', '\n', '\r', ',', ';'])
        .map(|part| {
            part.trim()
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches('.')
                .split('/')
                .next()
                .unwrap_or_default()
                .to_lowercase()
        })
        .filter(|part| !part.is_empty())
        .collect()
}

/// Does this cookie belong to one of the domains the person asked for?
pub fn matches_domains(host: &str, domains: &[String]) -> bool {
    if domains.is_empty() {
        return true;
    }
    let bare = host.trim_start_matches('.').to_lowercase();
    domains
        .iter()
        .any(|domain| bare == *domain || bare.ends_with(&format!(".{domain}")))
}

/// The same-site column, as a word. Both families use the same three numbers.
fn same_site(value: i64) -> Option<String> {
    Some(
        match value {
            0 => "none",
            1 => "lax",
            2 => "strict",
            _ => return None,
        }
        .to_string(),
    )
}

/// How many cookies a profile holds, without opening a single one.
pub fn count(source: &Source) -> Option<i64> {
    let aside = Aside::new(&source.file, &source.browser).ok()?;
    let db = aside.open().ok()?;
    let table = if source.family == "chromium" { "cookies" } else { "moz_cookies" };
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
        .ok()
}

/// Every usable cookie in a profile, with a tally of the ones that were not.
///
/// A session cookie is left behind on purpose: it is only in the file at all
/// because the browser was asked to restore its last session, and it means
/// nothing to a browser that was not part of that session.
pub fn read_profile(source: &Source, domains: &[String], now_ms: i64) -> Result<Harvest> {
    let chromium = source.family == "chromium";
    let key = if chromium { decrypt::key_for(source) } else { Key::None };
    let aside = Aside::new(&source.file, &source.browser)?;
    let db = aside.open()?;

    let mut cookies = Vec::new();
    let mut skipped = Skipped::default();
    let mut reasons: BTreeMap<String, u32> = BTreeMap::new();

    let sql = if chromium {
        "SELECT host_key, name, value, encrypted_value, path, expires_utc, is_secure, \
         is_httponly, samesite, is_persistent FROM cookies"
    } else {
        "SELECT host, name, value, NULL, path, expiry, isSecure, isHttpOnly, sameSite, \
         1 FROM moz_cookies WHERE originAttributes = ''"
    };

    let mut statement = db.prepare(sql).map_err(|error| {
        failed(format!("That cookie file is not the shape this reads ({error})."))
    })?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0).unwrap_or_default(),
                row.get::<_, String>(1).unwrap_or_default(),
                row.get::<_, String>(2).unwrap_or_default(),
                row.get::<_, Vec<u8>>(3).unwrap_or_default(),
                row.get::<_, String>(4).unwrap_or_default(),
                row.get::<_, i64>(5).unwrap_or_default(),
                row.get::<_, i64>(6).unwrap_or_default(),
                row.get::<_, i64>(7).unwrap_or_default(),
                row.get::<_, i64>(8).unwrap_or(-1),
                row.get::<_, i64>(9).unwrap_or(1),
            ))
        })
        .map_err(failed)?;

    for row in rows.flatten() {
        let (host, name, plain, blob, path, expires, secure, http_only, site, persistent) = row;
        if host.is_empty() {
            continue;
        }

        let expires_at = if chromium {
            expires.div_euclid(1_000_000) - CHROME_EPOCH_SECONDS
        } else {
            expires
        };
        let persistent = if chromium { persistent == 1 } else { expires_at > 0 };

        if !persistent || expires_at == 0 {
            skipped.session += 1;
            continue;
        }
        if expires_at.saturating_mul(1000) <= now_ms {
            skipped.expired += 1;
            continue;
        }
        if !matches_domains(&host, domains) {
            skipped.filtered += 1;
            continue;
        }

        let value = match decrypt::decrypt_value(&blob, &plain, &host, &key) {
            Ok(value) => value,
            Err(why) => {
                skipped.unreadable += 1;
                *reasons.entry(why).or_insert(0) += 1;
                continue;
            }
        };

        cookies.push(Cookie {
            host,
            name,
            value,
            path: if path.is_empty() { "/".into() } else { path },
            expires_at,
            secure: secure == 1,
            http_only: http_only == 1,
            same_site: same_site(site),
        });
    }

    let mut reasons: Vec<Reason> = reasons
        .into_iter()
        .map(|(reason, count)| Reason { reason, count })
        .collect();
    // Most common first: the one sentence worth reading is the one that
    // explains most of what was left behind.
    reasons.sort_by_key(|reason| std::cmp::Reverse(reason.count));

    Ok(Harvest { cookies, skipped, reasons })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_person_types_becomes_domains() {
        assert_eq!(
            parse_domains("Www.Example.com, .example.com; https://github.com/some/path"),
            vec!["www.example.com", "example.com", "github.com"]
        );
        assert!(parse_domains("   ").is_empty());
    }

    /// A domain filter matches the domain and everything under it, and nothing
    /// that merely ends with the same letters.
    #[test]
    fn a_domain_covers_its_subdomains_and_not_its_lookalikes() {
        let wanted = vec!["example.com".to_string()];
        assert!(matches_domains("example.com", &wanted));
        assert!(matches_domains(".example.com", &wanted));
        assert!(matches_domains("mail.example.com", &wanted));
        assert!(!matches_domains("notexample.com", &wanted));
        assert!(!matches_domains("example.com.evil.net", &wanted));
    }

    #[test]
    fn no_filter_means_everything() {
        assert!(matches_domains("anything.at.all", &[]));
    }

    /// Chromium counts microseconds from 1601. Getting this wrong reads every
    /// live cookie as expired, which looks exactly like a profile with nothing
    /// in it.
    #[test]
    fn chromiums_epoch_converts_to_the_unix_one() {
        // 2026-09-04T20:00:00Z in Chromium's units.
        let chrome = (1_788_000_000 + CHROME_EPOCH_SECONDS) * 1_000_000;
        assert_eq!(chrome.div_euclid(1_000_000) - CHROME_EPOCH_SECONDS, 1_788_000_000);
    }

    #[test]
    fn the_same_site_column_becomes_a_word_or_nothing() {
        assert_eq!(same_site(0).as_deref(), Some("none"));
        assert_eq!(same_site(1).as_deref(), Some("lax"));
        assert_eq!(same_site(2).as_deref(), Some("strict"));
        assert_eq!(same_site(-1), None);
    }

    /// The whole read, against a database shaped like Chromium's.
    #[test]
    fn a_chromium_store_is_read_and_sieved() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("Cookies");
        {
            let db = rusqlite::Connection::open(&file).expect("a database");
            db.execute_batch(
                "CREATE TABLE cookies (host_key TEXT, name TEXT, value TEXT, \
                 encrypted_value BLOB, path TEXT, expires_utc INTEGER, is_secure INTEGER, \
                 is_httponly INTEGER, samesite INTEGER, is_persistent INTEGER);",
            )
            .expect("the schema");

            let live = (1_788_000_000 + CHROME_EPOCH_SECONDS) * 1_000_000;
            let gone = (1_000_000_000 + CHROME_EPOCH_SECONDS) * 1_000_000;
            let mut add = db.prepare("INSERT INTO cookies VALUES (?,?,?,?,?,?,?,?,?,?)").expect("insert");
            add.execute(rusqlite::params![".example.com", "sid", "abc", Vec::<u8>::new(), "/", live, 1, 1, 1, 1]).expect("live");
            add.execute(rusqlite::params!["old.example.com", "old", "x", Vec::<u8>::new(), "/", gone, 0, 0, -1, 1]).expect("expired");
            add.execute(rusqlite::params!["other.net", "them", "y", Vec::<u8>::new(), "/", live, 0, 0, -1, 1]).expect("other");
            add.execute(rusqlite::params!["s.example.com", "tmp", "z", Vec::<u8>::new(), "/", 0, 0, 0, -1, 0]).expect("session");
        }

        let source = Source {
            id: "chrome:.".into(),
            browser_id: "chrome".into(),
            browser: "Google Chrome".into(),
            family: "chromium".into(),
            profile: "Default".into(),
            file,
            local_state: Default::default(),
            bytes: 0,
            updated_at: None,
            cookies: None,
        };

        let wanted = vec!["example.com".to_string()];
        let found = read_profile(&source, &wanted, 1_700_000_000_000).expect("a read");

        assert_eq!(found.cookies.len(), 1);
        assert_eq!(found.cookies[0].name, "sid");
        assert_eq!(found.cookies[0].value, "abc");
        assert_eq!(found.cookies[0].host, ".example.com");
        assert!(found.cookies[0].secure);
        assert_eq!(found.cookies[0].same_site.as_deref(), Some("lax"));

        assert_eq!(found.skipped.expired, 1, "the 2001 cookie");
        assert_eq!(found.skipped.session, 1, "the one with no expiry");
        assert_eq!(found.skipped.filtered, 1, "other.net");

        assert_eq!(count(&source), Some(4), "counting reads no values");
    }

    /// Firefox keeps container tabs in the same table, told apart by their
    /// origin attributes. A cookie from a container is not the cookie the
    /// person browsing normally is signed in with.
    #[test]
    fn firefox_containers_are_left_alone() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let file = dir.path().join("cookies.sqlite");
        {
            let db = rusqlite::Connection::open(&file).expect("a database");
            db.execute_batch(
                "CREATE TABLE moz_cookies (host TEXT, name TEXT, value TEXT, path TEXT, \
                 expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, sameSite INTEGER, \
                 originAttributes TEXT);",
            )
            .expect("the schema");
            let mut add = db
                .prepare("INSERT INTO moz_cookies VALUES (?,?,?,?,?,?,?,?,?)")
                .expect("insert");
            add.execute(rusqlite::params!["example.com", "sid", "plain", "/", 1_788_000_000i64, 1, 0, 1, ""]).expect("ordinary");
            add.execute(rusqlite::params!["example.com", "sid", "walled", "/", 1_788_000_000i64, 1, 0, 1, "^userContextId=2"]).expect("contained");
        }

        let source = Source {
            id: "firefox:p".into(),
            browser_id: "firefox".into(),
            browser: "Firefox".into(),
            family: "firefox".into(),
            profile: "default".into(),
            file,
            local_state: Default::default(),
            bytes: 0,
            updated_at: None,
            cookies: None,
        };

        let found = read_profile(&source, &[], 1_700_000_000_000).expect("a read");
        assert_eq!(found.cookies.len(), 1);
        assert_eq!(found.cookies[0].value, "plain", "not the container's");
    }
}

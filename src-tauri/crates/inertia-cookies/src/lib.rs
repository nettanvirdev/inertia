//! Bringing a signed-in session from this computer somewhere else.
//!
//! An agent driving a browser starts where every browser starts: signed in to
//! nothing. Every task that touches an account then begins with a login, which
//! means a password, which means either the person watching a form being filled
//! in or the app holding credentials it should not hold - and then a second
//! factor, which no amount of either solves.
//!
//! A cookie is the answer the web already has for "this is still me". Copying
//! the ones the person already has, once, because they asked, replaces the
//! whole problem with a button.
//!
//! ## What this deliberately does not do
//!
//! It is not a tool. An agent cannot call this and cannot ask for it. Handing
//! an agent the ability to move somebody's live sessions into a place it
//! controls is a different feature with a different risk, and nobody asked for
//! that one. The person picks a profile and the app does exactly that much.
//!
//! It also never touches the browsers it reads. Every file is copied before it
//! is opened, and nothing is ever written back.
//!
//! ## The three families
//!
//! Everything descended from Chromium keeps a SQLite database called `Cookies`
//! with the values encrypted under a key that belongs to the logged-in user -
//! which is the property that makes this safe to offer and impossible to abuse
//! from anywhere else. Everything descended from Firefox keeps a SQLite
//! database called `cookies.sqlite` with the values in plain text. Safari keeps
//! neither and is not read here.

// The house rule for tests, as every other crate here has it: a test that
// cannot unwrap is a test that spends more lines on the impossible than on
// what it is checking.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod decrypt;
pub mod inject;
pub mod read;
pub mod stores;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum CookieError {
    #[error("{0}")]
    Failed(String),
}

pub type Result<T> = std::result::Result<T, CookieError>;

/// One browser profile on this computer.
///
/// The unit is the profile rather than the browser, because the thing a person
/// is signed in to is a profile and one browser has several: the work Chrome
/// and the personal Chrome hold different cookies for the same site. The name
/// is the one the browser shows in its own profile switcher, because
/// "Profile 3" is not something anyone can pick from a list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    /// `chrome`, `edge`, `firefox` - which browser, for the key lookup.
    pub browser_id: String,
    /// What to call it on screen.
    pub browser: String,
    /// `chromium` or `firefox`.
    pub family: String,
    pub profile: String,
    #[serde(skip)]
    pub file: std::path::PathBuf,
    /// The Chromium key lives one level above the profile, shared by all of
    /// them. Empty for Firefox, which has no key.
    #[serde(skip)]
    pub local_state: std::path::PathBuf,
    pub bytes: u64,
    pub updated_at: Option<String>,
    /// How many rows the store holds, or `None` where it could not be opened.
    ///
    /// From `SELECT COUNT(*)`, which reads no values: the list is drawn before
    /// anybody has agreed to anything, and decrypting eight hundred cookies to
    /// label a row would be reading them in order to ask whether they may be
    /// read.
    pub cookies: Option<i64>,
}

/// One cookie, in the one shape everything downstream reads.
///
/// Chromium counts time in microseconds since 1601 and Firefox in seconds since
/// 1970; Chromium says `host_key` and Firefox says `host`. All of it is turned
/// into this here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    /// A leading dot means a domain cookie. Kept exactly as stored, because
    /// that dot is the difference between a cookie for one host and a cookie
    /// for a whole domain.
    pub host: String,
    pub name: String,
    pub value: String,
    pub path: String,
    /// Seconds since the Unix epoch.
    pub expires_at: i64,
    pub secure: bool,
    pub http_only: bool,
    /// `none`, `lax`, `strict`, or `None` where the store did not say.
    pub same_site: Option<String>,
}

/// What a profile gave up, and what it did not.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub expired: u32,
    pub session: u32,
    pub filtered: u32,
    pub unreadable: u32,
}

/// Why some values could not be read, most common first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reason {
    pub reason: String,
    pub count: u32,
}

/// The result of reading one profile.
#[derive(Debug, Clone, Default)]
pub struct Harvest {
    pub cookies: Vec<Cookie>,
    pub skipped: Skipped,
    pub reasons: Vec<Reason>,
}

/// Every browser profile on this computer with cookies in it.
pub fn sources() -> Vec<Source> {
    stores::profiles()
        .into_iter()
        .map(|mut source| {
            source.cookies = read::count(&source);
            source
        })
        .collect()
}

/// One profile by the id [`sources`] gave it.
pub fn source_by_id(id: &str) -> Option<Source> {
    stores::profiles().into_iter().find(|found| found.id == id)
}

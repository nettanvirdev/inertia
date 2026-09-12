//! Putting cookies into a machine's browser.
//!
//! The machine's Chromium keeps its cookies in the same kind of SQLite database
//! everything else does, and Python with sqlite3 is in the sandbox image, so the
//! whole job is a short program uploaded next to the cookies and run once.
//!
//! ## Why the values go in as plain text
//!
//! Chromium reads a row's `value` column when its `encrypted_value` is empty,
//! and encrypts on the way back out the next time it writes. So nothing here
//! has to reproduce Chromium's encryption - which is the difference between a
//! feature that works and one that breaks every time the browser changes how
//! it seals things.
//!
//! That the container's cookie store is readable by anything on the container
//! is not a weakening. It already was: the browser there is started with
//! `--password-store=basic`, because there is no keyring in a container, and
//! that key is a constant published in Chromium's source.
//!
//! ## Why the browser is closed first
//!
//! A running Chromium owns its cookie file and keeps its own copy in memory.
//! Rows written underneath it are not seen, and are overwritten the moment it
//! flushes. So it is closed, the rows are written, and the caller is told -
//! because a browser that vanished mid-page with no explanation is a bug
//! report.
//!
//! ## The payload does not stay
//!
//! What is uploaded is somebody's live sessions in plain text. It is deleted by
//! the script itself, in a `finally`, so a failed import does not leave it on a
//! disk an agent can read.

/// Where the script and its payload live while the import runs.
pub const WORK_DIR: &str = "/tmp/inertia-cookies";
pub const SCRIPT: &str = "/tmp/inertia-cookies/inject.py";
pub const PAYLOAD: &str = "/tmp/inertia-cookies/cookies.json";

/// The line the script prints, which everything else on stdout is not.
const MARKER: &str = "INERTIA_COOKIES ";

/// The program, as text.
///
/// The insert is built from the table's own columns rather than from a fixed
/// list. Chromium's cookie schema has gained four columns in as many years, and
/// a statement naming the ones this was written against fails on both an older
/// machine and a newer one - which for an image the user can rebuild is not
/// hypothetical.
pub const PROGRAM: &str = r#""""Write cookies into this machine's Chromium profile."""

import glob
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import time

EPOCH = 11644473600
PAYLOAD = sys.argv[1]


def row_for(cookie, columns, now):
    secure = 1 if cookie.get("secure") else 0
    same = {"none": 0, "lax": 1, "strict": 2}.get(cookie.get("sameSite"), -1)
    expires = int(cookie.get("expiresAt", 0)) + EPOCH
    known = {
        "creation_utc": now,
        "host_key": cookie["host"],
        "top_frame_site_key": "",
        "name": cookie["name"],
        "value": cookie.get("value", ""),
        "encrypted_value": b"",
        "path": cookie.get("path") or "/",
        "expires_utc": expires * 1000000,
        "is_secure": secure,
        "is_httponly": 1 if cookie.get("httpOnly") else 0,
        "last_access_utc": now,
        "has_expires": 1,
        "is_persistent": 1,
        "priority": 1,
        "samesite": same,
        "source_scheme": 2 if secure else 1,
        "source_port": 443 if secure else 80,
        "last_update_utc": now,
        "source_type": 1,
        "has_cross_site_ancestor": 0,
    }
    values = []
    for name, kind, notnull in columns:
        if name in known:
            values.append(known[name])
        elif notnull:
            values.append(b"" if kind.upper() == "BLOB" else (0 if "INT" in kind.upper() else ""))
        else:
            values.append(None)
    return values


def cookie_databases():
    """Every Chromium cookie store on this machine, newest layout first."""
    found = []
    roots = glob.glob(os.path.expanduser("~/.browser-profiles/*"))
    roots += glob.glob(os.path.expanduser("~/.config/chromium"))
    roots += glob.glob(os.path.expanduser("~/.config/google-chrome"))
    for root in roots:
        for pattern in ("Default/Network/Cookies", "Default/Cookies", "Network/Cookies", "Cookies"):
            candidate = os.path.join(root, pattern)
            if os.path.isfile(candidate):
                found.append(candidate)
                break
    return found


def close_browser():
    """Chromium, asked to stop, then made to."""
    running = subprocess.run(["pgrep", "-x", "chromium"], capture_output=True).returncode == 0
    if not running:
        return False
    subprocess.run(["pkill", "-x", "chromium"], check=False)
    for _ in range(40):
        time.sleep(0.25)
        if subprocess.run(["pgrep", "-x", "chromium"], capture_output=True).returncode != 0:
            break
    else:
        subprocess.run(["pkill", "-9", "-x", "chromium"], check=False)
        time.sleep(0.5)
    return True


def main():
    with open(PAYLOAD, encoding="utf-8") as handle:
        cookies = json.load(handle)["cookies"]

    databases = cookie_databases()
    if not databases:
        return {
            "ok": False,
            "reason": "no-profile",
            "message": "This machine has no browser profile yet.",
        }

    closed = close_browser()
    now = int(time.time() * 1000000) + EPOCH * 1000000
    counts = []
    failed = 0
    touched = []

    for database in databases:
        shutil.copyfile(database, database + ".inertia-backup")
        db = sqlite3.connect(database)
        try:
            columns = [
                (row[1], row[2] or "", row[3]) for row in db.execute("PRAGMA table_info(cookies)")
            ]
            if not columns:
                continue
            names = ",".join(name for name, _, _ in columns)
            marks = ",".join("?" for _ in columns)
            statement = "INSERT OR REPLACE INTO cookies (" + names + ") VALUES (" + marks + ")"
            here = 0
            for cookie in cookies:
                try:
                    db.execute(statement, row_for(cookie, columns, now))
                    here += 1
                except Exception:
                    failed += 1
            db.commit()
            counts.append(here)
            touched.append(database)
        finally:
            db.close()

    return {
        "ok": True,
        "written": max(counts) if counts else 0,
        "failed": failed,
        "profiles": len(touched),
        "browserClosed": closed,
    }


try:
    print("INERTIA_COOKIES " + json.dumps(main()))
finally:
    try:
        os.remove(PAYLOAD)
    except OSError:
        pass
"#;

/// The one line the script printed, out of everything else on stdout.
///
/// The last one, not the first: a machine whose shell prints a banner, or a
/// python that warns, puts text on either side of the answer.
pub fn parse_result(stdout: &str) -> Option<serde_json::Value> {
    stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(MARKER))
        .and_then(|json| serde_json::from_str(json.trim()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_is_found_among_whatever_else_was_printed() {
        let said = "Warning: something\nINERTIA_COOKIES {\"ok\":true,\"written\":12}\nbye\n";
        let answer = parse_result(said).expect("an answer");
        assert_eq!(answer["ok"], serde_json::json!(true));
        assert_eq!(answer["written"], serde_json::json!(12));
    }

    /// A run that said nothing this reads is not a run that succeeded quietly.
    #[test]
    fn nothing_readable_is_nothing_rather_than_a_guess() {
        assert!(parse_result("Traceback (most recent call last):").is_none());
        assert!(parse_result("INERTIA_COOKIES not json").is_none());
        assert!(parse_result("").is_none());
    }

    /// Two runs in one log: the last one is the one that just happened.
    #[test]
    fn the_last_answer_wins() {
        let said = "INERTIA_COOKIES {\"written\":1}\nINERTIA_COOKIES {\"written\":2}\n";
        assert_eq!(parse_result(said).expect("an answer")["written"], serde_json::json!(2));
    }

    /// The script deletes its own payload whatever happens. That is the line
    /// standing between a failed import and somebody's live sessions sitting in
    /// `/tmp` on a machine an agent can read.
    #[test]
    fn the_program_removes_its_payload_in_a_finally() {
        assert!(PROGRAM.contains("finally:"));
        assert!(PROGRAM.contains("os.remove(PAYLOAD)"));
    }
}

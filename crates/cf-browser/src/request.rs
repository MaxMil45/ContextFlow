//! Requests from ContextFlow to the browser extension: the only way anything
//! reaches the browser from ContextFlow, apart from starting it.
//!
//! ```text
//!   restore ──▶ request file ──▶ native host (polls) ──▶ extension ──▶ chrome.tabs
//! ```
//!
//! A browser's command line can open tabs but can't pin them. So when restore
//! opens a window whose saved tabs were pinned, it leaves a small file saying
//! which addresses to pin. Every running native host (one per browser profile
//! with the extension) passes each file on once; the extension of the right
//! browser pins the matching tabs that aren't pinned yet.
//!
//! Requests are best effort and never block a restore. They expire after
//! [`REQUEST_TTL`], and hosts delete expired files, so a request whose browser
//! never connects disappears by itself.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::protocol::is_restorable_url;

/// How long a request is passed on. A cold browser start takes a few seconds;
/// this leaves plenty of room without pinning tabs much later.
pub const REQUEST_TTL: Duration = Duration::from_secs(60);
/// The most addresses one request may carry.
pub const MAX_PINS: usize = 100;
/// Larger files are ignored (and deleted) unread.
const MAX_REQUEST_BYTES: u64 = 256 * 1024;
const PREFIX: &str = "pin-";

/// The most groups one request may carry.
pub const MAX_GROUPS: usize = 20;

/// "Put these tabs in a group": a title and colour and the addresses, in
/// order, of the tabs that were in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupRequest {
    pub title: String,
    pub color: String,
    pub urls: Vec<String>,
}

/// "Pin these tabs and group these", as written by restore and passed to the
/// extension. (Named for what it did first; it also carries groups.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinRequest {
    /// Unique, made of letters, digits and `-` only (it is part of a file name).
    pub id: String,
    /// `chrome` or `msedge`: the extension of any other browser ignores it.
    pub browser: String,
    /// Milliseconds since 1970, UTC.
    pub created_at: i64,
    /// Addresses to pin, in the order they should end up.
    pub urls: Vec<String>,
    /// Groups to make. Left out of the file when there are none, which is also
    /// how requests written before groups existed read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupRequest>,
}

impl PinRequest {
    /// The message the native host sends to the extension.
    pub fn to_message(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "pin",
            "id": self.id,
            "browser": self.browser,
            "urls": self.urls,
            "groups": self.groups,
        })
    }
}

/// Fixed per-user location next to the live state, for the same reason: the
/// browser starts the host and it cannot see the CLI's flags.
pub fn default_request_dir() -> Option<PathBuf> {
    cf_config::project_dirs().map(|d| d.data_dir().join("browser").join("requests"))
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The addresses that may be asked for: plain web addresses, at most
/// [`MAX_PINS`], each once.
fn usable(urls: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    urls.iter()
        .filter(|u| is_restorable_url(u))
        .filter(|u| seen.insert(u.to_string()))
        .take(MAX_PINS)
        .cloned()
        .collect()
}

/// The groups that may be asked for: a colour the browser has, a clean title,
/// plain web addresses, at most [`MAX_GROUPS`]. Empty ones are dropped.
fn usable_groups(groups: &[GroupRequest]) -> Vec<GroupRequest> {
    groups
        .iter()
        .filter_map(|g| {
            let checked = crate::protocol::TabGroup::checked(&g.title, &g.color)?;
            let urls = usable(&g.urls);
            (!urls.is_empty()).then_some(GroupRequest {
                title: checked.title,
                color: checked.color,
                urls,
            })
        })
        .take(MAX_GROUPS)
        .collect()
}

/// Leave a request to pin `urls` in `browser`. See [`write_request`].
pub fn write_pin_request(
    dir: &Path,
    browser: &str,
    urls: &[String],
    now: DateTime<Utc>,
) -> std::io::Result<Option<PinRequest>> {
    write_request(dir, browser, urls, &[], now)
}

/// Leave a request to pin `urls` and make `groups` in `browser`. Returns the
/// request, or `None` when there is nothing usable to ask for. Written
/// atomically, so a host never reads half a file.
pub fn write_request(
    dir: &Path,
    browser: &str,
    urls: &[String],
    groups: &[GroupRequest],
    now: DateTime<Utc>,
) -> std::io::Result<Option<PinRequest>> {
    let urls = usable(urls);
    let groups = usable_groups(groups);
    if (urls.is_empty() && groups.is_empty()) || !crate::is_browser(browser) {
        return Ok(None);
    }
    let request = PinRequest {
        id: uuid::Uuid::new_v4().to_string(),
        browser: browser.to_string(),
        created_at: now.timestamp_millis(),
        urls,
        groups,
    };
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{PREFIX}{}.json", request.id));
    crate::state::write_atomic(&path, &serde_json::to_vec(&request)?)?;
    Ok(Some(request))
}

/// The requests in `dir` not passed on yet (by this host: `sent` remembers),
/// oldest first. Expired, oversized or invalid files are deleted.
pub fn pending_requests(
    dir: &Path,
    now: DateTime<Utc>,
    sent: &mut HashSet<String>,
) -> Vec<PinRequest> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let ttl = REQUEST_TTL.as_millis() as i64;
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with(PREFIX) && name.ends_with(".json")) {
            continue;
        }
        let too_big = entry
            .metadata()
            .map_or(true, |m| m.len() > MAX_REQUEST_BYTES);
        let request: Option<PinRequest> = (!too_big)
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let Some(mut request) = request else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        let age = now.timestamp_millis() - request.created_at;
        let valid = is_valid_id(&request.id)
            && crate::is_browser(&request.browser)
            && (0..=ttl).contains(&age);
        if !valid {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if !sent.insert(request.id.clone()) {
            continue;
        }
        request.urls = usable(&request.urls);
        request.groups = usable_groups(&request.groups);
        if !(request.urls.is_empty() && request.groups.is_empty()) {
            out.push(request);
        }
    }
    out.sort_by_key(|r| r.created_at);
    out
}

/// A native messaging frame: the length as 4 little-endian bytes, then JSON.
pub fn frame(message: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(message).unwrap_or_default();
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_written_request_is_passed_on_once_by_each_host() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let req = write_pin_request(
            dir.path(),
            "chrome",
            &urls(&["https://mail.example.com/", "https://chat.example.com/"]),
            now,
        )
        .unwrap()
        .unwrap();

        let mut host_a = HashSet::new();
        let mut host_b = HashSet::new();
        assert_eq!(
            pending_requests(dir.path(), now, &mut host_a),
            std::slice::from_ref(&req)
        );
        assert!(pending_requests(dir.path(), now, &mut host_a).is_empty());
        // Another profile's host passes it on too: only the extension that
        // has the new window will find tabs to pin.
        assert_eq!(pending_requests(dir.path(), now, &mut host_b), [req]);
    }

    #[test]
    fn nothing_usable_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let none = |list: &[&str], browser: &str| {
            write_pin_request(dir.path(), browser, &urls(list), now)
                .unwrap()
                .is_none()
        };
        assert!(none(&[], "chrome"));
        assert!(none(
            &["chrome://settings", "file:///C:/x", "javascript:alert(1)"],
            "chrome"
        ));
        assert!(none(&["https://a.example/"], "firefox"));
        assert_eq!(std::fs::read_dir(dir.path()).map_or(0, |d| d.count()), 0);
    }

    #[test]
    fn only_web_addresses_are_kept_each_once_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let mut list = urls(&["https://a.example/", "chrome://flags", "https://a.example/"]);
        list.extend((0..200).map(|i| format!("https://x.example/{i}")));
        let req = write_pin_request(dir.path(), "msedge", &list, Utc::now())
            .unwrap()
            .unwrap();
        assert_eq!(req.urls.len(), MAX_PINS);
        assert_eq!(req.urls[0], "https://a.example/");
        assert_eq!(req.urls[1], "https://x.example/0");
    }

    #[test]
    fn expired_and_broken_requests_are_deleted_and_never_sent() {
        let dir = tempfile::tempdir().unwrap();
        let then = Utc::now();
        write_pin_request(dir.path(), "chrome", &urls(&["https://a.example/"]), then).unwrap();
        std::fs::write(dir.path().join("pin-broken.json"), b"{ not json").unwrap();
        // A request someone else wrote, with a file-name-unsafe id.
        std::fs::write(
            dir.path().join("pin-evil.json"),
            br#"{"id":"..\\x","browser":"chrome","created_at":0,"urls":["https://a.example/"]}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("unrelated.txt"), b"left alone").unwrap();

        let later = then + chrono::TimeDelta::seconds(REQUEST_TTL.as_secs() as i64 + 1);
        assert!(pending_requests(dir.path(), later, &mut HashSet::new()).is_empty());
        let left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["unrelated.txt"]);
    }

    #[test]
    fn the_message_and_its_frame_are_what_the_extension_reads() {
        let req = PinRequest {
            id: "abc-1".into(),
            browser: "chrome".into(),
            created_at: 5,
            urls: urls(&["https://a.example/"]),
            groups: Vec::new(),
        };
        let msg = req.to_message();
        assert_eq!(msg["type"], "pin");
        assert_eq!(msg["urls"][0], "https://a.example/");
        let bytes = frame(&msg);
        let len = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(len, bytes.len() - 4);
        let back: serde_json::Value = serde_json::from_slice(&bytes[4..]).unwrap();
        assert_eq!(back, msg);
    }

    fn group(title: &str, color: &str, list: &[&str]) -> GroupRequest {
        GroupRequest {
            title: title.into(),
            color: color.into(),
            urls: urls(list),
        }
    }

    #[test]
    fn groups_travel_with_the_request_and_only_clean_ones_do() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let asked = [
            group(
                "Work",
                "blue",
                &["https://a.example/", "https://b.example/"],
            ),
            group("Bad colour", "ultraviolet", &["https://c.example/"]),
            group("Unsafe", "red", &["chrome://settings", "javascript:x"]),
            group("Empty", "red", &[]),
        ];
        let req = write_request(dir.path(), "chrome", &[], &asked, now)
            .unwrap()
            .unwrap();
        assert_eq!(req.groups, [asked[0].clone()]);
        assert_eq!(req.to_message()["groups"][0]["title"], "Work");
        assert_eq!(
            pending_requests(dir.path(), now, &mut HashSet::new()),
            [req]
        );
        // Nothing usable at all: nothing is written.
        let none = write_request(dir.path(), "chrome", &[], &asked[1..], now).unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn a_request_written_before_groups_existed_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        std::fs::write(
            dir.path().join("pin-old-1.json"),
            format!(
                r#"{{"id":"old-1","browser":"chrome","created_at":{},"urls":["https://a.example/"]}}"#,
                now.timestamp_millis()
            ),
        )
        .unwrap();
        let got = pending_requests(dir.path(), now, &mut HashSet::new());
        assert_eq!(got.len(), 1);
        assert!(got[0].groups.is_empty());
    }
}

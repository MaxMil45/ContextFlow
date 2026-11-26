//! Messages from the browser extension, and their strict validation.
//!
//! Everything the extension sends is treated as untrusted input: it is
//! size-limited, URL-filtered and re-shaped before anything is stored.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Largest message the host accepts from the extension.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_WINDOWS: usize = 50;
pub const MAX_TABS_PER_WINDOW: usize = 500;
pub const MAX_URL_LEN: usize = 4096;
pub const MAX_TITLE_CHARS: usize = 300;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("unexpected message type '{0}'")]
    UnknownType(String),
    #[error("unsupported browser '{0}'")]
    UnknownBrowser(String),
    #[error("invalid profile id")]
    BadProfileId,
    #[error("too many windows ({0})")]
    TooManyWindows(usize),
}

/// What the extension sends.
#[derive(Debug, Deserialize)]
pub struct IncomingMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub browser: String,
    pub profile_id: String,
    #[serde(default)]
    pub windows: Vec<IncomingWindow>,
}

/// A window's position and size in the browser's own (CSS-pixel) units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Deserialize)]
pub struct IncomingWindow {
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub state: String,
    /// Optional (older extensions don't send it). Kept as raw JSON so a
    /// malformed value drops only the bounds, never the whole message.
    #[serde(default)]
    pub bounds: Option<serde_json::Value>,
    #[serde(default)]
    pub tabs: Vec<IncomingTab>,
}

/// A tab group as the browser names it: a title and one of its fixed colours.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabGroup {
    pub title: String,
    pub color: String,
}

/// The colours a browser gives tab groups.
pub const GROUP_COLORS: [&str; 9] = [
    "grey", "blue", "red", "yellow", "green", "pink", "purple", "cyan", "orange",
];
/// Longest group title kept, in characters.
pub const MAX_GROUP_TITLE_CHARS: usize = 100;

impl TabGroup {
    /// A group made of what the extension reported: the title is cleaned and
    /// shortened, and a colour the browser doesn't have is refused (`None`),
    /// so only known values are ever saved or sent back.
    pub fn checked(title: &str, color: &str) -> Option<Self> {
        GROUP_COLORS.contains(&color).then(|| Self {
            title: title
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_GROUP_TITLE_CHARS)
                .collect(),
            color: color.to_string(),
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct IncomingTab {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub active: bool,
    /// The group the tab is in, if any. Unchecked: see [`TabGroup::checked`].
    #[serde(default)]
    pub group: Option<IncomingGroup>,
}

#[derive(Debug, Deserialize)]
pub struct IncomingGroup {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub color: String,
}

/// Validated state of one browser profile, as stored by the host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveState {
    pub browser: String,
    pub profile_id: String,
    pub updated_at: DateTime<Utc>,
    pub windows: Vec<LiveWindow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveWindow {
    pub focused: bool,
    pub state: String,
    /// Where the browser says the window is (absent for older extensions and
    /// older state files).
    #[serde(default)]
    pub bounds: Option<Bounds>,
    pub tabs: Vec<LiveTab>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveTab {
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub active: bool,
    /// The tab group it is in. Older state files have none.
    #[serde(default)]
    pub group: Option<TabGroup>,
}

/// Only plain web addresses are ever saved or reopened. This excludes
/// `chrome://`, `file://`, `javascript:`, extension pages and anything that
/// could be mistaken for a command-line option.
pub fn is_restorable_url(url: &str) -> bool {
    if url.len() > MAX_URL_LEN || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    matches!(rest, Some(r) if !r.is_empty() && !r.starts_with('/'))
}

/// Profile ids become file names, so allow only a safe alphabet.
pub fn is_valid_profile_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Accept bounds only if they parse, every value is a plausible screen
/// coordinate, and the window has a real size. A zero-sized window means the
/// browser had no position to report, so it is dropped rather than treated as
/// "at the origin". Dropped bounds just mean the window is matched by title.
fn clean_bounds(v: serde_json::Value) -> Option<Bounds> {
    const LIMIT: i32 = 100_000;
    let b: Bounds = serde_json::from_value(v).ok()?;
    let plausible = [b.left, b.top, b.width, b.height]
        .iter()
        .all(|n| n.abs() <= LIMIT);
    (plausible && b.width > 0 && b.height > 0).then_some(b)
}

fn clean_title(t: &str) -> String {
    t.chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// Validate a message and reduce it to a [`LiveState`].
pub fn sanitize(msg: IncomingMessage, now: DateTime<Utc>) -> Result<LiveState, ProtocolError> {
    if msg.kind != "state" {
        return Err(ProtocolError::UnknownType(msg.kind));
    }
    if !crate::is_browser(&msg.browser) {
        return Err(ProtocolError::UnknownBrowser(msg.browser));
    }
    if !is_valid_profile_id(&msg.profile_id) {
        return Err(ProtocolError::BadProfileId);
    }
    if msg.windows.len() > MAX_WINDOWS {
        return Err(ProtocolError::TooManyWindows(msg.windows.len()));
    }
    let windows = msg
        .windows
        .into_iter()
        .map(|w| LiveWindow {
            focused: w.focused,
            state: match w.state.as_str() {
                "maximized" | "minimized" | "fullscreen" => w.state,
                _ => "normal".into(),
            },
            bounds: w.bounds.and_then(clean_bounds),
            tabs: w
                .tabs
                .into_iter()
                .take(MAX_TABS_PER_WINDOW)
                .filter(|t| is_restorable_url(&t.url))
                .map(|t| LiveTab {
                    url: t.url,
                    title: clean_title(&t.title),
                    pinned: t.pinned,
                    active: t.active,
                    // A pinned tab can't be in a group, so a group is only
                    // believed for an unpinned one.
                    group: t
                        .group
                        .filter(|_| !t.pinned)
                        .and_then(|g| TabGroup::checked(&g.title, &g.color)),
                })
                .collect(),
        })
        .collect();
    Ok(LiveState {
        browser: msg.browser,
        profile_id: msg.profile_id,
        updated_at: now,
        windows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(url: &str) -> IncomingTab {
        IncomingTab {
            url: url.into(),
            title: "t".into(),
            pinned: false,
            active: false,
            group: None,
        }
    }

    fn grouped(url: &str, title: &str, color: &str, pinned: bool) -> IncomingTab {
        IncomingTab {
            pinned,
            group: Some(IncomingGroup {
                title: title.into(),
                color: color.into(),
            }),
            ..tab(url)
        }
    }

    #[test]
    fn tab_groups_keep_a_clean_title_and_a_known_colour() {
        let state = sanitize(
            msg(
                "chrome",
                "p",
                vec![
                    grouped("https://a.example/", "Work\u{7}\n", "blue", false),
                    grouped("https://b.example/", &"x".repeat(500), "orange", false),
                    // Not a colour the browser has.
                    grouped("https://c.example/", "Odd", "ultraviolet", false),
                    // A pinned tab can't be in a group.
                    grouped("https://d.example/", "Pinned", "red", true),
                    tab("https://e.example/"),
                ],
            ),
            Utc::now(),
        )
        .unwrap();
        let groups: Vec<Option<TabGroup>> = state.windows[0]
            .tabs
            .iter()
            .map(|t| t.group.clone())
            .collect();
        assert_eq!(
            groups[0],
            Some(TabGroup {
                title: "Work".into(),
                color: "blue".into()
            })
        );
        assert_eq!(
            groups[1].as_ref().unwrap().title.chars().count(),
            MAX_GROUP_TITLE_CHARS
        );
        assert_eq!(&groups[2..], &[None, None, None]);
    }

    #[test]
    fn state_files_written_before_groups_still_load() {
        let old = r#"{"url":"https://a.example/","title":"t","pinned":false,"active":true}"#;
        let tab: LiveTab = serde_json::from_str(old).unwrap();
        assert_eq!(tab.group, None);
    }

    fn msg(browser: &str, profile: &str, tabs: Vec<IncomingTab>) -> IncomingMessage {
        IncomingMessage {
            kind: "state".into(),
            browser: browser.into(),
            profile_id: profile.into(),
            windows: vec![IncomingWindow {
                focused: true,
                bounds: None,
                state: "weird".into(),
                tabs,
            }],
        }
    }

    #[test]
    fn only_plain_web_addresses_are_restorable() {
        for ok in [
            "https://drive.google.com/drive/folders/abc?x=1#y",
            "HTTP://Example.com",
        ] {
            assert!(is_restorable_url(ok), "{ok}");
        }
        for bad in [
            "",
            "chrome://settings",
            "edge://newtab",
            "file:///C:/secret.txt",
            "javascript:alert(1)",
            "data:text/html,hi",
            "chrome-extension://abc/x.html",
            "--new-window",
            "-flag",
            "https://",
            "https:///x",
            "https://a b.com",
            "https://a.com/\n",
            &format!("https://a.com/{}", "x".repeat(MAX_URL_LEN)),
        ] {
            assert!(!is_restorable_url(bad), "{bad}");
        }
    }

    #[test]
    fn sanitize_filters_tabs_and_normalizes_state() {
        let m = msg(
            "chrome",
            "abc-123",
            vec![
                tab("https://a.com"),
                tab("chrome://newtab"),
                tab("file:///x"),
                tab("http://b.com"),
            ],
        );
        let s = sanitize(m, Utc::now()).unwrap();
        let urls: Vec<_> = s.windows[0].tabs.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(urls, ["https://a.com", "http://b.com"]);
        assert_eq!(s.windows[0].state, "normal");
    }

    #[test]
    fn sanitize_rejects_bad_envelopes() {
        let now = Utc::now();
        assert_eq!(
            sanitize(msg("firefox", "a", vec![]), now).unwrap_err(),
            ProtocolError::UnknownBrowser("firefox".into())
        );
        for bad in ["", "../evil", "a/b", "a b", &"x".repeat(65)] {
            assert_eq!(
                sanitize(msg("chrome", bad, vec![]), now).unwrap_err(),
                ProtocolError::BadProfileId,
                "{bad}"
            );
        }
        let mut m = msg("chrome", "ok", vec![]);
        m.kind = "other".into();
        assert!(matches!(
            sanitize(m, now),
            Err(ProtocolError::UnknownType(_))
        ));
        let mut many = msg("chrome", "ok", vec![]);
        many.windows = (0..=MAX_WINDOWS)
            .map(|_| IncomingWindow {
                focused: false,
                bounds: None,
                state: String::new(),
                tabs: vec![],
            })
            .collect();
        assert!(matches!(
            sanitize(many, now),
            Err(ProtocolError::TooManyWindows(_))
        ));
    }

    fn parse(json: &str) -> LiveState {
        let msg: IncomingMessage = serde_json::from_str(json).expect("message parses");
        sanitize(msg, Utc::now()).expect("message is valid")
    }

    #[test]
    fn bounds_are_optional_and_preserved() {
        let window = |extra: &str| {
            format!(
                r#"{{"type":"state","browser":"chrome","profile_id":"p","windows":[
                    {{"state":"normal"{extra},"tabs":[{{"url":"https://a.com","title":"A","active":true}}]}}]}}"#
            )
        };
        // Sent by the new extension: kept as is.
        let s = parse(&window(
            r#","bounds":{"left":-8,"top":20,"width":1280,"height":900}"#,
        ));
        assert_eq!(
            s.windows[0].bounds,
            Some(Bounds {
                left: -8,
                top: 20,
                width: 1280,
                height: 900
            })
        );
        // Sent by the old extension: the message is still accepted.
        assert_eq!(parse(&window("")).windows[0].bounds, None);
        // Junk or implausible bounds drop only the bounds, never the message.
        for bad in [
            r#","bounds":"left""#,
            r#","bounds":{"left":"x"}"#,
            r#","bounds":{"left":1,"top":1,"width":1}"#,
            r#","bounds":{"left":999999999,"top":0,"width":1,"height":1}"#,
            r#","bounds":null"#,
            // "No position known" as the extension sends it for such windows.
            r#","bounds":{"left":0,"top":0,"width":0,"height":0}"#,
        ] {
            let s = parse(&window(bad));
            assert_eq!(s.windows[0].bounds, None, "{bad}");
            assert_eq!(s.windows[0].tabs.len(), 1, "{bad}");
        }
        // State files written before this change still load.
        let old = r#"{"focused":true,"state":"normal","tabs":[]}"#;
        let w: LiveWindow = serde_json::from_str(old).unwrap();
        assert_eq!(w.bounds, None);
    }

    #[test]
    fn titles_are_cleaned_and_tabs_capped() {
        let mut t = tab("https://a.com");
        t.title = format!("a\u{0}b\n{}", "z".repeat(1000));
        let mut tabs = vec![t];
        tabs.extend((0..MAX_TABS_PER_WINDOW + 20).map(|i| tab(&format!("https://a.com/{i}"))));
        let s = sanitize(msg("msedge", "p", tabs), Utc::now()).unwrap();
        assert_eq!(s.windows[0].tabs.len(), MAX_TABS_PER_WINDOW);
        let title = &s.windows[0].tabs[0].title;
        assert!(title.starts_with("ab") && title.chars().count() <= MAX_TITLE_CHARS);
    }
}

//! Tabs as stored inside a snapshot, and how live tabs are matched to the
//! browser windows found by the Windows API.

use serde::{Deserialize, Serialize};

use crate::protocol::{LiveState, LiveWindow};

/// `ApplicationState::adapter_id` for applications carrying [`BrowserAppState`].
pub const ADAPTER_ID: &str = "browser";
pub const STATE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub pinned: bool,
    /// The tab group it was in. Left out when there is none, so snapshots
    /// saved before groups existed (and their hashes) are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<crate::protocol::TabGroup>,
}

/// One browser window and its tabs, tied to the Win32 window it was captured
/// from by that window's title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub window_title: String,
    pub tabs: Vec<SavedTab>,
    /// Index into `tabs` of the tab that was in front.
    #[serde(default)]
    pub active_tab: Option<usize>,
    /// The browser profile folder (`Default`, `Profile 2`) the window was in,
    /// so it reopens in the same profile. `None` when it couldn't be told:
    /// the window then opens in whichever profile the browser picks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserAppState {
    pub schema_version: u32,
    pub browser: String,
    pub windows: Vec<SavedWindow>,
}

impl BrowserAppState {
    pub fn tab_count(&self) -> usize {
        self.windows.iter().map(|w| w.tabs.len()).sum()
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("browser state always serialises")
    }

    /// Parse from `ApplicationState::adapter_state`; `None` if absent or from a
    /// newer, unknown schema.
    pub fn from_value(v: &serde_json::Value) -> Option<Self> {
        let s: Self = serde_json::from_value(v.clone()).ok()?;
        (s.schema_version == STATE_SCHEMA).then_some(s)
    }
}

/// Canonical form for comparing addresses: lower-case scheme and host, no
/// fragment, no trailing slash.
pub fn normalize_url(url: &str) -> String {
    let url = url.trim();
    let url = url.split('#').next().unwrap_or(url);
    let (scheme, rest) = match url.split_once("://") {
        Some(p) => p,
        None => return url.to_string(),
    };
    let (host, path) = match rest.find(['/', '?']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let path = if path == "/" { "" } else { path };
    let path = if path.len() > 1 && path.ends_with('/') && !path.contains('?') {
        &path[..path.len() - 1]
    } else {
        path
    };
    format!(
        "{}://{}{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        path
    )
}

pub struct MatchResult {
    pub state: BrowserAppState,
    /// Live windows with web tabs that matched no desktop window.
    pub unmatched_live: usize,
}

/// Window titles look like `<active tab title> - Google Chrome` (Edge adds
/// "and N more pages" and the profile), so a desktop window matches a live
/// window when its title starts with that window's active-tab title.
fn title_matches(win_title: &str, live: &LiveWindow) -> bool {
    let active = live
        .tabs
        .iter()
        .find(|t| t.active)
        .or_else(|| live.tabs.first());
    let Some(active) = active else { return false };
    let wanted = active.title.trim().to_lowercase();
    !wanted.is_empty() && win_title.trim().to_lowercase().starts_with(&wanted)
}

/// A browser window as found by the Windows API.
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopWindow {
    pub title: String,
    /// Top-left corner in physical screen pixels.
    pub x: i32,
    pub y: i32,
    /// The monitor's scale (DPI / 96; 1.0 if unknown). The browser reports
    /// positions in CSS pixels, so they are multiplied by this to compare.
    pub scale: f64,
}

/// How far a live window is from a desktop window, in physical pixels.
/// Windows without bounds (older extension) are infinitely far, so they only
/// ever win when nothing else can be told apart.
fn distance(live: &LiveWindow, desktop: &DesktopWindow) -> i64 {
    match live.bounds {
        Some(b) => {
            let dx = (f64::from(b.left) * desktop.scale - f64::from(desktop.x)).abs();
            let dy = (f64::from(b.top) * desktop.scale - f64::from(desktop.y)).abs();
            (dx + dy).round() as i64
        }
        None => i64::MAX,
    }
}

/// Attach live tabs to the desktop windows of one browser.
///
/// A desktop window matches a live window by title. When several live windows
/// have a matching title (e.g. two windows both showing "Same"), the one
/// nearest to the desktop window's position is used; with no position
/// information the first one wins, as before.
///
/// `desktop` are that browser's desktop windows as captured. `excluded` are
/// case-insensitive substrings; matching tabs are dropped. `profile_of` turns
/// the extension's profile id into the profile folder (see [`crate::profiles`]).
pub fn match_windows(
    browser: &str,
    desktop: &[DesktopWindow],
    live: &[LiveState],
    excluded: &[String],
    profile_of: &dyn Fn(&str) -> Option<String>,
) -> MatchResult {
    let excluded: Vec<String> = excluded.iter().map(|e| e.to_lowercase()).collect();
    // Each live window with the profile (folder) it was reported from.
    let candidates: Vec<(Option<String>, &LiveWindow)> = live
        .iter()
        .flat_map(|s| {
            let profile = profile_of(&s.profile_id);
            s.windows.iter().map(move |w| (profile.clone(), w))
        })
        .filter(|(_, w)| !w.tabs.is_empty())
        .collect();
    let mut used = vec![false; candidates.len()];
    let mut windows = Vec::new();

    for win in desktop {
        let title = &win.title;
        // `min_by_key` keeps the first of equal distances.
        let Some(i) = (0..candidates.len())
            .filter(|&i| !used[i] && title_matches(title, candidates[i].1))
            .min_by_key(|&i| distance(candidates[i].1, win))
        else {
            continue;
        };
        used[i] = true;
        let (profile, live_window) = &candidates[i];
        let kept: Vec<(bool, SavedTab)> = live_window
            .tabs
            .iter()
            .filter(|t| {
                let u = t.url.to_lowercase();
                !excluded.iter().any(|e| u.contains(e))
            })
            .map(|t| {
                (
                    t.active,
                    SavedTab {
                        url: t.url.clone(),
                        title: t.title.clone(),
                        pinned: t.pinned,
                        group: t.group.clone(),
                    },
                )
            })
            .collect();
        if kept.is_empty() {
            continue;
        }
        windows.push(SavedWindow {
            window_title: title.clone(),
            active_tab: kept.iter().position(|(active, _)| *active),
            tabs: kept.into_iter().map(|(_, t)| t).collect(),
            profile: profile.clone(),
        });
    }
    MatchResult {
        state: BrowserAppState {
            schema_version: STATE_SCHEMA,
            browser: browser.into(),
            windows,
        },
        unmatched_live: used.iter().filter(|u| !**u).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::LiveTab;
    use chrono::Utc;

    /// Desktop windows at the origin, for tests that only care about titles.
    fn desk(titles: &[String]) -> Vec<DesktopWindow> {
        titles
            .iter()
            .map(|t| DesktopWindow {
                title: t.clone(),
                x: 0,
                y: 0,
                scale: 1.0,
            })
            .collect()
    }

    fn desktop_window(title: &str, x: i32, y: i32, scale: f64) -> DesktopWindow {
        DesktopWindow {
            title: title.into(),
            x,
            y,
            scale,
        }
    }

    /// A live window with a position, in CSS pixels.
    fn at(mut w: LiveWindow, left: i32, top: i32) -> LiveWindow {
        w.bounds = Some(crate::protocol::Bounds {
            left,
            top,
            width: 800,
            height: 600,
        });
        w
    }

    fn live_window(tabs: &[(&str, &str, bool)]) -> LiveWindow {
        LiveWindow {
            focused: false,
            bounds: None,
            state: "normal".into(),
            tabs: tabs
                .iter()
                .map(|(url, title, active)| LiveTab {
                    url: (*url).into(),
                    title: (*title).into(),
                    pinned: false,
                    active: *active,
                    group: None,
                })
                .collect(),
        }
    }

    fn live(windows: Vec<LiveWindow>) -> Vec<LiveState> {
        vec![LiveState {
            browser: "chrome".into(),
            profile_id: "p".into(),
            updated_at: Utc::now(),
            windows,
        }]
    }

    #[test]
    fn groups_are_saved_with_their_tabs_and_left_out_when_there_are_none() {
        let mut w = live_window(&[
            ("https://a.example/", "A", true),
            ("https://b.example/", "B", false),
        ]);
        w.tabs[0].group = Some(crate::protocol::TabGroup {
            title: "Work".into(),
            color: "blue".into(),
        });
        let titles = ["A - Google Chrome".to_string()];
        let r = match_windows("chrome", &desk(&titles), &live(vec![w]), &[], &|_| None);
        let tabs = &r.state.windows[0].tabs;
        assert_eq!(
            tabs[0].group.as_ref().map(|g| g.title.as_str()),
            Some("Work")
        );
        assert_eq!(tabs[1].group, None);
        let json = serde_json::to_string(&r.state).unwrap();
        // Only the grouped tab mentions a group; old snapshots mention none.
        assert_eq!(json.matches("\"group\"").count(), 1, "{json}");
        let old = json.replace(r#","group":{"title":"Work","color":"blue"}"#, "");
        assert!(!old.contains("group"), "{old}");
        let back: BrowserAppState = serde_json::from_str(&old).unwrap();
        assert_eq!(back.windows[0].tabs[0].group, None);
    }

    #[test]
    fn each_window_carries_the_profile_it_was_reported_from() {
        // Two profiles of one browser, each with a window of the same title.
        let state = |id: &str, tab: &str| LiveState {
            browser: "chrome".into(),
            profile_id: id.into(),
            updated_at: Utc::now(),
            windows: vec![live_window(&[(tab, "Same", true)])],
        };
        let l = vec![
            state("work-id", "https://work.example/"),
            state("home-id", "https://home.example/"),
        ];
        let titles = [
            "Same - Google Chrome".to_string(),
            "Same - Google Chrome".to_string(),
        ];
        let folder = |id: &str| match id {
            "work-id" => Some("Profile 2".to_string()),
            _ => None, // the other profile can't be worked out
        };
        let r = match_windows("chrome", &desk(&titles), &l, &[], &folder);
        let profiles: Vec<_> = r
            .state
            .windows
            .iter()
            .map(|w| (w.tabs[0].url.as_str(), w.profile.as_deref()))
            .collect();
        assert_eq!(
            profiles,
            [
                ("https://work.example/", Some("Profile 2")),
                ("https://home.example/", None)
            ]
        );
    }

    #[test]
    fn a_window_without_a_known_profile_is_saved_exactly_as_before() {
        let l = live(vec![live_window(&[("https://docs.rs", "Docs.rs", true)])]);
        let titles = ["Docs.rs - Google Chrome".to_string()];
        let r = match_windows("chrome", &desk(&titles), &l, &[], &|_| None);
        let json = serde_json::to_string(&r.state).unwrap();
        // Old snapshots and their hashes stay valid: no `profile` key at all.
        assert!(!json.contains("profile"), "{json}");
        // And a snapshot saved before profiles existed still loads.
        let back: BrowserAppState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.windows[0].profile, None);
        // A profile that is known is written and read back.
        let mut with = r.state.clone();
        with.windows[0].profile = Some("Profile 1".into());
        let again: BrowserAppState =
            serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
        assert_eq!(again.windows[0].profile.as_deref(), Some("Profile 1"));
    }

    #[test]
    fn normalizes_for_comparison() {
        assert_eq!(
            normalize_url("HTTPS://Drive.Google.com/"),
            "https://drive.google.com"
        );
        assert_eq!(normalize_url("https://a.com/x/#frag"), "https://a.com/x");
        assert_eq!(normalize_url("https://a.com/x?y=1"), "https://a.com/x?y=1");
        assert_eq!(normalize_url("https://a.com?y=1"), "https://a.com?y=1");
        assert_eq!(normalize_url(" https://a.com/Path "), "https://a.com/Path");
    }

    #[test]
    fn matches_windows_by_active_tab_title() {
        let l = live(vec![
            live_window(&[
                ("https://mail.com", "Inbox", false),
                (
                    "https://drive.google.com/drive/x",
                    "Navy-152 - Google Drive",
                    true,
                ),
            ]),
            live_window(&[("https://docs.rs", "Docs.rs", true)]),
        ]);
        let titles = vec![
            "Docs.rs - Google Chrome".to_string(),
            "Navy-152 - Google Drive - Google Chrome".to_string(),
        ];
        let r = match_windows("chrome", &desk(&titles), &l, &[], &|_| None);
        assert_eq!(r.unmatched_live, 0);
        assert_eq!(r.state.windows.len(), 2);
        assert_eq!(r.state.windows[0].window_title, titles[0]);
        assert_eq!(r.state.windows[0].tabs.len(), 1);
        assert_eq!(r.state.windows[1].tabs.len(), 2);
        assert_eq!(r.state.windows[1].active_tab, Some(1));
        assert_eq!(r.state.tab_count(), 3);
    }

    #[test]
    fn edge_style_titles_and_duplicates_pair_in_order() {
        let l = live(vec![
            live_window(&[("https://a.com/1", "Same", true)]),
            live_window(&[("https://a.com/2", "Same", true)]),
        ]);
        let titles = vec![
            "Same and 3 more pages - Work - Microsoft Edge".to_string(),
            "Same and 1 more page - Work - Microsoft Edge".to_string(),
        ];
        let r = match_windows("msedge", &desk(&titles), &l, &[], &|_| None);
        assert_eq!(r.state.windows[0].tabs[0].url, "https://a.com/1");
        assert_eq!(r.state.windows[1].tabs[0].url, "https://a.com/2");
    }

    #[test]
    fn unmatched_and_excluded() {
        let l = live(vec![
            live_window(&[
                ("https://bank.example/login", "Bank", false),
                ("https://ok.com", "Ok", true),
            ]),
            live_window(&[("https://other.com", "Other", true)]),
        ]);
        let titles = vec![
            "Ok - Google Chrome".to_string(),
            "Mystery - Google Chrome".to_string(),
        ];
        let r = match_windows(
            "chrome",
            &desk(&titles),
            &l,
            &["BANK.example".into()],
            &|_| None,
        );
        assert_eq!(r.state.windows.len(), 1);
        assert_eq!(r.state.windows[0].tabs.len(), 1, "excluded tab dropped");
        assert_eq!(r.state.windows[0].active_tab, Some(0));
        assert_eq!(r.unmatched_live, 1);
    }

    #[test]
    fn identical_titles_are_matched_by_position() {
        // Two windows both titled "Same", on the left and the right.
        let left = at(live_window(&[("https://left.example", "Same", true)]), 0, 0);
        let right = at(
            live_window(&[("https://right.example", "Same", true)]),
            1000,
            0,
        );
        let l = live(vec![left, right]);
        // The desktop lists the right-hand window first: the opposite order.
        let desktop = vec![
            desktop_window("Same - Google Chrome", 1000, 0, 1.0),
            desktop_window("Same - Google Chrome", 0, 0, 1.0),
        ];
        let r = match_windows("chrome", &desktop, &l, &[], &|_| None);
        assert_eq!(r.state.windows.len(), 2);
        assert_eq!(r.state.windows[0].tabs[0].url, "https://right.example");
        assert_eq!(r.state.windows[1].tabs[0].url, "https://left.example");
        assert_eq!(r.unmatched_live, 0);
    }

    #[test]
    fn scale_is_applied_to_browser_coordinates() {
        // The browser says 400 CSS px; on a 150% monitor that is 600 physical px.
        // Without scaling, the window at "left 600" would look closer.
        let a = at(live_window(&[("https://a.example", "Same", true)]), 400, 0);
        let b = at(live_window(&[("https://b.example", "Same", true)]), 600, 0);
        let l = live(vec![b, a]); // listed so that "first match" would pick b
        let desktop = vec![desktop_window("Same - Google Chrome", 600, 0, 1.5)];
        let r = match_windows("chrome", &desktop, &l, &[], &|_| None);
        assert_eq!(r.state.windows[0].tabs[0].url, "https://a.example");
        assert_eq!(r.unmatched_live, 1, "the other window stays unmatched");
    }

    #[test]
    fn windows_without_bounds_still_match_in_order() {
        // An older extension sends no bounds: behaviour is the same as before.
        let l = live(vec![
            live_window(&[("https://one.example", "Same", true)]),
            live_window(&[("https://two.example", "Same", true)]),
        ]);
        let desktop = vec![
            desktop_window("Same - Google Chrome", 500, 500, 1.0),
            desktop_window("Same - Google Chrome", 0, 0, 1.0),
        ];
        let r = match_windows("chrome", &desktop, &l, &[], &|_| None);
        assert_eq!(r.state.windows[0].tabs[0].url, "https://one.example");
        assert_eq!(r.state.windows[1].tabs[0].url, "https://two.example");
    }

    #[test]
    fn state_round_trips_through_adapter_state() {
        let s = BrowserAppState {
            schema_version: STATE_SCHEMA,
            browser: "chrome".into(),
            windows: vec![],
        };
        assert_eq!(BrowserAppState::from_value(&s.to_value()), Some(s));
        assert_eq!(BrowserAppState::from_value(&serde_json::Value::Null), None);
        let future = serde_json::json!({"schema_version": 99, "browser": "chrome", "windows": []});
        assert_eq!(BrowserAppState::from_value(&future), None);
    }
}

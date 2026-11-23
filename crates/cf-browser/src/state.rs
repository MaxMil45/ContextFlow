//! Live tab state files written by the native host and read by capture/restore.
//!
//! One file per browser profile, `<browser>-<profile_id>.json`, present only
//! while that browser is running with the extension connected. Files not
//! refreshed recently are treated as stale (the browser crashed or was killed).

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::protocol::{is_valid_profile_id, LiveState};
use crate::saved::normalize_url;

/// The extension re-sends its state every ~20 s; anything older is stale.
pub const FRESH_FOR: Duration = Duration::from_secs(120);

/// Fixed per-user location, independent of `--db`, because the browser starts
/// the host and it cannot see the CLI's flags.
pub fn default_state_dir() -> Option<PathBuf> {
    cf_config::project_dirs().map(|d| d.data_dir().join("browser").join("live"))
}

pub fn state_file(dir: &Path, browser: &str, profile_id: &str) -> PathBuf {
    dir.join(format!("{browser}-{profile_id}.json"))
}

/// Write atomically (temp file + rename) so readers never see a partial file.
pub fn write_state(dir: &Path, state: &LiveState) -> std::io::Result<()> {
    if !is_valid_profile_id(&state.profile_id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid profile id",
        ));
    }
    std::fs::create_dir_all(dir)?;
    let path = state_file(dir, &state.browser, &state.profile_id);
    write_atomic(&path, &serde_json::to_vec(state)?)
}

/// Write `bytes` to `path` so a reader sees the old file or the new one, never
/// half of one: written next to it, flushed to disk, then renamed over it.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

pub fn remove_state(dir: &Path, browser: &str, profile_id: &str) {
    if is_valid_profile_id(profile_id) {
        let _ = std::fs::remove_file(state_file(dir, browser, profile_id));
    }
}

/// All fresh states for `browser` (every profile), oldest first.
pub fn read_live_states(dir: &Path, browser: &str) -> Vec<LiveState> {
    read_live_states_at(dir, browser, Utc::now())
}

pub fn read_live_states_at(dir: &Path, browser: &str, now: DateTime<Utc>) -> Vec<LiveState> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let prefix = format!("{browser}-");
    let mut out: Vec<LiveState> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with(&prefix) && name.ends_with(".json")
        })
        .filter_map(|e| std::fs::read(e.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<LiveState>(&bytes).ok())
        .filter(|s| s.browser == browser)
        .filter(|s| {
            now.signed_duration_since(s.updated_at)
                .to_std()
                .map(|age| age <= FRESH_FOR)
                // A timestamp slightly in the future (clock skew) counts as fresh.
                .unwrap_or(true)
        })
        .collect();
    out.sort_by_key(|s| s.updated_at);
    out
}

/// Normalised addresses of every tab currently open in `browser`, or `None` if
/// no extension is connected (so the caller cannot know).
pub fn live_urls(dir: &Path, browser: &str) -> Option<Vec<String>> {
    let states = read_live_states(dir, browser);
    if states.is_empty() {
        return None;
    }
    let set: HashSet<String> = states
        .iter()
        .flat_map(|s| &s.windows)
        .flat_map(|w| &w.tabs)
        .map(|t| normalize_url(&t.url))
        .collect();
    Some(set.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{LiveTab, LiveWindow};
    use chrono::TimeDelta;

    fn state(browser: &str, profile: &str, at: DateTime<Utc>, urls: &[&str]) -> LiveState {
        LiveState {
            browser: browser.into(),
            profile_id: profile.into(),
            updated_at: at,
            windows: vec![LiveWindow {
                focused: true,
                bounds: None,
                state: "normal".into(),
                tabs: urls
                    .iter()
                    .map(|u| LiveTab {
                        url: (*u).into(),
                        title: String::new(),
                        pinned: false,
                        active: false,
                    })
                    .collect(),
            }],
        }
    }

    #[test]
    fn write_read_remove_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = state("chrome", "p1", Utc::now(), &["https://a.com"]);
        write_state(dir.path(), &s).unwrap();
        assert_eq!(read_live_states(dir.path(), "chrome"), vec![s]);
        assert!(read_live_states(dir.path(), "msedge").is_empty());
        remove_state(dir.path(), "chrome", "p1");
        assert!(read_live_states(dir.path(), "chrome").is_empty());
    }

    #[test]
    fn stale_and_corrupt_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let old = Utc::now() - TimeDelta::seconds(600);
        write_state(dir.path(), &state("chrome", "old", old, &["https://a.com"])).unwrap();
        std::fs::write(dir.path().join("chrome-bad.json"), b"{not json").unwrap();
        assert!(read_live_states(dir.path(), "chrome").is_empty());
        assert_eq!(live_urls(dir.path(), "chrome"), None, "unknown, not empty");
    }

    #[test]
    fn live_urls_merge_profiles_and_normalize() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        write_state(
            dir.path(),
            &state("chrome", "a", now, &["https://A.com/x/#top"]),
        )
        .unwrap();
        write_state(dir.path(), &state("chrome", "b", now, &["https://b.com"])).unwrap();
        let mut urls = live_urls(dir.path(), "chrome").unwrap();
        urls.sort();
        assert_eq!(urls, ["https://a.com/x", "https://b.com"]);
    }

    #[test]
    fn unsafe_profile_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = state("chrome", "../x", Utc::now(), &[]);
        assert!(write_state(dir.path(), &s).is_err());
    }
}

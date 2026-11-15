//! Visual Studio (`devenv.exe`): a window's title starts with the solution or
//! folder name: `CorpusAudit - README.md - Microsoft Visual Studio`. Visual
//! Studio records the solutions and folders it has opened, with their full
//! paths and when they were last opened, in `ApplicationPrivateSettings.xml`
//! (the list behind its start window). Restore reopens one with
//! `devenv.exe <path>`.
//!
//! Several entries can share a name (two copies of a project). Visual Studio
//! updates an entry when it opens it, so the most recently opened one is the
//! one on screen.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Candidate, Sources};

/// Largest settings file we are willing to read.
const MAX_SETTINGS_BYTES: u64 = 16 * 1024 * 1024;
const TITLE_MARKER: &str = " - Microsoft Visual Studio";

/// The solution or folder name from a window title.
///
/// `"CorpusAudit - README.md - Microsoft Visual Studio"`,
/// `"CorpusAudit (Debugging) - Microsoft Visual Studio Preview"` and
/// `"CorpusAudit - Microsoft Visual Studio (Administrator)"` all give
/// `CorpusAudit`. The start window (`"Microsoft Visual Studio"`) gives `None`.
pub fn name_from_title(title: &str) -> Option<String> {
    let (before, _) = title.trim().split_once(TITLE_MARKER)?;
    let first = before.split(" - ").next()?.trim();
    let mut name = first;
    for suffix in [
        " (Debugging)",
        " (Running)",
        " (Administrator)",
        " (Not Responding)",
    ] {
        name = name.strip_suffix(suffix).unwrap_or(name).trim();
    }
    (!name.is_empty()).then(|| name.to_string())
}

/// One solution or folder Visual Studio has opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: String,
    pub is_solution: bool,
    /// When Visual Studio last opened it (ISO 8601, as Visual Studio writes it).
    pub last_accessed: String,
}

impl RecentEntry {
    /// The name its windows show: a solution's file name without extension, or a folder's name.
    pub fn name(&self) -> String {
        let p = Path::new(&self.path);
        let part = if self.is_solution {
            p.file_stem()
        } else {
            p.file_name()
        };
        part.map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct Container {
    #[serde(rename = "Value")]
    value: ContainerValue,
}

#[derive(Deserialize)]
struct ContainerValue {
    #[serde(rename = "LocalProperties")]
    local: Option<LocalProperties>,
    #[serde(rename = "LastAccessed", default)]
    last_accessed: String,
}

#[derive(Deserialize)]
struct LocalProperties {
    #[serde(rename = "FullPath")]
    full_path: Option<String>,
    /// 0 = solution file, 1 = folder.
    #[serde(rename = "Type", default)]
    kind: i64,
}

fn unescape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else { break };
        let entity = &rest[1..end];
        let ch = match entity {
            "quot" => Some('"'),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The recent solutions and folders in one `ApplicationPrivateSettings.xml`.
/// Anything unreadable gives an empty list.
pub fn parse_recent(xml: &str) -> Vec<RecentEntry> {
    let mut out = Vec::new();
    for collection in ["CodeContainers.Offline", "CodeContainers.Roaming"] {
        let marker = format!("name=\"{collection}\"");
        let Some(start) = xml.find(&marker) else {
            continue;
        };
        let tail = &xml[start..];
        let Some(v) = tail.find("<value name=\"value\">") else {
            continue;
        };
        let body = &tail[v + "<value name=\"value\">".len()..];
        let Some(end) = body.find("</value>") else {
            continue;
        };
        let json = unescape_xml(&body[..end]);
        let Ok(items) = serde_json::from_str::<Vec<Container>>(&json) else {
            continue;
        };
        for c in items {
            let Some(local) = c.value.local else { continue };
            let Some(path) = local.full_path.filter(|p| !p.trim().is_empty()) else {
                continue;
            };
            if out
                .iter()
                .any(|e: &RecentEntry| e.path.eq_ignore_ascii_case(&path))
            {
                continue;
            }
            out.push(RecentEntry {
                path,
                is_solution: local.kind == 0,
                last_accessed: c.value.last_accessed,
            });
        }
    }
    out
}

/// Every installed Visual Studio's settings file (one per version and instance).
pub fn default_settings_files() -> Vec<PathBuf> {
    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        return Vec::new();
    };
    let dir = PathBuf::from(base).join("Microsoft").join("VisualStudio");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .map(|e| e.path().join("ApplicationPrivateSettings.xml"))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

/// The recent solutions and folders of every installed Visual Studio.
pub fn read_recent(files: &[PathBuf]) -> Vec<RecentEntry> {
    let mut all: Vec<RecentEntry> = Vec::new();
    for f in files {
        let small = std::fs::metadata(f).is_ok_and(|m| m.len() <= MAX_SETTINGS_BYTES);
        let Ok(xml) = small.then(|| std::fs::read_to_string(f)).transpose() else {
            continue;
        };
        for e in parse_recent(xml.as_deref().unwrap_or_default()) {
            match all
                .iter_mut()
                .find(|a| a.path.eq_ignore_ascii_case(&e.path))
            {
                // Seen by two versions: keep the more recent time.
                Some(a) if e.last_accessed > a.last_accessed => a.last_accessed = e.last_accessed,
                Some(_) => {}
                None => all.push(e),
            }
        }
    }
    all
}

/// The recent entries as candidates, ranked by when each was last opened.
/// The times are text in one format, so their order is their rank.
fn to_candidates(recent: &[RecentEntry]) -> Vec<Candidate> {
    let mut times: Vec<&str> = recent.iter().map(|e| e.last_accessed.as_str()).collect();
    times.sort_unstable();
    times.dedup();
    recent
        .iter()
        .map(|e| Candidate {
            names: vec![e.name()],
            path: e.path.clone(),
            rank: times.binary_search(&e.last_accessed.as_str()).unwrap_or(0) as u64,
        })
        .collect()
}

pub(crate) fn candidates(sources: &Sources) -> Option<Vec<Candidate>> {
    let files = &sources.visual_studio_settings;
    (!files.is_empty()).then(|| to_candidates(&read_recent(files)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;

    fn build_state(
        titles: &[String],
        list: &[RecentEntry],
        exists: &dyn Fn(&str) -> bool,
    ) -> State {
        crate::build_state(
            crate::app(crate::VISUAL_STUDIO),
            titles,
            &to_candidates(list),
            exists,
        )
    }

    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<content>
  <indexed>
    <collection name="CodeContainers.Offline">
      <value name="value">[{&quot;Key&quot;:&quot;C:\\Users\\me\\CorpusAudit_starter\\CorpusAudit&quot;,&quot;Value&quot;:{&quot;LocalProperties&quot;:{&quot;FullPath&quot;:&quot;C:\\Users\\me\\CorpusAudit_starter\\CorpusAudit&quot;,&quot;Type&quot;:1,&quot;SourceControl&quot;:null},&quot;Remote&quot;:null,&quot;IsFavorite&quot;:false,&quot;LastAccessed&quot;:&quot;2026-11-14T11:10:36.2857011+00:00&quot;,&quot;IsLocal&quot;:true}},{&quot;Key&quot;:&quot;x&quot;,&quot;Value&quot;:{&quot;LocalProperties&quot;:{&quot;FullPath&quot;:&quot;C:\\Users\\me\\Docs\\CorpusAudit&quot;,&quot;Type&quot;:1},&quot;LastAccessed&quot;:&quot;2026-10-28T20:21:00.7472023+00:00&quot;}},{&quot;Key&quot;:&quot;y&quot;,&quot;Value&quot;:{&quot;LocalProperties&quot;:{&quot;FullPath&quot;:&quot;C:\\Users\\me\\Sentinel\\CheatSheet.slnx&quot;,&quot;Type&quot;:0},&quot;LastAccessed&quot;:&quot;2026-10-16T19:32:05.7012584+00:00&quot;}}]</value>
    </collection>
  </indexed>
</content>"#;

    #[test]
    fn names_come_from_the_start_of_the_title() {
        assert_eq!(
            name_from_title("CorpusAudit - README.md - Microsoft Visual Studio").as_deref(),
            Some("CorpusAudit")
        );
        assert_eq!(
            name_from_title("CorpusAudit (Debugging) - Microsoft Visual Studio Preview").as_deref(),
            Some("CorpusAudit")
        );
        assert_eq!(
            name_from_title("CheatSheet - Microsoft Visual Studio (Administrator)").as_deref(),
            Some("CheatSheet")
        );
        assert_eq!(name_from_title("Microsoft Visual Studio"), None);
        assert_eq!(
            name_from_title("main.rs - ContextFlow - Visual Studio Code"),
            None
        );
    }

    #[test]
    fn the_recent_list_is_read_from_visual_studios_settings() {
        let recent = parse_recent(XML);
        assert_eq!(recent.len(), 3);
        assert_eq!(
            recent[0].path,
            r"C:\Users\me\CorpusAudit_starter\CorpusAudit"
        );
        assert!(!recent[0].is_solution);
        assert!(recent[2].is_solution);
        assert_eq!(recent[2].name(), "CheatSheet");
        assert!(parse_recent("not xml").is_empty());
    }

    #[test]
    fn the_most_recently_opened_of_two_same_named_entries_wins() {
        let recent = parse_recent(XML);
        let titles = vec!["CorpusAudit - README.md - Microsoft Visual Studio".to_string()];
        let s = build_state(&titles, &recent, &|_| true);
        assert_eq!(
            s.windows[0].path.as_deref(),
            Some(r"C:\Users\me\CorpusAudit_starter\CorpusAudit")
        );
        // Two windows with the same name get the two entries, newest first.
        let two = vec![titles[0].clone(), titles[0].clone()];
        let s = build_state(&two, &recent, &|_| true);
        assert_eq!(
            s.windows[1].path.as_deref(),
            Some(r"C:\Users\me\Docs\CorpusAudit")
        );
    }

    #[test]
    fn missing_ambiguous_or_unsafe_paths_are_never_saved() {
        let recent = parse_recent(XML);
        let t = vec!["CorpusAudit - Microsoft Visual Studio".to_string()];
        assert_eq!(
            build_state(&t, &recent, &|_| false).path_count(),
            0,
            "gone from disk"
        );

        let tie = vec![
            RecentEntry {
                path: r"C:\a\X".into(),
                is_solution: false,
                last_accessed: "2026-01-01".into(),
            },
            RecentEntry {
                path: r"C:\b\X".into(),
                is_solution: false,
                last_accessed: "2026-01-01".into(),
            },
        ];
        let x = vec!["X - Microsoft Visual Studio".to_string()];
        assert_eq!(
            build_state(&x, &tie, &|_| true).path_count(),
            0,
            "equally recent"
        );
    }
}

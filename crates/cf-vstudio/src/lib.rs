//! Visual Studio (`devenv.exe`) integration: work out which solution or folder
//! each Visual Studio window has open, without needing a Visual Studio extension.
//!
//! A Visual Studio window's title starts with the solution or folder name:
//! `CorpusAudit - README.md - Microsoft Visual Studio`. Visual Studio records
//! the solutions and folders it has opened, with their full paths and when they
//! were last opened, in `ApplicationPrivateSettings.xml` (the list behind its
//! start window). We look the name up there and keep the path, so restore can
//! reopen it with `devenv.exe <path>`.
//!
//! Several entries can share a name (two copies of a project). Visual Studio
//! updates an entry when it opens it, so the most recently opened one is the
//! one on screen; if two are equally recent, no path is saved rather than a guess.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `ApplicationState::adapter_id` for applications carrying [`VisualStudioState`].
pub const ADAPTER_ID: &str = "visualstudio";
pub const STATE_SCHEMA: u32 = 1;

/// Largest settings file we are willing to read.
const MAX_SETTINGS_BYTES: u64 = 16 * 1024 * 1024;
const TITLE_MARKER: &str = " - Microsoft Visual Studio";
const MAX_PATH_LEN: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VsWindow {
    pub window_title: String,
    /// The solution or folder name taken from the title, if there was one.
    pub name: Option<String>,
    /// Full path of the `.sln`/`.slnx` file or the folder, if it could be worked out.
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualStudioState {
    pub schema_version: u32,
    pub windows: Vec<VsWindow>,
}

impl VisualStudioState {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// `None` for anything that isn't this schema.
    pub fn from_value(v: &serde_json::Value) -> Option<Self> {
        serde_json::from_value::<Self>(v.clone())
            .ok()
            .filter(|s| s.schema_version == STATE_SCHEMA)
    }

    /// Windows that have a path to reopen.
    pub fn path_count(&self) -> usize {
        self.windows.iter().filter(|w| w.path.is_some()).count()
    }
}

/// The application id of Visual Studio (`devenv.exe`).
pub fn is_visual_studio(application_id: &str) -> bool {
    application_id == "devenv"
}

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

/// Only plain, absolute local or network paths are ever saved or reopened:
/// nothing that could be read as a command-line switch.
pub fn is_safe_path(path: &str) -> bool {
    let p = path.trim();
    let bytes = p.as_bytes();
    let drive = bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let unc = p.starts_with("\\\\") && p.len() > 4;
    (drive || unc)
        && p.len() <= MAX_PATH_LEN
        && !p.contains('"')
        && !p.chars().any(char::is_control)
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

/// Whether a saved path still exists on disk.
pub fn path_exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// Work out each window's solution or folder from its title. Windows whose
/// name matches nothing, or matches two equally recent entries, get no path.
pub fn build_state(
    titles: &[String],
    recent: &[RecentEntry],
    exists: &dyn Fn(&str) -> bool,
) -> VisualStudioState {
    let mut taken: Vec<String> = Vec::new();
    let windows = titles
        .iter()
        .map(|title| {
            let name = name_from_title(title);
            let path = name.as_deref().and_then(|n| {
                let mut candidates: Vec<&RecentEntry> = recent
                    .iter()
                    .filter(|e| e.name().eq_ignore_ascii_case(n))
                    .filter(|e| is_safe_path(&e.path) && exists(&e.path))
                    .filter(|e| !taken.iter().any(|t| t.eq_ignore_ascii_case(&e.path)))
                    .collect();
                candidates.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));
                match candidates.as_slice() {
                    [] => None,
                    [a, b, ..] if a.last_accessed == b.last_accessed => None,
                    [a, ..] => Some(a.path.clone()),
                }
            });
            if let Some(p) = &path {
                taken.push(p.clone());
            }
            VsWindow {
                window_title: title.clone(),
                name,
                path,
            }
        })
        .collect();
    VisualStudioState {
        schema_version: STATE_SCHEMA,
        windows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(!is_safe_path("--help"));
        assert!(!is_safe_path(r#"C:\a" /run"#));
        assert!(!is_safe_path("relative\\path"));
        assert!(is_safe_path(r"C:\Users\me\App.sln"));
        assert!(is_safe_path(r"\\server\share\App.sln"));
    }

    #[test]
    fn state_round_trips() {
        let recent = parse_recent(XML);
        let t = vec!["CheatSheet - Program.cs - Microsoft Visual Studio".to_string()];
        let s = build_state(&t, &recent, &|_| true);
        assert_eq!(VisualStudioState::from_value(&s.to_value()), Some(s));
    }
}

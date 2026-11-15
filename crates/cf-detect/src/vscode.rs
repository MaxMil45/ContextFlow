//! VS Code: a window's title ends with `<folder name> - Visual Studio Code`.
//! VS Code records the folders it knows about in its own `storage.json`; the
//! name is looked up there and the matching `file:///` address is kept, so
//! restore can reopen the folder with `code --folder-uri`.

use std::path::{Path, PathBuf};

use crate::{Candidate, Sources};

/// Largest `storage.json` we are willing to read.
const MAX_STORAGE_BYTES: u64 = 8 * 1024 * 1024;
const TITLE_SUFFIX: &str = " - Visual Studio Code";
const MAX_URI_LEN: usize = 2048;

/// The folder name from a window title.
///
/// `"main.rs - ContextFlow - Visual Studio Code"` and
/// `"ContextFlow - Visual Studio Code"` both give `ContextFlow`.
pub fn root_name_from_title(title: &str) -> Option<String> {
    let rest = title.trim().strip_suffix(TITLE_SUFFIX)?;
    let name = rest.rsplit_once(" - ").map_or(rest, |(_, last)| last);
    let name = name.trim();
    let name = name.strip_suffix(" (Workspace)").unwrap_or(name).trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Only plain `file:///` folder addresses are ever stored or reopened.
pub fn is_safe_folder_uri(uri: &str) -> bool {
    uri.starts_with("file:///")
        && uri.len() <= MAX_URI_LEN
        && !uri.chars().any(|c| c.is_whitespace() || c.is_control())
        && !uri.to_ascii_lowercase().contains("%00")
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = hex(*bytes.get(i + 1)?)?;
            let lo = hex(*bytes.get(i + 2)?)?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The local path a `file:///` address points at, e.g.
/// `file:///c%3A/My%20Projects/app` -> `c:/My Projects/app`.
pub fn folder_path(uri: &str) -> Option<String> {
    if !is_safe_folder_uri(uri) {
        return None;
    }
    let path = percent_decode(uri.strip_prefix("file:///")?)?;
    (!path.is_empty() && !path.chars().any(char::is_control)).then_some(path)
}

/// The last path segment of a folder address, percent-decoded.
pub fn folder_base_name(uri: &str) -> Option<String> {
    let path = folder_path(uri)?;
    path.split(['/', '\\'])
        .rfind(|s| !s.is_empty())
        .map(String::from)
}

/// Does the folder an address points at exist on this computer?
pub fn folder_exists(uri: &str) -> bool {
    folder_path(uri).is_some_and(|p| Path::new(&p).is_dir())
}

/// Folders VS Code knows about, in two tiers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownFolders {
    /// Folders of windows open now or last time, in priority order: the
    /// window that was active, then windows with unsaved-work backups, then
    /// other recorded open windows.
    pub open: Vec<String>,
    /// Every other folder VS Code has ever recorded, in no particular order.
    pub others: Vec<String>,
}

fn push_unique(list: &mut Vec<String>, uri: &str) {
    if is_safe_folder_uri(uri) && !list.iter().any(|u| u == uri) {
        list.push(uri.to_string());
    }
}

/// Read the folder addresses out of `storage.json` text.
pub fn parse_known_folders(json: &str) -> KnownFolders {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return KnownFolders::default();
    };
    let mut known = KnownFolders::default();

    let windows = &root["windowsState"];
    if let Some(uri) = windows["lastActiveWindow"]["folder"].as_str() {
        push_unique(&mut known.open, uri);
    }
    for entry in root["backupWorkspaces"]["folders"]
        .as_array()
        .into_iter()
        .flatten()
    {
        if let Some(uri) = entry["folderUri"].as_str() {
            push_unique(&mut known.open, uri);
        }
    }
    for window in windows["openedWindows"].as_array().into_iter().flatten() {
        if let Some(uri) = window["folder"].as_str() {
            push_unique(&mut known.open, uri);
        }
    }

    if let Some(workspaces) = root["profileAssociations"]["workspaces"].as_object() {
        for uri in workspaces.keys() {
            if !known.open.contains(uri) {
                push_unique(&mut known.others, uri);
            }
        }
    }
    known
}

/// `%APPDATA%\Code\User\globalStorage\storage.json`.
pub fn default_storage_json() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| {
        PathBuf::from(appdata)
            .join("Code")
            .join("User")
            .join("globalStorage")
            .join("storage.json")
    })
}

/// Read VS Code's `storage.json` (read-only). Empty on any problem.
pub fn read_known_folders(storage_json: &Path) -> KnownFolders {
    let too_big = std::fs::metadata(storage_json).map_or(true, |m| m.len() > MAX_STORAGE_BYTES);
    if too_big {
        return KnownFolders::default();
    }
    std::fs::read_to_string(storage_json)
        .map(|text| parse_known_folders(&text))
        .unwrap_or_default()
}

/// The known folders as candidates. Folders of open windows come first, in
/// their priority order. All the others rank equally, so a name that fits
/// several of them is not guessed.
fn to_candidates(known: &KnownFolders) -> Vec<Candidate> {
    let open = known.open.iter().enumerate();
    open.map(|(i, uri)| (uri, crate::rank_by_position(i)))
        .chain(known.others.iter().map(|uri| (uri, 0)))
        .filter_map(|(uri, rank)| {
            Some(Candidate {
                names: vec![folder_base_name(uri)?],
                path: uri.clone(),
                rank,
            })
        })
        .collect()
}

pub(crate) fn candidates(sources: &Sources) -> Option<Vec<Candidate>> {
    let storage = sources.vscode_storage.as_deref()?;
    Some(to_candidates(&read_known_folders(storage)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;

    fn build_state(titles: &[String], list: &KnownFolders, exists: &dyn Fn(&str) -> bool) -> State {
        crate::build_state(
            crate::app(crate::VSCODE),
            titles,
            &to_candidates(list),
            exists,
        )
    }

    fn s(v: &str) -> String {
        v.to_string()
    }

    #[test]
    fn root_names_from_titles() {
        let cases = [
            (
                "main.rs - ContextFlow - Visual Studio Code",
                Some("ContextFlow"),
            ),
            ("ContextFlow - Visual Studio Code", Some("ContextFlow")),
            (
                "● a.rs - my proj (Workspace) - Visual Studio Code",
                Some("my proj"),
            ),
            (
                "TESTING.md - ContextFlow - Visual Studio Code",
                Some("ContextFlow"),
            ),
            ("Notepad", None),
            (" - Visual Studio Code", None),
            ("Visual Studio Code", None),
        ];
        for (title, want) in cases {
            assert_eq!(root_name_from_title(title).as_deref(), want, "{title}");
        }
    }

    #[test]
    fn safe_uris_only() {
        assert!(is_safe_folder_uri("file:///c%3A/Users/me/app"));
        for bad in [
            "",
            "https://example.com",
            "vscode-remote://ssh-remote+host/home/me",
            "file://server/share",
            "--extensionDevelopmentPath=x",
            "file:///c:/a b",
            "file:///c:/a\nb",
            "file:///c:/a%00b",
            &format!("file:///{}", "x".repeat(MAX_URI_LEN)),
        ] {
            assert!(!is_safe_folder_uri(bad), "{bad}");
        }
    }

    #[test]
    fn base_names_are_percent_decoded() {
        assert_eq!(
            folder_base_name("file:///c%3A/My%20Projects/Cool%20App/").as_deref(),
            Some("Cool App")
        );
        assert_eq!(
            folder_base_name("file:///c%3A/x/%E2%9C%93").as_deref(),
            Some("✓")
        );
        assert_eq!(
            folder_path("file:///c%3A/My%20Projects/app").as_deref(),
            Some("c:/My Projects/app")
        );
        // Broken escapes and invalid UTF-8 are refused, not guessed at.
        assert_eq!(folder_base_name("file:///c%3A/bad%zz"), None);
        assert_eq!(folder_base_name("file:///c%3A/bad%FF"), None);
        assert_eq!(folder_base_name("file:///c%3A/bad%"), None);
        assert_eq!(folder_base_name("https://x"), None);
    }

    const STORAGE: &str = r#"{
        "theme": "x",
        "windowsState": {
            "lastActiveWindow": { "folder": "file:///c%3A/work/Active" },
            "openedWindows": [ { "folder": "file:///c%3A/work/Second" }, { "backupPath": "x" } ]
        },
        "backupWorkspaces": { "folders": [ { "folderUri": "file:///c%3A/work/Backup" } ] },
        "profileAssociations": { "workspaces": {
            "file:///c%3A/work/Active": "__default__profile__",
            "file:///c%3A/old/Other": "__default__profile__",
            "vscode-remote://ssh/x": "__default__profile__"
        } }
    }"#;

    #[test]
    fn known_folders_are_read_in_priority_order() {
        let k = parse_known_folders(STORAGE);
        assert_eq!(
            k.open,
            [
                "file:///c%3A/work/Active",
                "file:///c%3A/work/Backup",
                "file:///c%3A/work/Second"
            ]
        );
        // Not repeated, and non-file addresses are dropped.
        assert_eq!(k.others, ["file:///c%3A/old/Other"]);
        assert_eq!(parse_known_folders("not json"), KnownFolders::default());
        assert_eq!(parse_known_folders("{}"), KnownFolders::default());
    }

    fn known(open: &[&str], others: &[&str]) -> KnownFolders {
        KnownFolders {
            open: open.iter().map(|u| s(u)).collect(),
            others: others.iter().map(|u| s(u)).collect(),
        }
    }

    #[test]
    fn build_state_matches_case_insensitively_and_prefers_open_windows() {
        let k = known(
            &["file:///c%3A/work/ContextFlow"],
            &["file:///c%3A/old/contextflow", "file:///c%3A/old/Other"],
        );
        let st = build_state(
            &[
                s("main.rs - contextflow - Visual Studio Code"),
                s("Other - Visual Studio Code"),
            ],
            &k,
            &|_| true,
        );
        // The open window's folder beats the older duplicate.
        assert_eq!(
            st.windows[0].path.as_deref(),
            Some("file:///c%3A/work/ContextFlow")
        );
        assert_eq!(st.windows[0].name.as_deref(), Some("contextflow"));
        // Unique among the others: resolved.
        assert_eq!(
            st.windows[1].path.as_deref(),
            Some("file:///c%3A/old/Other")
        );
        assert_eq!(st.path_count(), 2);
    }

    #[test]
    fn ambiguous_missing_and_unnamed_windows_get_no_folder() {
        let k = known(
            &[],
            &[
                "file:///c%3A/a/backend",
                "file:///c%3A/b/backend",
                "file:///c%3A/a/gone",
            ],
        );
        let exists = |u: &str| !u.ends_with("gone");
        let st = build_state(
            &[
                s("backend - Visual Studio Code"), // two existing folders: ambiguous
                s("gone - Visual Studio Code"),    // folder no longer exists
                s("Welcome - Visual Studio Code"), // not a known folder
                s("Untitled-1 - Notepad"),         // not a VS Code title
            ],
            &k,
            &exists,
        );
        assert!(st.windows.iter().all(|w| w.path.is_none()), "{st:#?}");
        assert_eq!(st.windows.len(), 4, "every window is still listed");
        assert_eq!(st.path_count(), 0);

        // Once only one of the two still exists, it is no longer ambiguous.
        let only_a = |u: &str| u.contains("/a/");
        let st = build_state(&[s("backend - Visual Studio Code")], &k, &only_a);
        assert_eq!(
            st.windows[0].path.as_deref(),
            Some("file:///c%3A/a/backend")
        );
    }

    #[test]
    fn missing_or_huge_files_give_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_known_folders(&dir.path().join("nope.json")),
            KnownFolders::default()
        );
        let file = dir.path().join("storage.json");
        std::fs::write(&file, STORAGE).unwrap();
        assert_eq!(read_known_folders(&file).open.len(), 3);
        assert!(!folder_exists("file:///definitely/not/a/real/folder"));
    }
}

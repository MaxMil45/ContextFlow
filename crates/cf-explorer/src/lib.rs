//! File Explorer integration: which folder each Explorer window has open.
//!
//! Windows' own Shell (`Shell.Application`) lists every open Explorer window
//! with the folder it shows. At capture, each Explorer window's title
//! (`Downloads - File Explorer`, or just `Downloads` on Windows 10) is matched
//! to an open folder of that name, and the folder's full path is kept so
//! restore can reopen it with `explorer.exe <path>` in its own window.
//!
//! Only real folders on disk are saved: special places (This PC, Recycle Bin,
//! search results) have no path and are skipped, never guessed.

use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// `ApplicationState::adapter_id` for File Explorer carrying [`ExplorerState`].
pub const ADAPTER_ID: &str = "explorer";
pub const STATE_SCHEMA: u32 = 1;

const TITLE_SUFFIX: &str = " - File Explorer";
const MAX_PATH_LEN: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderWindow {
    pub window_title: String,
    /// The folder name from the title.
    pub name: Option<String>,
    /// Full path of the folder, if it could be worked out.
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorerState {
    pub schema_version: u32,
    pub windows: Vec<FolderWindow>,
}

impl ExplorerState {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    pub fn from_value(v: &serde_json::Value) -> Option<Self> {
        serde_json::from_value::<Self>(v.clone())
            .ok()
            .filter(|s| s.schema_version == STATE_SCHEMA)
    }

    pub fn path_count(&self) -> usize {
        self.windows.iter().filter(|w| w.path.is_some()).count()
    }
}

/// The application id of File Explorer (`explorer.exe`).
pub fn is_explorer(application_id: &str) -> bool {
    application_id == "explorer"
}

/// The folder name from a window title (`Downloads - File Explorer` -> `Downloads`).
pub fn name_from_title(title: &str) -> Option<String> {
    let t = title.trim();
    let name = t.strip_suffix(TITLE_SUFFIX).unwrap_or(t).trim();
    (!name.is_empty() && name != "File Explorer").then(|| name.to_string())
}

/// Only plain, absolute local or network folders are saved or reopened.
pub fn is_safe_path(path: &str) -> bool {
    let p = path.trim();
    let b = p.as_bytes();
    let drive = b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/');
    let unc = p.starts_with("\\\\") && p.len() > 4;
    (drive || unc)
        && p.len() <= MAX_PATH_LEN
        && !p.contains('"')
        && !p.chars().any(char::is_control)
}

/// One folder Explorer has open (a window, or a tab of one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenFolder {
    /// What the Shell calls it: the name in the window title.
    pub name: String,
    pub path: String,
}

/// The folders open in Explorer right now, from Windows' Shell. Empty when the
/// Shell can't be asked (it is never an error to have none).
pub fn open_folders() -> Vec<OpenFolder> {
    const SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
(New-Object -ComObject Shell.Application).Windows() | ForEach-Object {
  if ((Split-Path -Leaf $_.FullName) -ieq 'explorer.exe') {
    $p = $_.Document.Folder.Self.Path
    if ($p) { [Console]::Out.WriteLine($_.LocationName + "`t" + $p) }
  }
}
"#;
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(out) = cmd.output() else {
        return Vec::new();
    };
    parse_open_folders(&String::from_utf8_lossy(&out.stdout))
}

/// `name<TAB>path` lines, as [`open_folders`] reads them.
pub fn parse_open_folders(text: &str) -> Vec<OpenFolder> {
    text.lines()
        .filter_map(|l| {
            let (name, path) = l.trim_end_matches('\r').split_once('\t')?;
            (!name.trim().is_empty() && !path.trim().is_empty()).then(|| OpenFolder {
                name: name.trim().to_string(),
                path: path.trim().to_string(),
            })
        })
        .collect()
}

/// Whether a saved folder still exists.
pub fn folder_exists(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Match each Explorer window title to an open folder of that name. Windows with
/// the same name take the matching folders in turn; a name that matches nothing
/// (This PC, a search) gets no path.
pub fn build_state(
    titles: &[String],
    open: &[OpenFolder],
    exists: &dyn Fn(&str) -> bool,
) -> ExplorerState {
    let mut used = vec![false; open.len()];
    let windows = titles
        .iter()
        .map(|title| {
            let name = name_from_title(title);
            let path = name.as_deref().and_then(|n| {
                let i = (0..open.len()).find(|&i| {
                    !used[i]
                        && open[i].name.eq_ignore_ascii_case(n)
                        && is_safe_path(&open[i].path)
                        && exists(&open[i].path)
                })?;
                used[i] = true;
                Some(open[i].path.clone())
            });
            FolderWindow {
                window_title: title.clone(),
                name,
                path,
            }
        })
        .collect();
    ExplorerState {
        schema_version: STATE_SCHEMA,
        windows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_explorer_titles() {
        assert_eq!(
            name_from_title("Downloads - File Explorer").as_deref(),
            Some("Downloads")
        );
        assert_eq!(
            name_from_title("Local").as_deref(),
            Some("Local"),
            "Windows 10 titles"
        );
        assert_eq!(name_from_title("File Explorer"), None);
        assert_eq!(name_from_title("  "), None);
    }

    #[test]
    fn titles_are_matched_to_open_folders_in_turn() {
        let open = parse_open_folders(
            "Local\tC:\\Users\\me\\AppData\\Local\r\nsrc\tC:\\work\\a\\src\nsrc\tC:\\work\\b\\src\nThis PC\t\n",
        );
        assert_eq!(open.len(), 3, "special places without a path are dropped");
        let titles = vec![
            "src - File Explorer".to_string(),
            "src - File Explorer".to_string(),
            "Local - File Explorer".to_string(),
            "This PC - File Explorer".to_string(),
        ];
        let s = build_state(&titles, &open, &|_| true);
        let paths: Vec<_> = s.windows.iter().map(|w| w.path.as_deref()).collect();
        assert_eq!(
            paths,
            vec![
                Some(r"C:\work\a\src"),
                Some(r"C:\work\b\src"),
                Some(r"C:\Users\me\AppData\Local"),
                None
            ]
        );
        assert_eq!(
            build_state(&titles, &open, &|_| false).path_count(),
            0,
            "gone from disk"
        );
    }

    #[test]
    fn only_plain_folders_are_safe() {
        assert!(is_safe_path(r"C:\Users\me"));
        assert!(is_safe_path(r"\\nas\share\photos"));
        assert!(
            !is_safe_path("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
            "shell namespaces"
        );
        assert!(!is_safe_path("/select,C:\\x"));
        assert!(!is_safe_path(r#"C:\a" /e"#));
    }
}

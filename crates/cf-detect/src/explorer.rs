//! File Explorer: Windows' own Shell (`Shell.Application`) lists every open
//! Explorer window with the folder it shows. Each window's title (`Downloads -
//! File Explorer`, or just `Downloads` on Windows 10) is matched to an open
//! folder of that name, and restore reopens it with `explorer.exe <path>`.
//!
//! Only real folders on disk are saved: special places (This PC, Recycle Bin,
//! search results) have no path and are skipped, never guessed.

use std::process::{Command, Stdio};

use crate::{Candidate, Sources};

const TITLE_SUFFIX: &str = " - File Explorer";

/// The folder name from a window title (`Downloads - File Explorer` -> `Downloads`).
pub fn name_from_title(title: &str) -> Option<String> {
    let t = title.trim();
    let name = t.strip_suffix(TITLE_SUFFIX).unwrap_or(t).trim();
    (!name.is_empty() && name != "File Explorer").then(|| name.to_string())
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

/// The open folders as candidates, in the Shell's order, so windows with the
/// same name take the matching folders in turn.
fn to_candidates(open: &[OpenFolder]) -> Vec<Candidate> {
    open.iter()
        .enumerate()
        .map(|(i, f)| Candidate {
            names: vec![f.name.clone()],
            path: f.path.clone(),
            rank: crate::rank_by_position(i),
        })
        .collect()
}

pub(crate) fn candidates(sources: &Sources) -> Option<Vec<Candidate>> {
    sources.explorer.then(|| to_candidates(&open_folders()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::State;

    fn build_state(titles: &[String], list: &[OpenFolder], exists: &dyn Fn(&str) -> bool) -> State {
        crate::build_state(
            crate::app(crate::EXPLORER),
            titles,
            &to_candidates(list),
            exists,
        )
    }

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
}

//! Finding an application again after an update moved it.
//!
//! Some apps (Discord, Slack and other Squirrel-installed apps) install into a
//! folder named after their version, such as
//! `C:\Users\me\AppData\Local\Discord\app-1.0.9267\Discord.exe`. After an
//! update the old folder is deleted and a new `app-<newer version>` appears
//! next to it, so the saved path stops existing. This module finds the newest
//! sibling folder that still contains the same program.

/// `app-1.2.3` -> `[1, 2, 3]`. Anything else (`packages`, `app-beta`,
/// `app-1.0.9.extra`, `app-`) is not a version folder.
fn version_of(dir_name: &str) -> Option<Vec<u64>> {
    let prefix = dir_name.get(..4)?;
    if !prefix.eq_ignore_ascii_case("app-") {
        return None;
    }
    let numbers = &dir_name[4..];
    if numbers.is_empty() {
        return None;
    }
    numbers
        .split('.')
        .map(|part| {
            if !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()) {
                part.parse().ok()
            } else {
                None
            }
        })
        .collect()
}

/// If `exe_path` is `<base>\app-<version>\<file>`, return
/// `<base>\<newest app-* folder that contains <file>>\<file>`.
///
/// `list_dirs(base)` returns the names of the sub-folders of `base`;
/// `exists(path)` says whether a file exists. Versions are compared
/// numerically, one component at a time, so 1.0.10 is newer than 1.0.9.
/// Returns `None` when the path is not in a version-shaped folder or no sibling
/// contains the file.
pub(crate) fn newest_versioned_sibling(
    exe_path: &str,
    list_dirs: &dyn Fn(&str) -> Vec<String>,
    exists: &dyn Fn(&str) -> bool,
) -> Option<String> {
    let (parent, file) = exe_path.rsplit_once('\\')?;
    let (base, folder) = parent.rsplit_once('\\')?;
    if base.is_empty() || file.is_empty() {
        return None;
    }
    version_of(folder)?;

    list_dirs(base)
        .into_iter()
        .filter_map(|dir| {
            let version = version_of(&dir)?;
            let candidate = format!("{base}\\{dir}\\{file}");
            exists(&candidate).then_some((version, candidate))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const SAVED: &str = r"C:\Users\u\AppData\Local\Discord\app-1.0.9\Discord.exe";
    const BASE: &str = r"C:\Users\u\AppData\Local\Discord";

    fn resolve(dirs: &[&str], existing: &[&str]) -> Option<String> {
        let dirs: Vec<String> = dirs.iter().map(|d| d.to_string()).collect();
        let existing: HashSet<String> = existing.iter().map(|p| p.to_string()).collect();
        newest_versioned_sibling(
            SAVED,
            &|base| {
                assert_eq!(base, BASE, "only the parent folder is listed");
                dirs.clone()
            },
            &|p| existing.contains(p),
        )
    }

    #[test]
    fn picks_newest_numeric_version() {
        let found = resolve(
            &["app-1.0.9", "app-1.0.10", "app-1.0.2", "app-0.9.99"],
            &[
                r"C:\Users\u\AppData\Local\Discord\app-1.0.10\Discord.exe",
                r"C:\Users\u\AppData\Local\Discord\app-1.0.2\Discord.exe",
                r"C:\Users\u\AppData\Local\Discord\app-0.9.99\Discord.exe",
            ],
        );
        // 1.0.10 beats 1.0.9 and 1.0.2 even though "10" < "9" as text.
        assert_eq!(
            found.as_deref(),
            Some(r"C:\Users\u\AppData\Local\Discord\app-1.0.10\Discord.exe")
        );
    }

    #[test]
    fn ignores_non_version_folders() {
        for bad in [
            "packages",
            "app-beta",
            "app-1.x",
            "app-",
            "app-1.0.9.extra",
            "ap-1.0.1",
            "app-1..2",
        ] {
            assert_eq!(version_of(bad), None, "{bad}");
        }
        assert_eq!(version_of("app-1.0.10"), Some(vec![1, 0, 10]));
        assert_eq!(version_of("APP-2"), Some(vec![2]));

        // Non-version siblings are never chosen, even if they contain the file.
        let found = resolve(
            &["packages", "app-beta", "app-1.0.3"],
            &[
                r"C:\Users\u\AppData\Local\Discord\packages\Discord.exe",
                r"C:\Users\u\AppData\Local\Discord\app-beta\Discord.exe",
                r"C:\Users\u\AppData\Local\Discord\app-1.0.3\Discord.exe",
            ],
        );
        assert_eq!(
            found.as_deref(),
            Some(r"C:\Users\u\AppData\Local\Discord\app-1.0.3\Discord.exe")
        );
    }

    #[test]
    fn none_when_file_missing_everywhere() {
        // Version folders exist but none contains the program.
        assert_eq!(resolve(&["app-1.0.9", "app-1.0.10"], &[]), None);
        assert_eq!(resolve(&[], &[]), None);
    }

    #[test]
    fn none_when_saved_path_is_not_in_a_version_folder() {
        let never = |_: &str| -> Vec<String> { panic!("must not list anything") };
        let no = |_: &str| true;
        for path in [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Apps\app\x.exe",
            "x.exe",
            r"\app-1.0.0\x.exe",
        ] {
            assert_eq!(newest_versioned_sibling(path, &never, &no), None, "{path}");
        }
    }
}

// ---------------------------------------------------------------------------
// Helper processes
//
// Some apps show their window from a helper process that their main program
// starts: Steam's window belongs to `bin\cef\cef.win64\steamwebhelper.exe`,
// started by `Steam.exe`. Starting the helper on its own does nothing, so the
// program to start is the one that launched it.

/// Folder names that are shared by many apps. A program directly inside one of
/// these is never treated as an app's own launcher.
const SHARED_FOLDERS: &[&str] = &[
    "program files",
    "program files (x86)",
    "programs",
    "appdata",
    "local",
    "roaming",
    "users",
    "windows",
];

fn parent_dir(path: &str) -> Option<&str> {
    path.rsplit_once('\\').map(|(d, _)| d)
}

fn file_stem(path: &str) -> &str {
    let file = path.rsplit_once('\\').map_or(path, |(_, f)| f);
    file.rsplit_once('.').map_or(file, |(s, _)| s)
}

/// Whether `dir` is an app's own folder: at least three levels deep (e.g.
/// `C:\Program Files (x86)\Steam`), not a shared folder like `Program Files`,
/// and not inside Windows.
fn is_app_folder(dir: &str) -> bool {
    let parts: Vec<&str> = dir.split('\\').filter(|p| !p.is_empty()).collect();
    let Some(last) = parts.last() else {
        return false;
    };
    parts.len() >= 3
        && !SHARED_FOLDERS.contains(&last.to_lowercase().as_str())
        && !parts
            .get(1)
            .is_some_and(|p| p.eq_ignore_ascii_case("windows"))
}

/// `dir` is `ancestor` or inside it (case-insensitive, as on Windows).
fn is_within(dir: &str, ancestor: &str) -> bool {
    let (d, a) = (dir.to_lowercase(), ancestor.to_lowercase());
    d == a || d.starts_with(&format!("{a}\\"))
}

/// The program to start instead of `exe`, given the programs that started it
/// (`ancestors`: parent first, then its parent, ...).
///
/// An ancestor counts only while it is installed in an app folder that
/// contains `exe` (so `explorer.exe`, a terminal, or an unrelated launcher
/// never does). The outermost such ancestor wins. Returns `None` when `exe`
/// itself is the program to start.
pub fn launcher_from_ancestry(exe: &str, ancestors: &[String]) -> Option<String> {
    let exe_dir = parent_dir(exe)?;
    let mut launcher: Option<&String> = None;
    for a in ancestors {
        match parent_dir(a) {
            Some(dir) if is_app_folder(dir) && is_within(exe_dir, dir) => launcher = Some(a),
            _ => break,
        }
    }
    launcher.filter(|l| !l.eq_ignore_ascii_case(exe)).cloned()
}

/// For snapshots saved before [`launcher_from_ancestry`] existed: if `exe` is
/// a helper (its name ends in "helper", like `steamwebhelper.exe`), look in
/// the folders above it, within the app's own folder, for the program whose
/// name it starts with (`Steam.exe`). The longest matching name wins.
pub(crate) fn helper_launcher(exe: &str, exists: &dyn Fn(&str) -> bool) -> Option<String> {
    let stem = file_stem(exe);
    let lower = stem.to_lowercase();
    if !lower.ends_with("helper") {
        return None;
    }
    // Candidate names: every prefix of the helper's name, longest first, with
    // separators trimmed ("Battle.net Helper" -> "Battle.net").
    let char_ends: Vec<usize> = stem.char_indices().map(|(i, _)| i).skip(3).collect();
    let mut names: Vec<&str> = char_ends
        .iter()
        .rev()
        .map(|&i| stem[..i].trim_end_matches([' ', '-', '_', '.']))
        .filter(|n| n.chars().count() >= 3)
        .collect();
    names.dedup();

    let mut dir = parent_dir(exe)?;
    for _ in 0..4 {
        if !is_app_folder(dir) {
            break;
        }
        for name in &names {
            let candidate = format!("{dir}\\{name}.exe");
            if exists(&candidate) {
                return Some(candidate);
            }
        }
        dir = parent_dir(dir)?;
    }
    None
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    const HELPER: &str = r"C:\Program Files (x86)\Steam\bin\cef\cef.win64\steamwebhelper.exe";
    const STEAM: &str = r"C:\Program Files (x86)\Steam\Steam.exe";

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn steam_helper_is_started_through_steam() {
        let ancestry = s(&[STEAM, r"C:\Windows\explorer.exe"]);
        assert_eq!(
            launcher_from_ancestry(HELPER, &ancestry).as_deref(),
            Some(STEAM)
        );
    }

    #[test]
    fn nested_helpers_resolve_to_the_outermost_launcher() {
        let ancestry = s(&[HELPER, HELPER, STEAM, r"C:\Windows\explorer.exe"]);
        assert_eq!(
            launcher_from_ancestry(HELPER, &ancestry).as_deref(),
            Some(STEAM)
        );
    }

    #[test]
    fn unrelated_parents_are_ignored() {
        let code = r"C:\Users\me\AppData\Local\Programs\Microsoft VS Code\Code.exe";
        // Started from Explorer, a terminal, or another app's folder.
        for parent in [
            r"C:\Windows\explorer.exe",
            r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1\WindowsTerminal.exe",
            r"C:\Program Files (x86)\Steam\Steam.exe",
        ] {
            assert_eq!(
                launcher_from_ancestry(code, &s(&[parent])),
                None,
                "{parent}"
            );
        }
        // Same program as its parent (Chrome, Electron): nothing to change.
        let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
        assert_eq!(launcher_from_ancestry(chrome, &s(&[chrome])), None);
    }

    #[test]
    fn a_program_loose_in_a_shared_folder_is_never_a_launcher() {
        let child = r"C:\Program Files\Foo\foo.exe";
        let loose = r"C:\Program Files\setup.exe";
        assert_eq!(launcher_from_ancestry(child, &s(&[loose])), None);
        assert!(!is_app_folder(r"C:\Users\me\AppData\Local"));
        assert!(!is_app_folder(r"C:\Windows\System32"));
        assert!(is_app_folder(r"C:\Program Files (x86)\Steam"));
    }

    #[test]
    fn old_snapshots_find_steam_above_its_helper() {
        let exists = |p: &str| p.eq_ignore_ascii_case(STEAM);
        assert_eq!(
            helper_launcher(HELPER, &exists).map(|p| p.to_lowercase()),
            Some(STEAM.to_lowercase())
        );
    }

    #[test]
    fn helper_with_spaces_finds_its_app() {
        let helper = r"C:\Program Files (x86)\Battle.net\Battle.net Helper.exe";
        let app = r"C:\Program Files (x86)\Battle.net\Battle.net.exe";
        let exists = |p: &str| p == app;
        assert_eq!(helper_launcher(helper, &exists).as_deref(), Some(app));
    }

    #[test]
    fn non_helpers_and_unfound_launchers_are_left_alone() {
        let yes = |_: &str| true;
        assert_eq!(helper_launcher(r"C:\Program Files\Foo\foo.exe", &yes), None);
        let no = |_: &str| false;
        assert_eq!(helper_launcher(HELPER, &no), None);
    }
}

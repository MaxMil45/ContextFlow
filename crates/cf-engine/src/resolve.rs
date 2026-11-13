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

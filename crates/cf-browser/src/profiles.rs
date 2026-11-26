//! Which browser profile a window belongs to.
//!
//! A browser opens a window in a profile when started with
//! `--profile-directory=<folder>`: `Default`, `Profile 1`, ... The extension
//! can't say which folder it runs in. It does keep the id it sends us in its
//! own storage inside that profile's folder, as plain text in the log of the
//! storage database (`Local Extension Settings/<extension id>/*.log`). So the
//! folder is found by looking for that id in each profile's copy.
//!
//! Nothing is guessed: no match, or a match in two folders (a copied profile),
//! gives `None`, and the window is then opened in whatever profile the browser
//! picks, as before.

use std::path::{Path, PathBuf};

/// Storage files larger than this are not read (the id is written once, early).
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// The browser's folder of profile folders (`User Data`), if it is installed.
pub fn user_data_dir(browser: &str) -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let rest = match browser {
        "chrome" => r"Google\Chrome\User Data",
        "msedge" => r"Microsoft\Edge\User Data",
        _ => return None,
    };
    let dir = PathBuf::from(local).join(rest);
    dir.is_dir().then_some(dir)
}

/// Whether `name` is a profile folder the browsers create: `Default`, or
/// `Profile` and a number. Nothing else is ever put on a command line.
pub fn is_safe_folder(name: &str) -> bool {
    name == "Default"
        || name
            .strip_prefix("Profile ")
            .is_some_and(|n| is_number(n) && n.len() <= 4)
}

/// Whether any storage file of `folder` holds `profile_id` (as text).
fn holds_id(folder: &Path, extension_id: &str, profile_id: &str) -> bool {
    let storage = folder.join("Local Extension Settings").join(extension_id);
    let Ok(files) = std::fs::read_dir(storage) else {
        return false;
    };
    let wanted = profile_id.as_bytes();
    files.flatten().any(|f| {
        let name = f.file_name().to_string_lossy().to_lowercase();
        if !(name.ends_with(".log") || name.ends_with(".ldb")) {
            return false;
        }
        if f.metadata().map_or(true, |m| m.len() > MAX_FILE_BYTES) {
            return false;
        }
        std::fs::read(f.path())
            .is_ok_and(|bytes| bytes.windows(wanted.len()).any(|window| window == wanted))
    })
}

/// The profile folder (`Profile 2`) whose extension storage holds
/// `profile_id`, or `None` when none or more than one does.
///
/// `user_data` is the browser's `User Data` folder. Only the folders a browser
/// names profiles with (`Default`, `Profile <n>`) are looked at, and only a
/// safe name is returned.
pub fn profile_folder(user_data: &Path, extension_id: &str, profile_id: &str) -> Option<String> {
    if profile_id.is_empty() {
        return None;
    }
    let mut found: Vec<String> = std::fs::read_dir(user_data)
        .ok()?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_safe_folder(n))
        .filter(|n| holds_id(&user_data.join(n), extension_id, profile_id))
        .collect();
    (found.len() == 1).then(|| found.remove(0))
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXT: &str = "dklnklfompfninkhpanjobnohejplfnj";

    /// A `User Data` folder with the given (profile folder, stored text) pairs.
    fn user_data(profiles: &[(&str, Option<&str>)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, stored) in profiles {
            let folder = dir.path().join(name);
            std::fs::create_dir_all(&folder).unwrap();
            if let Some(text) = stored {
                let storage = folder.join("Local Extension Settings").join(EXT);
                std::fs::create_dir_all(&storage).unwrap();
                std::fs::write(storage.join("000003.log"), text.as_bytes()).unwrap();
            }
        }
        dir
    }

    #[test]
    fn the_profile_holding_the_id_is_found() {
        let d = user_data(&[
            ("Default", None),
            ("Profile 1", Some("\u{1}profileId\"aaaa-1111\"")),
            ("Profile 2", Some("\u{1}profileId\"bbbb-2222\"")),
            ("Profile 3", Some("unrelated")),
        ]);
        assert_eq!(
            profile_folder(d.path(), EXT, "bbbb-2222").as_deref(),
            Some("Profile 2")
        );
        assert_eq!(
            profile_folder(d.path(), EXT, "aaaa-1111").as_deref(),
            Some("Profile 1")
        );
    }

    #[test]
    fn no_match_or_two_matches_give_nothing() {
        let d = user_data(&[
            ("Profile 1", Some("\"copy-id\"")),
            ("Profile 2", Some("\"copy-id\"")),
            ("Default", Some("\"only-here\"")),
        ]);
        assert_eq!(profile_folder(d.path(), EXT, "nowhere"), None);
        // A copied profile holds the same id twice: not told apart, not guessed.
        assert_eq!(profile_folder(d.path(), EXT, "copy-id"), None);
        assert_eq!(
            profile_folder(d.path(), EXT, "only-here").as_deref(),
            Some("Default")
        );
        assert_eq!(profile_folder(d.path(), EXT, ""), None);
        assert_eq!(profile_folder(&d.path().join("missing"), EXT, "x"), None);
    }

    #[test]
    fn only_profile_folders_are_looked_at() {
        let d = user_data(&[
            ("Guest Profile", Some("\"guest-id\"")),
            ("System Profile", Some("\"guest-id\"")),
            ("Profile x; calc", Some("\"guest-id\"")),
        ]);
        assert_eq!(profile_folder(d.path(), EXT, "guest-id"), None);
    }

    #[test]
    fn folder_names_for_the_command_line_are_plain() {
        for ok in ["Default", "Profile 1", "Profile 12"] {
            assert!(is_safe_folder(ok), "{ok}");
        }
        for bad in [
            "",
            "Profile 1; calc",
            r"..\x",
            "\"quoted\"",
            " Default",
            "Profile ",
            "Profile 1 --incognito",
            "Work",
            "Guest Profile",
            "Profile 12345",
        ] {
            assert!(!is_safe_folder(bad), "{bad}");
        }
    }

    #[test]
    fn unknown_browsers_have_no_user_data_folder() {
        assert_eq!(user_data_dir("firefox"), None);
    }

    /// Against this computer's real browsers; prints what it finds. Run with
    /// `cargo test -p cf-browser real_profiles -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the real browser folders"]
    fn real_profiles_are_found_for_the_connected_extensions() {
        let live = crate::default_state_dir().expect("a data folder");
        for (browser, _) in crate::BROWSERS {
            let Some(user_data) = user_data_dir(browser) else {
                println!("{browser}: not installed");
                continue;
            };
            for state in crate::read_live_states(&live, browser) {
                let folder =
                    profile_folder(&user_data, crate::install::EXTENSION_ID, &state.profile_id);
                println!("{browser}: connected extension is in {folder:?}");
            }
        }
    }
}

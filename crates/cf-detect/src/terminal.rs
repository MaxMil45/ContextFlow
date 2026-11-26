//! Windows Terminal: the folder each tab's shell is in.
//!
//! A terminal's title is whatever the shell last said, so it names no folder.
//! Instead the terminal window's own process is asked which shells run below
//! it (`cf_windows::window_shell_folders`), and the folder of each is saved:
//! the first tab's as the window's `path`, the others in `tabs`.
//!
//! What this can and cannot see:
//! * `cmd` reports the folder it is in now. PowerShell does not change the
//!   folder of its process when you `cd`, so it reports the folder it was
//!   **started** in (a tab opened by "Open in Terminal", or `wt -d`, starts in
//!   the project folder; a tab opened in the home folder and moved from there
//!   is saved as the home folder), unless the user has installed the opt-in
//!   hook (`contextflow terminal hook install`): then it is the folder the
//!   shell itself reported last.
//! * WSL and `bash` tabs are not saved: their Windows-side folder says nothing
//!   about where the Linux shell is.
//! * Split panes come back as tabs, and tab order is the order the shells were
//!   started in.
//!
//! When two windows belong to one terminal process their shells can't be
//! told apart, so nothing is saved for them rather than a guess.

use std::collections::HashMap;

use crate::{is_dir, is_safe_path, OpenWindow, Sources, State, Window, STATE_SCHEMA};

/// Only plain absolute folders that cannot be read as part of the terminal's
/// own command line: a `;` would start a new command.
pub fn is_safe(path: &str) -> bool {
    is_safe_path(path) && !path.contains(';')
}

/// `wt -w new -d <first> ; new-tab -d <second> ; ...`: one new window with a
/// tab per folder. Each `;` is an argument of its own, which is how the
/// terminal reads a separator.
pub fn open_args(path: &str, more: &[String]) -> Vec<String> {
    let mut args: Vec<String> = ["-w", "new", "-d", path].map(String::from).into();
    for folder in more.iter().filter(|f| is_safe(f)) {
        args.extend([";", "new-tab", "-d", folder].map(String::from));
    }
    args
}

/// Windows Terminal is a Store app, which can't be started with arguments by
/// its saved path. Its alias `wt.exe` can.
pub fn launcher() -> Option<String> {
    let alias = std::path::Path::new(&std::env::var_os("LOCALAPPDATA")?)
        .join(r"Microsoft\WindowsApps\wt.exe");
    alias
        .is_file()
        .then(|| alias.to_string_lossy().into_owned())
}

pub fn detect(windows: &[OpenWindow<'_>], sources: &Sources) -> Option<State> {
    sources
        .terminal
        .then(|| detect_with(windows, &cf_windows::window_shell_folders, &is_dir))
}

type Shells<'a> = &'a dyn Fn(isize) -> Option<(u32, Vec<String>)>;

fn detect_with(
    windows: &[OpenWindow<'_>],
    shells: Shells<'_>,
    exists: &dyn Fn(&str) -> bool,
) -> State {
    let found: Vec<Option<(u32, Vec<String>)>> = windows.iter().map(|w| shells(w.handle)).collect();
    let mut per_process: HashMap<u32, usize> = HashMap::new();
    for (pid, _) in found.iter().flatten() {
        *per_process.entry(*pid).or_default() += 1;
    }
    let windows = windows
        .iter()
        .zip(found)
        .map(|(w, found)| {
            let folders: Vec<String> = match found {
                Some((pid, folders)) if per_process.get(&pid) == Some(&1) => folders
                    .into_iter()
                    .filter(|f| is_safe(f) && exists(f))
                    .collect(),
                _ => Vec::new(),
            };
            let mut folders = folders.into_iter();
            Window {
                window_title: w.title.to_string(),
                name: None,
                path: folders.next(),
                tabs: folders.collect(),
            }
        })
        .collect();
    State {
        schema_version: STATE_SCHEMA,
        windows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows<'a>(titles: &[&'a str]) -> Vec<OpenWindow<'a>> {
        titles
            .iter()
            .enumerate()
            .map(|(i, t)| OpenWindow {
                title: t,
                handle: i as isize + 1,
            })
            .collect()
    }

    #[test]
    fn each_window_gets_its_tabs_folders_first_tab_first() {
        let shells = |h: isize| match h {
            1 => Some((
                10,
                vec![r"C:\work\api".to_string(), r"C:\work\web".to_string()],
            )),
            2 => Some((20, vec![r"C:\notes".to_string()])),
            _ => None,
        };
        let state = detect_with(&windows(&["a", "b", "c"]), &shells, &|_| true);
        assert_eq!(state.windows[0].path.as_deref(), Some(r"C:\work\api"));
        assert_eq!(state.windows[0].tabs, [r"C:\work\web"]);
        assert_eq!(state.windows[1].path.as_deref(), Some(r"C:\notes"));
        assert!(state.windows[1].tabs.is_empty());
        assert_eq!(state.windows[2].path, None);
        assert_eq!(state.path_count(), 2);
    }

    #[test]
    fn windows_sharing_one_process_are_not_guessed() {
        let shells = |_| Some((10, vec![r"C:\a".to_string()]));
        let state = detect_with(&windows(&["a", "b"]), &shells, &|_| true);
        assert_eq!(state.path_count(), 0);
    }

    #[test]
    fn folders_that_are_gone_or_unsafe_are_dropped_and_the_next_one_leads() {
        let shells = |_| {
            Some((
                10,
                vec![
                    r"C:\gone".to_string(),
                    "--help".to_string(),
                    r"C:\a;b".to_string(),
                    r"C:\here".to_string(),
                ],
            ))
        };
        let state = detect_with(&windows(&["a"]), &shells, &|p| p != r"C:\gone");
        assert_eq!(state.windows[0].path.as_deref(), Some(r"C:\here"));
        assert!(state.windows[0].tabs.is_empty());
    }

    #[test]
    fn the_command_line_is_one_new_window_with_a_tab_per_folder() {
        let args = open_args(r"C:\a", &[r"C:\b".to_string(), r"C:\c;d".to_string()]);
        assert_eq!(
            args,
            ["-w", "new", "-d", r"C:\a", ";", "new-tab", "-d", r"C:\b"]
        );
        assert_eq!(open_args(r"C:\a", &[]), ["-w", "new", "-d", r"C:\a"]);
    }

    #[test]
    fn terminal_state_round_trips_and_a_window_without_tabs_does_not_mention_them() {
        let state = detect_with(
            &windows(&["a"]),
            &|_| Some((1, vec![r"C:\a".to_string(), r"C:\b".to_string()])),
            &|_| true,
        );
        let (app, back) = crate::saved(Some(crate::TERMINAL), &state.to_value()).unwrap();
        assert_eq!(app.label, "Windows Terminal");
        assert_eq!(back, state);
        // Old hashes stay valid: the field is only written when it has content.
        let plain = detect_with(
            &windows(&["a"]),
            &|_| Some((1, vec![r"C:\a".to_string()])),
            &|_| true,
        );
        assert!(!plain.to_value().to_string().contains("tabs"));
    }
}

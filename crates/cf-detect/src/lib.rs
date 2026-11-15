//! Works out what each window of an editor or of File Explorer has open (a
//! folder, solution or project), without needing an extension in that app.
//!
//! Every app works the same way: the window title gives a name, the name is
//! looked up in a list the app (or Windows) keeps, and the matching path is
//! saved so restore can reopen it. [`build_state`] is that one routine; each
//! entry of [`APPS`] says how to read one app's titles and where its list is.
//! Adding an app is a table entry and a module with its two parsers.
//!
//! Matching is cautious: when two entries fit equally well, no path is saved
//! rather than a guess.

mod explorer;
mod jetbrains;
mod vscode;
mod vstudio;

use std::cmp::Reverse;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const STATE_SCHEMA: u32 = 1;
const MAX_PATH_LEN: usize = 1024;

/// `ApplicationState::adapter_id` of each app's saved [`State`].
pub const VSCODE: &str = "vscode";
pub const VISUAL_STUDIO: &str = "visualstudio";
pub const EXPLORER: &str = "explorer";
pub const JETBRAINS: &str = "jetbrains";

/// One window and what it had open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub window_title: String,
    /// The folder, solution or project name taken from the title.
    /// (`root_name` in VS Code states saved by older versions.)
    #[serde(alias = "root_name")]
    pub name: Option<String>,
    /// What to reopen: a full path, or for VS Code a `file:///` address. `None`
    /// if it could not be worked out. (`folder_uri` in older VS Code states.)
    #[serde(alias = "folder_uri")]
    pub path: Option<String>,
}

/// What an application's windows had open: its `adapter_state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub schema_version: u32,
    pub windows: Vec<Window>,
}

impl State {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Windows that have something to reopen.
    pub fn path_count(&self) -> usize {
        self.windows.iter().filter(|w| w.path.is_some()).count()
    }
}

/// The state saved with an application and the app it belongs to. `None` when
/// the application carries none, or one from a newer, unknown schema.
pub fn saved(
    adapter_id: Option<&str>,
    adapter_state: &serde_json::Value,
) -> Option<(&'static App, State)> {
    let app = APPS.iter().find(|a| Some(a.adapter_id) == adapter_id)?;
    let state: State = serde_json::from_value(adapter_state.clone()).ok()?;
    (state.schema_version == STATE_SCHEMA).then_some((app, state))
}

/// Where each app keeps the list its windows are looked up in. The default
/// saves nothing; [`Sources::installed`] is what this computer has.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// VS Code's `storage.json`.
    pub vscode_storage: Option<PathBuf>,
    /// Each installed Visual Studio's settings file, with its recent solutions.
    pub visual_studio_settings: Vec<PathBuf>,
    /// Ask Windows which folders File Explorer has open.
    pub explorer: bool,
    /// Each JetBrains IDE's `recentProjects.xml`.
    pub jetbrains_recent: Vec<PathBuf>,
}

impl Sources {
    pub fn installed() -> Self {
        Self {
            vscode_storage: vscode::default_storage_json(),
            visual_studio_settings: vstudio::default_settings_files(),
            explorer: true,
            jetbrains_recent: jetbrains::default_recent_files(),
        }
    }
}

/// Something a window may have open, from the app's own list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The names a window showing it may have in its title.
    pub names: Vec<String>,
    pub path: String,
    /// Higher is the better match (more recently used, or listed first).
    /// Equal ranks are a tie, and a tie is never guessed.
    pub rank: u64,
}

/// A window of an app, as capture saw it.
#[derive(Debug, Clone, Copy)]
pub struct OpenWindow<'a> {
    pub title: &'a str,
    /// The system's handle for it (`HWND`): valid only while the window lives,
    /// and never saved.
    pub handle: isize,
}

/// How an app's windows are told apart.
enum Lookup {
    /// The name in the window's title is looked up in a list the app keeps.
    ByTitle(fn(&Sources) -> Option<Vec<Candidate>>),
    /// The system says what each window shows (File Explorer does).
    ByWindow(fn(&[OpenWindow<'_>], &Sources) -> Option<State>),
}

/// One supported app: how to read its titles, where its list is, how to reopen.
pub struct App {
    pub adapter_id: &'static str,
    /// Its name, for messages.
    pub label: &'static str,
    /// Whether an application id is this app.
    pub is_app: fn(&str) -> bool,
    /// The name in a window title, and a path when the title shows one.
    pub parse_title: fn(&str) -> (Option<String>, Option<String>),
    /// Whether a saved path may be put on a command line.
    pub is_safe: fn(&str) -> bool,
    /// Whether a saved path still exists on this computer.
    pub exists: fn(&str) -> bool,
    /// The command-line arguments that reopen a saved path in its own window.
    pub open_args: fn(&str) -> Vec<String>,
    lookup: Lookup,
    /// Several windows may show the same entry (otherwise each is used once).
    shared: bool,
    /// Report unmatched windows even when no window was matched.
    always_report: bool,
    /// What capture says about `{n}` windows that could not be matched.
    unmatched: &'static str,
}

pub static APPS: [App; 4] = [
    App {
        adapter_id: VSCODE,
        label: "VS Code",
        is_app: |id| id == "code",
        parse_title: |t| (vscode::root_name_from_title(t), None),
        is_safe: vscode::is_safe_folder_uri,
        exists: vscode::folder_exists,
        open_args: |uri| vec!["--new-window".into(), "--folder-uri".into(), uri.into()],
        lookup: Lookup::ByTitle(vscode::candidates),
        shared: true,
        always_report: true,
        unmatched:
            "the folder for {n} window(s) could not be worked out; they will reopen without one",
    },
    App {
        adapter_id: VISUAL_STUDIO,
        label: "Visual Studio",
        is_app: |id| id == "devenv",
        parse_title: |t| (vstudio::name_from_title(t), None),
        is_safe: is_safe_path,
        exists: |p| Path::new(p).exists(),
        // Each `devenv.exe <path>` starts its own Visual Studio window.
        open_args: one_arg,
        lookup: Lookup::ByTitle(vstudio::candidates),
        shared: false,
        always_report: true,
        unmatched: "the solution or folder for {n} window(s) could not be worked out; they will reopen without one",
    },
    App {
        adapter_id: EXPLORER,
        label: "File Explorer",
        is_app: |id| id == "explorer",
        parse_title: |t| (explorer::name_from_title(t), None),
        is_safe: is_safe_path,
        exists: is_dir,
        // `explorer.exe <folder>` opens it in a new window.
        open_args: one_arg,
        lookup: Lookup::ByWindow(explorer::detect),
        shared: false,
        always_report: false,
        unmatched: "{n} window(s) show a place that isn't a folder on disk; they will reopen as ordinary Explorer windows",
    },
    App {
        adapter_id: JETBRAINS,
        label: "JetBrains IDE",
        is_app: jetbrains::is_jetbrains,
        parse_title: jetbrains::parse_title,
        is_safe: is_safe_path,
        exists: is_dir,
        // `<ide>64.exe <project folder>` opens it (in the running IDE if there is one).
        open_args: one_arg,
        lookup: Lookup::ByTitle(jetbrains::candidates),
        shared: false,
        always_report: false,
        unmatched:
            "the project for {n} window(s) could not be worked out; they will reopen without one",
    },
];

fn one_arg(path: &str) -> Vec<String> {
    vec![path.to_string()]
}

pub(crate) fn is_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Only plain, absolute local or network paths are ever saved or reopened:
/// nothing that could be read as a command-line switch.
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

impl App {
    /// The name an open window of this app shows in its title.
    pub fn name_from_title(&self, title: &str) -> Option<String> {
        (self.parse_title)(title).0
    }

    /// A saved path as a plain local path, for showing to the user.
    pub fn local_path(&self, saved: &str) -> Option<String> {
        match self.adapter_id {
            VSCODE => vscode::folder_path(saved),
            _ => Some(saved.to_string()),
        }
    }

    /// What these windows have open: looked up in the app's own list by the
    /// name in their titles, or, for File Explorer, asked of the system by
    /// window. `None` when `sources` has no list for this app.
    pub fn detect(&self, windows: &[OpenWindow<'_>], sources: &Sources) -> Option<State> {
        match &self.lookup {
            Lookup::ByTitle(candidates) => {
                let candidates = candidates(sources)?;
                let titles: Vec<String> = windows.iter().map(|w| w.title.to_string()).collect();
                Some(build_state(self, &titles, &candidates, &self.exists))
            }
            Lookup::ByWindow(detect) => detect(windows, sources),
        }
    }

    /// What to tell the user about windows that showed a name but could not
    /// be matched, if anything.
    pub fn unmatched_warning(&self, state: &State) -> Option<String> {
        let unmatched = state
            .windows
            .iter()
            .filter(|w| w.name.is_some() && w.path.is_none())
            .count();
        (unmatched > 0 && (self.always_report || state.path_count() > 0)).then(|| {
            format!(
                "{}: {}",
                self.label,
                self.unmatched.replace("{n}", &unmatched.to_string())
            )
        })
    }
}

/// Work out what each window has open from its title.
///
/// A path shown in the title wins. Otherwise the name in the title is looked
/// up among `candidates`: the highest-ranked safe, existing match is taken,
/// and if the best two rank equally nothing is taken. Unless the app shares
/// entries between windows, each candidate is used for one window only, so
/// same-named windows get the matches in turn.
pub fn build_state(
    app: &App,
    titles: &[String],
    candidates: &[Candidate],
    exists: &dyn Fn(&str) -> bool,
) -> State {
    let usable = |path: &str| (app.is_safe)(path) && exists(path);
    let mut taken = vec![false; candidates.len()];
    let windows = titles
        .iter()
        .map(|title| {
            let (name, shown) = (app.parse_title)(title);
            let path = match shown.filter(|p| usable(p)) {
                Some(shown) => {
                    for (i, c) in candidates.iter().enumerate() {
                        taken[i] |= !app.shared && c.path.eq_ignore_ascii_case(&shown);
                    }
                    Some(shown)
                }
                None => name.as_deref().and_then(|n| {
                    let mut matches: Vec<usize> = (0..candidates.len())
                        .filter(|&i| {
                            let names = &candidates[i].names;
                            !taken[i] && names.iter().any(|x| x.eq_ignore_ascii_case(n))
                        })
                        .filter(|&i| usable(&candidates[i].path))
                        .collect();
                    matches.sort_by_key(|&i| Reverse(candidates[i].rank));
                    match matches.as_slice() {
                        [] => None,
                        [a, b, ..] if candidates[*a].rank == candidates[*b].rank => None,
                        [a, ..] => {
                            taken[*a] = !app.shared;
                            Some(candidates[*a].path.clone())
                        }
                    }
                }),
            };
            Window {
                window_title: title.clone(),
                name,
                path,
            }
        })
        .collect();
    State {
        schema_version: STATE_SCHEMA,
        windows,
    }
}

/// The rank of entry `index` in a list whose order is its priority.
fn rank_by_position(index: usize) -> u64 {
    u64::MAX - index as u64
}

#[cfg(test)]
fn app(adapter_id: &str) -> &'static App {
    APPS.iter().find(|a| a.adapter_id == adapter_id).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_absolute_paths_are_safe() {
        assert!(is_safe_path(r"C:\Users\me\App.sln"));
        assert!(is_safe_path(r"C:\"));
        assert!(is_safe_path(r"\\server\share\App.sln"));
        for bad in [
            "--help",
            r#"C:\a" /run"#,
            "relative\\path",
            "/select,C:\\x",
            "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
        ] {
            assert!(!is_safe_path(bad), "{bad}");
        }
    }

    #[test]
    fn state_round_trips_and_rejects_unknown_apps_and_schemas() {
        let state = State {
            schema_version: STATE_SCHEMA,
            windows: vec![Window {
                window_title: "x - Visual Studio Code".into(),
                name: Some("x".into()),
                path: Some("file:///c%3A/x".into()),
            }],
        };
        let (app, back) = saved(Some(VSCODE), &state.to_value()).unwrap();
        assert_eq!((app.adapter_id, &back), (VSCODE, &state));
        assert!(saved(Some("browser"), &state.to_value()).is_none());
        assert!(saved(None, &state.to_value()).is_none());
        assert!(saved(Some(VSCODE), &serde_json::Value::Null).is_none());
        let future = serde_json::json!({"schema_version": 99, "windows": []});
        assert!(saved(Some(VSCODE), &future).is_none());
    }

    #[test]
    fn vscode_states_saved_by_older_versions_still_load() {
        let old = serde_json::json!({"schema_version": 1, "windows": [{
            "window_title": "main.rs - app - Visual Studio Code",
            "root_name": "app",
            "folder_uri": "file:///c%3A/work/app"
        }]});
        let (app, state) = saved(Some(VSCODE), &old).unwrap();
        assert_eq!(state.windows[0].name.as_deref(), Some("app"));
        assert_eq!(
            state.windows[0].path.as_deref(),
            Some("file:///c%3A/work/app")
        );
        assert_eq!(
            app.local_path("file:///c%3A/work/app").as_deref(),
            Some("c:/work/app")
        );
    }

    #[test]
    fn unmatched_windows_are_reported_per_app() {
        let window = |name: Option<&str>, path: Option<&str>| Window {
            window_title: String::new(),
            name: name.map(String::from),
            path: path.map(String::from),
        };
        let state = |windows| State {
            schema_version: STATE_SCHEMA,
            windows,
        };
        let none_matched = state(vec![window(Some("a"), None), window(None, None)]);
        assert_eq!(
            app(VSCODE).unmatched_warning(&none_matched).as_deref(),
            Some("VS Code: the folder for 1 window(s) could not be worked out; they will reopen without one")
        );
        // "This PC" alone is not worth a warning; next to a saved folder it is.
        assert_eq!(app(EXPLORER).unmatched_warning(&none_matched), None);
        let some = state(vec![
            window(Some("a"), None),
            window(Some("b"), Some(r"C:\b")),
        ]);
        assert!(app(EXPLORER).unmatched_warning(&some).is_some());
        let all = state(vec![window(Some("b"), Some(r"C:\b"))]);
        assert_eq!(app(VSCODE).unmatched_warning(&all), None);
    }
}

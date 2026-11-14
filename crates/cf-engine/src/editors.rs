//! Reopening what an app had open: VS Code's folders, Visual Studio's
//! solutions and folders, and File Explorer's folders.
//!
//! Each saved window that had something open is started on its own command
//! line (`Code.exe --new-window --folder-uri <file:///...>`, or
//! `devenv.exe <path>`). The new desktop window is identified (the one that
//! was not there a moment ago) and moved to its saved position straight away.
//!
//! If the editor is already running, items that already have a window are left
//! alone (the normal step just repositions them), so a second double-click
//! never opens duplicates. Windows without a resolved folder or solution are
//! not touched here: they follow the ordinary behaviour for any app.

use std::collections::HashSet;
use std::time::Duration;

use cf_core::{ApplicationState, MonitorInfo, WindowState};

use crate::browser::{TabRestore, Wording};
use crate::window_open::{place_saved, wait_for_new_window};
use crate::{window_app_id, Budget, Desktop, PlaceError, RestoreOptions};

/// One thing to reopen in its own window.
#[derive(Debug, Clone)]
pub(crate) struct OpenItem {
    /// Command-line arguments that open it.
    pub args: Vec<String>,
    /// The name its window title shows, to tell whether it is already open.
    pub name: Option<String>,
    /// The saved window's title, to find its saved position.
    pub window_title: String,
}

/// What an editor needs reopened.
#[derive(Debug, Clone)]
pub(crate) struct EditorWork {
    pub items: Vec<OpenItem>,
    /// Saved items that were refused (not safe to put on a command line).
    pub notes: Vec<String>,
    /// The name an open window of this editor shows in its title.
    pub title_name: fn(&str) -> Option<String>,
    pub wording: &'static Wording,
    /// The editor's name, for messages.
    pub label: &'static str,
}

impl EditorWork {
    pub(crate) fn count(&self) -> usize {
        self.items.len()
    }
}

/// The saved folders or solutions to reopen for `app`, if it carries any.
pub(crate) fn editor_work(app: &ApplicationState) -> Option<EditorWork> {
    let mut notes: Vec<String> = Vec::new();
    let work = match app.adapter_id.as_deref() {
        Some(cf_vscode::ADAPTER_ID) => {
            let state = cf_vscode::VsCodeState::from_value(&app.adapter_state)?;
            let mut items = Vec::new();
            for w in &state.windows {
                let Some(uri) = w.folder_uri.as_deref() else {
                    continue;
                };
                // Never pass anything but a plain folder address on the command line.
                if !cf_vscode::is_safe_folder_uri(uri) {
                    notes
                        .push("a saved folder address was not safe to open and was skipped".into());
                    continue;
                }
                items.push(OpenItem {
                    args: vec!["--new-window".into(), "--folder-uri".into(), uri.into()],
                    name: w.root_name.clone(),
                    window_title: w.window_title.clone(),
                });
            }
            EditorWork {
                items,
                notes,
                title_name: cf_vscode::root_name_from_title,
                wording: &crate::browser::FOLDERS,
                label: "VS Code",
            }
        }
        Some(cf_vstudio::ADAPTER_ID) => {
            let state = cf_vstudio::VisualStudioState::from_value(&app.adapter_state)?;
            let mut items = Vec::new();
            for w in &state.windows {
                let Some(path) = w.path.as_deref() else {
                    continue;
                };
                if !cf_vstudio::is_safe_path(path) {
                    notes.push("a saved solution path was not safe to open and was skipped".into());
                    continue;
                }
                items.push(OpenItem {
                    // Each `devenv.exe <path>` starts its own Visual Studio window.
                    args: vec![path.to_string()],
                    name: w.name.clone(),
                    window_title: w.window_title.clone(),
                });
            }
            EditorWork {
                items,
                notes,
                title_name: cf_vstudio::name_from_title,
                wording: &crate::browser::SOLUTIONS,
                label: "Visual Studio",
            }
        }
        Some(cf_explorer::ADAPTER_ID) => {
            let state = cf_explorer::ExplorerState::from_value(&app.adapter_state)?;
            let mut items = Vec::new();
            for w in &state.windows {
                let Some(path) = w.path.as_deref() else {
                    continue;
                };
                if !cf_explorer::is_safe_path(path) {
                    notes.push("a saved folder path was not safe to open and was skipped".into());
                    continue;
                }
                items.push(OpenItem {
                    // `explorer.exe <folder>` opens it in a new window.
                    args: vec![path.to_string()],
                    name: w.name.clone(),
                    window_title: w.window_title.clone(),
                });
            }
            EditorWork {
                items,
                notes,
                title_name: cf_explorer::name_from_title,
                wording: &crate::browser::FOLDERS,
                label: "File Explorer",
            }
        }
        _ => return None,
    };
    (!work.items.is_empty() || !work.notes.is_empty()).then_some(work)
}

/// Open a window for every saved item that is not already open.
///
/// `current_titles` are the titles of the editor's windows right now (empty
/// when it was not running). Counts in the result are *items*: `tabs_opened` is
/// the number of folders or solutions opened.
#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_windows(
    desktop: &dyn Desktop,
    app: &ApplicationState,
    exe: &str,
    work: &EditorWork,
    saved: &[&WindowState],
    monitors: &[MonitorInfo],
    current_titles: &[String],
    opts: &RestoreOptions,
    budget: &Budget,
    log: &mut dyn FnMut(&str),
) -> TabRestore {
    let mut out = TabRestore::default();
    out.notes.extend(work.notes.iter().cloned());

    let open_names: Vec<String> = current_titles
        .iter()
        .filter_map(|t| (work.title_name)(t))
        .map(|n| n.to_lowercase())
        .collect();
    let mut known: HashSet<isize> = desktop
        .windows()
        .unwrap_or_default()
        .iter()
        .filter(|w| window_app_id(w).as_deref() == Some(app.application_id.as_str()))
        .map(|w| w.hwnd)
        .collect();
    let mut used_saved = vec![false; saved.len()];
    let mut opened = 0usize;

    for window in &work.items {
        // Already open: the ordinary step repositions that window.
        let already_open = window
            .name
            .as_deref()
            .is_some_and(|n| open_names.contains(&n.to_lowercase()));
        if already_open {
            continue;
        }

        let geometry = (0..saved.len())
            .find(|&i| !used_saved[i] && saved[i].title == window.window_title)
            .inspect(|&i| used_saved[i] = true);

        if budget.expired() {
            out.failures
                .push("not opened: the restore ran out of time".into());
            break;
        }

        log(&format!(
            "opening a saved {} in {}",
            work.wording.singular, work.label
        ));
        if let Err(e) = desktop.open_app_window(exe, &window.args) {
            out.failures.push(format!(
                "could not open a window for a saved {}: {e}",
                work.wording.singular
            ));
            continue;
        }
        out.tabs_opened += 1;

        // The first window may need the whole app to start; later ones are quick.
        let wait = if opened == 0 {
            opts.timeout
        } else {
            opts.timeout.min(Duration::from_secs(15))
        };
        opened += 1;
        let wait = wait.min(budget.remaining());
        match wait_for_new_window(desktop, &app.application_id, &known, wait, opts.settle) {
            None => out.failures.push(format!(
                "a window for a saved {} did not appear within {} seconds",
                work.wording.singular,
                wait.as_secs()
            )),
            Some(hwnd) => {
                known.insert(hwnd);
                out.claimed.insert(hwnd);
                if let Some(i) = geometry {
                    out.used_saved.push(i);
                    match place_saved(desktop, hwnd, saved[i], monitors) {
                        Ok(()) => out.placed += 1,
                        Err(PlaceError::AccessDenied) => out.failures.push(format!(
                            "a {} window belongs to a program running as administrator and can't be moved",
                            work.label
                        )),
                        Err(PlaceError::Other(e)) => out
                            .failures
                            .push(format!("a window opened but couldn't be moved: {e}")),
                    }
                }
            }
        }
    }
    out
}

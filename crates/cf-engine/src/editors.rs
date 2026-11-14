//! Reopening VS Code's saved folders.
//!
//! Each saved window that had a folder is opened with
//! `Code.exe --new-window --folder-uri <file:///...>`. The new desktop window is
//! identified (the one that was not there a moment ago) and moved to its saved
//! position straight away.
//!
//! If VS Code is already running, folders that already have a window are left
//! alone (the normal step just repositions them), so a second double-click
//! never opens duplicates. Windows without a resolved folder are not touched
//! here: they follow the ordinary behaviour for any app.

use std::collections::HashSet;
use std::time::Duration;

use cf_core::{ApplicationState, MonitorInfo, WindowState};
use cf_vscode::{is_safe_folder_uri, root_name_from_title, VsCodeState};

use crate::browser::TabRestore;
use crate::window_open::{place_saved, wait_for_new_window};
use crate::{window_app_id, Budget, Desktop, PlaceError, RestoreOptions};

/// The saved folders to restore for `app`, if it carries any.
pub(crate) fn folder_state(app: &ApplicationState) -> Option<VsCodeState> {
    if app.adapter_id.as_deref() != Some(cf_vscode::ADAPTER_ID) {
        return None;
    }
    VsCodeState::from_value(&app.adapter_state).filter(|s| s.folder_count() > 0)
}

/// Open a window for every saved folder that is not already open.
///
/// `current_titles` are the titles of VS Code's windows right now (empty when
/// it was not running). Counts in the result are *folders*: `tabs_opened` is
/// the number of folders opened.
#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_folders(
    desktop: &dyn Desktop,
    app: &ApplicationState,
    exe: &str,
    state: &VsCodeState,
    saved: &[&WindowState],
    monitors: &[MonitorInfo],
    current_titles: &[String],
    opts: &RestoreOptions,
    budget: &Budget,
    log: &mut dyn FnMut(&str),
) -> TabRestore {
    let mut out = TabRestore::default();

    let open_names: Vec<String> = current_titles
        .iter()
        .filter_map(|t| root_name_from_title(t))
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

    for window in &state.windows {
        let Some(uri) = window.folder_uri.as_deref() else {
            continue;
        };
        // Never pass anything but a plain folder address on the command line.
        if !is_safe_folder_uri(uri) {
            out.notes
                .push("a saved folder address was not safe to open and was skipped".into());
            continue;
        }
        // Already open: the ordinary step repositions that window.
        let already_open = window
            .root_name
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

        log(&format!("opening a folder in {}", app.application_id));
        let args = vec![
            "--new-window".to_string(),
            "--folder-uri".to_string(),
            uri.to_string(),
        ];
        if let Err(e) = desktop.open_app_window(exe, &args) {
            out.failures
                .push(format!("could not open a window for a saved folder: {e}"));
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
                "a window for a saved folder did not appear within {} seconds",
                wait.as_secs()
            )),
            Some(hwnd) => {
                known.insert(hwnd);
                out.claimed.insert(hwnd);
                if let Some(i) = geometry {
                    out.used_saved.push(i);
                    match place_saved(desktop, hwnd, saved[i], monitors) {
                        Ok(()) => out.placed += 1,
                        Err(PlaceError::AccessDenied) => out.failures.push(
                            "a VS Code window belongs to a program running as administrator and can't be moved"
                                .into(),
                        ),
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

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
use crate::{window_app_id, Budget, Desktop, RestoreOptions};

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
    pub title_name: fn(&str) -> (Option<String>, Option<String>),
    pub wording: &'static Wording,
    /// The editor's name, for messages.
    pub label: &'static str,
    /// A program to start it with instead of the saved one (see
    /// `cf_detect::App::launcher`).
    pub launcher: Option<String>,
    /// Windows are told apart by number, not by name (see
    /// `cf_detect::App::counted`).
    pub counted: bool,
}

impl EditorWork {
    pub(crate) fn count(&self) -> usize {
        self.items.len()
    }
}

/// The saved folders, solutions or projects to reopen for `app`, if it carries any.
pub(crate) fn editor_work(app: &ApplicationState) -> Option<EditorWork> {
    let (editor, state) = cf_detect::saved(app.adapter_id.as_deref(), &app.adapter_state)?;
    let wording = match editor.adapter_id {
        cf_detect::VISUAL_STUDIO => &crate::browser::SOLUTIONS,
        cf_detect::JETBRAINS => &crate::browser::PROJECTS,
        _ => &crate::browser::FOLDERS,
    };
    let mut items = Vec::new();
    let mut notes = Vec::new();
    for w in &state.windows {
        let Some(path) = w.path.as_deref() else {
            continue;
        };
        // Never pass anything but a plain path or folder address on the command line.
        if !(editor.is_safe)(path) {
            notes.push(format!(
                "a saved {} was not safe to open and was skipped",
                wording.singular
            ));
            continue;
        }
        items.push(OpenItem {
            args: (editor.open_args)(path, &w.tabs),
            name: w.name.clone(),
            window_title: w.window_title.clone(),
        });
    }
    (!items.is_empty() || !notes.is_empty()).then_some(EditorWork {
        items,
        notes,
        title_name: editor.parse_title,
        wording,
        label: editor.label,
        launcher: editor.launcher.and_then(|find| find()),
        counted: editor.counted,
    })
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
    // Runs while waiting for a window, with `log` (see `wait_for_new_window`).
    meanwhile: &mut crate::Meanwhile<'_>,
) -> TabRestore {
    let mut out = TabRestore::default();
    out.notes.extend(work.notes.iter().cloned());
    let exe = work.launcher.as_deref().unwrap_or(exe);

    let open_names: Vec<String> = current_titles
        .iter()
        .filter_map(|t| (work.title_name)(t).0)
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

    for (index, window) in work.items.iter().enumerate() {
        // Already open: the ordinary step repositions that window. Windows
        // that can't be told apart by name count as open while there are as
        // many open as were saved.
        let already_open = if work.counted {
            index < current_titles.len()
        } else {
            window
                .name
                .as_deref()
                .is_some_and(|n| open_names.contains(&n.to_lowercase()))
        };
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
        out.take_new_window(
            desktop,
            &app.application_id,
            &mut known,
            wait,
            geometry.map(|i| (i, saved[i])),
            monitors,
            opts.settle,
            &format!("a window for a saved {}", work.wording.singular),
            work.label,
            &mut || meanwhile(&mut *log),
        );
    }
    out
}

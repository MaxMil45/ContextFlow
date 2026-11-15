//! Reopening a browser's saved tabs.
//!
//! Restore needs no browser extension: each saved window is opened with
//! `browser.exe --new-window <address>...`. The new desktop window is then
//! identified (the one that was not there a moment ago) and moved to its saved
//! position straight away, so windows can't be mixed up.
//!
//! If the browser is already running, only tabs that are not open yet are
//! added (the extension tells us what is open); without the extension nothing
//! is added, so a second double-click never duplicates tabs.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use cf_browser::{is_restorable_url, normalize_url, BrowserAppState};
use cf_core::RestoreStatus;
use cf_core::{ApplicationState, MonitorInfo, WindowState};

use crate::window_open::{place_saved, wait_for_new_window};
use crate::{window_app_id, Budget, Desktop, PlaceError, RestoreOptions, StepReport};

/// Windows limits a command line to 32,767 characters; stay well under.
const MAX_COMMAND_LINE_CHARS: usize = 28_000;

#[derive(Debug, Default)]
pub(crate) struct TabRestore {
    pub tabs_opened: usize,
    pub placed: usize,
    /// Desktop windows opened here; the generic arranging step must leave them alone.
    pub claimed: HashSet<isize>,
    /// Indices into the app's saved windows that a window opened here took;
    /// the generic arranging step must not hand those positions out again.
    pub used_saved: Vec<usize>,
    pub failures: Vec<String>,
    pub notes: Vec<String>,
}

/// How step reports describe what was opened (shared with VS Code folders).
#[derive(Debug)]
pub(crate) struct Wording {
    pub noun: &'static str,
    /// One of them, for messages ("folder", "solution or folder").
    pub singular: &'static str,
    pub partial_code: &'static str,
}

pub(crate) const TABS: Wording = Wording {
    noun: "tab(s)",
    singular: "tab",
    partial_code: "TABS_PARTIAL",
};

pub(crate) const FOLDERS: Wording = Wording {
    noun: "folder(s)",
    singular: "folder",
    partial_code: "FOLDERS_PARTIAL",
};

pub(crate) const PROJECTS: Wording = Wording {
    noun: "project(s)",
    singular: "project",
    partial_code: "PROJECTS_PARTIAL",
};

pub(crate) const SOLUTIONS: Wording = Wording {
    noun: "solution(s) or folder(s)",
    singular: "solution or folder",
    partial_code: "SOLUTIONS_PARTIAL",
};

/// The browser tabs to restore for `app`, if it carries any.
pub(crate) fn tab_state(app: &ApplicationState) -> Option<BrowserAppState> {
    if app.adapter_id.as_deref() != Some(cf_browser::ADAPTER_ID) {
        return None;
    }
    BrowserAppState::from_value(&app.adapter_state).filter(|s| s.tab_count() > 0)
}

/// Drop trailing addresses until the command line fits.
fn fit_command_line(urls: &mut Vec<String>) -> usize {
    let mut dropped = 0;
    while urls.iter().map(|u| u.len() + 1).sum::<usize>() > MAX_COMMAND_LINE_CHARS {
        urls.pop();
        dropped += 1;
    }
    dropped
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_tabs(
    desktop: &dyn Desktop,
    app: &ApplicationState,
    exe: &str,
    state: &BrowserAppState,
    saved: &[&WindowState],
    monitors: &[MonitorInfo],
    running: bool,
    opts: &RestoreOptions,
    budget: &Budget,
    log: &mut dyn FnMut(&str),
    // Runs while waiting for a window, with `log` (see `wait_for_new_window`).
    meanwhile: &mut crate::Meanwhile<'_>,
) -> TabRestore {
    let mut out = TabRestore::default();

    // Already running: only add what is missing, and only if we can tell.
    let open_now: Option<HashSet<String>> = if running {
        match desktop.live_browser_urls(&app.application_id) {
            Some(urls) => Some(urls.into_iter().collect()),
            None => {
                out.notes.push(
                    "tabs were not reopened: the ContextFlow extension isn't connected, so it can't tell which are already open"
                        .into(),
                );
                return out;
            }
        }
    } else {
        None
    };

    let mut known: HashSet<isize> = desktop
        .windows()
        .unwrap_or_default()
        .iter()
        .filter(|w| window_app_id(w).as_deref() == Some(app.application_id.as_str()))
        .map(|w| w.hwnd)
        .collect();
    let mut used_saved = vec![false; saved.len()];

    for (n, window) in state.windows.iter().enumerate() {
        let mut seen = HashSet::new();
        let mut urls: Vec<String> = window
            .tabs
            .iter()
            .map(|t| t.url.clone())
            .filter(|u| is_restorable_url(u))
            .filter(|u| seen.insert(normalize_url(u)))
            .collect();
        if let Some(open) = &open_now {
            urls.retain(|u| !open.contains(&normalize_url(u)));
        }
        if urls.len() > opts.max_tabs_per_window {
            out.notes.push(format!(
                "a window had {} tabs; only the first {} were reopened",
                urls.len(),
                opts.max_tabs_per_window
            ));
            urls.truncate(opts.max_tabs_per_window);
        }
        let dropped = fit_command_line(&mut urls);
        if dropped > 0 {
            out.notes
                .push(format!("{dropped} tab(s) were too long to reopen"));
        }
        if urls.is_empty() {
            continue;
        }

        let geometry = (0..saved.len())
            .find(|&i| !used_saved[i] && saved[i].title == window.window_title)
            .inspect(|&i| used_saved[i] = true);

        // Out of overall time (or cancelled): open nothing more.
        if budget.expired() {
            out.failures
                .push("not opened: the restore ran out of time".into());
            break;
        }

        log(&format!(
            "opening {} tab(s) in {}",
            urls.len(),
            app.application_id
        ));
        if let Err(e) = desktop.open_browser_window(exe, &urls) {
            out.failures.push(format!(
                "could not open a window with {} tab(s): {e}",
                urls.len()
            ));
            continue;
        }
        out.tabs_opened += urls.len();

        // The first window may need the whole browser to start; later ones are quick.
        let wait = if n == 0 {
            opts.timeout
        } else {
            opts.timeout.min(Duration::from_secs(10))
        };
        // Never wait past the overall budget.
        let wait = wait.min(budget.remaining());
        let new_window = wait_for_new_window(
            desktop,
            &app.application_id,
            &known,
            wait,
            opts.settle,
            &mut || meanwhile(&mut *log),
        );

        match new_window {
            None => out.failures.push(format!(
                "a window with {} tab(s) did not appear within {} seconds",
                urls.len(),
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
                            "a browser window belongs to a program running as administrator and can't be moved"
                                .into(),
                        ),
                        Err(PlaceError::Other(e)) => {
                            out.failures.push(format!("a window opened but couldn't be moved: {e}"))
                        }
                    }
                }
            }
        }
    }
    out
}

/// Report for a browser that was not running and was started with its tabs.
pub(crate) fn started_report(
    app_id: &str,
    wording: &Wording,
    out: &TabRestore,
    began: Instant,
    out_of_time: bool,
) -> StepReport {
    let mut message = if out.claimed.is_empty() && !out.failures.is_empty() {
        out.failures.join("; ")
    } else {
        format!(
            "started; {} {} opened in {} window(s), {} arranged",
            out.tabs_opened,
            wording.noun,
            out.claimed.len(),
            out.placed
        )
    };
    if !out.claimed.is_empty() && !out.failures.is_empty() {
        message = format!("{message}; {}", out.failures.join("; "));
    }
    if !out.notes.is_empty() {
        message = format!("{message}; {}", out.notes.join("; "));
    }
    let (status, code) = if out.claimed.is_empty() && !out.failures.is_empty() {
        // No window at all: the overall budget ran out, or the browser was too slow.
        let code = if out_of_time {
            "OUT_OF_TIME"
        } else {
            "WINDOW_TIMEOUT"
        };
        (RestoreStatus::Failed, Some(code))
    } else if !out.failures.is_empty() {
        (RestoreStatus::PartialSuccess, Some(wording.partial_code))
    } else {
        (RestoreStatus::Success, None)
    };
    StepReport {
        application_id: app_id.to_string(),
        status,
        code,
        message,
        duration_ms: began.elapsed().as_millis() as u64,
    }
}

/// Add the outcome of adding missing tabs to an already-running browser to the
/// normal "windows arranged" report.
pub(crate) fn merge_into(
    mut report: StepReport,
    wording: &Wording,
    out: &TabRestore,
) -> StepReport {
    if !out.claimed.is_empty() {
        report.message = format!(
            "{}; added {} missing {} in {} new window(s)",
            report.message,
            out.tabs_opened,
            wording.noun,
            out.claimed.len()
        );
    }
    if !out.failures.is_empty() {
        report.message = format!("{}; {}", report.message, out.failures.join("; "));
        if report.status == RestoreStatus::Success {
            report.status = RestoreStatus::PartialSuccess;
            report.code = Some(wording.partial_code);
        }
    }
    if !out.notes.is_empty() {
        report.message = format!("{}; {}", report.message, out.notes.join("; "));
    }
    report
}

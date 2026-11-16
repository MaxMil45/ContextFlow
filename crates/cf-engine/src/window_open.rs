//! Steps shared by every "open a new window for this app" restore: find the
//! window that just appeared, and move it to its saved position.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use cf_core::geometry::placement_for;
use cf_core::{MonitorInfo, WindowState};

use crate::{window_app_id, Desktop, PlaceError};

/// Move `hwnd` to where `saved` was, falling back to an available monitor if
/// the original one is gone.
pub(crate) fn place_saved(
    desktop: &dyn Desktop,
    hwnd: isize,
    saved: &WindowState,
    monitors: &[MonitorInfo],
) -> Result<(), PlaceError> {
    let target = if monitors.is_empty() {
        saved.rect
    } else {
        placement_for(&saved.rect, saved.monitor_id.as_deref(), monitors).unwrap_or(saved.rect)
    };
    let placed = desktop.place(hwnd, target, saved.display_state);
    if placed.is_ok() {
        desktop.placed(saved, hwnd);
    }
    placed
}

/// Longest wait before a new window is put in place a second time.
const RECHECK_MAX: Duration = Duration::from_millis(750);

/// A window put in place the moment it appeared, to be put there once more.
///
/// Browsers and VS Code size and position a new window themselves just after
/// creating it, which can undo a placement made the instant it appeared. The
/// window is placed at once, so it is where it belongs as early as possible,
/// and placed again when `due`; if the app didn't move it, nothing visible
/// happens. Rechecks don't hold anything up: other windows open meanwhile.
#[derive(Debug, Clone)]
pub(crate) struct Recheck {
    pub hwnd: isize,
    pub saved: WindowState,
    pub due: Instant,
}

/// Put a window that has just appeared in its saved place now, and schedule
/// it to be placed once more after `settle` (at most [`RECHECK_MAX`]).
pub(crate) fn place_new(
    desktop: &dyn Desktop,
    hwnd: isize,
    saved: &WindowState,
    monitors: &[MonitorInfo],
    settle: Duration,
    rechecks: &mut Vec<Recheck>,
) -> Result<(), PlaceError> {
    let placed = place_saved(desktop, hwnd, saved, monitors);
    if placed.is_ok() {
        rechecks.push(Recheck {
            hwnd,
            saved: saved.clone(),
            due: Instant::now() + settle.min(RECHECK_MAX),
        });
    }
    placed
}

/// Wait up to `wait` for a window of `app_id` that is not in `known` (the
/// windows that existed before we asked for a new one). Returns its handle
/// as soon as it appears.
///
/// `meanwhile` runs between looks, so other applications that have finished
/// opening are put in place while this one is still on its way.
pub(crate) fn wait_for_new_window(
    desktop: &dyn Desktop,
    app_id: &str,
    known: &HashSet<isize>,
    wait: Duration,
    meanwhile: &mut dyn FnMut(),
) -> Option<isize> {
    let deadline = Instant::now() + wait;
    let mut poll = Duration::from_millis(25);
    loop {
        let found = desktop.windows().ok().and_then(|ws| {
            ws.iter()
                .find(|w| window_app_id(w).as_deref() == Some(app_id) && !known.contains(&w.hwnd))
                .map(|w| w.hwnd)
        });
        if found.is_some() {
            return found;
        }
        if Instant::now() >= deadline {
            return None;
        }
        meanwhile();
        std::thread::sleep(poll);
        poll = (poll * 2).min(Duration::from_millis(500));
    }
}

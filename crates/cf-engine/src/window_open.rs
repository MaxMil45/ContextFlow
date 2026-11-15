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
    desktop.place(hwnd, target, saved.display_state)
}

/// Longest pause after a new window appears before it is moved.
const NEW_WINDOW_SETTLE_MAX: Duration = Duration::from_millis(750);

/// Wait up to `wait` for a window of `app_id` that is not in `known` (the
/// windows that existed before we asked for a new one). Returns its handle.
///
/// Once found, pause for `settle` (at most [`NEW_WINDOW_SETTLE_MAX`]):
/// browsers and VS Code size and position a new window themselves just after
/// creating it, which would undo a placement made the instant it appeared.
///
/// `meanwhile` runs between looks, so other applications that have finished
/// opening are put in place while this one is still on its way.
pub(crate) fn wait_for_new_window(
    desktop: &dyn Desktop,
    app_id: &str,
    known: &HashSet<isize>,
    wait: Duration,
    settle: Duration,
    meanwhile: &mut dyn FnMut(),
) -> Option<isize> {
    let deadline = Instant::now() + wait;
    let mut poll = Duration::from_millis(100);
    loop {
        let found = desktop.windows().ok().and_then(|ws| {
            ws.iter()
                .find(|w| window_app_id(w).as_deref() == Some(app_id) && !known.contains(&w.hwnd))
                .map(|w| w.hwnd)
        });
        if found.is_some() {
            std::thread::sleep(settle.min(NEW_WINDOW_SETTLE_MAX));
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

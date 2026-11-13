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

/// Wait up to `wait` for a window of `app_id` that is not in `known` (the
/// windows that existed before we asked for a new one). Returns its handle.
pub(crate) fn wait_for_new_window(
    desktop: &dyn Desktop,
    app_id: &str,
    known: &HashSet<isize>,
    wait: Duration,
) -> Option<isize> {
    let deadline = Instant::now() + wait;
    let mut poll = Duration::from_millis(100);
    loop {
        let found = desktop.windows().ok().and_then(|ws| {
            ws.iter()
                .find(|w| window_app_id(w).as_deref() == Some(app_id) && !known.contains(&w.hwnd))
                .map(|w| w.hwnd)
        });
        if found.is_some() || Instant::now() >= deadline {
            return found;
        }
        std::thread::sleep(poll);
        poll = (poll * 2).min(Duration::from_millis(500));
    }
}

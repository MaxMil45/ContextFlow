//! Display-aware layouts: one workspace, a layout per display setup.
//!
//! Every snapshot records the monitors that were connected. So a workspace
//! captured docked and again on the laptop screen already holds a layout for
//! each. When nothing chooses a version (no `--version`, nothing pinned),
//! restore prefers the newest snapshot saved with the same monitors as now,
//! and falls back to the newest snapshot when none matches.

use cf_core::MonitorInfo;

/// How many recent versions are looked at for a matching display setup.
pub const LOOKBACK: usize = 20;

/// The parts of a display setup that make a layout fit: which screens, how
/// big, how they are arranged, and which one is primary.
fn fingerprint(monitors: &[MonitorInfo]) -> Vec<(String, i32, i32, i32, i32, bool)> {
    let mut f: Vec<_> = monitors
        .iter()
        .map(|m| {
            (
                m.id.clone(),
                m.rect.x,
                m.rect.y,
                m.rect.width,
                m.rect.height,
                m.primary,
            )
        })
        .collect();
    f.sort();
    f
}

/// Whether two snapshots were taken with the same screens in the same places.
pub fn same_setup(a: &[MonitorInfo], b: &[MonitorInfo]) -> bool {
    !a.is_empty() && fingerprint(a) == fingerprint(b)
}

/// From `(version, monitors)` pairs, newest first, the version to restore with
/// the `current` screens: the newest matching one, or `None` when the newest
/// already matches or nothing does (then the newest is used as before).
pub fn pick(candidates: &[(u32, Vec<MonitorInfo>)], current: &[MonitorInfo]) -> Option<u32> {
    let (newest, newest_monitors) = candidates.first()?;
    if current.is_empty() || same_setup(newest_monitors, current) {
        return None;
    }
    candidates
        .iter()
        .find(|(_, m)| same_setup(m, current))
        .map(|(v, _)| *v)
        .filter(|v| v != newest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_core::Rect;

    fn screen(id: &str, x: i32, w: i32, primary: bool) -> MonitorInfo {
        let rect = Rect {
            x,
            y: 0,
            width: w,
            height: 1080,
        };
        MonitorInfo {
            id: id.into(),
            primary,
            rect,
            work_area: rect,
            dpi: None,
        }
    }

    #[test]
    fn the_newest_layout_for_these_screens_is_chosen() {
        let laptop = vec![screen("LAPTOP", 0, 1920, true)];
        let docked = vec![
            screen("LAPTOP", 0, 1920, false),
            screen("DELL", 1920, 2560, true),
        ];
        // Newest first: v9 laptop, v8 docked, v7 docked.
        let history = vec![
            (9, laptop.clone()),
            (8, docked.clone()),
            (7, docked.clone()),
        ];
        assert_eq!(
            pick(&history, &docked),
            Some(8),
            "docked again: the newest docked layout"
        );
        assert_eq!(pick(&history, &laptop), None, "the newest already fits");
        let other = vec![screen("TV", 0, 3840, true)];
        assert_eq!(
            pick(&history, &other),
            None,
            "nothing fits: keep the newest"
        );
    }

    #[test]
    fn order_of_monitors_does_not_matter_but_arrangement_does() {
        let a = vec![screen("A", 0, 1920, true), screen("B", 1920, 1920, false)];
        let b = vec![screen("B", 1920, 1920, false), screen("A", 0, 1920, true)];
        assert!(same_setup(&a, &b));
        let swapped = vec![screen("A", 1920, 1920, true), screen("B", 0, 1920, false)];
        assert!(!same_setup(&a, &swapped));
    }
}

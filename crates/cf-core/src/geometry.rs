//! Pure window-placement calculations used by the restoration engine.
//! Kept free of OS calls so they can be unit-tested exhaustively.

use crate::{MonitorInfo, Rect};

impl Rect {
    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn intersection_area(&self, other: &Rect) -> i64 {
        let w = (self.right().min(other.right()) - self.x.max(other.x)).max(0) as i64;
        let h = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0) as i64;
        w * h
    }

    /// Shrink and shift this rectangle so it lies entirely inside `area`.
    pub fn fit_within(&self, area: &Rect) -> Rect {
        if area.width <= 0 || area.height <= 0 {
            return *self;
        }
        let width = self.width.min(area.width);
        let height = self.height.min(area.height);
        Rect {
            x: self.x.clamp(area.x, area.right() - width),
            y: self.y.clamp(area.y, area.bottom() - height),
            width,
            height,
        }
    }
}

/// Choose the display a saved window should be restored to:
/// 1. the monitor it was saved on, if still connected;
/// 2. otherwise the monitor it overlaps most;
/// 3. otherwise the primary monitor;
/// 4. otherwise any monitor.
pub fn select_monitor<'a>(
    window: &Rect,
    saved_monitor_id: Option<&str>,
    monitors: &'a [MonitorInfo],
) -> Option<&'a MonitorInfo> {
    if let Some(id) = saved_monitor_id {
        if let Some(m) = monitors.iter().find(|m| m.id == id) {
            return Some(m);
        }
    }
    monitors
        .iter()
        .filter(|m| m.rect.intersection_area(window) > 0)
        .max_by_key(|m| m.rect.intersection_area(window))
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first())
}

/// Where a window saved on one display layout belongs on another, keeping its
/// share of the screen: two windows saved side by side on a large display stay
/// side by side on a small one, instead of both being clamped to the corner.
///
/// Nothing changes when the display the window sat on is still the same size.
/// Otherwise the window's position and size are scaled by the ratio between
/// the work area it was saved on and the one it goes to. The result may still
/// need [`Rect::fit_within`].
pub fn scale_to_layout(
    window: &Rect,
    saved_monitor_id: Option<&str>,
    saved: &[MonitorInfo],
    current: &[MonitorInfo],
) -> Rect {
    let (Some(from), Some(to)) = (
        select_monitor(window, saved_monitor_id, saved),
        select_monitor(window, saved_monitor_id, current),
    ) else {
        return *window;
    };
    let (src, dst) = (&from.work_area, &to.work_area);
    if src.width <= 0 || src.height <= 0 || dst.width <= 0 || dst.height <= 0 {
        return *window;
    }
    if src.width == dst.width && src.height == dst.height {
        return *window;
    }
    let fx = f64::from(dst.width) / f64::from(src.width);
    let fy = f64::from(dst.height) / f64::from(src.height);
    let scale = |v: i32, origin: i32, f: f64| ((f64::from(v - origin)) * f).round() as i32;
    Rect {
        x: dst.x + scale(window.x, src.x, fx),
        y: dst.y + scale(window.y, src.y, fy),
        width: ((f64::from(window.width) * fx).round() as i32).max(1),
        height: ((f64::from(window.height) * fy).round() as i32).max(1),
    }
}

/// Final placement for a saved window on the current display configuration,
/// guaranteeing the window is fully reachable.
pub fn placement_for(
    window: &Rect,
    saved_monitor_id: Option<&str>,
    monitors: &[MonitorInfo],
) -> Option<Rect> {
    select_monitor(window, saved_monitor_id, monitors).map(|m| window.fit_within(&m.work_area))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn monitor(id: &str, primary: bool, rect: Rect) -> MonitorInfo {
        let work_area = Rect {
            height: rect.height - 40,
            ..rect
        };
        MonitorInfo {
            id: id.into(),
            primary,
            rect,
            work_area,
            dpi: Some(96),
        }
    }

    #[test]
    fn intersection() {
        assert_eq!(r(0, 0, 10, 10).intersection_area(&r(5, 5, 10, 10)), 25);
        assert_eq!(r(0, 0, 10, 10).intersection_area(&r(20, 20, 5, 5)), 0);
    }

    #[test]
    fn fit_within_shifts_and_shrinks() {
        let area = r(0, 0, 1920, 1040);
        assert_eq!(
            r(1800, 900, 400, 300).fit_within(&area),
            r(1520, 740, 400, 300)
        );
        assert_eq!(
            r(-50, -50, 3000, 2000).fit_within(&area),
            r(0, 0, 1920, 1040)
        );
        assert_eq!(
            r(100, 100, 800, 600).fit_within(&area),
            r(100, 100, 800, 600)
        );
    }

    #[test]
    fn side_by_side_windows_stay_side_by_side_on_a_smaller_display() {
        let big = vec![monitor("A", true, r(0, 0, 3840, 2200))];
        let small = vec![monitor("L", true, r(0, 0, 1920, 1120))];
        let left = r(0, 0, 1920, 2160);
        let right = r(1920, 0, 1920, 2160);
        let l = scale_to_layout(&left, Some("A"), &big, &small).fit_within(&small[0].work_area);
        let rr = scale_to_layout(&right, Some("A"), &big, &small).fit_within(&small[0].work_area);
        assert_eq!((l.x, l.width), (0, 960));
        assert_eq!((rr.x, rr.width), (960, 960));
    }

    #[test]
    fn the_same_size_display_changes_nothing() {
        let a = vec![monitor("A", true, r(0, 0, 1920, 1080))];
        let b = vec![monitor("B", true, r(0, 0, 1920, 1080))];
        let w = r(123, 45, 801, 599);
        assert_eq!(scale_to_layout(&w, Some("A"), &a, &b), w);
        assert_eq!(scale_to_layout(&w, Some("A"), &[], &b), w);
        assert_eq!(scale_to_layout(&w, Some("A"), &a, &[]), w);
    }

    #[test]
    fn a_second_monitor_is_scaled_into_the_one_it_falls_back_to() {
        let saved = vec![
            monitor("A", true, r(0, 0, 1920, 1080)),
            monitor("B", false, r(1920, 0, 2560, 1440)),
        ];
        let now = vec![monitor("A", true, r(0, 0, 1920, 1080))];
        // The right half of B moves to the right half of A.
        let w = r(1920 + 1280, 0, 1280, 1400);
        let placed = scale_to_layout(&w, Some("B"), &saved, &now);
        assert_eq!((placed.x, placed.width), (960, 960));
    }

    #[test]
    fn missing_monitor_falls_back_to_primary() {
        // Window was on a right-hand monitor that has been disconnected.
        let monitors = vec![monitor("DISPLAY1", true, r(0, 0, 1920, 1080))];
        let saved = r(2100, 100, 1200, 800);
        let placed = placement_for(&saved, Some("DISPLAY2"), &monitors).unwrap();
        assert_eq!(placed, r(720, 100, 1200, 800));
    }

    #[test]
    fn prefers_saved_monitor_then_overlap() {
        let monitors = vec![
            monitor("A", true, r(0, 0, 1920, 1080)),
            monitor("B", false, r(1920, 0, 2560, 1440)),
        ];
        let w = r(1800, 0, 600, 400); // mostly on B
        assert_eq!(select_monitor(&w, Some("A"), &monitors).unwrap().id, "A");
        assert_eq!(select_monitor(&w, None, &monitors).unwrap().id, "B");
        assert_eq!(select_monitor(&w, Some("gone"), &monitors).unwrap().id, "B");
        assert!(select_monitor(&w, None, &[]).is_none());
    }
}

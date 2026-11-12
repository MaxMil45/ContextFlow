//! Windows platform integration.
//!
//! Currently: read-only discovery of top-level windows and monitors.
//! Phase 2 adds launching, window identification and placement.
//!
//! Window handles (`hwnd`) are exposed for the current session only and must
//! never be persisted: Windows reuses them.

use cf_core::{DisplayState, MonitorInfo, Rect};
use serde::Serialize;
use thiserror::Error;

#[cfg(windows)]
mod imp;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("this operation is only supported on Windows")]
    Unsupported,
    #[error("operating system error: {0}")]
    Os(#[from] std::io::Error),
}

pub type Result<T, E = PlatformError> = std::result::Result<T, E>;

/// A visible, user-facing top-level window found on the desktop.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiscoveredWindow {
    /// Session-only handle. Do not store.
    pub hwnd: isize,
    pub process_id: u32,
    pub executable_path: Option<String>,
    pub title: String,
    pub class_name: String,
    pub rect: Rect,
    pub display_state: DisplayState,
    pub monitor_id: Option<String>,
}

/// Opt this process into per-monitor-v2 DPI awareness so that all coordinates
/// are physical pixels. Call once, before any window or monitor query.
pub fn init_dpi_awareness() {
    #[cfg(windows)]
    imp::init_dpi_awareness();
}

pub fn enumerate_windows() -> Result<Vec<DiscoveredWindow>> {
    #[cfg(windows)]
    {
        imp::enumerate_windows()
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::Unsupported)
    }
}

pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    #[cfg(windows)]
    {
        imp::enumerate_monitors()
    }
    #[cfg(not(windows))]
    {
        Err(PlatformError::Unsupported)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn monitors_are_discoverable() {
        init_dpi_awareness();
        let monitors = enumerate_monitors().unwrap();
        // CI runners and interactive desktops both expose at least one display.
        assert!(!monitors.is_empty());
        assert!(monitors.iter().filter(|m| m.primary).count() <= 1);
        for m in &monitors {
            assert!(m.rect.width > 0 && m.rect.height > 0);
        }
    }

    #[test]
    fn window_enumeration_does_not_fail() {
        init_dpi_awareness();
        // May legitimately be empty on a headless runner.
        let windows = enumerate_windows().unwrap();
        for w in &windows {
            assert!(!w.title.is_empty());
        }
    }
}

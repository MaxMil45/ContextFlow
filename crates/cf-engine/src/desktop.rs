use std::path::Path;

use cf_core::{DisplayState, MonitorInfo, Rect};
use cf_windows::{DiscoveredWindow, PlatformError};

#[derive(Debug)]
pub enum PlaceError {
    /// The window belongs to a more privileged (elevated) process.
    AccessDenied,
    Other(String),
}

/// Everything the restore engine needs from the operating system. A trait so
/// tests can use a scripted fake instead of opening real windows.
pub trait Desktop {
    fn windows(&self) -> Result<Vec<DiscoveredWindow>, String>;
    fn monitors(&self) -> Result<Vec<MonitorInfo>, String>;
    fn exe_exists(&self, path: &str) -> bool;
    fn launch(&self, exe: &str, args: &[String], cwd: Option<&str>) -> Result<u32, String>;
    /// Package id (`Family!AppId`) of a Microsoft Store app's executable.
    fn packaged_app_id(&self, exe: &str) -> Option<String>;
    /// Start a Microsoft Store app by its package id.
    fn launch_packaged(&self, app_user_model_id: &str) -> Result<(), String>;
    fn place(&self, hwnd: isize, rect: Rect, state: DisplayState) -> Result<(), PlaceError>;
}

/// The real desktop.
pub struct WindowsDesktop;

impl Desktop for WindowsDesktop {
    fn windows(&self) -> Result<Vec<DiscoveredWindow>, String> {
        cf_windows::enumerate_windows().map_err(|e| e.to_string())
    }

    fn monitors(&self) -> Result<Vec<MonitorInfo>, String> {
        cf_windows::enumerate_monitors().map_err(|e| e.to_string())
    }

    fn exe_exists(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn launch(&self, exe: &str, args: &[String], cwd: Option<&str>) -> Result<u32, String> {
        cf_windows::launch_process(exe, args, cwd).map_err(|e| e.to_string())
    }

    fn packaged_app_id(&self, exe: &str) -> Option<String> {
        cf_windows::packaged_app_id(exe)
    }

    fn launch_packaged(&self, app_user_model_id: &str) -> Result<(), String> {
        cf_windows::launch_packaged(app_user_model_id).map_err(|e| e.to_string())
    }

    fn place(&self, hwnd: isize, rect: Rect, state: DisplayState) -> Result<(), PlaceError> {
        match cf_windows::place_window(hwnd, rect, state) {
            Ok(()) => Ok(()),
            Err(PlatformError::Os(e)) if e.raw_os_error() == Some(5) => {
                Err(PlaceError::AccessDenied)
            }
            Err(e) => Err(PlaceError::Other(e.to_string())),
        }
    }
}

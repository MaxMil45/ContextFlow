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
    /// Names of the immediate sub-folders of `dir` (empty if it can't be read).
    fn list_dirs(&self, dir: &str) -> Vec<String>;
    fn launch(&self, exe: &str, args: &[String], cwd: Option<&str>) -> Result<u32, String>;
    /// Package id (`Family!AppId`) of a Microsoft Store app's executable.
    fn packaged_app_id(&self, exe: &str) -> Option<String>;
    /// Start a Microsoft Store app by its package id.
    fn launch_packaged(&self, app_user_model_id: &str) -> Result<(), String>;
    /// Start `exe` with `args` to open another window of an app that is
    /// already known to be safe to launch (no shell, no inherited console).
    fn open_app_window(&self, exe: &str, args: &[String]) -> Result<(), String>;
    /// Open a new browser window showing `urls` (one tab each, in order).
    fn open_browser_window(&self, exe: &str, urls: &[String]) -> Result<(), String>;
    /// Normalised addresses of the tabs open right now in `browser`, or `None`
    /// when the extension isn't connected and so the answer is unknown.
    fn live_browser_urls(&self, browser: &str) -> Option<Vec<String>>;
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

    fn list_dirs(&self, dir: &str) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
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

    fn open_app_window(&self, exe: &str, args: &[String]) -> Result<(), String> {
        cf_windows::launch_process(exe, args, None)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn open_browser_window(&self, exe: &str, urls: &[String]) -> Result<(), String> {
        // Only plain web addresses ever reach the command line, so none can be
        // mistaken for a browser option.
        if let Some(bad) = urls.iter().find(|u| !cf_browser::is_restorable_url(u)) {
            return Err(format!("refusing to open '{}'", cf_core::redact::url(bad)));
        }
        let mut args = vec!["--new-window".to_string()];
        args.extend(urls.iter().cloned());
        cf_windows::launch_process(exe, &args, None)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn live_browser_urls(&self, browser: &str) -> Option<Vec<String>> {
        cf_browser::live_urls(&cf_browser::default_state_dir()?, browser)
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

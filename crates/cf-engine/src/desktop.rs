use std::cell::{Cell, RefCell};
use std::path::Path;
use std::time::{Duration, Instant};

use cf_core::{DisplayState, MonitorInfo, Rect, WindowState};
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
    /// Put a window above the others without activating it.
    fn raise(&self, hwnd: isize);
    /// Give a window the keyboard focus, if the system allows it.
    fn focus(&self, hwnd: isize);
    /// Told when `saved` (a window of the snapshot being restored) has been
    /// put on `hwnd`, so the stacking order can be rebuilt at the end.
    fn placed(&self, _saved: &WindowState, _hwnd: isize) {}
}

/// A desktop that adds up how long the engine spends in each kind of call, so
/// a restore can say where its time went.
pub(crate) struct Timed<'a> {
    inner: &'a dyn Desktop,
    /// The snapshot's windows, top of the stack first: a window's place in
    /// this list is its place in the stack.
    stack_order: &'a [WindowState],
    /// (place in the stack, window) for each window of the snapshot put on one.
    stack: RefCell<Vec<(usize, isize)>>,
    began: Instant,
    /// When the first window was put in place, after `began`.
    first_placed: Cell<Option<Duration>>,
    scan: Cell<Duration>,
    start: Cell<Duration>,
    place: Cell<Duration>,
}

impl<'a> Timed<'a> {
    pub(crate) fn new(inner: &'a dyn Desktop, stack_order: &'a [WindowState]) -> Self {
        Self {
            inner,
            stack_order,
            stack: RefCell::default(),
            began: Instant::now(),
            first_placed: Cell::default(),
            scan: Cell::default(),
            start: Cell::default(),
            place: Cell::default(),
        }
    }

    /// Put the windows that were placed back in their saved stacking order,
    /// bottom to top, and give the top one the focus. Minimized windows are
    /// left alone: raising one would bring it back up.
    pub(crate) fn restore_stack(&self) {
        let mut stack = self.stack.borrow().clone();
        // Highest number = lowest in the stack = raised first.
        stack.sort_by_key(|&(rank, _)| std::cmp::Reverse(rank));
        stack.dedup_by_key(|&mut (_, hwnd)| hwnd);
        for &(_, hwnd) in &stack {
            self.raise(hwnd);
        }
        if let Some(&(_, top)) = stack.last() {
            self.focus(top);
        }
    }

    /// Milliseconds until the first window was in place, if one was.
    pub(crate) fn first_placed_ms(&self) -> Option<u64> {
        self.first_placed.get().map(|d| d.as_millis() as u64)
    }

    /// `total` split into phases, in milliseconds: reading the desktop
    /// (`scan`), starting programs (`start`), moving windows (`place`), and
    /// everything else, which is waiting for windows to appear (`wait`).
    pub(crate) fn phases(&self, total: Duration) -> Vec<(&'static str, u64)> {
        let (scan, start, place) = (self.scan.get(), self.start.get(), self.place.get());
        let wait = total.saturating_sub(scan + start + place);
        [
            ("scan", scan),
            ("start", start),
            ("wait", wait),
            ("place", place),
        ]
        .map(|(name, d)| (name, d.as_millis() as u64))
        .to_vec()
    }
}

fn timed<T>(sum: &Cell<Duration>, f: impl FnOnce() -> T) -> T {
    let began = Instant::now();
    let out = f();
    sum.set(sum.get() + began.elapsed());
    out
}

impl Desktop for Timed<'_> {
    fn windows(&self) -> Result<Vec<DiscoveredWindow>, String> {
        timed(&self.scan, || self.inner.windows())
    }
    fn monitors(&self) -> Result<Vec<MonitorInfo>, String> {
        timed(&self.scan, || self.inner.monitors())
    }
    fn exe_exists(&self, path: &str) -> bool {
        self.inner.exe_exists(path)
    }
    fn list_dirs(&self, dir: &str) -> Vec<String> {
        self.inner.list_dirs(dir)
    }
    fn launch(&self, exe: &str, args: &[String], cwd: Option<&str>) -> Result<u32, String> {
        timed(&self.start, || self.inner.launch(exe, args, cwd))
    }
    fn packaged_app_id(&self, exe: &str) -> Option<String> {
        self.inner.packaged_app_id(exe)
    }
    fn launch_packaged(&self, app_user_model_id: &str) -> Result<(), String> {
        timed(&self.start, || {
            self.inner.launch_packaged(app_user_model_id)
        })
    }
    fn open_app_window(&self, exe: &str, args: &[String]) -> Result<(), String> {
        timed(&self.start, || self.inner.open_app_window(exe, args))
    }
    fn open_browser_window(&self, exe: &str, urls: &[String]) -> Result<(), String> {
        timed(&self.start, || self.inner.open_browser_window(exe, urls))
    }
    fn live_browser_urls(&self, browser: &str) -> Option<Vec<String>> {
        self.inner.live_browser_urls(browser)
    }
    fn raise(&self, hwnd: isize) {
        timed(&self.place, || self.inner.raise(hwnd));
    }
    fn focus(&self, hwnd: isize) {
        timed(&self.place, || self.inner.focus(hwnd));
    }
    fn placed(&self, saved: &WindowState, hwnd: isize) {
        // Minimized windows stay minimized; they are not part of the stack.
        if saved.display_state == DisplayState::Minimized {
            return;
        }
        if let Some(rank) = self.stack_order.iter().position(|w| std::ptr::eq(w, saved)) {
            self.stack.borrow_mut().push((rank, hwnd));
        }
    }
    fn place(&self, hwnd: isize, rect: Rect, state: DisplayState) -> Result<(), PlaceError> {
        let placed = timed(&self.place, || self.inner.place(hwnd, rect, state));
        if placed.is_ok() && self.first_placed.get().is_none() {
            self.first_placed.set(Some(self.began.elapsed()));
        }
        placed
    }
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

    fn raise(&self, hwnd: isize) {
        if let Err(e) = cf_windows::raise_window(hwnd) {
            tracing::debug!(hwnd, error = %e, "could not raise a window");
        }
    }

    fn focus(&self, hwnd: isize) {
        if !cf_windows::focus_window(hwnd) {
            tracing::debug!(hwnd, "Windows did not allow giving a window the focus");
        }
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

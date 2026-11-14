use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

use cf_core::RestoreStatus;
use cf_core::{ApplicationState, DisplayState, MonitorInfo, Rect, SnapshotPayload, WindowState};
use cf_windows::DiscoveredWindow;

use super::*;

fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

fn app(id: &str) -> ApplicationState {
    ApplicationState {
        application_id: id.into(),
        executable_path: format!(r"C:\Apps\{id}.exe"),
        launch_arguments: vec![],
        working_directory: None,
        adapter_id: None,
        adapter_state: serde_json::Value::Null,
    }
}

fn saved_window(app: &str, title: &str, rect: Rect, monitor: &str) -> WindowState {
    WindowState {
        application_id: app.into(),
        title: title.into(),
        window_class: None,
        monitor_id: Some(monitor.into()),
        rect,
        display_state: DisplayState::Normal,
    }
}

fn open_window(hwnd: isize, app: &str, title: &str) -> DiscoveredWindow {
    DiscoveredWindow {
        hwnd,
        process_id: hwnd as u32,
        executable_path: Some(format!(r"C:\Apps\{app}.exe")),
        title: title.into(),
        class_name: "C".into(),
        rect: r(0, 0, 100, 100),
        display_state: DisplayState::Normal,
        monitor_id: None,
    }
}

fn monitor(id: &str, primary: bool, rect: Rect) -> MonitorInfo {
    MonitorInfo {
        id: id.into(),
        primary,
        rect,
        work_area: rect,
        dpi: None,
    }
}

/// A scripted desktop. Launching an app makes its configured windows appear.
#[derive(Default)]
struct Fake {
    monitors: Vec<MonitorInfo>,
    /// Windows already open before restore.
    open: RefCell<Vec<DiscoveredWindow>>,
    /// Windows that appear when an app (by id) is launched.
    on_launch: Vec<(String, Vec<DiscoveredWindow>)>,
    missing_exes: HashSet<String>,
    /// Sub-folders by parent folder, for `list_dirs`: (parent path, folder names).
    dirs: Vec<(String, Vec<String>)>,
    /// When set, only these exact paths exist (overrides `missing_exes`).
    existing_paths: Option<HashSet<String>>,
    /// Full path of every executable launched.
    launched_paths: RefCell<Vec<String>>,
    /// Everything done, in order: "launch:<app>" and "open:<number of urls>".
    events: RefCell<Vec<String>>,
    launch_fails: HashSet<String>,
    /// Store apps the fake knows: (application id, package id).
    packaged: Vec<(String, String)>,
    packaged_launched: RefCell<Vec<String>>,
    /// Tabs open in the running browser, as the extension reports (None = no extension).
    live_urls: Option<Vec<String>>,
    /// Windows that appear, one per call, when a browser window is opened.
    browser_windows: RefCell<Vec<DiscoveredWindow>>,
    browser_opens: RefCell<Vec<Vec<String>>>,
    /// Arguments of every `open_app_window` call (e.g. VS Code folders).
    app_window_opens: RefCell<Vec<Vec<String>>>,
    denied: HashSet<isize>,
    launched: RefCell<Vec<String>>,
    placed: RefCell<Vec<(isize, Rect, DisplayState)>>,
}

impl Desktop for Fake {
    fn windows(&self) -> Result<Vec<DiscoveredWindow>, String> {
        Ok(self.open.borrow().clone())
    }
    fn monitors(&self) -> Result<Vec<MonitorInfo>, String> {
        Ok(self.monitors.clone())
    }
    fn exe_exists(&self, path: &str) -> bool {
        // Windows file names are case-insensitive.
        if let Some(existing) = &self.existing_paths {
            return existing.iter().any(|p| p.eq_ignore_ascii_case(path));
        }
        !self.missing_exes.contains(&application_id(path))
    }
    fn list_dirs(&self, dir: &str) -> Vec<String> {
        self.dirs
            .iter()
            .find(|(parent, _)| parent == dir)
            .map(|(_, names)| names.clone())
            .unwrap_or_default()
    }
    fn open_app_window(&self, _exe: &str, args: &[String]) -> Result<(), String> {
        self.app_window_opens.borrow_mut().push(args.to_vec());
        let mut queue = self.browser_windows.borrow_mut();
        if !queue.is_empty() {
            let w = queue.remove(0);
            self.open.borrow_mut().push(w);
        }
        Ok(())
    }
    fn open_browser_window(&self, _exe: &str, urls: &[String]) -> Result<(), String> {
        self.browser_opens.borrow_mut().push(urls.to_vec());
        self.events
            .borrow_mut()
            .push(format!("open:{}", urls.len()));
        let mut queue = self.browser_windows.borrow_mut();
        if !queue.is_empty() {
            let w = queue.remove(0);
            self.open.borrow_mut().push(w);
        }
        Ok(())
    }
    fn live_browser_urls(&self, _browser: &str) -> Option<Vec<String>> {
        self.live_urls.clone()
    }
    fn packaged_app_id(&self, exe: &str) -> Option<String> {
        let id = application_id(exe);
        self.packaged
            .iter()
            .find(|(app, _)| *app == id)
            .map(|(_, aumid)| aumid.clone())
    }
    fn launch_packaged(&self, aumid: &str) -> Result<(), String> {
        let (app, _) = self
            .packaged
            .iter()
            .find(|(_, a)| a == aumid)
            .ok_or("unknown package")?;
        self.packaged_launched.borrow_mut().push(aumid.to_string());
        for (id, wins) in &self.on_launch {
            if id == app {
                self.open.borrow_mut().extend(wins.clone());
            }
        }
        Ok(())
    }
    fn launch(&self, exe: &str, _args: &[String], _cwd: Option<&str>) -> Result<u32, String> {
        let id = application_id(exe);
        if self.launch_fails.contains(&id) {
            return Err("access is denied".into());
        }
        self.launched.borrow_mut().push(id.clone());
        self.launched_paths.borrow_mut().push(exe.to_string());
        self.events.borrow_mut().push(format!("launch:{id}"));
        for (app, wins) in &self.on_launch {
            if *app == id {
                self.open.borrow_mut().extend(wins.clone());
            }
        }
        Ok(1)
    }
    fn place(&self, hwnd: isize, rect: Rect, state: DisplayState) -> Result<(), PlaceError> {
        if self.denied.contains(&hwnd) {
            return Err(PlaceError::AccessDenied);
        }
        self.placed.borrow_mut().push((hwnd, rect, state));
        Ok(())
    }
}

fn fast() -> RestoreOptions {
    RestoreOptions {
        timeout: Duration::from_millis(300),
        settle: Duration::from_millis(20),
        ..Default::default()
    }
}

fn payload() -> SnapshotPayload {
    SnapshotPayload {
        applications: vec![app("code"), app("chrome"), app("notes")],
        windows: vec![
            saved_window("code", "main.rs", r(0, 0, 1000, 800), "M1"),
            saved_window("chrome", "Docs", r(1000, 0, 900, 800), "M1"),
            saved_window("notes", "Todo", r(100, 100, 400, 300), "M1"),
        ],
        ..Default::default()
    }
}

fn run(d: &Fake, p: &SnapshotPayload, o: &RestoreOptions) -> RestoreRun {
    restore(d, p, o, &mut |_| {})
}

fn step<'a>(run: &'a RestoreRun, app: &str) -> &'a StepReport {
    run.steps.iter().find(|s| s.application_id == app).unwrap()
}

#[test]
fn launches_apps_and_places_windows() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        on_launch: vec![
            (
                "code".into(),
                vec![open_window(1, "code", "main.rs - Code")],
            ),
            (
                "chrome".into(),
                vec![open_window(2, "chrome", "Docs - Chrome")],
            ),
            ("notes".into(), vec![open_window(3, "notes", "Todo")]),
        ],
        ..Default::default()
    };
    let result = run(&d, &payload(), &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(d.launched.borrow().len(), 3);
    let placed = d.placed.borrow();
    assert!(placed.contains(&(1, r(0, 0, 1000, 800), DisplayState::Normal)));
    assert!(placed.contains(&(3, r(100, 100, 400, 300), DisplayState::Normal)));
}

#[test]
fn missing_app_is_reported_and_others_still_restore() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        missing_exes: ["chrome".to_string()].into(),
        on_launch: vec![
            ("code".into(), vec![open_window(1, "code", "main.rs")]),
            ("notes".into(), vec![open_window(3, "notes", "Todo")]),
        ],
        ..Default::default()
    };
    let result = run(&d, &payload(), &fast());
    let chrome = step(&result, "chrome");
    assert_eq!(chrome.status, RestoreStatus::Failed);
    assert_eq!(chrome.code, Some("EXE_NOT_FOUND"));
    assert_eq!(step(&result, "code").status, RestoreStatus::Success);
    assert_eq!(step(&result, "notes").status, RestoreStatus::Success);
    assert_eq!(result.overall, RestoreStatus::PartialSuccess);
    assert_eq!(result.problems().count(), 1);
}

#[test]
fn launch_failure_and_window_timeout_are_isolated() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        launch_fails: ["code".to_string()].into(),
        // chrome starts but never shows a window; notes works.
        on_launch: vec![("notes".into(), vec![open_window(3, "notes", "Todo")])],
        ..Default::default()
    };
    let result = run(&d, &payload(), &fast());
    assert_eq!(step(&result, "code").code, Some("LAUNCH_FAILED"));
    assert_eq!(step(&result, "chrome").code, Some("WINDOW_TIMEOUT"));
    assert_eq!(step(&result, "notes").status, RestoreStatus::Success);
}

#[test]
fn restoring_twice_does_not_launch_duplicates() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![
            open_window(1, "code", "main.rs"),
            open_window(2, "chrome", "Docs"),
            open_window(3, "notes", "Todo"),
        ]),
        ..Default::default()
    };
    let result = run(&d, &payload(), &fast());
    assert!(d.launched.borrow().is_empty(), "nothing should be launched");
    assert_eq!(d.placed.borrow().len(), 3, "windows are just repositioned");
    assert_eq!(result.overall, RestoreStatus::Success);
    assert!(step(&result, "code").message.contains("already running"));
}

#[test]
fn missing_monitor_moves_window_onto_available_display() {
    // The saved window lived on a second monitor at x=2000 that is gone now.
    let p = SnapshotPayload {
        applications: vec![app("code")],
        windows: vec![saved_window(
            "code",
            "main.rs",
            r(2100, 100, 1200, 800),
            "M2",
        )],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        on_launch: vec![("code".into(), vec![open_window(1, "code", "main.rs")])],
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(result.overall, RestoreStatus::Success);
    let (_, rect, _) = d.placed.borrow()[0];
    assert!(
        rect.x >= 0 && rect.right() <= 1920,
        "window must be reachable, got {rect:?}"
    );
}

#[test]
fn fewer_windows_than_saved_is_partial() {
    let p = SnapshotPayload {
        applications: vec![app("code")],
        windows: vec![
            saved_window("code", "a.rs", r(0, 0, 500, 500), "M1"),
            saved_window("code", "b.rs", r(500, 0, 500, 500), "M1"),
        ],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        on_launch: vec![("code".into(), vec![open_window(1, "code", "welcome")])],
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    let s = step(&result, "code");
    assert_eq!(s.status, RestoreStatus::PartialSuccess);
    assert!(s.message.contains("1 of 2"), "{}", s.message);
}

#[test]
fn elevated_windows_require_user_intervention() {
    let p = SnapshotPayload {
        applications: vec![app("code")],
        windows: vec![saved_window("code", "x", r(0, 0, 500, 500), "M1")],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        denied: [1].into(),
        on_launch: vec![("code".into(), vec![open_window(1, "code", "x")])],
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(
        step(&result, "code").status,
        RestoreStatus::RequiresUserIntervention
    );
}

#[test]
fn only_filter_and_dry_run() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        missing_exes: ["chrome".to_string()].into(),
        ..Default::default()
    };
    let opts = RestoreOptions {
        dry_run: true,
        only: vec!["Code".into(), "chrome".into()],
        ..fast()
    };
    let result = run(&d, &payload(), &opts);
    assert_eq!(result.steps.len(), 2, "notes is filtered out");
    assert_eq!(step(&result, "code").code, Some("DRY_RUN"));
    assert_eq!(step(&result, "chrome").code, Some("EXE_NOT_FOUND"));
    assert!(d.launched.borrow().is_empty() && d.placed.borrow().is_empty());
}

#[test]
fn store_apps_without_a_package_id_are_skipped() {
    let mut store = app("applicationframehost");
    store.executable_path = r"C:\Windows\System32\ApplicationFrameHost.exe".into();
    let mut packaged = app("windowsterminal");
    packaged.executable_path = r"C:\Program Files\WindowsApps\Terminal\wt.exe".into();
    let p = SnapshotPayload {
        applications: vec![store, packaged],
        ..Default::default()
    };
    let d = Fake::default();
    let result = run(&d, &p, &fast());
    assert!(result
        .steps
        .iter()
        .all(|s| s.status == RestoreStatus::Skipped && s.code == Some("UNSUPPORTED_APP")));
    assert!(d.launched.borrow().is_empty());
    assert_eq!(result.overall, RestoreStatus::Success);
}

fn store_app(id: &str) -> ApplicationState {
    let mut a = app(id);
    a.executable_path = format!(r"C:\Program Files\WindowsApps\{id}_1.0_x64__pub\{id}.exe");
    a
}

#[test]
fn store_apps_start_by_package_id_and_get_placed() {
    let p = SnapshotPayload {
        applications: vec![store_app("teams")],
        windows: vec![saved_window("teams", "Chat", r(200, 100, 900, 700), "M1")],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        packaged: vec![("teams".into(), "Teams_pub!Teams".into())],
        on_launch: vec![("teams".into(), vec![open_window(7, "teams", "Chat")])],
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(*d.packaged_launched.borrow(), vec!["Teams_pub!Teams"]);
    assert!(d.launched.borrow().is_empty(), "never launched by exe path");
    assert_eq!(d.placed.borrow()[0].1, r(200, 100, 900, 700));
}

#[test]
fn saved_package_id_wins_and_survives_app_updates() {
    // The install folder changed after an update, but the saved id still works
    // and nothing is reported as "not found".
    let mut a = store_app("teams");
    a.adapter_state = serde_json::json!({ "aumid": "Teams_pub!Teams" });
    let p = SnapshotPayload {
        applications: vec![a],
        ..Default::default()
    };
    let d = Fake {
        packaged: vec![("other".into(), "Teams_pub!Teams".into())],
        on_launch: vec![("other".into(), vec![open_window(1, "teams", "x")])],
        missing_exes: ["teams".to_string()].into(),
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(*d.packaged_launched.borrow(), vec!["Teams_pub!Teams"]);
    assert_ne!(result.steps[0].code, Some("EXE_NOT_FOUND"));
}

#[test]
fn running_store_app_is_repositioned_not_skipped() {
    let p = SnapshotPayload {
        applications: vec![store_app("claude")],
        windows: vec![saved_window("claude", "Claude", r(10, 10, 800, 600), "M1")],
        ..Default::default()
    };
    // No package id is known, but the app is already open so it does not matter.
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![open_window(5, "claude", "Claude")]),
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(result.steps[0].status, RestoreStatus::Success);
    assert_eq!(d.placed.borrow().len(), 1);
}

#[test]
fn relative_or_non_exe_paths_are_never_launched() {
    let mut relative = app("evil");
    relative.executable_path = "evil.exe".into();
    let mut script = app("script");
    script.executable_path = r"C:\Apps\script.bat".into();
    let p = SnapshotPayload {
        applications: vec![relative, script],
        ..Default::default()
    };
    let d = Fake::default();
    let result = run(&d, &p, &fast());
    assert_eq!(result.steps.len(), 2);
    assert!(
        result
            .steps
            .iter()
            .all(|s| s.status == RestoreStatus::Failed && s.code == Some("UNSAFE_PATH")),
        "{result:#?}"
    );
    assert!(d.launched.borrow().is_empty(), "nothing may be started");
}

#[test]
fn updated_app_is_found_in_its_new_version_folder() {
    let base = r"C:\Users\u\AppData\Local\Discord";
    let mut discord = app("discord");
    discord.executable_path = format!(r"{base}\app-1.0.9\Discord.exe");
    let p = SnapshotPayload {
        applications: vec![discord],
        windows: vec![saved_window("discord", "Friends", r(0, 0, 800, 600), "M1")],
        ..Default::default()
    };
    let new_exe = format!(r"{base}\app-1.0.10\Discord.exe");
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        // The 1.0.9 folder is gone; only 1.0.10 has the program.
        dirs: vec![(
            base.to_string(),
            vec!["app-1.0.9".into(), "app-1.0.10".into(), "packages".into()],
        )],
        existing_paths: Some([new_exe.clone()].into()),
        on_launch: vec![("discord".into(), vec![open_window(1, "discord", "Friends")])],
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(result.steps[0].status, RestoreStatus::Success);
    let launched = d.launched_paths.borrow();
    assert_eq!(launched.len(), 1);
    assert!(launched[0].contains("app-1.0.10"), "launched {launched:?}");
    assert_eq!(launched[0], new_exe);
}

#[test]
fn saved_path_wins_when_it_still_exists() {
    let base = r"C:\Users\u\AppData\Local\Discord";
    let saved = format!(r"{base}\app-1.0.9\Discord.exe");
    let mut discord = app("discord");
    discord.executable_path = saved.clone();
    let p = SnapshotPayload {
        applications: vec![discord],
        ..Default::default()
    };
    let d = Fake {
        dirs: vec![(
            base.to_string(),
            vec!["app-1.0.9".into(), "app-1.0.10".into()],
        )],
        // Both exist: the saved one is used, not the newer one.
        existing_paths: Some([saved.clone(), format!(r"{base}\app-1.0.10\Discord.exe")].into()),
        on_launch: vec![("discord".into(), vec![open_window(1, "discord", "x")])],
        ..Default::default()
    };
    run(&d, &p, &fast());
    assert_eq!(*d.launched_paths.borrow(), vec![saved]);
}

#[test]
fn missing_app_with_no_newer_folder_is_still_reported() {
    let base = r"C:\Users\u\AppData\Local\Discord";
    let mut discord = app("discord");
    discord.executable_path = format!(r"{base}\app-1.0.9\Discord.exe");
    let p = SnapshotPayload {
        applications: vec![discord],
        ..Default::default()
    };
    let d = Fake {
        dirs: vec![(
            base.to_string(),
            vec!["app-1.0.9".into(), "packages".into()],
        )],
        existing_paths: Some(HashSet::new()),
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(result.steps[0].code, Some("EXE_NOT_FOUND"));
    assert!(d.launched.borrow().is_empty());
}

#[test]
fn ordinary_apps_start_before_browser_tabs() {
    // The browser is listed first, but its slow window-opening must wait.
    let mut p = chrome_payload();
    p.applications.push(app("code"));
    p.windows
        .push(saved_window("code", "main.rs", r(0, 0, 500, 500), "M1"));
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        browser_windows: RefCell::new(vec![
            open_window(11, "chrome", "x"),
            open_window(12, "chrome", "y"),
        ]),
        on_launch: vec![("code".into(), vec![open_window(1, "code", "main.rs")])],
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    let events = d.events.borrow();
    let launch_code = events
        .iter()
        .position(|e| e == "launch:code")
        .expect("code was launched");
    let first_open = events
        .iter()
        .position(|e| e.starts_with("open:"))
        .expect("browser windows were opened");
    assert!(launch_code < first_open, "order was {events:?}");
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    // Reports still follow the snapshot's order.
    assert_eq!(result.steps[0].application_id, "chrome");
}

#[test]
fn overall_timeout_bounds_the_whole_restore() {
    // Three apps start but never show a window; each would wait 10 s on its own.
    let p = SnapshotPayload {
        applications: vec![app("a"), app("b"), app("c")],
        ..Default::default()
    };
    let d = Fake::default();
    let opts = RestoreOptions {
        timeout: Duration::from_secs(10),
        overall_timeout: Duration::from_millis(200),
        ..fast()
    };
    let started = std::time::Instant::now();
    let result = run(&d, &p, &opts);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(result.steps.len(), 3);
    assert!(
        result
            .steps
            .iter()
            .all(|s| s.status == RestoreStatus::Failed),
        "{result:#?}"
    );
    assert_eq!(result.overall, RestoreStatus::Failed);
}

#[test]
fn cancel_before_start_launches_nothing() {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let opts = RestoreOptions {
        cancel: Some(cancel),
        ..fast()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        ..Default::default()
    };
    let result = run(&d, &payload(), &opts);
    assert_eq!(result.steps.len(), 3);
    assert!(
        result
            .steps
            .iter()
            .all(|s| s.status == RestoreStatus::Skipped && s.code == Some("CANCELLED")),
        "{result:#?}"
    );
    assert!(d.launched.borrow().is_empty() && d.events.borrow().is_empty());
    // Nothing failed, so the run is not reported as a failure.
    assert_eq!(result.overall, RestoreStatus::Success);
}

#[test]
fn maximized_state_is_applied() {
    let mut w = saved_window("code", "x", r(100, 100, 800, 600), "M1");
    w.display_state = DisplayState::Maximized;
    let p = SnapshotPayload {
        applications: vec![app("code")],
        windows: vec![w],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        on_launch: vec![("code".into(), vec![open_window(1, "code", "x")])],
        ..Default::default()
    };
    run(&d, &p, &fast());
    assert_eq!(d.placed.borrow()[0].2, DisplayState::Maximized);
}

include!("tests_browser.rs");

include!("tests_vscode.rs");

include!("tests_vstudio.rs");

/// The "R" report: Steam was saved by its window's helper process.
/// Restore must start Steam.exe, then still find and place the helper's window.
#[test]
fn helper_process_is_started_through_its_app() {
    let helper = r"C:\Program Files (x86)\Steam\bin\cef\cef.win64\steamwebhelper.exe";
    let steam = r"C:\Program Files (x86)\Steam\Steam.exe";
    let mut a = app("steamwebhelper");
    a.executable_path = helper.into();
    let payload = SnapshotPayload {
        applications: vec![a],
        windows: vec![saved_window(
            "steamwebhelper",
            "Steam",
            r(69, 519, 1515, 900),
            "M1",
        )],
        ..Default::default()
    };
    let mut window = open_window(7, "steamwebhelper", "Steam");
    window.executable_path = Some(helper.into());
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 2560, 1440))],
        existing_paths: Some([helper.to_string(), steam.to_string()].into()),
        on_launch: vec![("steam".into(), vec![window])],
        ..Default::default()
    };
    let result = run(&d, &payload, &fast());
    let launched = d.launched_paths.borrow();
    assert_eq!(launched.len(), 1);
    assert!(launched[0].eq_ignore_ascii_case(steam), "{launched:?}");
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(d.placed.borrow()[0].0, 7);
}

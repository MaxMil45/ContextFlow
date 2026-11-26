//! Developer tool: run the real restore engine against a *throwaway* Chrome
//! profile, so browser restore can be tried without touching your own browser.
//!
//! ```text
//! contextflow show <workspace> --json > snapshot.json
//! cargo run -p cf-engine --example real_chrome -- <profile-dir> snapshot.json
//! ```
//!
//! Windows that already exist (your own browser) are hidden from the engine,
//! and new windows are opened with `--user-data-dir=<profile-dir>`.

use std::collections::HashSet;

use cf_core::{DisplayState, MonitorInfo, Rect, SnapshotPayload};
use cf_engine::{restore, Desktop, PlaceError, RestoreOptions, WindowsDesktop};
use cf_windows::DiscoveredWindow;

struct Isolated {
    real: WindowsDesktop,
    profile: String,
    hidden: HashSet<isize>,
}

impl Desktop for Isolated {
    fn windows(&self) -> Result<Vec<DiscoveredWindow>, String> {
        Ok(self
            .real
            .windows()?
            .into_iter()
            .filter(|w| !self.hidden.contains(&w.hwnd))
            .collect())
    }
    fn monitors(&self) -> Result<Vec<MonitorInfo>, String> {
        self.real.monitors()
    }
    fn exe_exists(&self, path: &str) -> bool {
        self.real.exe_exists(path)
    }
    fn list_dirs(&self, dir: &str) -> Vec<String> {
        self.real.list_dirs(dir)
    }
    fn launch(&self, exe: &str, args: &[String], cwd: Option<&str>) -> Result<u32, String> {
        self.real.launch(exe, args, cwd)
    }
    fn packaged_app_id(&self, exe: &str) -> Option<String> {
        self.real.packaged_app_id(exe)
    }
    fn launch_packaged(&self, id: &str) -> Result<(), String> {
        self.real.launch_packaged(id)
    }
    fn open_app_window(&self, exe: &str, args: &[String]) -> Result<(), String> {
        self.real.open_app_window(exe, args)
    }
    fn open_browser_window(
        &self,
        exe: &str,
        _profile: Option<&str>,
        urls: &[String],
    ) -> Result<(), String> {
        if let Some(bad) = urls.iter().find(|u| !cf_browser::is_restorable_url(u)) {
            return Err(format!("refusing to open '{bad}'"));
        }
        let mut args = vec![
            format!("--user-data-dir={}", self.profile),
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            "--new-window".into(),
        ];
        args.extend(urls.iter().cloned());
        cf_windows::launch_process(exe, &args, None)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn live_browser_urls(&self, _browser: &str) -> Option<Vec<String>> {
        None
    }
    fn request_pins(
        &self,
        _browser: &str,
        _urls: &[String],
        _groups: &[cf_browser::request::GroupRequest],
    ) {
        // A throwaway profile has no extension to pass the request to.
    }
    fn place(&self, hwnd: isize, rect: Rect, state: DisplayState) -> Result<(), PlaceError> {
        self.real.place(hwnd, rect, state)
    }
    fn raise(&self, hwnd: isize) {
        self.real.raise(hwnd)
    }
    fn focus(&self, hwnd: isize) {
        self.real.focus(hwnd)
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(profile), Some(snapshot)) = (args.next(), args.next()) else {
        eprintln!("usage: real_chrome <profile-dir> <snapshot.json>");
        std::process::exit(2);
    };
    cf_windows::init_dpi_awareness();
    let text = std::fs::read_to_string(&snapshot).expect("read snapshot");
    let value: serde_json::Value = serde_json::from_str(&text).expect("snapshot is JSON");
    let payload: SnapshotPayload =
        serde_json::from_value(value["payload"].clone()).expect("snapshot payload");

    let real = WindowsDesktop;
    let hidden = real
        .windows()
        .expect("list windows")
        .into_iter()
        .map(|w| w.hwnd)
        .collect();
    let desktop = Isolated {
        real,
        profile,
        hidden,
    };
    let run = restore(
        &desktop,
        &payload,
        &RestoreOptions {
            only: vec!["chrome".into()],
            ..Default::default()
        },
        &mut |m| println!("  {m}"),
    );
    for s in &run.steps {
        println!(
            "{:?} {} {:?}: {}",
            s.status, s.application_id, s.code, s.message
        );
    }
}

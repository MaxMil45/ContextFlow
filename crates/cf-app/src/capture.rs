//! Builds a [`SnapshotPayload`] from the live environment.
//!
//! Capture is strictly non-destructive: it only reads window, monitor and Git
//! state. Failures in one component downgrade the snapshot to `Partial` rather
//! than aborting the whole capture.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cf_config::{BrowserConfig, CaptureConfig};
use cf_core::{ApplicationState, CaptureStatus, DisplayState, SnapshotPayload, WindowState};

/// What a private window title is stored as.
pub const HIDDEN_TITLE: &str = "(hidden)";

/// Shell / system processes whose windows are never part of a workspace.
const BUILTIN_EXCLUDED_PROCESSES: &[&str] = &[
    "textinputhost",
    "searchhost",
    "shellexperiencehost",
    "startmenuexperiencehost",
    "lockapp",
    "applicationframehost",
    "systemsettings",
    "contextflow",
    "contextflow-app",
];

pub struct CaptureOptions<'a> {
    /// Capture only these application ids (empty = every application).
    pub only: &'a [String],
    /// Optional extra: Git repositories to record alongside the applications.
    pub repos: &'a [PathBuf],
    pub notes: Vec<String>,
    pub include_windows: bool,
    /// Save minimized windows as normal windows.
    pub unminimize: bool,
    pub rules: &'a CaptureConfig,
    pub git_timeout: Duration,
    /// Browser tab saving settings.
    pub browser: &'a BrowserConfig,
    /// Where the browser host keeps live tab state (`None` = don't save tabs).
    pub browser_state_dir: Option<PathBuf>,
    /// VS Code's `storage.json` (`None` = do not save VS Code folders).
    pub vscode_storage: Option<PathBuf>,
    /// Visual Studio's settings files with its recent solutions (empty = do not
    /// save Visual Studio solutions).
    pub visual_studio_settings: Vec<PathBuf>,
    /// Save which folder each File Explorer window shows.
    pub explorer_folders: bool,
    /// JetBrains IDEs' recent-project lists (empty = don't save their projects).
    pub jetbrains_recent: Vec<PathBuf>,
}

pub struct CaptureResult {
    pub payload: SnapshotPayload,
    pub status: CaptureStatus,
    pub warnings: Vec<String>,
    /// (browser name, windows, tabs) saved for each browser.
    pub browser_tabs: Vec<(String, usize, usize)>,
    /// Windows skipped because of configured exclusion rules.
    pub excluded_windows: usize,
    /// Windows saved as minimized (they will reopen minimized).
    pub minimized_windows: usize,
}

pub use cf_engine::application_id;

pub fn capture(opts: CaptureOptions<'_>) -> CaptureResult {
    let started = Instant::now();
    let mut payload = SnapshotPayload {
        notes: opts.notes,
        ..Default::default()
    };
    let mut warnings = Vec::new();
    let mut component_failed = false;
    let mut excluded_windows = 0;
    let mut minimized_windows = 0;

    let user_excluded = opts.rules.normalized_excluded_processes();
    let is_excluded_process = |id: &str| {
        BUILTIN_EXCLUDED_PROCESSES.contains(&id) || user_excluded.iter().any(|e| e == id)
    };

    if opts.include_windows {
        match cf_windows::enumerate_monitors() {
            Ok(m) => payload.monitors = m,
            Err(e) => {
                component_failed = true;
                warnings.push(format!("monitors: {e}"));
            }
        }
        match cf_windows::enumerate_windows() {
            Ok(windows) => {
                let own_pid = std::process::id();
                let mut apps: BTreeMap<String, ApplicationState> = BTreeMap::new();
                for w in windows {
                    if w.process_id == own_pid || w.rect.width <= 0 || w.rect.height <= 0 {
                        continue;
                    }
                    let Some(exe) = w.executable_path.clone() else {
                        warnings.push(format!(
                            "a window of process {} was skipped: the process is not accessible (probably running as administrator)",
                            w.process_id
                        ));
                        continue;
                    };
                    let app_id = application_id(&exe);
                    if is_excluded_process(&app_id)
                        || (!opts.only.is_empty() && !opts.only.contains(&app_id))
                    {
                        continue;
                    }
                    if opts.rules.title_is_excluded(&w.title) {
                        excluded_windows += 1;
                        continue;
                    }
                    apps.entry(app_id.clone()).or_insert_with(|| {
                        // Microsoft Store apps are started by package id; save it
                        // now so restore still works after the app updates.
                        let aumid = cf_windows::packaged_app_id(&exe);
                        // A window may belong to a helper process (Steam's
                        // steamwebhelper.exe) that does nothing when started on
                        // its own: save the program from the same install that
                        // started it. The application id stays the window's own,
                        // so restore still recognises the window.
                        let executable_path = if aumid.is_some() {
                            exe
                        } else {
                            let ancestors = cf_windows::process_ancestors(w.process_id);
                            cf_engine::launcher_from_ancestry(&exe, &ancestors).unwrap_or(exe)
                        };
                        ApplicationState {
                            application_id: app_id.clone(),
                            executable_path,
                            launch_arguments: Vec::new(),
                            working_directory: None,
                            adapter_id: aumid.as_ref().map(|_| "packaged-app".to_string()),
                            adapter_state: aumid
                                .map(|id| serde_json::json!({ "aumid": id }))
                                .unwrap_or(serde_json::Value::Null),
                        }
                    });
                    // A minimized window reopens minimized; --unminimize saves it as normal.
                    let display_state =
                        if opts.unminimize && w.display_state == DisplayState::Minimized {
                            DisplayState::Normal
                        } else {
                            w.display_state
                        };
                    if display_state == DisplayState::Minimized {
                        minimized_windows += 1;
                    }
                    payload.windows.push(WindowState {
                        application_id: app_id,
                        title: w.title,
                        window_class: Some(w.class_name),
                        monitor_id: w.monitor_id,
                        rect: w.rect,
                        display_state,
                    });
                }
                payload.applications = apps.into_values().collect();
            }
            Err(e) => {
                component_failed = true;
                warnings.push(format!("windows: {e}"));
            }
        }
    }

    let mut browser_tabs = Vec::new();
    if opts.include_windows && opts.browser.capture_tabs {
        if let Some(dir) = &opts.browser_state_dir {
            attach_browser_tabs(
                &mut payload,
                opts.browser,
                dir,
                &mut warnings,
                &mut browser_tabs,
            );
        }
    }

    if opts.include_windows {
        if let Some(storage) = &opts.vscode_storage {
            attach_vscode_folders(&mut payload, storage, &mut warnings);
        }
        if !opts.visual_studio_settings.is_empty() {
            attach_visual_studio(&mut payload, &opts.visual_studio_settings, &mut warnings);
        }
        if opts.explorer_folders {
            attach_explorer_folders(&mut payload, &mut warnings);
        }
        if !opts.jetbrains_recent.is_empty() {
            attach_jetbrains(&mut payload, &opts.jetbrains_recent, &mut warnings);
        }
    }

    for repo in opts.repos {
        match cf_git::inspect_with_timeout(repo, opts.git_timeout) {
            Ok(state) => payload.git.push(state),
            Err(e) => {
                component_failed = true;
                warnings.push(format!("git {}: {e}", repo.display()));
            }
        }
    }

    // Private titles: the window is kept (so it comes back), its title isn't.
    // Done last, because the adapters above match windows by their titles.
    let mut hidden = 0;
    for w in &mut payload.windows {
        if opts.rules.title_is_hidden(&w.title) {
            w.title = HIDDEN_TITLE.into();
            hidden += 1;
        }
    }
    if hidden > 0 {
        for a in &mut payload.applications {
            if let Some(windows) = a
                .adapter_state
                .get_mut("windows")
                .and_then(|v| v.as_array_mut())
            {
                for w in windows {
                    let private = w
                        .get("window_title")
                        .and_then(|t| t.as_str())
                        .is_some_and(|t| opts.rules.title_is_hidden(t));
                    if private {
                        w["window_title"] = HIDDEN_TITLE.into();
                    }
                }
            }
        }
    }

    payload.metadata.insert(
        "contextflow_version".into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    payload.metadata.insert(
        "capture_duration_ms".into(),
        (started.elapsed().as_millis() as u64).into(),
    );

    let status = if component_failed {
        CaptureStatus::Partial
    } else {
        CaptureStatus::Complete
    };
    tracing::info!(
        applications = payload.applications.len(),
        windows = payload.windows.len(),
        monitors = payload.monitors.len(),
        repositories = payload.git.len(),
        excluded_windows,
        status = status.as_str(),
        duration_ms = started.elapsed().as_millis() as u64,
        "capture finished"
    );
    CaptureResult {
        payload,
        status,
        warnings,
        browser_tabs,
        excluded_windows,
        minimized_windows,
    }
}

/// Work out which folder each captured VS Code window has open, so restore can
/// reopen it. Windows whose folder is unknown or ambiguous are saved without
/// one (and reported), never guessed.
fn attach_vscode_folders(
    payload: &mut SnapshotPayload,
    storage: &std::path::Path,
    warnings: &mut Vec<String>,
) {
    let Some(i) = payload
        .applications
        .iter()
        .position(|a| cf_vscode::is_vscode(&a.application_id))
    else {
        return;
    };
    let titles: Vec<String> = payload
        .windows
        .iter()
        .filter(|w| cf_vscode::is_vscode(&w.application_id))
        .map(|w| w.title.clone())
        .collect();
    let known = cf_vscode::read_known_folders(storage);
    let state = cf_vscode::build_state(&titles, &known, &cf_vscode::folder_exists);

    let unresolved = state.windows.len() - state.folder_count();
    if unresolved > 0 {
        warnings.push(format!(
            "VS Code: the folder for {unresolved} window(s) could not be worked out; they will reopen without one"
        ));
    }
    if state.folder_count() > 0 {
        let app = &mut payload.applications[i];
        app.adapter_id = Some(cf_vscode::ADAPTER_ID.to_string());
        app.adapter_state = state.to_value();
    }
}

/// Work out which solution or folder each captured Visual Studio window has
/// open, from Visual Studio's own list of recent ones. Windows that can't be
/// matched with confidence are saved without one (and reported), never guessed.
fn attach_visual_studio(
    payload: &mut SnapshotPayload,
    settings: &[PathBuf],
    warnings: &mut Vec<String>,
) {
    let Some(i) = payload
        .applications
        .iter()
        .position(|a| cf_vstudio::is_visual_studio(&a.application_id))
    else {
        return;
    };
    let titles: Vec<String> = payload
        .windows
        .iter()
        .filter(|w| cf_vstudio::is_visual_studio(&w.application_id))
        .map(|w| w.title.clone())
        .collect();
    let recent = cf_vstudio::read_recent(settings);
    let state = cf_vstudio::build_state(&titles, &recent, &cf_vstudio::path_exists);

    // The start window has no solution; only count windows that showed a name.
    let unresolved = state
        .windows
        .iter()
        .filter(|w| w.name.is_some() && w.path.is_none())
        .count();
    if unresolved > 0 {
        warnings.push(format!(
            "Visual Studio: the solution or folder for {unresolved} window(s) could not be worked out; they will reopen without one"
        ));
    }
    if state.path_count() > 0 {
        let app = &mut payload.applications[i];
        app.adapter_id = Some(cf_vstudio::ADAPTER_ID.to_string());
        app.adapter_state = state.to_value();
    }
}

/// Work out which folder each captured File Explorer window shows, from
/// Windows' own list of open folders. Special places (This PC, a search) have
/// no folder and come back as an ordinary Explorer window.
fn attach_explorer_folders(payload: &mut SnapshotPayload, warnings: &mut Vec<String>) {
    let Some(i) = payload
        .applications
        .iter()
        .position(|a| cf_explorer::is_explorer(&a.application_id))
    else {
        return;
    };
    let titles: Vec<String> = payload
        .windows
        .iter()
        .filter(|w| cf_explorer::is_explorer(&w.application_id))
        .map(|w| w.title.clone())
        .collect();
    let open = cf_explorer::open_folders();
    let state = cf_explorer::build_state(&titles, &open, &cf_explorer::folder_exists);
    let unresolved = state.windows.len() - state.path_count();
    if unresolved > 0 && state.path_count() > 0 {
        warnings.push(format!(
            "File Explorer: {unresolved} window(s) show a place that isn't a folder on disk; they will reopen as ordinary Explorer windows"
        ));
    }
    if state.path_count() > 0 {
        let app = &mut payload.applications[i];
        app.adapter_id = Some(cf_explorer::ADAPTER_ID.to_string());
        app.adapter_state = state.to_value();
    }
}

/// Work out which project each JetBrains IDE window has open (each IDE is its
/// own application: PyCharm and CLion are saved separately).
fn attach_jetbrains(payload: &mut SnapshotPayload, files: &[PathBuf], warnings: &mut Vec<String>) {
    let ides: Vec<usize> = payload
        .applications
        .iter()
        .enumerate()
        .filter(|(_, a)| cf_jetbrains::is_jetbrains(&a.application_id))
        .map(|(i, _)| i)
        .collect();
    if ides.is_empty() {
        return;
    }
    let recent = cf_jetbrains::read_recent(files);
    for i in ides {
        let id = payload.applications[i].application_id.clone();
        let titles: Vec<String> = payload
            .windows
            .iter()
            .filter(|w| w.application_id == id)
            .map(|w| w.title.clone())
            .collect();
        let state = cf_jetbrains::build_state(&titles, &recent, &cf_jetbrains::folder_exists);
        let unresolved = state.windows.len() - state.path_count();
        if unresolved > 0 && state.path_count() > 0 {
            warnings.push(format!(
                "{id}: the project for {unresolved} window(s) could not be worked out; they will reopen without one"
            ));
        }
        if state.path_count() > 0 {
            let app = &mut payload.applications[i];
            app.adapter_id = Some(cf_jetbrains::ADAPTER_ID.to_string());
            app.adapter_state = state.to_value();
        }
    }
}

/// The captured windows of one application as the browser matcher needs them:
/// title, top-left corner, and the scale (DPI / 96) of the monitor they are on.
fn desktop_windows(
    payload: &SnapshotPayload,
    application_id: &str,
) -> Vec<cf_browser::saved::DesktopWindow> {
    payload
        .windows
        .iter()
        .filter(|w| w.application_id == application_id)
        .map(|w| {
            let scale = w
                .monitor_id
                .as_deref()
                .and_then(|id| payload.monitors.iter().find(|m| m.id == id))
                .and_then(|m| m.dpi)
                .map_or(1.0, |dpi| f64::from(dpi) / 96.0);
            cf_browser::saved::DesktopWindow {
                title: w.title.clone(),
                x: w.rect.x,
                y: w.rect.y,
                scale,
            }
        })
        .collect()
}

/// Attach the tabs open in each captured browser to its windows. Uses the live
/// state kept by the browser extension; without it only the windows are saved.
fn attach_browser_tabs(
    payload: &mut SnapshotPayload,
    cfg: &BrowserConfig,
    dir: &std::path::Path,
    warnings: &mut Vec<String>,
    summary: &mut Vec<(String, usize, usize)>,
) {
    for i in 0..payload.applications.len() {
        let id = payload.applications[i].application_id.clone();
        if !cf_browser::is_browser(&id) {
            continue;
        }
        let name = cf_browser::BROWSERS
            .iter()
            .find(|(b, _)| *b == id)
            .map_or(id.as_str(), |(_, n)| *n)
            .to_string();
        let live = cf_browser::read_live_states(dir, &id);
        if live.is_empty() {
            warnings.push(format!(
                "{name}: tabs were not saved because the ContextFlow extension isn't connected (check Settings in the app, or run `contextflow browser status`)"
            ));
            continue;
        }
        let desktop = desktop_windows(payload, &id);
        let result =
            cf_browser::saved::match_windows(&id, &desktop, &live, &cfg.excluded_url_patterns);
        if result.unmatched_live > 0 {
            warnings.push(format!(
                "{name}: {} open window(s) couldn't be matched to a desktop window and were not saved (minimized to the tray, or on another desktop?)",
                result.unmatched_live
            ));
        }
        let tabs = result.state.tab_count();
        if tabs > 0 {
            summary.push((name, result.state.windows.len(), tabs));
            let app = &mut payload.applications[i];
            app.adapter_id = Some(cf_browser::ADAPTER_ID.to_string());
            app.adapter_state = result.state.to_value();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_ids_are_lowercase_stems() {
        assert_eq!(
            application_id(r"C:\Program Files\Microsoft VS Code\Code.exe"),
            "code"
        );
        assert_eq!(application_id(r"C:\Windows\explorer.exe"), "explorer");
    }

    #[test]
    fn browser_windows_carry_position_and_monitor_scale() {
        use cf_core::{DisplayState, MonitorInfo, Rect};
        let rect = |x| Rect {
            x,
            y: 40,
            width: 800,
            height: 600,
        };
        let monitor = |id: &str, dpi| MonitorInfo {
            id: id.into(),
            primary: false,
            rect: rect(0),
            work_area: rect(0),
            dpi,
        };
        let win = |app: &str, title: &str, x, mon: Option<&str>| WindowState {
            application_id: app.into(),
            title: title.into(),
            window_class: None,
            monitor_id: mon.map(String::from),
            rect: rect(x),
            display_state: DisplayState::Normal,
        };
        let payload = SnapshotPayload {
            monitors: vec![monitor("HI", Some(144)), monitor("LO", Some(96))],
            windows: vec![
                win("chrome", "A - Google Chrome", 100, Some("HI")),
                win("chrome", "B - Google Chrome", 2000, Some("LO")),
                win("chrome", "C - Google Chrome", 3000, Some("GONE")),
                win("chrome", "D - Google Chrome", 4000, None),
                win("code", "not a browser window", 5, Some("HI")),
            ],
            ..Default::default()
        };
        let d = desktop_windows(&payload, "chrome");
        let got: Vec<(&str, i32, i32, f64)> = d
            .iter()
            .map(|w| (w.title.as_str(), w.x, w.y, w.scale))
            .collect();
        assert_eq!(
            got,
            vec![
                ("A - Google Chrome", 100, 40, 1.5), // 144 dpi = 150%
                ("B - Google Chrome", 2000, 40, 1.0),
                ("C - Google Chrome", 3000, 40, 1.0), // monitor no longer known
                ("D - Google Chrome", 4000, 40, 1.0),
            ]
        );
    }

    #[test]
    fn excluded_processes_never_appear() {
        // Exclude every process: the capture must then contain no windows,
        // whatever happens to be open on this machine.
        let rules = CaptureConfig {
            excluded_title_patterns: vec!["".into()],
            ..Default::default()
        };
        let r = capture(CaptureOptions {
            only: &[],
            repos: &[],
            notes: vec![],
            include_windows: true,
            unminimize: false,
            rules: &rules,
            git_timeout: Duration::from_secs(5),
            browser: &BrowserConfig::default(),
            browser_state_dir: None,
            vscode_storage: None,
            visual_studio_settings: Vec::new(),
            explorer_folders: false,
            jetbrains_recent: Vec::new(),
        });
        assert!(r.payload.windows.is_empty());
        assert!(r.payload.applications.is_empty());
    }
}

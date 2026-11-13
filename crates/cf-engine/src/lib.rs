//! The restore pipeline: bring a saved workspace back on the desktop.
//!
//! For every saved application the engine
//!   1. reuses it if it is already running (so restore is safe to run twice),
//!   2. otherwise checks the executable and launches it,
//!   3. waits (bounded) for its windows to appear and settle,
//!   4. moves each window to its saved position, falling back to an available
//!      monitor when the original one is gone.
//!
//! Applications are independent: a failure in one never stops the others, and
//! nothing is ever force-closed. Every application gets an explicit result.

mod desktop;

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use cf_adapters::RestoreStatus;
use cf_core::geometry::placement_for;
use cf_core::{ApplicationState, MonitorInfo, SnapshotPayload, WindowState};
use cf_windows::DiscoveredWindow;
use serde::Serialize;

pub use desktop::{Desktop, PlaceError, WindowsDesktop};

#[derive(Debug, Clone)]
pub struct RestoreOptions {
    /// Longest wait for an application's windows after launching it.
    pub timeout: Duration,
    /// After the first window appears, wait this long without further changes
    /// before placing (apps often open several windows in quick succession).
    pub settle: Duration,
    /// Restore only these application ids (empty = all).
    pub only: Vec<String>,
    /// Report what would happen without launching or moving anything.
    pub dry_run: bool,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(20),
            settle: Duration::from_millis(1500),
            only: Vec::new(),
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepReport {
    pub application_id: String,
    pub status: RestoreStatus,
    /// Stable machine-readable code for failures and skips.
    pub code: Option<&'static str>,
    pub message: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestoreRun {
    pub steps: Vec<StepReport>,
    pub overall: RestoreStatus,
}

impl RestoreRun {
    /// Steps a user should look at.
    pub fn problems(&self) -> impl Iterator<Item = &StepReport> {
        self.steps
            .iter()
            .filter(|s| !matches!(s.status, RestoreStatus::Success | RestoreStatus::Skipped))
    }
}

/// Lower-case executable stem, e.g. `C:\...\Code.exe` -> `code`.
pub fn application_id(executable_path: &str) -> String {
    Path::new(executable_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "unknown".into())
}

/// How an application that is not running gets started.
enum LaunchPlan {
    /// Run its executable.
    Exe,
    /// Microsoft Store app: start it by its package id (`Family!AppId`).
    Packaged(String),
}

type Unlaunchable = (RestoreStatus, &'static str, String);

/// Decide how to start `app`, or explain why it can't be started.
fn plan_launch(desktop: &dyn Desktop, app: &ApplicationState) -> Result<LaunchPlan, Unlaunchable> {
    if app.application_id == "applicationframehost" {
        return Err((
            RestoreStatus::Skipped,
            "UNSUPPORTED_APP",
            "not started: this is a Microsoft Store app window host; open the app yourself".into(),
        ));
    }
    if app
        .executable_path
        .to_lowercase()
        .contains("\\windowsapps\\")
    {
        // Prefer the id saved at capture time: it survives app updates, which
        // change the install folder.
        let saved = app
            .adapter_state
            .get("aumid")
            .and_then(|v| v.as_str())
            .map(String::from);
        return match saved.or_else(|| desktop.packaged_app_id(&app.executable_path)) {
            Some(id) => Ok(LaunchPlan::Packaged(id)),
            None => Err((
                RestoreStatus::Skipped,
                "UNSUPPORTED_APP",
                "not started: could not work out this Store app's package id; open it yourself"
                    .into(),
            )),
        };
    }
    if desktop.exe_exists(&app.executable_path) {
        Ok(LaunchPlan::Exe)
    } else {
        Err((
            RestoreStatus::Failed,
            "EXE_NOT_FOUND",
            format!(
                "{} was not found; is it still installed?",
                app.executable_path
            ),
        ))
    }
}

struct Pending<'a> {
    app: &'a ApplicationState,
    saved: Vec<&'a WindowState>,
    /// Windows of this app that existed before we launched it.
    baseline: HashSet<isize>,
    launched: bool,
    started: Instant,
    last_count: usize,
    last_change: Instant,
}

/// Restore `payload` onto `desktop`. `log` receives short progress lines.
pub fn restore(
    desktop: &dyn Desktop,
    payload: &SnapshotPayload,
    opts: &RestoreOptions,
    log: &mut dyn FnMut(&str),
) -> RestoreRun {
    let mut steps: Vec<StepReport> = Vec::new();
    let monitors: Vec<MonitorInfo> = desktop.monitors().unwrap_or_else(|e| {
        log(&format!("warning: could not read monitors ({e})"));
        Vec::new()
    });
    let baseline_windows = desktop.windows().unwrap_or_else(|e| {
        log(&format!("warning: could not read open windows ({e})"));
        Vec::new()
    });

    let only: Vec<String> = opts.only.iter().map(|s| s.to_lowercase()).collect();
    let mut pending: Vec<Pending> = Vec::new();

    for app in &payload.applications {
        if !only.is_empty() && !only.contains(&app.application_id) {
            continue;
        }
        let began = Instant::now();
        let step = |status, code, message: String| StepReport {
            application_id: app.application_id.clone(),
            status,
            code,
            message,
            duration_ms: began.elapsed().as_millis() as u64,
        };

        let existing: HashSet<isize> = baseline_windows
            .iter()
            .filter(|w| window_app_id(w).as_deref() == Some(app.application_id.as_str()))
            .map(|w| w.hwnd)
            .collect();
        let saved: Vec<&WindowState> = payload
            .windows
            .iter()
            .filter(|w| w.application_id == app.application_id)
            .collect();

        // A running app is only repositioned; otherwise work out how to start it.
        let plan = if existing.is_empty() {
            match plan_launch(desktop, app) {
                Ok(p) => Some(p),
                Err((status, code, message)) => {
                    steps.push(step(status, Some(code), message));
                    continue;
                }
            }
        } else {
            None
        };

        if opts.dry_run {
            let msg = if plan.is_some() {
                "would be started"
            } else {
                "already running; its windows would be repositioned"
            };
            steps.push(step(RestoreStatus::Skipped, Some("DRY_RUN"), msg.into()));
            continue;
        }

        let mut launched = false;
        if let Some(plan) = plan {
            log(&format!("starting {}", app.application_id));
            let result = match &plan {
                LaunchPlan::Exe => desktop.launch(
                    &app.executable_path,
                    &app.launch_arguments,
                    app.working_directory.as_deref(),
                ),
                LaunchPlan::Packaged(id) => desktop.launch_packaged(id).map(|()| 0),
            };
            match result {
                Ok(_) => launched = true,
                Err(e) => {
                    steps.push(step(
                        RestoreStatus::Failed,
                        Some("LAUNCH_FAILED"),
                        format!("could not start {}: {e}", app.application_id),
                    ));
                    continue;
                }
            }
        } else {
            log(&format!("{} is already running", app.application_id));
        }
        let now = Instant::now();
        pending.push(Pending {
            app,
            saved,
            baseline: existing,
            launched,
            started: now,
            last_count: 0,
            last_change: now,
        });
    }

    // Wait for all launched apps together, so total time is bounded by the
    // slowest app rather than the sum of them.
    let mut poll = Duration::from_millis(50);
    while !pending.is_empty() {
        let windows = desktop.windows().unwrap_or_default();
        let now = Instant::now();
        let mut still_waiting = Vec::new();
        for mut p in pending {
            let current: Vec<&DiscoveredWindow> = windows
                .iter()
                .filter(|w| window_app_id(w).as_deref() == Some(p.app.application_id.as_str()))
                .collect();
            let new_count = if p.launched {
                current
                    .iter()
                    .filter(|w| !p.baseline.contains(&w.hwnd))
                    .count()
            } else {
                current.len()
            };
            if new_count != p.last_count {
                p.last_count = new_count;
                p.last_change = now;
            }
            let ready =
                !p.launched || (new_count > 0 && now.duration_since(p.last_change) >= opts.settle);
            let timed_out = now.duration_since(p.started) >= opts.timeout;

            if ready || timed_out {
                log(&format!("arranging {}", p.app.application_id));
                steps.push(finish(desktop, &p, &current, &monitors, opts.timeout));
            } else {
                still_waiting.push(p);
            }
        }
        pending = still_waiting;
        if !pending.is_empty() {
            std::thread::sleep(poll);
            poll = (poll * 2).min(Duration::from_millis(500));
        }
    }

    // Report in the snapshot's application order.
    steps.sort_by_key(|s| {
        payload
            .applications
            .iter()
            .position(|a| a.application_id == s.application_id)
            .unwrap_or(usize::MAX)
    });
    let overall = overall_status(&steps);
    RestoreRun { steps, overall }
}

fn window_app_id(w: &DiscoveredWindow) -> Option<String> {
    w.executable_path.as_deref().map(application_id)
}

fn overall_status(steps: &[StepReport]) -> RestoreStatus {
    let any = |s: RestoreStatus| steps.iter().any(|x| x.status == s);
    let all_failed = !steps.is_empty() && steps.iter().all(|s| s.status == RestoreStatus::Failed);
    if all_failed {
        RestoreStatus::Failed
    } else if any(RestoreStatus::Failed)
        || any(RestoreStatus::PartialSuccess)
        || any(RestoreStatus::RequiresUserIntervention)
    {
        RestoreStatus::PartialSuccess
    } else {
        RestoreStatus::Success
    }
}

/// Place the windows of one app and produce its report.
fn finish(
    desktop: &dyn Desktop,
    p: &Pending<'_>,
    current: &[&DiscoveredWindow],
    monitors: &[MonitorInfo],
    timeout: Duration,
) -> StepReport {
    let app_id = p.app.application_id.clone();
    let report = |status, code, message: String| StepReport {
        application_id: app_id.clone(),
        status,
        code,
        message,
        duration_ms: p.started.elapsed().as_millis() as u64,
    };

    if current.is_empty() {
        return report(
            RestoreStatus::Failed,
            Some("WINDOW_TIMEOUT"),
            format!(
                "started, but no window appeared within {} seconds",
                timeout.as_secs()
            ),
        );
    }
    if p.saved.is_empty() {
        return report(RestoreStatus::Success, None, "started".into());
    }

    // Pair saved windows with open ones: identical title first, then in order.
    let mut used = vec![false; current.len()];
    let mut pairs: Vec<(&WindowState, &DiscoveredWindow)> = Vec::new();
    let mut unpaired: Vec<&WindowState> = Vec::new();
    for s in &p.saved {
        match (0..current.len()).find(|&i| !used[i] && current[i].title == s.title) {
            Some(i) => {
                used[i] = true;
                pairs.push((s, current[i]));
            }
            None => unpaired.push(s),
        }
    }
    for s in unpaired {
        match (0..current.len()).find(|&i| !used[i]) {
            Some(i) => {
                used[i] = true;
                pairs.push((s, current[i]));
            }
            None => break,
        }
    }

    let (mut placed, mut denied, mut failed) = (0usize, 0usize, 0usize);
    for (saved, win) in &pairs {
        let target = if monitors.is_empty() {
            saved.rect
        } else {
            placement_for(&saved.rect, saved.monitor_id.as_deref(), monitors).unwrap_or(saved.rect)
        };
        match desktop.place(win.hwnd, target, saved.display_state) {
            Ok(()) => placed += 1,
            Err(PlaceError::AccessDenied) => denied += 1,
            Err(PlaceError::Other(e)) => {
                tracing::warn!(app = %app_id, error = %e, "could not place window");
                failed += 1;
            }
        }
    }

    let total = p.saved.len();
    if denied > 0 {
        return report(
            RestoreStatus::RequiresUserIntervention,
            Some("ACCESS_DENIED"),
            format!(
                "{denied} window(s) belong to an app running as administrator and can't be moved; run ContextFlow as administrator or move them yourself"
            ),
        );
    }
    if failed > 0 && placed == 0 {
        return report(
            RestoreStatus::Failed,
            Some("PLACE_FAILED"),
            "the windows opened but could not be moved".into(),
        );
    }
    if failed > 0 || placed < total {
        return report(
            RestoreStatus::PartialSuccess,
            Some("FEWER_WINDOWS"),
            format!(
                "arranged {placed} of {total} saved window(s); the app only opened {} (ContextFlow can't yet reopen individual documents or tabs)",
                current.len()
            ),
        );
    }
    let how = if p.launched {
        "started"
    } else {
        "already running"
    };
    report(
        RestoreStatus::Success,
        None,
        format!("{how}; {placed} window(s) arranged"),
    )
}

#[cfg(test)]
mod tests;

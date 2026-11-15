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

mod browser;
mod desktop;
mod editors;
mod resolve;
mod window_open;

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cf_core::geometry::placement_for;
use cf_core::RestoreStatus;
use cf_core::{ApplicationState, MonitorInfo, SnapshotPayload, WindowState};
use cf_windows::DiscoveredWindow;
use serde::Serialize;

pub use desktop::{Desktop, PlaceError, WindowsDesktop};
pub use resolve::launcher_from_ancestry;

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
    /// Most saved tabs reopened per browser window.
    pub max_tabs_per_window: usize,
    /// Longest the whole restore may take, however many applications there are.
    pub overall_timeout: Duration,
    /// Set this flag (from another thread) to stop the restore early. Nothing
    /// already started is closed; applications not yet started are skipped.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(20),
            settle: Duration::from_millis(1500),
            only: Vec::new(),
            dry_run: false,
            max_tabs_per_window: 50,
            overall_timeout: Duration::from_secs(120),
            cancel: None,
        }
    }
}

/// Work done while waiting for a window: putting other applications in place.
/// It gets the progress log, which the waiting code also holds.
pub(crate) type Meanwhile<'a> = dyn FnMut(&mut dyn FnMut(&str)) + 'a;

/// The time and cancellation limits of one restore.
pub(crate) struct Budget {
    end: Instant,
    cancel: Option<Arc<AtomicBool>>,
}

impl Budget {
    fn new(opts: &RestoreOptions) -> Self {
        Self {
            end: Instant::now() + opts.overall_timeout,
            cancel: opts.cancel.clone(),
        }
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
    }

    /// Out of time, or cancelled.
    pub(crate) fn expired(&self) -> bool {
        self.cancelled() || Instant::now() >= self.end
    }

    /// Time left; zero once expired or cancelled.
    pub(crate) fn remaining(&self) -> Duration {
        if self.cancelled() {
            Duration::ZERO
        } else {
            self.end.saturating_duration_since(Instant::now())
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
    /// Where the time went, in milliseconds: `scan` (reading the desktop),
    /// `start` (starting programs), `wait` (for their windows) and `place`.
    pub phases: Vec<(&'static str, u64)>,
    /// Milliseconds from the start until the first window was in place.
    pub first_window_ms: Option<u64>,
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
    /// Run this executable (the saved path, or the same program in a newer
    /// version folder if the app updated).
    Exe(String),
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
    // Only ever start a full path to a real program file. A relative path would
    // be resolved against whatever folder ContextFlow runs in, and scripts or
    // shortcuts are not programs the user chose to save.
    let path = Path::new(&app.executable_path);
    let is_exe = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    if !path.is_absolute() || !is_exe {
        return Err((
            RestoreStatus::Failed,
            "UNSAFE_PATH",
            "not started: only full paths to .exe files can be launched".into(),
        ));
    }
    // A helper process (Steam's `steamwebhelper.exe`) does nothing when started
    // on its own; start the app it belongs to. Newer captures already save the
    // launcher; this covers snapshots saved before that.
    if let Some(launcher) =
        resolve::helper_launcher(&app.executable_path, &|p| desktop.exe_exists(p))
    {
        return Ok(LaunchPlan::Exe(launcher));
    }
    if desktop.exe_exists(&app.executable_path) {
        return Ok(LaunchPlan::Exe(app.executable_path.clone()));
    }
    // Apps that install into `app-<version>` folders move on every update.
    if let Some(updated) = resolve::newest_versioned_sibling(
        &app.executable_path,
        &|dir| desktop.list_dirs(dir),
        &|path| desktop.exe_exists(path),
    ) {
        return Ok(LaunchPlan::Exe(updated));
    }
    Err((
        RestoreStatus::Failed,
        "EXE_NOT_FOUND",
        format!(
            "{} was not found; is it still installed?",
            app.executable_path
        ),
    ))
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
    /// Tabs or folders added to an already-running browser / VS Code, if any.
    tabs: Option<browser::TabRestore>,
    /// How `tabs` is described in the report.
    wording: &'static browser::Wording,
    /// Its windows were put in place as soon as the first one showed.
    placed_early: bool,
}

impl<'a> Pending<'a> {
    fn new(
        app: &'a ApplicationState,
        saved: Vec<&'a WindowState>,
        baseline: HashSet<isize>,
        launched: bool,
    ) -> Self {
        let now = Instant::now();
        Self {
            app,
            saved,
            baseline,
            launched,
            started: now,
            last_count: 0,
            last_change: now,
            tabs: None,
            wording: &browser::TABS,
            placed_early: false,
        }
    }
}

/// What the engine is still waiting on: applications that are starting, and
/// windows to put in place once more (see [`window_open::Recheck`]).
struct Waiting<'a> {
    apps: Vec<Pending<'a>>,
    rechecks: Vec<window_open::Recheck>,
}

impl Waiting<'_> {
    fn is_empty(&self) -> bool {
        self.apps.is_empty() && self.rechecks.is_empty()
    }

    /// How long to sleep before looking again: `poll`, or less when a
    /// recheck is due sooner.
    fn sleep_for(&self, poll: Duration) -> Duration {
        let now = Instant::now();
        self.rechecks
            .iter()
            .map(|r| r.due.saturating_duration_since(now))
            .fold(poll, Duration::min)
    }
}

/// What a deferred application needs opened.
enum DeferredWork {
    Tabs(cf_browser::BrowserAppState),
    Folders(editors::EditorWork),
}

/// A browser or VS Code that is not running and has saved tabs or folders. It
/// is started only after every other application, because opening its windows
/// is the slowest part.
struct Deferred<'a> {
    app: &'a ApplicationState,
    exe: String,
    work: DeferredWork,
    saved: Vec<&'a WindowState>,
    began: Instant,
}

/// A running browser or editor with tabs or folders that may need adding.
struct RunningWork<'a> {
    app: &'a ApplicationState,
    saved: Vec<&'a WindowState>,
    baseline: HashSet<isize>,
    tabs: Option<cf_browser::BrowserAppState>,
    folders: Option<editors::EditorWork>,
}

/// An application that only needs starting.
struct Launch<'a> {
    app: &'a ApplicationState,
    plan: LaunchPlan,
    saved: Vec<&'a WindowState>,
    baseline: HashSet<isize>,
    began: Instant,
}

fn cancelled_step(app: &ApplicationState) -> StepReport {
    StepReport {
        application_id: app.application_id.clone(),
        status: RestoreStatus::Skipped,
        code: Some("CANCELLED"),
        message: "restore was cancelled".into(),
        duration_ms: 0,
    }
}

/// The finished applications, and a progress line as each one finishes
/// ("code is back (3 of 7)"), so a front end can show the desk filling up.
struct Results {
    steps: Vec<StepReport>,
    total: usize,
    /// A dry run says what it would do, not what came back.
    quiet: bool,
}

impl Results {
    fn push(&mut self, step: StepReport, log: &mut dyn FnMut(&str)) {
        if !self.quiet {
            let what = match step.status {
                RestoreStatus::Failed => "didn't come back",
                RestoreStatus::Skipped => "was skipped",
                _ => "is back",
            };
            log(&format!(
                "{} {what} ({} of {})",
                step.application_id,
                self.steps.len() + 1,
                self.total
            ));
        }
        self.steps.push(step);
    }
}

/// The order applications come back in: the one whose window was on top when
/// the snapshot was saved first (capture lists windows top to bottom), then
/// the next, and applications with no saved window last.
fn importance_order(payload: &SnapshotPayload) -> Vec<&ApplicationState> {
    let mut apps: Vec<&ApplicationState> = payload.applications.iter().collect();
    apps.sort_by_key(|a| {
        payload
            .windows
            .iter()
            .position(|w| w.application_id == a.application_id)
            .unwrap_or(usize::MAX)
    });
    apps
}

/// Restore `payload` onto `desktop`. `log` receives short progress lines.
///
/// The desk fills up in stages, most important application first within each:
///   1. applications already running are put in place at once;
///   2. the others are started, without waiting for any of them;
///   3. running browsers and editors get the tabs and folders they are missing;
///   4. browsers and editors that are not running are started with theirs;
///
/// and every started application is put in place as soon as its windows
/// settle, also while stages 3 and 4 are still waiting for windows.
pub fn restore(
    desktop: &dyn Desktop,
    payload: &SnapshotPayload,
    opts: &RestoreOptions,
    log: &mut dyn FnMut(&str),
) -> RestoreRun {
    let began = Instant::now();
    let timed = desktop::Timed::new(desktop, &payload.windows);
    let desktop: &dyn Desktop = &timed;
    let monitors: Vec<MonitorInfo> = desktop.monitors().unwrap_or_else(|e| {
        log(&format!("warning: could not read monitors ({e})"));
        Vec::new()
    });
    let baseline_windows = desktop.windows().unwrap_or_else(|e| {
        log(&format!("warning: could not read open windows ({e})"));
        Vec::new()
    });

    let budget = Budget::new(opts);
    let only: Vec<String> = opts.only.iter().map(|s| s.to_lowercase()).collect();
    let apps: Vec<&ApplicationState> = importance_order(payload)
        .into_iter()
        .filter(|a| only.is_empty() || only.contains(&a.application_id))
        .collect();
    let mut results = Results {
        steps: Vec::new(),
        total: apps.len(),
        quiet: opts.dry_run,
    };
    let mut waiting = Waiting {
        apps: Vec::new(),
        rechecks: Vec::new(),
    };
    let mut running_work: Vec<RunningWork> = Vec::new();
    let mut launches: Vec<Launch> = Vec::new();
    let mut deferred: Vec<Deferred> = Vec::new();

    // Work out what each application needs. Nothing is started here.
    for app in apps {
        // Once cancelled, this and every remaining application is skipped.
        if budget.cancelled() {
            results.push(cancelled_step(app), log);
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
                    results.push(step(status, Some(code), message), log);
                    continue;
                }
            }
        } else {
            None
        };

        let tabs = browser::tab_state(app);
        let folders = editors::editor_work(app);

        if opts.dry_run {
            let msg = match (&plan, &tabs, &folders) {
                (Some(_), Some(t), _) => format!(
                    "would be started with {} tab(s) in {} window(s)",
                    t.tab_count(),
                    t.windows.len()
                ),
                (None, Some(t), _) => format!(
                    "already running; its windows would be repositioned and any of its {} saved tab(s) that aren't open would be added",
                    t.tab_count()
                ),
                (Some(_), None, Some(f)) => {
                    format!("would be started with {} {}", f.count(), f.wording.noun)
                }
                (None, None, Some(f)) => format!(
                    "already running; its windows would be repositioned and any of its {} saved {} that aren't open would be added",
                    f.count(),
                    f.wording.noun
                ),
                (Some(_), None, None) => "would be started".to_string(),
                (None, None, None) => "already running; its windows would be repositioned".to_string(),
            };
            results.push(step(RestoreStatus::Skipped, Some("DRY_RUN"), msg), log);
            continue;
        }

        match (plan, tabs, folders) {
            // A browser or VS Code that isn't running is started by opening its
            // saved windows. That is slow, so it waits until everything else has
            // started.
            (Some(LaunchPlan::Exe(exe)), Some(t), _) => deferred.push(Deferred {
                app,
                exe,
                work: DeferredWork::Tabs(t),
                saved,
                began,
            }),
            (Some(LaunchPlan::Exe(exe)), None, Some(f)) => deferred.push(Deferred {
                app,
                exe,
                work: DeferredWork::Folders(f),
                saved,
                began,
            }),
            (Some(plan), _, _) => launches.push(Launch {
                app,
                plan,
                saved,
                baseline: existing,
                began,
            }),
            (None, None, None) => waiting.apps.push(Pending::new(app, saved, existing, false)),
            (None, tabs, folders) => running_work.push(RunningWork {
                app,
                saved,
                baseline: existing,
                tabs,
                folders,
            }),
        }
    }

    // 1. Everything already running goes into place now.
    place_ready(
        desktop,
        &mut waiting,
        &mut results,
        &monitors,
        opts,
        &budget,
        log,
    );

    // 2. Start everything that only needs starting, without waiting for it.
    for l in launches {
        if budget.cancelled() {
            results.push(cancelled_step(l.app), log);
            continue;
        }
        log(&format!("starting {}", l.app.application_id));
        let started = match &l.plan {
            LaunchPlan::Exe(exe) => desktop
                .launch(
                    exe,
                    &l.app.launch_arguments,
                    l.app.working_directory.as_deref(),
                )
                .map(drop),
            LaunchPlan::Packaged(id) => desktop.launch_packaged(id),
        };
        match started {
            Ok(()) => waiting
                .apps
                .push(Pending::new(l.app, l.saved, l.baseline, true)),
            Err(e) => results.push(
                StepReport {
                    application_id: l.app.application_id.clone(),
                    status: RestoreStatus::Failed,
                    code: Some("LAUNCH_FAILED"),
                    message: format!("could not start {}: {e}", l.app.application_id),
                    duration_ms: l.began.elapsed().as_millis() as u64,
                },
                log,
            ),
        }
    }

    // 3. A running browser gets any tabs it is missing, a running editor any
    // folders that have no window yet; then its windows go into place.
    for w in running_work {
        if budget.cancelled() {
            results.push(cancelled_step(w.app), log);
            continue;
        }
        let current_titles: Vec<String> = baseline_windows
            .iter()
            .filter(|x| window_app_id(x).as_deref() == Some(w.app.application_id.as_str()))
            .map(|x| x.title.clone())
            .collect();
        let mut meanwhile = |log: &mut dyn FnMut(&str)| {
            place_ready(
                desktop,
                &mut waiting,
                &mut results,
                &monitors,
                opts,
                &budget,
                log,
            )
        };
        let (mut out, wording) = match (&w.tabs, &w.folders) {
            (Some(t), _) => (
                browser::restore_tabs(
                    desktop,
                    w.app,
                    &w.app.executable_path,
                    t,
                    &w.saved,
                    &monitors,
                    true,
                    opts,
                    &budget,
                    log,
                    &mut meanwhile,
                ),
                &browser::TABS,
            ),
            (None, Some(f)) => (
                editors::restore_windows(
                    desktop,
                    w.app,
                    &w.app.executable_path,
                    f,
                    &w.saved,
                    &monitors,
                    &current_titles,
                    opts,
                    &budget,
                    log,
                    &mut meanwhile,
                ),
                f.wording,
            ),
            (None, None) => unreachable!("running work always has tabs or folders"),
        };
        // Positions taken by windows opened for tabs or folders are not handed
        // out again: otherwise an already-open window could be moved into the
        // spot of a window that was just reopened.
        let saved: Vec<&WindowState> = w
            .saved
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !out.used_saved.contains(i))
            .map(|(_, s)| s)
            .collect();
        waiting.rechecks.append(&mut out.rechecks);
        let mut p = Pending::new(w.app, saved, w.baseline, false);
        p.tabs = Some(out);
        p.wording = wording;
        waiting.apps.push(p);
        place_ready(
            desktop,
            &mut waiting,
            &mut results,
            &monitors,
            opts,
            &budget,
            log,
        );
    }

    // 4. The browsers and editors that need their windows opened, one after
    // another, within the overall budget.
    for d in deferred {
        if budget.cancelled() {
            results.push(cancelled_step(d.app), log);
            continue;
        }
        let mut meanwhile = |log: &mut dyn FnMut(&str)| {
            place_ready(
                desktop,
                &mut waiting,
                &mut results,
                &monitors,
                opts,
                &budget,
                log,
            )
        };
        let (mut out, wording) = match &d.work {
            DeferredWork::Tabs(t) => {
                log(&format!(
                    "starting {} with saved tabs",
                    d.app.application_id
                ));
                (
                    browser::restore_tabs(
                        desktop,
                        d.app,
                        &d.exe,
                        t,
                        &d.saved,
                        &monitors,
                        false,
                        opts,
                        &budget,
                        log,
                        &mut meanwhile,
                    ),
                    &browser::TABS,
                )
            }
            DeferredWork::Folders(f) => {
                log(&format!(
                    "starting {} with saved folders",
                    d.app.application_id
                ));
                (
                    editors::restore_windows(
                        desktop,
                        d.app,
                        &d.exe,
                        f,
                        &d.saved,
                        &monitors,
                        &[],
                        opts,
                        &budget,
                        log,
                        &mut meanwhile,
                    ),
                    f.wording,
                )
            }
        };
        waiting.rechecks.append(&mut out.rechecks);
        let report = browser::started_report(
            &d.app.application_id,
            wording,
            &out,
            d.began,
            budget.expired(),
        );
        results.push(report, log);
    }

    // 5. Whatever was started and is not in place yet: wait for all of them
    // together, so the total is bounded by the slowest, not the sum.
    let mut poll = Duration::from_millis(50);
    loop {
        place_ready(
            desktop,
            &mut waiting,
            &mut results,
            &monitors,
            opts,
            &budget,
            log,
        );
        if waiting.is_empty() {
            break;
        }
        std::thread::sleep(waiting.sleep_for(poll));
        poll = (poll * 2).min(Duration::from_millis(500));
    }

    // Everything is in place. Rebuild the saved stacking order, so the window
    // that was on top is on top, and give it the focus: placing and starting
    // windows leaves them in whatever order they happened to be handled, and
    // the last app to start would otherwise keep the keyboard.
    if !opts.dry_run && !budget.cancelled() {
        timed.restore_stack();
    }

    // Report in the snapshot's application order.
    let mut steps = results.steps;
    steps.sort_by_key(|s| {
        payload
            .applications
            .iter()
            .position(|a| a.application_id == s.application_id)
            .unwrap_or(usize::MAX)
    });
    let overall = overall_status(&steps);
    RestoreRun {
        steps,
        overall,
        phases: timed.phases(began.elapsed()),
        first_window_ms: timed.first_placed_ms(),
    }
}

/// Put in place every waiting application that is ready (a running one at
/// once, a started one once its windows have stopped changing for `settle`)
/// or out of time, and place again the new windows whose recheck is due.
/// Called between stages and while waiting for windows.
fn place_ready(
    desktop: &dyn Desktop,
    waiting: &mut Waiting<'_>,
    results: &mut Results,
    monitors: &[MonitorInfo],
    opts: &RestoreOptions,
    budget: &Budget,
    log: &mut dyn FnMut(&str),
) {
    // New windows were placed the moment they appeared; place them once more
    // when due, in case their app moved them straight afterwards. A failure
    // here was already reported at the first placement.
    let now = Instant::now();
    let cancelled = budget.cancelled();
    waiting.rechecks.retain(|r| {
        if cancelled || r.due > now {
            return !cancelled;
        }
        let _ = window_open::place_saved(desktop, r.hwnd, &r.saved, monitors);
        false
    });
    if waiting.apps.is_empty() {
        return;
    }
    let windows = desktop.windows().unwrap_or_default();
    let now = Instant::now();
    let mut still_waiting = Vec::new();
    for mut p in waiting.apps.drain(..) {
        // Windows opened for tabs were already placed; leave them alone.
        let claimed = p.tabs.as_ref().map(|t| &t.claimed);
        let current: Vec<&DiscoveredWindow> = windows
            .iter()
            .filter(|w| window_app_id(w).as_deref() == Some(p.app.application_id.as_str()))
            .filter(|w| !claimed.is_some_and(|c| c.contains(&w.hwnd)))
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
        // Out of overall time (or cancelled): stop waiting for everything.
        let timed_out = now.duration_since(p.started) >= opts.timeout || budget.expired();

        if ready || timed_out {
            let report = finish(desktop, &p, &current, monitors, opts.timeout);
            let report = match &p.tabs {
                Some(t) => browser::merge_into(report, p.wording, t),
                None => report,
            };
            results.push(report, log);
        } else {
            // A started app's windows go into place as soon as the first one
            // shows, not only once they have settled. They are placed again,
            // and reported, when they have (a splash screen may come first).
            if p.launched && new_count > 0 && !p.placed_early {
                let _ = finish(desktop, &p, &current, monitors, opts.timeout);
                p.placed_early = true;
            }
            still_waiting.push(p);
        }
    }
    waiting.apps = still_waiting;
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
    // Said of every result below: did this restore start the app, or was it
    // already running (for example a browser that only needed tabs added)?
    let how = if p.launched {
        "started"
    } else {
        "already running"
    };
    if p.saved.is_empty() {
        return report(RestoreStatus::Success, None, how.into());
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
            Ok(()) => {
                placed += 1;
                desktop.placed(saved, win.hwnd);
            }
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
    report(
        RestoreStatus::Success,
        None,
        format!("{how}; {placed} window(s) arranged"),
    )
}

#[cfg(test)]
mod tests;

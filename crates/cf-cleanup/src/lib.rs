//! Workspace cleanup: close the programs that distract from a workspace, or
//! that it no longer needs, and report what that gives back.
//!
//! This is a workspace tool, not a task manager. It only ever acts on programs
//! the user listed in a workspace's rules, never on critical system programs
//! (see [`protection`]), and it always tells you what it did to each one.
//!
//! Everything goes through [`Processes`], so the logic is testable without
//! touching real processes.

mod protection;
mod real;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use cf_core::{CleanupTarget, CloseMode};
use cf_windows::ProcessInfo;
use serde::Serialize;

pub use protection::protected_reason;
pub use real::WindowsProcesses;

/// The system, as far as cleanup needs it.
pub trait Processes {
    fn list(&self) -> Vec<ProcessInfo>;
    /// Ask a process to close; returns how many windows were asked.
    fn request_close(&self, pid: u32) -> usize;
    fn terminate(&self, pid: u32) -> Result<(), String>;
    /// The session cleanup may act in; processes in other sessions are left alone.
    fn own_session(&self) -> Option<u32>;
    /// ContextFlow itself and the programs that started it: never closed.
    fn protected_pids(&self) -> HashSet<u32>;
    fn logical_cpus(&self) -> usize;
    fn sleep(&self, d: Duration);
}

/// All running processes of one program (same executable name).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunningApp {
    /// Executable name without `.exe`, lower case: what rules refer to.
    pub process: String,
    /// Executable file name as the system shows it, e.g. `Discord.exe`.
    pub display_name: String,
    pub process_count: usize,
    /// Memory in use, in bytes (working set).
    pub ram_bytes: u64,
    /// Share of the whole CPU used over the sampling window, in percent.
    pub cpu_percent: f32,
    /// Set when cleanup would refuse to close it.
    pub protected: Option<String>,
    /// The distinct executable files running under this name, so a rule can
    /// show exactly what it would close (two programs can share a name).
    pub executables: Vec<String>,
}

fn stem(name: &str) -> String {
    let lower = name.to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// A process cleanup may consider: in our session, inspectable, not ContextFlow
/// or its parents.
fn eligible(p: &ProcessInfo, own_session: Option<u32>, protected: &HashSet<u32>) -> bool {
    p.path.is_some()
        && p.pid > 4
        && !protected.contains(&p.pid)
        && (own_session.is_none() || p.session_id == own_session)
}

/// Programs that are running ContextFlow: the name of every process that
/// started it (a terminal, the app that launched it). Closing any process of
/// such a program could end ContextFlow itself, so the whole program is
/// protected, not only the one process.
fn host_programs(procs: &dyn Processes, protected: &HashSet<u32>) -> HashSet<String> {
    procs
        .list()
        .iter()
        .filter(|p| protected.contains(&p.pid))
        .map(|p| stem(&p.name))
        .collect()
}

const HOST_REASON: &str = "it is running ContextFlow";

/// The programs that are running now, grouped by name, with their memory and
/// recent CPU use (measured over `sample`). Programs cleanup would refuse to
/// close are included but marked, so the UI can show them greyed out.
pub fn running_apps(procs: &dyn Processes, sample: Duration) -> Vec<RunningApp> {
    let own_session = procs.own_session();
    let protected_pids = procs.protected_pids();
    let hosts = host_programs(procs, &protected_pids);
    let before = procs.list();
    let started = Instant::now();
    procs.sleep(sample);
    let after = procs.list();
    let elapsed = started.elapsed().max(Duration::from_millis(1));

    let cpu_before: HashMap<u32, u64> = before.iter().map(|p| (p.pid, p.cpu_time_100ns)).collect();
    let cpus = procs.logical_cpus().max(1) as f64;
    // 100 ns ticks used by all processes over the window, as a share of the
    // CPU time available (window length x number of logical processors).
    let available_ticks = elapsed.as_secs_f64() * 10_000_000.0 * cpus;

    let mut groups: BTreeMap<String, RunningApp> = BTreeMap::new();
    for p in after
        .iter()
        .filter(|p| eligible(p, own_session, &protected_pids))
    {
        let used = p
            .cpu_time_100ns
            .saturating_sub(cpu_before.get(&p.pid).copied().unwrap_or(p.cpu_time_100ns));
        let key = stem(&p.name);
        let g = groups.entry(key.clone()).or_insert_with(|| RunningApp {
            process: key,
            display_name: p.name.clone(),
            process_count: 0,
            ram_bytes: 0,
            cpu_percent: 0.0,
            protected: protected_reason(&p.name, p.path.as_deref())
                .or_else(|| hosts.contains(&stem(&p.name)).then_some(HOST_REASON))
                .map(String::from),
            executables: Vec::new(),
        });
        if let Some(path) = &p.path {
            if !g.executables.iter().any(|e| e.eq_ignore_ascii_case(path)) {
                g.executables.push(path.clone());
            }
        }
        g.process_count += 1;
        g.ram_bytes += p.ram_bytes;
        g.cpu_percent += (used as f64 / available_ticks * 100.0) as f32;
    }
    let mut apps: Vec<RunningApp> = groups.into_values().collect();
    apps.sort_by(|a, b| {
        b.ram_bytes
            .cmp(&a.ram_bytes)
            .then(a.process.cmp(&b.process))
    });
    apps
}

/// What would happen to one listed program.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlannedAction {
    /// Running now; would be closed.
    Close {
        process_count: usize,
        ram_bytes: u64,
        cpu_percent: f32,
    },
    NotRunning,
    /// Critical system program: never closed.
    Protected {
        reason: String,
    },
    /// The workspace being entered needs it, so it is left alone.
    KeptByWorkspace,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanItem {
    pub process: String,
    pub display_name: String,
    pub mode: CloseMode,
    pub action: PlannedAction,
    /// Executable files the rule matches right now (empty when not running).
    pub executables: Vec<String>,
}

/// What a cleanup would do, and what it would give back.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CleanupPlan {
    pub items: Vec<PlanItem>,
    /// Memory that would be freed (programs that would be closed), in bytes.
    pub ram_bytes: u64,
    /// CPU that would be freed, as a share of the whole CPU, in percent.
    pub cpu_percent: f32,
}

impl CleanupPlan {
    pub fn closing(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i.action, PlannedAction::Close { .. }))
            .count()
    }
}

/// Preview `targets`. `keep` holds the programs the workspace itself uses
/// (never closed, even if listed). `apps` comes from [`running_apps`].
pub fn plan(targets: &[CleanupTarget], keep: &HashSet<String>, apps: &[RunningApp]) -> CleanupPlan {
    let by_name: HashMap<&str, &RunningApp> =
        apps.iter().map(|a| (a.process.as_str(), a)).collect();
    let mut items = Vec::new();
    let (mut ram, mut cpu) = (0u64, 0f32);
    for t in targets {
        let running = by_name.get(t.process.as_str());
        let display_name =
            running.map_or_else(|| format!("{}.exe", t.process), |a| a.display_name.clone());
        // Protection is by name too, so a protected program is refused even
        // when it isn't running or can't be inspected.
        let action = if let Some(reason) = protected_reason(&format!("{}.exe", t.process), None) {
            PlannedAction::Protected {
                reason: reason.into(),
            }
        } else if keep.contains(&t.process) {
            PlannedAction::KeptByWorkspace
        } else {
            match running {
                None => PlannedAction::NotRunning,
                Some(a) => match &a.protected {
                    Some(reason) => PlannedAction::Protected {
                        reason: reason.clone(),
                    },
                    None => {
                        ram += a.ram_bytes;
                        cpu += a.cpu_percent;
                        PlannedAction::Close {
                            process_count: a.process_count,
                            ram_bytes: a.ram_bytes,
                            cpu_percent: a.cpu_percent,
                        }
                    }
                },
            }
        };
        items.push(PlanItem {
            process: t.process.clone(),
            display_name,
            mode: t.mode,
            action,
            executables: running.map(|a| a.executables.clone()).unwrap_or_default(),
        });
    }
    CleanupPlan {
        items,
        ram_bytes: ram,
        cpu_percent: cpu,
    }
}

/// What happened to one listed program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Closed when asked.
    Closed,
    /// Did not close when asked, so it was ended (force mode).
    Terminated,
    /// Asked to close but still running (graceful mode): many apps hide to the
    /// tray, or ask to save first.
    StillRunning,
    NotRunning,
    Protected,
    KeptByWorkspace,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TargetResult {
    pub process: String,
    pub mode: CloseMode,
    pub outcome: Outcome,
    pub message: String,
}

/// Timing of a cleanup run.
#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// How long a program gets to close after being asked.
    pub close_wait: Duration,
    pub poll: Duration,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            close_wait: Duration::from_secs(6),
            poll: Duration::from_millis(250),
        }
    }
}

fn pids_of(procs: &dyn Processes, process: &str) -> Vec<u32> {
    let own_session = procs.own_session();
    let protected = procs.protected_pids();
    procs
        .list()
        .iter()
        .filter(|p| eligible(p, own_session, &protected) && stem(&p.name) == process)
        .map(|p| p.pid)
        .collect()
}

/// The pids of every process running right now.
fn running_pids(procs: &dyn Processes) -> HashSet<u32> {
    procs.list().iter().map(|p| p.pid).collect()
}

fn result(t: &CleanupTarget, outcome: Outcome, message: impl Into<String>) -> TargetResult {
    TargetResult {
        process: t.process.clone(),
        mode: t.mode,
        outcome,
        message: message.into(),
    }
}

/// A program that was asked to close, and what became of it so far.
struct Asked<'a> {
    target: &'a CleanupTarget,
    /// Windows that were asked to close.
    asked: usize,
    /// Its processes that are still running.
    remaining: Vec<u32>,
    /// Why some of them could not be ended, if they could not.
    errors: Vec<String>,
}

impl Asked<'_> {
    /// Whether it is worth waiting for. With nothing to ask (no window) in force
    /// mode, waiting would only delay ending it.
    fn waits(&self) -> bool {
        !self.remaining.is_empty() && (self.asked > 0 || self.target.mode == CloseMode::Graceful)
    }
}

/// Why a program must not be touched, or the processes to close.
fn check(
    procs: &dyn Processes,
    t: &CleanupTarget,
    keep: &HashSet<String>,
) -> Result<Vec<u32>, TargetResult> {
    if let Some(reason) = protected_reason(&format!("{}.exe", t.process), None) {
        return Err(result(
            t,
            Outcome::Protected,
            format!("not closed: {reason}"),
        ));
    }
    if keep.contains(&t.process) {
        return Err(result(
            t,
            Outcome::KeptByWorkspace,
            "left open: the workspace you're entering uses it",
        ));
    }
    if host_programs(procs, &procs.protected_pids()).contains(&t.process) {
        return Err(result(
            t,
            Outcome::Protected,
            format!("not closed: {HOST_REASON}"),
        ));
    }
    let pids = pids_of(procs, &t.process);
    if pids.is_empty() {
        return Err(result(t, Outcome::NotRunning, "not running"));
    }
    Ok(pids)
}

/// Close `targets`. Never touches a protected program or one in `keep`. A
/// failure on one program doesn't stop the others. Results are in the order of
/// `targets`.
///
/// Every program is asked to close first, and then all are waited for
/// together, so the wait is one `close_wait` however many ignore the request
/// (several apps hide to the tray instead of exiting). Closing them one after
/// another would cost a full wait for each.
pub fn run(
    procs: &dyn Processes,
    targets: &[CleanupTarget],
    keep: &HashSet<String>,
    opts: &RunOptions,
) -> Vec<TargetResult> {
    // 1. Ask nicely, everything at once.
    let mut steps: Vec<Result<Asked, TargetResult>> = targets
        .iter()
        .map(|t| {
            let pids = check(procs, t, keep)?;
            let asked = pids.iter().map(|&p| procs.request_close(p)).sum();
            Ok(Asked {
                target: t,
                asked,
                remaining: pids,
                errors: Vec::new(),
            })
        })
        .collect();

    // 2. Wait for all of them together.
    let deadline = Instant::now() + opts.close_wait;
    loop {
        let running = running_pids(procs);
        for a in steps.iter_mut().flatten() {
            a.remaining.retain(|p| running.contains(p));
        }
        let waiting = steps.iter().flatten().any(Asked::waits);
        if !waiting || Instant::now() >= deadline {
            break;
        }
        procs.sleep(opts.poll);
    }

    // 3. What is still running: end it (force mode), all at once, and give
    // them one poll to go.
    let mut ended = false;
    for a in steps.iter_mut().flatten() {
        if a.target.mode == CloseMode::Force {
            for pid in &a.remaining {
                ended = true;
                if let Err(e) = procs.terminate(*pid) {
                    a.errors.push(e);
                }
            }
        }
    }
    if ended {
        procs.sleep(opts.poll);
    }
    let running = running_pids(procs);

    steps
        .into_iter()
        .map(|step| {
            let a = match step {
                Ok(a) => a,
                Err(done) => return done,
            };
            let t = a.target;
            let left: Vec<u32> = a
                .remaining
                .iter()
                .copied()
                .filter(|p| running.contains(p))
                .collect();
            if a.remaining.is_empty() {
                return result(t, Outcome::Closed, "closed");
            }
            if t.mode == CloseMode::Graceful {
                let why = if a.asked == 0 {
                    "it has no window to ask"
                } else {
                    "it may have hidden to the tray or be asking to save"
                };
                return result(
                    t,
                    Outcome::StillRunning,
                    format!("still running ({why}); use force close to end it"),
                );
            }
            if left.is_empty() {
                return result(
                    t,
                    Outcome::Terminated,
                    "did not close when asked, so it was ended",
                );
            }
            tracing::warn!(process = %t.process, errors = ?a.errors, "cleanup could not end a program");
            result(
                t,
                Outcome::Failed,
                format!(
                    "could not end it{}",
                    a.errors
                        .first()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests;

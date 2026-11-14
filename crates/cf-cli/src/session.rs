//! Resolve paths and run a restore: the flow shared by `contextflow restore`
//! and the desktop launcher.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::Utc;

use cf_cleanup::{CleanupPlan, TargetResult, WindowsProcesses};
use cf_config::Config;
use cf_core::{CleanupTarget, Context, ContextId, Snapshot};
use cf_engine::{RestoreOptions, RestoreRun, WindowsDesktop};
use cf_storage::{Database, RestoreStepRecord};

pub struct Paths {
    pub config: PathBuf,
    pub db: PathBuf,
    pub logs: PathBuf,
    pub backups: PathBuf,
}

pub fn default_config_path() -> Result<PathBuf> {
    Ok(cf_config::project_dirs()
        .context("could not determine the per-user config directory")?
        .config_dir()
        .join("config.toml"))
}

/// Work out where everything lives. `db_override` beats the config file, which
/// beats the per-user default. Logs and backups sit next to the database.
pub fn resolve_paths(
    db_override: Option<PathBuf>,
    config_path: PathBuf,
    cfg: &Config,
) -> Result<Paths> {
    let db = match db_override.or_else(|| cfg.storage.database_path.clone()) {
        Some(p) => p,
        None => cf_config::project_dirs()
            .context("could not determine the per-user data directory")?
            .data_dir()
            .join("contextflow.db"),
    };
    let base = db
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(Paths {
        config: config_path,
        logs: base.join("logs"),
        backups: cfg
            .storage
            .backup_dir
            .clone()
            .unwrap_or_else(|| base.join("backups")),
        db,
    })
}

/// Name of the lock that lets only one restore run at a time.
pub const RESTORE_LOCK: &str = r"Local\ContextFlow.Restore";

/// Returned when another restore already holds [`RESTORE_LOCK`].
#[derive(Debug)]
pub struct RestoreBusy;

impl std::fmt::Display for RestoreBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "another ContextFlow restore is already running")
    }
}

impl std::error::Error for RestoreBusy {}

pub struct RestoreArgs {
    pub version: Option<u32>,
    pub only: Vec<String>,
    pub dry_run: bool,
    pub timeout: Duration,
    /// Most saved tabs reopened per browser window.
    pub max_tabs_per_window: usize,
    /// Longest the whole restore may take.
    pub overall_timeout: Duration,
    /// Set from another thread to stop the restore early (the desktop app's
    /// Cancel button). Applications already started are left running.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Don't close any programs (the workspaces' cleanup rules are ignored).
    pub skip_cleanup: bool,
}

/// Which side of a workspace a cleanup belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupPhase {
    /// Closing programs when leaving a workspace.
    Leaving,
    /// Closing programs when entering a workspace.
    Entering,
}

/// One cleanup that ran during a restore.
#[derive(Debug, Clone)]
pub struct CleanupReport {
    pub phase: CleanupPhase,
    /// The workspace whose rules were applied.
    pub workspace: String,
    pub results: Vec<TargetResult>,
}

impl CleanupReport {
    /// Results worth telling the user about: a program still running, or one
    /// that could not be closed.
    pub fn problems(&self) -> impl Iterator<Item = &TargetResult> {
        self.results.iter().filter(|r| {
            matches!(
                r.outcome,
                cf_cleanup::Outcome::StillRunning | cf_cleanup::Outcome::Failed
            )
        })
    }
}

/// Everything a restore did.
pub struct RestoreOutcome {
    pub context: Context,
    pub snapshot: Snapshot,
    pub run: RestoreRun,
    /// Cleanups that ran first: leaving the previous workspace, entering this one.
    pub cleanups: Vec<CleanupReport>,
}

/// Application ids a snapshot uses: cleanup never closes these.
pub fn snapshot_app_ids(snapshot: &Snapshot) -> HashSet<String> {
    snapshot
        .payload
        .applications
        .iter()
        .map(|a| a.application_id.clone())
        .collect()
}

fn run_cleanup(
    phase: CleanupPhase,
    workspace: &str,
    targets: &[CleanupTarget],
    keep: &HashSet<String>,
    log: &mut dyn FnMut(&str),
) -> Option<CleanupReport> {
    if targets.is_empty() {
        return None;
    }
    log(match phase {
        CleanupPhase::Leaving => "closing programs from the workspace you're leaving",
        CleanupPhase::Entering => "closing distracting programs",
    });
    let results = cf_cleanup::run(
        &WindowsProcesses,
        targets,
        keep,
        &cf_cleanup::RunOptions::default(),
    );
    tracing::info!(
        workspace,
        closed = results
            .iter()
            .filter(|r| matches!(
                r.outcome,
                cf_cleanup::Outcome::Closed | cf_cleanup::Outcome::Terminated
            ))
            .count(),
        "workspace cleanup finished"
    );
    Some(CleanupReport {
        phase,
        workspace: workspace.to_string(),
        results,
    })
}

/// How long [`preview_cleanup`] watches programs to estimate their CPU use.
const CPU_SAMPLE: Duration = Duration::from_millis(600);

/// A workspace's rules for one side, and the programs it keeps open.
/// Entering: the programs of the snapshot a restore would use are kept.
/// Leaving: the next workspace isn't known, so nothing is kept.
fn cleanup_inputs(
    db: &Database,
    workspace: &str,
    phase: CleanupPhase,
) -> Result<(Context, Vec<CleanupTarget>, HashSet<String>)> {
    let ctx = db.find_context(workspace)?;
    let config = db.cleanup_config(ctx.id)?;
    let (targets, keep) = match phase {
        CleanupPhase::Entering => {
            let version = db.pinned_version(ctx.id)?;
            let keep = db
                .get_snapshot(ctx.id, version)
                .map(|s| snapshot_app_ids(&s))
                .unwrap_or_default();
            (config.on_enter, keep)
        }
        CleanupPhase::Leaving => (config.on_exit, HashSet::new()),
    };
    Ok((ctx, targets, keep))
}

/// What a workspace's cleanup would do right now, without doing it.
pub fn preview_cleanup(db: &Database, workspace: &str, phase: CleanupPhase) -> Result<CleanupPlan> {
    let (_, targets, keep) = cleanup_inputs(db, workspace, phase)?;
    let apps = cf_cleanup::running_apps(&WindowsProcesses, CPU_SAMPLE);
    Ok(cf_cleanup::plan(&targets, &keep, &apps))
}

/// Run one side of a workspace's cleanup now, on request (not as part of a
/// restore). The active workspace is not changed.
pub fn run_cleanup_now(
    db: &Database,
    workspace: &str,
    phase: CleanupPhase,
    log: &mut dyn FnMut(&str),
) -> Result<CleanupReport> {
    let (ctx, targets, keep) = cleanup_inputs(db, workspace, phase)?;
    Ok(
        run_cleanup(phase, &ctx.name, &targets, &keep, log).unwrap_or(CleanupReport {
            phase,
            workspace: ctx.name,
            results: Vec::new(),
        }),
    )
}

/// Leave the active workspace without entering another: run its "when
/// leaving" cleanup and clear it. `None` when no workspace is active.
pub fn leave_active_workspace(
    db: &Database,
    log: &mut dyn FnMut(&str),
) -> Result<Option<CleanupReport>> {
    let Some(active) = db.active_context()? else {
        return Ok(None);
    };
    let ctx = db.find_context(&active.to_string())?;
    let config = db.cleanup_config(active)?;
    let report = run_cleanup(
        CleanupPhase::Leaving,
        &ctx.name,
        &config.on_exit,
        &HashSet::new(),
        log,
    );
    db.set_active_context(None)?;
    Ok(report)
}

/// Restore a workspace's snapshot onto the real desktop and record the run.
pub fn restore_workspace(
    db: &mut Database,
    workspace: &str,
    args: &RestoreArgs,
    log: &mut dyn FnMut(&str),
) -> Result<RestoreOutcome> {
    let ctx = db.find_context(workspace)?;

    // Imported workspaces can name any program to start, so they must be
    // reviewed and approved first. A dry run only reads, so it is always allowed.
    if !args.dry_run && !db.is_trusted(ctx.id)? {
        anyhow::bail!(
            "'{name}' was imported from a file and has not been approved. Review it with: contextflow workspace trust \"{name}\"",
            name = ctx.name
        );
    }
    // Which snapshot: an explicit --version, else the pinned one, else the latest.
    let version = match args.version {
        Some(v) => Some(v),
        None => db.pinned_version(ctx.id)?,
    };
    let snapshot = db.get_snapshot(ctx.id, version)?;

    // Refuse to act on a snapshot that fails its integrity check.
    let check = db.verify_snapshot(ctx.id, Some(snapshot.version))?;
    if !check.ok() {
        anyhow::bail!(
            "snapshot v{} of '{}' failed its integrity check and was not restored",
            snapshot.version,
            ctx.name
        );
    }

    // Only one real restore at a time: a second double-click would otherwise
    // see "not running" and start every app again. The guard is held until this
    // function returns (and the OS frees it if the process dies).
    let _restore_lock = if args.dry_run {
        None
    } else {
        match cf_windows::try_lock_instance(RESTORE_LOCK)? {
            Some(lock) => Some(lock),
            None => return Err(RestoreBusy.into()),
        }
    };

    // Cleanup comes first: leave the workspace we were in, then close what
    // this one doesn't want. Programs this snapshot restores are never closed,
    // and a partial restore (--only) isn't a workspace switch.
    let mut cleanups = Vec::new();
    let mut left_previous = false;
    if !args.dry_run && !args.skip_cleanup && args.only.is_empty() {
        let keep = snapshot_app_ids(&snapshot);
        let previous: Option<ContextId> = db.active_context()?.filter(|p| *p != ctx.id);
        if let Some(previous) = previous {
            if let Ok(prev_ctx) = db.find_context(&previous.to_string()) {
                let config = db.cleanup_config(previous)?;
                if let Some(report) = run_cleanup(
                    CleanupPhase::Leaving,
                    &prev_ctx.name,
                    &config.on_exit,
                    &keep,
                    log,
                ) {
                    cleanups.push(report);
                }
                left_previous = true;
            }
        }
        let config = db.cleanup_config(ctx.id)?;
        if let Some(report) = run_cleanup(
            CleanupPhase::Entering,
            &ctx.name,
            &config.on_enter,
            &keep,
            log,
        ) {
            cleanups.push(report);
        }
    }

    let started = Utc::now();
    let opts = RestoreOptions {
        timeout: args.timeout,
        only: args.only.clone(),
        dry_run: args.dry_run,
        max_tabs_per_window: args.max_tabs_per_window,
        overall_timeout: args.overall_timeout,
        cancel: args.cancel.clone(),
        ..Default::default()
    };
    let run = cf_engine::restore(&WindowsDesktop, &snapshot.payload, &opts, log);

    tracing::info!(
        workspace = %ctx.name,
        version = snapshot.version,
        overall = run.overall.as_str(),
        dry_run = args.dry_run,
        "restore finished"
    );
    if !args.dry_run {
        let records: Vec<RestoreStepRecord> = run
            .steps
            .iter()
            .map(|s| RestoreStepRecord {
                component_type: "application".into(),
                component_identifier: s.application_id.clone(),
                status: s.status.as_str().into(),
                error_code: s.code.map(String::from),
                error_message: (!s.message.is_empty()).then(|| s.message.clone()),
                duration_ms: s.duration_ms,
            })
            .collect();
        db.record_restore_run(
            snapshot.id,
            started,
            Utc::now(),
            run.overall.as_str(),
            &records,
        )?;
    }
    // Remember where we are, so the next restore knows what to leave. A restore
    // that achieved nothing doesn't enter the workspace.
    if !args.dry_run && args.only.is_empty() {
        if run.overall != cf_core::RestoreStatus::Failed {
            db.set_active_context(Some(ctx.id))?;
        } else if left_previous {
            db.set_active_context(None)?;
        }
    }
    Ok(RestoreOutcome {
        context: ctx,
        snapshot,
        run,
        cleanups,
    })
}

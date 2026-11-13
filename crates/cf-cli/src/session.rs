//! Resolve paths and run a restore: the flow shared by `contextflow restore`
//! and the desktop launcher.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result};
use chrono::Utc;

use cf_config::Config;
use cf_core::{Context, Snapshot};
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

pub struct RestoreArgs {
    pub version: Option<u32>,
    pub only: Vec<String>,
    pub dry_run: bool,
    pub timeout: Duration,
}

/// Restore a workspace's snapshot onto the real desktop and record the run.
pub fn restore_workspace(
    db: &mut Database,
    workspace: &str,
    args: &RestoreArgs,
    log: &mut dyn FnMut(&str),
) -> Result<(Context, Snapshot, RestoreRun)> {
    let ctx = db.find_context(workspace)?;
    let snapshot = db.get_snapshot(ctx.id, args.version)?;

    // Refuse to act on a snapshot that fails its integrity check.
    let check = db.verify_snapshot(ctx.id, Some(snapshot.version))?;
    if !check.ok() {
        anyhow::bail!(
            "snapshot v{} of '{}' failed its integrity check and was not restored",
            snapshot.version,
            ctx.name
        );
    }

    let started = Utc::now();
    let opts = RestoreOptions {
        timeout: args.timeout,
        only: args.only.clone(),
        dry_run: args.dry_run,
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
    Ok((ctx, snapshot, run))
}

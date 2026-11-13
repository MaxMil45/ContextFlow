//! Deleting, pruning, exporting, importing, backing up and verifying.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use chrono::Utc;

use cf_cli::session::Paths;
use cf_config::Config;
use cf_core::Context;
use cf_storage::{Database, ExportFile};

use super::util::{fmt_versions, policy_from, require_yes};

pub(crate) fn delete_snapshot(
    db: &Database,
    workspace: &str,
    version: u32,
    yes: bool,
) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    db.get_snapshot(ctx.id, Some(version))?; // fail early if it does not exist
    require_yes(yes, &format!("snapshot v{version} of '{}'", ctx.name))?;
    db.delete_snapshot(ctx.id, version)?;
    println!("Deleted '{}' v{version}", ctx.name);
    Ok(())
}

pub(crate) fn prune(
    db: &mut Database,
    cfg: &Config,
    workspace: Option<String>,
    all: bool,
    keep_last: Option<u32>,
    max_age_days: Option<u32>,
    yes: bool,
) -> Result<()> {
    if keep_last == Some(0) || max_age_days == Some(0) {
        bail!("--keep-last and --max-age-days must be at least 1");
    }
    let policy = policy_from(cfg, keep_last, max_age_days);
    if policy.keep_last.is_none() && policy.max_age.is_none() {
        bail!(
            "no retention rules: set retention.keep_last / retention.max_age_days in the config, or pass --keep-last / --max-age-days"
        );
    }
    let targets: Vec<Context> = match (workspace, all) {
        (Some(w), _) => vec![db.find_context(&w)?],
        (None, true) => db.list_contexts(true)?,
        (None, false) => bail!("name a workspace, or use --all"),
    };

    let now = Utc::now();
    let mut total = 0;
    for ctx in &targets {
        let candidates = db.retention_candidates(ctx.id, &policy, now)?;
        if candidates.is_empty() {
            continue;
        }
        let versions: Vec<u32> = candidates.iter().map(|s| s.version).collect();
        total += versions.len();
        if yes {
            let deleted = db.prune(ctx.id, &policy, now)?;
            println!("{}: deleted {}", ctx.name, fmt_versions(&deleted));
        } else {
            println!("{}: would delete {}", ctx.name, fmt_versions(&versions));
        }
    }
    if total == 0 {
        println!("Nothing to prune.");
    } else if !yes {
        println!("\n{total} snapshot(s) would be deleted. Re-run with --yes to delete them.");
    }
    Ok(())
}

pub(crate) fn export(
    db: &Database,
    names: Vec<String>,
    all: bool,
    out: &Path,
    force: bool,
) -> Result<()> {
    let contexts: Vec<Context> = if all {
        db.list_contexts(true)?
    } else if names.is_empty() {
        bail!("name one or more workspaces to export, or use --all");
    } else {
        names
            .iter()
            .map(|n| db.find_context(n))
            .collect::<Result<_, _>>()?
    };
    if out.exists() && !force {
        bail!("{} already exists; use --force to overwrite", out.display());
    }
    let workspaces = contexts
        .iter()
        .map(|c| db.export_context(c.id))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshots: usize = workspaces.iter().map(|w| w.snapshots.len()).sum();
    let file = ExportFile::new(workspaces, env!("CARGO_PKG_VERSION"));

    // Write to a temporary file first so a failure never leaves a half-written export.
    let tmp = out.with_extension("partial");
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&tmp, serde_json::to_vec_pretty(&file)?)
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, out).with_context(|| format!("writing {}", out.display()))?;
    tracing::info!(workspaces = contexts.len(), snapshots, "export written");
    println!(
        "Exported {} workspace(s), {snapshots} snapshot(s) to {}",
        contexts.len(),
        out.display()
    );
    println!("Note: the file contains window titles and repository paths. Store it accordingly.");
    Ok(())
}

pub(crate) fn import(db: &mut Database, file: &Path, rename: Option<&str>) -> Result<()> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let export: ExportFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a valid ContextFlow export", file.display()))?;
    export.check_header()?;
    if rename.is_some() && export.workspaces.len() != 1 {
        bail!(
            "--as only works for files with one workspace (this one has {})",
            export.workspaces.len()
        );
    }
    for ws in &export.workspaces {
        let ctx = db
            .import_workspace(ws, rename)
            .with_context(|| format!("importing '{}'", ws.name))?;
        println!(
            "Imported '{}' with {} snapshot(s), all hashes verified",
            ctx.name,
            ws.snapshots.len()
        );
    }
    Ok(())
}

pub(crate) fn backup(db: &Database, paths: &Paths, out: Option<PathBuf>) -> Result<()> {
    let dest = out.unwrap_or_else(|| {
        paths.backups.join(format!(
            "contextflow-{}.db",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ))
    });
    db.backup_to(&dest)?;
    // Prove the backup is usable before reporting success.
    let copy = Database::open(&dest).context("the backup could not be reopened")?;
    let problems = copy.integrity_check()?;
    if !problems.is_empty() {
        bail!(
            "the backup failed its integrity check: {}",
            problems.join("; ")
        );
    }
    println!("Backup written and verified: {}", dest.display());
    println!(
        "To use it, point ContextFlow at it with --db, or copy it over {}",
        paths.db.display()
    );
    Ok(())
}

pub(crate) fn verify(db: &Database, workspace: Option<&str>, version: Option<u32>) -> Result<()> {
    let problems = db.integrity_check()?;
    if !problems.is_empty() {
        bail!("database integrity check failed:\n{}", problems.join("\n"));
    }
    println!("database: ok");
    let contexts = match workspace {
        Some(w) => vec![db.find_context(w)?],
        None => db.list_contexts(true)?,
    };
    let mut failures = 0;
    let mut checked = 0;
    for ctx in contexts {
        let versions: Vec<u32> = match version {
            Some(v) => vec![v],
            None => db
                .list_snapshots(ctx.id)?
                .into_iter()
                .map(|s| s.version)
                .collect(),
        };
        for v in versions {
            checked += 1;
            let r = db.verify_snapshot(ctx.id, Some(v))?;
            if !r.ok() {
                failures += 1;
                println!(
                    "{} v{v}: CORRUPT (stored {}, computed {}, payload readable: {})",
                    ctx.name,
                    &r.stored_hash[..12.min(r.stored_hash.len())],
                    &r.computed_hash[..12],
                    r.payload_parses
                );
            }
        }
    }
    if failures > 0 {
        tracing::error!(failures, "snapshot verification failed");
        bail!("{failures} of {checked} snapshot(s) failed verification");
    }
    println!("{checked} snapshot(s): ok");
    Ok(())
}

pub(crate) fn info(db: &Database, paths: &Paths) -> Result<()> {
    let contexts = db.list_contexts(true)?;
    let snapshots: usize = contexts
        .iter()
        .map(|c| db.list_snapshots(c.id).map(|s| s.len()))
        .sum::<Result<usize, _>>()?;
    let size = std::fs::metadata(&paths.db).map(|m| m.len()).unwrap_or(0);
    println!("ContextFlow {}", env!("CARGO_PKG_VERSION"));
    println!(
        "database  {} ({} KB, schema v{})",
        paths.db.display(),
        size / 1024,
        db.schema_version()?
    );
    println!(
        "config    {}{}",
        paths.config.display(),
        if paths.config.exists() {
            ""
        } else {
            " (not created; using defaults)"
        }
    );
    println!("logs      {}", paths.logs.display());
    println!("backups   {}", paths.backups.display());
    println!(
        "contents  {} workspace(s), {snapshots} snapshot(s)",
        contexts.len()
    );
    Ok(())
}

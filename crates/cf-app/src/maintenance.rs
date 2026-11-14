//! Export, import, backup, integrity checks and retention: the data
//! maintenance shared by `contextflow` and the desktop app. These return what
//! they did; printing is the caller's business.

use std::path::Path;

use anyhow::{bail, Context as _, Result};
use chrono::{TimeDelta, Utc};

use cf_config::Config;
use cf_core::Context;
use cf_storage::{Database, ExportFile, RetentionPolicy};

pub struct ExportSummary {
    pub workspaces: usize,
    pub snapshots: usize,
}

/// Write `contexts` to a portable, hash-verified file. Written to a temporary
/// file first, so a failure never leaves a half-written export.
pub fn export_workspaces(
    db: &Database,
    contexts: &[Context],
    out: &Path,
    overwrite: bool,
    passphrase: Option<&str>,
) -> Result<ExportSummary> {
    if out.exists() && !overwrite {
        bail!("{} already exists; use --force to overwrite", out.display());
    }
    let workspaces = contexts
        .iter()
        .map(|c| db.export_context(c.id))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshots: usize = workspaces.iter().map(|w| w.snapshots.len()).sum();
    let file = ExportFile::new(workspaces, env!("CARGO_PKG_VERSION"));

    let tmp = out.with_extension("partial");
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(&file)?;
    if let Some(p) = passphrase {
        bytes = crate::seal::seal(&bytes, p)?;
    }
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, out).with_context(|| format!("writing {}", out.display()))?;
    tracing::info!(workspaces = contexts.len(), snapshots, "export written");
    Ok(ExportSummary {
        workspaces: contexts.len(),
        snapshots,
    })
}

#[derive(Debug)]
pub struct ImportedWorkspace {
    pub name: String,
    pub snapshots: usize,
}

/// Import every workspace in an export file. They arrive unapproved: they
/// can't be restored until the user reviews and approves them.
pub fn import_file(
    db: &mut Database,
    file: &Path,
    rename: Option<&str>,
    passphrase: Option<&str>,
) -> Result<Vec<ImportedWorkspace>> {
    let mut bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    if crate::seal::is_sealed(&bytes) {
        let Some(p) = passphrase else {
            return Err(crate::seal::PassphraseRequired.into());
        };
        bytes = crate::seal::open(&bytes, p)?;
    }
    let export: ExportFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a valid ContextFlow export", file.display()))?;
    export.check_header()?;
    if rename.is_some() && export.workspaces.len() != 1 {
        bail!(
            "a new name only works for files with one workspace (this one has {})",
            export.workspaces.len()
        );
    }
    let mut out = Vec::new();
    for ws in &export.workspaces {
        let ctx = db
            .import_workspace(ws, rename)
            .with_context(|| format!("importing '{}'", ws.name))?;
        out.push(ImportedWorkspace {
            name: ctx.name,
            snapshots: ws.snapshots.len(),
        });
    }
    Ok(out)
}

/// Copy the whole database to `dest` and prove the copy opens and is healthy.
pub fn backup_verified(db: &Database, dest: &Path) -> Result<()> {
    db.backup_to(dest)?;
    let copy = Database::open(dest).context("the backup could not be reopened")?;
    let problems = copy.integrity_check()?;
    if !problems.is_empty() {
        bail!(
            "the backup failed its integrity check: {}",
            problems.join("; ")
        );
    }
    Ok(())
}

pub struct VerifyFailure {
    pub workspace: String,
    pub version: u32,
}

pub struct VerifyReport {
    /// Problems SQLite itself found; empty when the database is healthy.
    pub database_problems: Vec<String>,
    pub snapshots_checked: usize,
    /// Snapshots whose stored hash no longer matches their contents. They are
    /// reported, never repaired.
    pub failures: Vec<VerifyFailure>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.database_problems.is_empty() && self.failures.is_empty()
    }
}

/// Check the database and re-hash every snapshot of every workspace.
pub fn verify_all(db: &Database) -> Result<VerifyReport> {
    let database_problems = db.integrity_check()?;
    let mut checked = 0;
    let mut failures = Vec::new();
    for ctx in db.list_contexts(true)? {
        for s in db.list_snapshots(ctx.id)? {
            checked += 1;
            let r = db.verify_snapshot(ctx.id, Some(s.version))?;
            if !r.ok() {
                failures.push(VerifyFailure {
                    workspace: ctx.name.clone(),
                    version: s.version,
                });
            }
        }
    }
    Ok(VerifyReport {
        database_problems,
        snapshots_checked: checked,
        failures,
    })
}

/// The retention rules from the configuration, if any are set.
pub fn retention_policy(cfg: &Config) -> Option<RetentionPolicy> {
    cfg.retention.is_configured().then(|| RetentionPolicy {
        keep_last: cfg.retention.keep_last,
        max_age: cfg
            .retention
            .max_age_days
            .map(|d| TimeDelta::days(i64::from(d))),
    })
}

pub struct PruneItem {
    pub workspace: String,
    pub versions: Vec<u32>,
}

/// Snapshots that `policy` would delete (`apply = false`), or delete now
/// (`apply = true`), across every workspace. The latest snapshot of a workspace
/// and any pinned one are never touched.
pub fn prune_all(
    db: &mut Database,
    policy: &RetentionPolicy,
    apply: bool,
) -> Result<Vec<PruneItem>> {
    let now = Utc::now();
    let mut out = Vec::new();
    for ctx in db.list_contexts(true)? {
        let versions: Vec<u32> = if apply {
            db.prune(ctx.id, policy, now)?
        } else {
            db.retention_candidates(ctx.id, policy, now)?
                .iter()
                .map(|s| s.version)
                .collect()
        };
        if !versions.is_empty() {
            out.push(PruneItem {
                workspace: ctx.name,
                versions,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_core::{CaptureStatus, SnapshotPayload};

    fn db_with(name: &str, snapshots: u32) -> (Database, Context) {
        let mut db = Database::open_in_memory().unwrap();
        let ctx = db.create_context(name, None).unwrap();
        for n in 1..=snapshots {
            let mut p = SnapshotPayload::default();
            p.notes.push(format!("note {n}"));
            db.create_snapshot(ctx.id, p, CaptureStatus::Complete)
                .unwrap();
        }
        (db, ctx)
    }

    #[test]
    fn export_then_import_round_trips_and_never_overwrites_silently() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("w.json");
        let (db, ctx) = db_with("Work", 3);

        let summary =
            export_workspaces(&db, std::slice::from_ref(&ctx), &file, false, None).unwrap();
        assert_eq!((summary.workspaces, summary.snapshots), (1, 3));
        // A second export to the same file needs an explicit overwrite.
        assert!(export_workspaces(&db, std::slice::from_ref(&ctx), &file, false, None).is_err());

        let mut other = Database::open_in_memory().unwrap();
        let imported = import_file(&mut other, &file, Some("Copy"), None).unwrap();
        assert_eq!(imported[0].name, "Copy");
        assert_eq!(imported[0].snapshots, 3);
        let copy = other.find_context("Copy").unwrap();
        // Imported workspaces must be approved before they can be restored.
        assert!(!other.is_trusted(copy.id).unwrap());
        assert_eq!(other.list_snapshots(copy.id).unwrap().len(), 3);
    }

    #[test]
    fn a_protected_export_needs_its_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("w.json");
        let (db, ctx) = db_with("Work", 2);
        export_workspaces(
            &db,
            std::slice::from_ref(&ctx),
            &file,
            false,
            Some("long passphrase"),
        )
        .unwrap();
        let bytes = std::fs::read(&file).unwrap();
        assert!(crate::seal::is_sealed(&bytes));
        assert!(
            !String::from_utf8_lossy(&bytes).contains("note 1"),
            "notes aren't readable in the file"
        );

        let mut other = Database::open_in_memory().unwrap();
        let err = import_file(&mut other, &file, None, None).unwrap_err();
        assert!(err.is::<crate::seal::PassphraseRequired>());
        assert!(import_file(&mut other, &file, None, Some("wrong passphrase")).is_err());
        let imported = import_file(&mut other, &file, None, Some("long passphrase")).unwrap();
        assert_eq!(imported[0].snapshots, 2);
    }

    #[test]
    fn a_file_that_is_not_an_export_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.json");
        std::fs::write(&file, b"{\"hello\": 1}").unwrap();
        let mut db = Database::open_in_memory().unwrap();
        assert!(import_file(&mut db, &file, None, None).is_err());
    }

    #[test]
    fn a_backup_is_a_usable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("live.db");
        let dest = dir.path().join("backup.db");
        let mut db = Database::open(&path).unwrap();
        let ctx = db.create_context("Work", None).unwrap();
        db.create_snapshot(ctx.id, SnapshotPayload::default(), CaptureStatus::Complete)
            .unwrap();

        backup_verified(&db, &dest).unwrap();
        let copy = Database::open(&dest).unwrap();
        assert_eq!(
            copy.list_snapshots(copy.find_context("Work").unwrap().id)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn verify_reports_a_clean_database() {
        let (db, _) = db_with("Work", 2);
        let r = verify_all(&db).unwrap();
        assert!(r.ok());
        assert_eq!(r.snapshots_checked, 2);
    }

    #[test]
    fn retention_previews_without_deleting_and_then_applies() {
        let (mut db, ctx) = db_with("Work", 5);
        let mut cfg = Config::default();
        assert!(retention_policy(&cfg).is_none(), "no rules, no policy");
        cfg.retention.keep_last = Some(2);
        let policy = retention_policy(&cfg).unwrap();

        let preview = prune_all(&mut db, &policy, false).unwrap();
        assert_eq!(preview[0].workspace, "Work");
        assert_eq!(preview[0].versions.len(), 3);
        assert_eq!(
            db.list_snapshots(ctx.id).unwrap().len(),
            5,
            "a preview deletes nothing"
        );

        let done = prune_all(&mut db, &policy, true).unwrap();
        assert_eq!(done[0].versions.len(), 3);
        let left: Vec<u32> = db
            .list_snapshots(ctx.id)
            .unwrap()
            .iter()
            .map(|s| s.version)
            .collect();
        assert_eq!(left, vec![5, 4], "the newest are kept");
    }
}

//! Small helpers shared by several commands.

use anyhow::{bail, Result};
use cf_config::Config;
use cf_core::{Context, Snapshot};
use cf_storage::{Database, RetentionPolicy, StorageError};
use chrono::TimeDelta;

pub(crate) fn require_yes(yes: bool, what: &str) -> Result<()> {
    if yes {
        return Ok(());
    }
    println!("This will permanently delete {what}. It cannot be undone.");
    bail!("nothing deleted; re-run with --yes to confirm")
}

pub(crate) fn latest(db: &Database, ctx: &Context) -> Result<Option<Snapshot>> {
    match db.get_snapshot(ctx.id, None) {
        Ok(s) => Ok(Some(s)),
        Err(StorageError::NotFound(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn policy_from(
    cfg: &Config,
    keep_last: Option<u32>,
    max_age_days: Option<u32>,
) -> RetentionPolicy {
    RetentionPolicy {
        keep_last: keep_last.or(cfg.retention.keep_last),
        max_age: max_age_days
            .or(cfg.retention.max_age_days)
            .map(|d| TimeDelta::days(d as i64)),
    }
}

/// `Code.exe` / `code` / ` CODE ` all mean the application id `code`.
pub(crate) fn normalize_app_names(names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .map(|n| {
            let n = n.trim().to_lowercase();
            n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
        })
        .filter(|n| !n.is_empty())
        .collect()
}

pub(crate) fn fmt_versions(vs: &[u32]) -> String {
    vs.iter()
        .map(|v| format!("v{v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Progress lines from work the ContextFlow app did, shown as they would have been live.
pub(crate) fn print_log(lines: &[String]) {
    for line in lines {
        println!("  {line}...");
    }
}

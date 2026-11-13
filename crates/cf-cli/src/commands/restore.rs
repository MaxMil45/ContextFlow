//! `contextflow restore` and `contextflow shortcut`

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};

use cf_cli::session::{self, Paths, RestoreArgs};
use cf_cli::shortcut;
use cf_storage::Database;

use super::util::{latest, normalize_app_names};

#[allow(clippy::too_many_arguments)]
pub(crate) fn restore_cmd(
    db: &mut Database,
    workspace: &str,
    version: Option<u32>,
    only: Vec<String>,
    dry_run: bool,
    timeout: u64,
    max_tabs_per_window: usize,
    overall_timeout: Duration,
) -> Result<()> {
    use cf_core::RestoreStatus as S;
    let args = RestoreArgs {
        version,
        only: normalize_app_names(only),
        dry_run,
        timeout: Duration::from_secs(timeout.clamp(1, 600)),
        max_tabs_per_window,
        overall_timeout,
    };
    let (ctx, snap, run) =
        session::restore_workspace(db, workspace, &args, &mut |m| println!("  {m}..."))?;

    println!(
        "{}'{}' v{}",
        if dry_run { "Dry run for " } else { "Restored " },
        ctx.name,
        snap.version
    );
    if let Some(note) = snap.payload.notes.last() {
        println!("Your note: {note}");
    }
    for s in &run.steps {
        let mark = match s.status {
            S::Success => "ok     ",
            S::PartialSuccess => "partial",
            S::Failed => "FAILED ",
            S::Skipped => "skipped",
            S::RequiresUserIntervention => "ACTION ",
        };
        println!("  {mark} {:<22} {}", s.application_id, s.message);
    }
    if run.steps.is_empty() {
        println!("  nothing to restore (no applications in this snapshot match)");
    }
    println!("Result: {}", run.overall.as_str().replace('_', " "));
    if run.overall == S::Failed {
        bail!("nothing could be restored");
    }
    Ok(())
}

pub(crate) fn shortcut_cmd(
    db: &Database,
    paths: &Paths,
    db_flag: &Option<PathBuf>,
    workspace: &str,
    name: Option<String>,
    folder: Option<PathBuf>,
    force: bool,
) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    if latest(db, &ctx)?.is_none() {
        bail!(
            "'{}' has no snapshot yet; run `contextflow capture \"{}\"` first",
            ctx.name,
            ctx.name
        );
    }
    // Remember a non-default database/config so the icon opens the same data.
    let db_arg = db_flag.as_deref().map(|_| paths.db.as_path());
    let config_arg = std::env::var_os("CONTEXTFLOW_CONFIG")
        .map(PathBuf::from)
        .filter(|p| p.exists());
    let id = ctx.id.to_string();
    let path = shortcut::create(&shortcut::ShortcutSpec {
        workspace_id: &id,
        display_name: name.as_deref().unwrap_or(&ctx.name),
        folder: folder.as_deref(),
        db: db_arg,
        config: config_arg.as_deref(),
        overwrite: force,
    })?;
    println!("Created {}", path.display());
    println!("Double-click it to reopen '{}'.", ctx.name);
    Ok(())
}

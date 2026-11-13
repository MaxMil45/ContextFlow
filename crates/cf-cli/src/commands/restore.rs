//! `contextflow restore` and `contextflow shortcut`

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context as _, Result};

use cf_cli::session::{self, Paths, RestoreArgs};
use cf_cli::{icons, shortcut};
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
        cancel: None,
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

pub(crate) struct ShortcutOpts {
    pub name: Option<String>,
    pub folder: Option<PathBuf>,
    pub force: bool,
    pub icon: Option<PathBuf>,
    pub default_icon: bool,
}

pub(crate) fn shortcut_cmd(
    db: &Database,
    paths: &Paths,
    db_flag: &Option<PathBuf>,
    workspace: &str,
    opts: ShortcutOpts,
) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    if latest(db, &ctx)?.is_none() {
        bail!(
            "'{}' has no snapshot yet; run `contextflow capture \"{}\"` first",
            ctx.name,
            ctx.name
        );
    }
    let id = ctx.id.to_string();
    let icon_dir = icons::icon_dir(&paths.db);
    let icon_changed = opts.icon.is_some() || opts.default_icon;
    if let Some(picture) = &opts.icon {
        let bytes =
            std::fs::read(picture).with_context(|| format!("reading {}", picture.display()))?;
        icons::set_custom_icon(&icon_dir, &id, &bytes)?;
    } else if opts.default_icon {
        icons::clear_custom_icon(&icon_dir, &id);
    }
    let icon = icons::custom_icon(&icon_dir, &id);

    // Remember a non-default database/config so the icon opens the same data.
    let db_arg = db_flag.as_deref().map(|_| paths.db.as_path());
    let config_arg = std::env::var_os("CONTEXTFLOW_CONFIG")
        .map(PathBuf::from)
        .filter(|p| p.exists());
    let display_name = opts.name.as_deref().unwrap_or(&ctx.name);
    let created = shortcut::create(&shortcut::ShortcutSpec {
        workspace_id: &id,
        display_name,
        folder: opts.folder.as_deref(),
        db: db_arg,
        config: config_arg.as_deref(),
        overwrite: opts.force,
        icon: icon.as_deref(),
    });
    match created {
        Ok(path) => {
            println!("Created {}", path.display());
            println!("Double-click it to reopen '{}'.", ctx.name);
            Ok(())
        }
        // Only the icon was asked for: change it on the shortcut that's there.
        Err(e) if icon_changed && e.downcast_ref::<shortcut::ShortcutExists>().is_some() => {
            let path =
                shortcut::update_icon(display_name, opts.folder.as_deref(), icon.as_deref())?
                    .context("the shortcut disappeared while its icon was being changed")?;
            println!("Changed the icon of {}", path.display());
            Ok(())
        }
        Err(e) => Err(e),
    }
}

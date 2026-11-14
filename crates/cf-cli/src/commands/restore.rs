//! `contextflow restore` and `contextflow shortcut`

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context as _, Result};

use cf_cli::session::{self, Paths, RestoreArgs, RestoreBusy};
use cf_cli::{icons, shortcut};
use cf_ipc::{RequestBody, ResponseBody, RestoreRequest, RestoreSummary};
use cf_storage::Database;

use super::util::{latest, normalize_app_names};

/// What `contextflow restore` was asked to do.
pub(crate) struct RestoreOpts {
    pub version: Option<u32>,
    pub only: Vec<String>,
    pub dry_run: bool,
    pub timeout: u64,
    pub no_cleanup: bool,
    pub no_save: bool,
}

/// Restore a workspace. A real restore goes to the running agent when there
/// is one (`pipe`), and otherwise runs here; a dry run only reads, so it
/// always runs here.
pub(crate) fn restore_cmd(
    db: &mut Database,
    paths: &Paths,
    pipe: Option<&str>,
    workspace: &str,
    opts: RestoreOpts,
    cfg: &cf_config::Config,
) -> Result<()> {
    let only = normalize_app_names(opts.only);
    if opts.dry_run {
        let args = RestoreArgs {
            version: opts.version,
            only,
            dry_run: true,
            timeout: Duration::from_secs(opts.timeout.clamp(1, 600)),
            max_tabs_per_window: cfg.browser.max_tabs_per_window,
            overall_timeout: cfg.timeouts.restore_overall(),
            cancel: None,
            skip_cleanup: opts.no_cleanup,
            always_on: cfg.capture.normalized_always_on(),
            leave_note: None,
            match_display: cfg.app.match_display_setup,
            save_before_leaving: None,
        };
        let outcome =
            session::restore_workspace(db, workspace, &args, &mut |m| println!("  {m}..."))?;
        let summary = cf_agent::summarize(&outcome);
        println!("Dry run for '{}' v{}", summary.workspace, summary.version);
        if !opts.no_cleanup {
            super::cleanup::print_restore_previews(
                db,
                &outcome.context,
                &cfg.capture.normalized_always_on(),
            );
        }
        return print_summary(&summary);
    }

    let request = RequestBody::Restore(RestoreRequest {
        workspace: workspace.to_owned(),
        version: opts.version,
        only,
        timeout_secs: Some(opts.timeout),
        no_cleanup: opts.no_cleanup,
        no_save: opts.no_save,
    });
    let (reply, _via) = cf_agent::call_or_fallback(pipe, request, |request| {
        Ok(cf_agent::handle(db, &paths.config, request, &mut |m| {
            println!("  {m}...")
        }))
    })?;
    match reply {
        ResponseBody::Restored(summary) => {
            super::util::print_log(&summary.log);
            println!("Restored '{}' v{}", summary.workspace, summary.version);
            print_summary(&summary)
        }
        ResponseBody::Busy => Err(RestoreBusy.into()),
        ResponseBody::Error(e) => bail!("{}", e.message),
        other => bail!("unexpected reply from the agent: {other:?}"),
    }
}

/// Everything after the headline, the same wherever the restore ran.
fn print_summary(summary: &RestoreSummary) -> Result<()> {
    if let Some(left) = &summary.left {
        match (left.version, &left.not_saved) {
            (Some(v), _) => println!("Saved '{}' as v{v} before switching.", left.workspace),
            (None, Some(why)) => println!("Left '{}': {why}", left.workspace),
            (None, None) => {}
        }
    }
    for cleanup in &summary.cleanups {
        super::cleanup::print_cleanup(cleanup);
    }
    if let Some(note) = &summary.note {
        println!("Your note: {note}");
    }
    for s in &summary.steps {
        let mark = match s.status.as_str() {
            "success" => "ok     ",
            "partial_success" => "partial",
            "failed" => "FAILED ",
            "skipped" => "skipped",
            _ => "ACTION ",
        };
        println!("  {mark} {:<22} {}", s.application, s.message);
    }
    if summary.steps.is_empty() {
        println!("  nothing to restore (no applications in this snapshot match)");
    }
    println!("Result: {}", summary.overall.replace('_', " "));
    if summary.overall == "failed" {
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

    // Changing only the icon: update the workspace's existing icon(s), wherever
    // they are and whatever they are called now, instead of making another one.
    if icon_changed && !opts.force && opts.name.is_none() {
        let existing = shortcut::update_icon(&id, opts.folder.as_deref(), icon.as_deref())?;
        if !existing.is_empty() {
            for path in existing {
                println!("Changed the icon of {}", path.display());
            }
            return Ok(());
        }
    }

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
        // Only the icon was asked for: change it on the shortcut(s) that are there.
        Err(e) if icon_changed && e.downcast_ref::<shortcut::ShortcutExists>().is_some() => {
            let changed = shortcut::update_icon(&id, opts.folder.as_deref(), icon.as_deref())?;
            if changed.is_empty() {
                bail!("no shortcut for '{}' was found to change", ctx.name);
            }
            for path in changed {
                println!("Changed the icon of {}", path.display());
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

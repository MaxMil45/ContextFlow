//! `contextflow workspace ...`

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use cf_cli::session::Paths;
use cf_cli::{icons, shortcut};
use cf_storage::Database;

use super::util::{fmt_versions, require_yes};
use crate::WorkspaceCmd;

pub(crate) fn workspace(db: &Database, paths: &Paths, cmd: WorkspaceCmd) -> Result<()> {
    match cmd {
        WorkspaceCmd::Create { name, description } => {
            let ctx = db.create_context(&name, description.as_deref())?;
            tracing::info!(workspace = %ctx.name, "workspace created");
            println!("Created workspace '{}' ({})", ctx.name, ctx.id);
        }
        WorkspaceCmd::List { all } => {
            let contexts = db.list_contexts(all)?;
            if contexts.is_empty() {
                println!("No workspaces yet. Create one with: contextflow workspace create <name>");
            }
            for c in contexts {
                let snaps = db.list_snapshots(c.id)?;
                println!(
                    "{:<32} {:<6} {:>3} snapshot(s)  {}{}{}",
                    c.name,
                    snaps
                        .first()
                        .map(|s| format!("v{}", s.version))
                        .unwrap_or_else(|| "-".into()),
                    snaps.len(),
                    c.updated_at.format("%Y-%m-%d %H:%M"),
                    if c.is_archived { "  [archived]" } else { "" },
                    if db.is_trusted(c.id)? {
                        ""
                    } else {
                        "  [needs approval]"
                    }
                );
            }
        }
        WorkspaceCmd::Rename { name, new_name } => {
            let ctx = db.find_context(&name)?;
            let renamed = db.rename_context(ctx.id, &new_name)?;
            println!("Renamed '{}' to '{}'", ctx.name, renamed.name);
            println!(
                "An existing desktop icon keeps working; re-create it to give it the new name."
            );
        }
        WorkspaceCmd::Archive { name } => {
            let ctx = db.find_context(&name)?;
            db.set_archived(ctx.id, true)?;
            println!("Archived '{}'", ctx.name);
        }
        WorkspaceCmd::Unarchive { name } => {
            let ctx = db.find_context(&name)?;
            db.set_archived(ctx.id, false)?;
            println!("Unarchived '{}'", ctx.name);
        }
        WorkspaceCmd::Delete {
            name,
            yes,
            keep_shortcuts,
        } => {
            let ctx = db.find_context(&name)?;
            let n = db.list_snapshots(ctx.id)?.len();
            require_yes(
                yes,
                &format!("workspace '{}' and its {n} snapshot(s)", ctx.name),
            )?;
            let id = ctx.id.to_string();
            // Look before deleting; a failed lookup must not block the delete.
            let shortcuts = if keep_shortcuts {
                Vec::new()
            } else {
                shortcut::find_for_workspace(&id, None).unwrap_or_else(|e| {
                    eprintln!("warning: could not look for desktop icons: {e:#}");
                    Vec::new()
                })
            };
            db.delete_context(ctx.id)?;
            icons::clear_custom_icon(&icons::icon_dir(&paths.db), &id);
            println!("Deleted workspace '{}' and {n} snapshot(s)", ctx.name);
            match shortcut::recycle(&shortcuts) {
                Ok(()) => {
                    for s in &shortcuts {
                        println!("Moved the desktop icon to the Recycle Bin: {}", s.display());
                    }
                }
                Err(e) => eprintln!(
                    "warning: the workspace is deleted, but its icon was not removed: {e:#}"
                ),
            }
        }
        WorkspaceCmd::Trust { name, yes } => trust(db, &name, yes)?,
        WorkspaceCmd::SaveMode { name, mode } => {
            let ctx = db.find_context(&name)?;
            if let Some(mode) = mode {
                db.set_save_mode(ctx.id, cf_storage::SaveMode::parse(&mode))?;
            }
            let explain = match db.save_mode(ctx.id)? {
                cf_storage::SaveMode::Auto => "saves its changes when you switch away",
                cf_storage::SaveMode::Notify => {
                    "saves its changes when you switch away, and says so"
                }
                cf_storage::SaveMode::Fixed => {
                    "is never saved automatically: it always restores as captured"
                }
            };
            println!("'{}' {explain}.", ctx.name);
        }
    }
    Ok(())
}

/// Show every program a workspace could start, then (with `--yes`) approve it.
///
/// Approval covers the whole workspace, and `restore --version N` can pick any
/// snapshot, so the review lists the programs of **all** versions, not just
/// the latest.
fn trust(db: &Database, name: &str, yes: bool) -> Result<()> {
    let ctx = db.find_context(name)?;
    if db.is_trusted(ctx.id)? {
        println!("'{}' is already approved.", ctx.name);
        return Ok(());
    }

    // (application id, executable, arguments) -> versions it appears in
    let mut programs: BTreeMap<(String, String, String), Vec<u32>> = BTreeMap::new();
    for s in db.list_snapshots(ctx.id)? {
        let snapshot = db.get_snapshot(ctx.id, Some(s.version))?;
        for a in &snapshot.payload.applications {
            programs
                .entry((
                    a.application_id.clone(),
                    a.executable_path.clone(),
                    a.launch_arguments.join(" "),
                ))
                .or_default()
                .push(s.version);
        }
    }

    println!("'{}' would be allowed to start these programs:", ctx.name);
    if programs.is_empty() {
        println!("  (none)");
    }
    for ((id, exe, args), versions) in &programs {
        println!("  {id:<20} {exe}");
        if !args.is_empty() {
            println!("  {:<20} arguments: {args}", "");
        }
        println!("  {:<20} in {}", "", fmt_versions(versions));
    }

    if !yes {
        println!("Nothing approved. Re-run with --yes if you trust every program listed.");
        bail!("nothing approved");
    }
    db.set_trusted(ctx.id, true)?;
    println!("Approved '{}'.", ctx.name);
    Ok(())
}

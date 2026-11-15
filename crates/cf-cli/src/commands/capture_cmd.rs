//! `contextflow capture`

use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;

use cf_cli::capture;
use cf_config::Config;
use cf_storage::Database;

use super::util::{fmt_versions, latest, normalize_app_names, policy_from};

#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_cmd(
    db: &mut Database,
    cfg: &Config,
    workspace: &str,
    only: Vec<String>,
    mut repos: Vec<PathBuf>,
    note: Option<String>,
    no_windows: bool,
    unminimize: bool,
    dry_run: bool,
) -> Result<()> {
    let only = normalize_app_names(only);
    let ctx = db.find_context(workspace)?;
    let previous = latest(db, &ctx)?;
    if repos.is_empty() {
        if let Some(prev) = &previous {
            repos = prev
                .payload
                .git
                .iter()
                .map(|g| PathBuf::from(&g.repository_path))
                .collect();
        }
    }

    let result = capture::capture(capture::CaptureOptions {
        only: &only,
        repos: &repos,
        notes: note.into_iter().collect(),
        include_windows: cfg.capture.include_windows && !no_windows,
        unminimize,
        rules: &cfg.capture,
        git_timeout: cfg.timeouts.git(),
        browser: &cfg.browser,
        browser_state_dir: cf_browser::default_state_dir(),
        detect: cf_detect::Sources::installed(),
    });

    let p = &result.payload;
    println!(
        "Discovered {} application(s), {} window(s), {} monitor(s), {} repositor{}",
        p.applications.len(),
        p.windows.len(),
        p.monitors.len(),
        p.git.len(),
        if p.git.len() == 1 { "y" } else { "ies" }
    );
    for a in &p.applications {
        let n = p
            .windows
            .iter()
            .filter(|w| w.application_id == a.application_id)
            .count();
        let tabs = cf_browser::BrowserAppState::from_value(&a.adapter_state)
            .map(|s| format!(", {} tab(s) saved", s.tab_count()))
            .unwrap_or_default();
        let folders = cf_detect::saved(a.adapter_id.as_deref(), &a.adapter_state)
            .map(|(_, s)| format!(", {} folder(s) saved", s.path_count()))
            .unwrap_or_default();
        println!("  {:<24} {} window(s){tabs}{folders}", a.application_id, n);
    }
    for g in &p.git {
        println!(
            "  git {} [{}] {} staged, {} modified, {} untracked",
            g.repository_path,
            g.branch.as_deref().unwrap_or("detached"),
            g.staged,
            g.modified,
            g.untracked
        );
    }
    if result.excluded_windows > 0 {
        println!(
            "  ({} window(s) skipped by your exclusion rules)",
            result.excluded_windows
        );
    }
    for w in &result.warnings {
        eprintln!("  warning: {w}");
    }
    if result.minimized_windows > 0 {
        eprintln!(
            "  warning: {} window(s) are minimized and will reopen minimized. Use --unminimize to save them as normal windows.",
            result.minimized_windows
        );
    }

    if dry_run {
        println!("Dry run: nothing saved.");
        return Ok(());
    }

    let snap = db.create_snapshot(ctx.id, result.payload, result.status)?;
    let identical = previous.as_ref().is_some_and(|p| p.hash == snap.hash);
    tracing::info!(
        workspace = %ctx.name,
        version = snap.version,
        hash = %&snap.hash[..12],
        status = snap.capture_status.as_str(),
        "snapshot saved"
    );
    println!(
        "Saved '{}' v{} ({}, {}){}",
        ctx.name,
        snap.version,
        snap.capture_status.as_str(),
        &snap.hash[..12],
        if identical {
            " (identical to the previous version)"
        } else {
            ""
        }
    );

    if let Some(pinned) = db.pinned_version(ctx.id)? {
        if pinned != snap.version {
            println!(
                "Note: restore and the desktop icon still use v{pinned} (pinned). To switch: contextflow pin \"{}\"",
                ctx.name
            );
        }
    }

    if cfg.retention.auto_prune {
        let pruned = db.prune(ctx.id, &policy_from(cfg, None, None), Utc::now())?;
        if !pruned.is_empty() {
            println!(
                "Auto-pruned {} old snapshot(s): {}",
                pruned.len(),
                fmt_versions(&pruned)
            );
        }
    }
    Ok(())
}

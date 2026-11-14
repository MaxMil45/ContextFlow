//! `snapshots`, `show` and `diff`: reading a workspace's history.

use std::path::PathBuf;

use anyhow::{bail, Result};

use cf_cli::capture;
use cf_config::Config;
use cf_storage::Database;

use super::util::latest;

pub(crate) fn snapshots(db: &Database, workspace: &str) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    let list = db.list_snapshots(ctx.id)?;
    if list.is_empty() {
        println!("'{}' has no snapshots yet.", ctx.name);
    }
    let pinned = db.pinned_version(ctx.id)?;
    for s in list {
        println!(
            "v{:<5} {}  {:<9} {}{}",
            s.version,
            s.created_at.format("%Y-%m-%d %H:%M:%S"),
            s.capture_status.as_str(),
            &s.hash[..12],
            if pinned == Some(s.version) {
                "  (pinned)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

pub(crate) fn pin(db: &Database, workspace: &str, version: Option<u32>) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    let version = match version {
        Some(v) => v,
        None => latest(db, &ctx)?
            .map(|s| s.version)
            .ok_or_else(|| anyhow::anyhow!("'{}' has no snapshots to pin yet", ctx.name))?,
    };
    db.set_pinned_version(ctx.id, Some(version))?;
    println!("'{}' is pinned to v{version}.", ctx.name);
    Ok(())
}

pub(crate) fn unpin(db: &Database, workspace: &str) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    db.set_pinned_version(ctx.id, None)?;
    println!("'{}' restores the latest snapshot again.", ctx.name);
    Ok(())
}

pub(crate) fn show(db: &Database, workspace: &str, version: Option<u32>, json: bool) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    let s = db.get_snapshot(ctx.id, version)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&s)?);
        return Ok(());
    }
    let p = &s.payload;
    println!(
        "{} v{}: {} ({})",
        ctx.name,
        s.version,
        s.created_at.format("%Y-%m-%d %H:%M:%S"),
        s.capture_status.as_str()
    );
    println!("hash {}", s.hash);
    if !p.notes.is_empty() {
        println!("\nNotes:");
        for n in &p.notes {
            println!("  {n}");
        }
    }
    println!("\nApplications:");
    for a in &p.applications {
        println!("  {:<24} {}", a.application_id, a.executable_path);
    }
    for a in &p.applications {
        if let Some(state) = cf_vscode::VsCodeState::from_value(&a.adapter_state) {
            println!("\nFolders in {}:", a.application_id);
            for w in &state.windows {
                if let Some(uri) = &w.folder_uri {
                    println!("  {uri}");
                }
            }
        }
        if let Some(state) = cf_browser::BrowserAppState::from_value(&a.adapter_state) {
            println!("\nTabs in {}:", a.application_id);
            for (n, w) in state.windows.iter().enumerate() {
                println!("  window {} ({} tab(s))", n + 1, w.tabs.len());
                for (i, t) in w.tabs.iter().enumerate() {
                    println!(
                        "    {}{} {}",
                        if w.active_tab == Some(i) { "*" } else { " " },
                        if t.pinned { "[pinned]" } else { "" },
                        t.url
                    );
                }
            }
        }
    }
    println!("\nWindows:");
    for w in &p.windows {
        println!(
            "  [{}] {} @ {},{} {}x{} {} {}",
            w.application_id,
            w.title,
            w.rect.x,
            w.rect.y,
            w.rect.width,
            w.rect.height,
            w.display_state.as_str(),
            w.monitor_id.as_deref().unwrap_or("")
        );
    }
    if !p.monitors.is_empty() {
        println!("\nMonitors:");
        for m in &p.monitors {
            println!(
                "  {} {}x{} at {},{}{}{}",
                m.id,
                m.rect.width,
                m.rect.height,
                m.rect.x,
                m.rect.y,
                m.dpi.map(|d| format!(", {d} dpi")).unwrap_or_default(),
                if m.primary { ", primary" } else { "" }
            );
        }
    }
    if !p.git.is_empty() {
        println!("\nRepositories:");
        for g in &p.git {
            println!(
                "  {} [{}] {}: {} staged, {} modified, {} untracked",
                g.repository_path,
                g.branch.as_deref().unwrap_or("detached"),
                g.head_commit
                    .as_deref()
                    .map(|h| &h[..10])
                    .unwrap_or("(no commits)"),
                g.staged,
                g.modified,
                g.untracked
            );
        }
    }
    Ok(())
}

pub(crate) fn diff(
    db: &Database,
    cfg: &Config,
    workspace: &str,
    from: u32,
    to: Option<u32>,
    live: bool,
    json: bool,
) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    let base = db.get_snapshot(ctx.id, Some(from))?;
    let (label, target) = if live {
        let repos: Vec<PathBuf> = base
            .payload
            .git
            .iter()
            .map(|g| PathBuf::from(&g.repository_path))
            .collect();
        let r = capture::capture(capture::CaptureOptions {
            only: &[],
            repos: &repos,
            notes: latest(db, &ctx)?
                .map(|s| s.payload.notes)
                .unwrap_or_default(),
            include_windows: cfg.capture.include_windows,
            unminimize: false,
            rules: &cfg.capture,
            git_timeout: cfg.timeouts.git(),
            browser: &cfg.browser,
            browser_state_dir: cf_browser::default_state_dir(),
            vscode_storage: None,
            visual_studio_settings: Vec::new(),
        });
        ("live environment".to_string(), r.payload)
    } else {
        let s = db.get_snapshot(ctx.id, to)?;
        if s.version == from {
            bail!("--from and --to refer to the same snapshot (v{from})");
        }
        (format!("v{}", s.version), s.payload)
    };

    let report = cf_diff::diff(&base.payload, &target);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("{}: v{} -> {}", ctx.name, from, label);
    if report.is_empty() {
        println!("No differences.");
        return Ok(());
    }
    for line in report.summary_lines() {
        println!("  {line}");
    }
    if !report.notices.is_empty() {
        println!("\nBefore restoring:");
        for n in &report.notices {
            println!("  • {n}");
        }
    }
    Ok(())
}

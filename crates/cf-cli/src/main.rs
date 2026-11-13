//! `contextflow`: command-line front end to the ContextFlow engine.
//!
//! The CLI is a thin layer: all behaviour lives in the library crates so the
//! Tauri desktop app (Phase 5) can reuse it unchanged.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use chrono::{TimeDelta, Utc};
use clap::{Parser, Subcommand};

use cf_cli::session::{self, Paths, RestoreArgs};
use cf_cli::{capture, logging, shortcut};
use cf_config::Config;
use cf_core::{redact, Context, Snapshot};
use cf_storage::{Database, ExportFile, RetentionPolicy, StorageError};

#[derive(Parser)]
#[command(
    name = "contextflow",
    version,
    about = "Capture your environment. Preserve your progress. Resume exactly where you left off."
)]
struct Cli {
    /// Database file. Overrides `storage.database_path` in the config.
    #[arg(long, global = true, env = "CONTEXTFLOW_DB")]
    db: Option<PathBuf>,

    /// Config file (default: config.toml in the per-user config directory).
    #[arg(long, global = true, env = "CONTEXTFLOW_CONFIG")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create, list, archive and delete workspaces.
    #[command(subcommand)]
    Workspace(WorkspaceCmd),

    /// Save the applications and windows that are open right now.
    Capture {
        workspace: String,
        /// Save only these applications, e.g. --only code,chrome (see `windows`
        /// for the names). Default: everything that is open.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Optional extra: also record a Git repository's branch and changes
        /// (repeatable). Not needed to restore your applications.
        #[arg(long = "repo")]
        repos: Vec<PathBuf>,
        /// Progress note to store with this snapshot.
        #[arg(long)]
        note: Option<String>,
        /// Skip window and monitor capture.
        #[arg(long)]
        no_windows: bool,
        /// Show what would be captured without saving it.
        #[arg(long)]
        dry_run: bool,
    },

    /// Reopen a workspace's applications and put their windows back.
    Restore {
        workspace: String,
        /// Snapshot version to restore (default: the latest).
        #[arg(long)]
        version: Option<u32>,
        /// Restore only these applications, e.g. --only code,chrome.
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Show what would happen without starting or moving anything.
        #[arg(long)]
        dry_run: bool,
        /// Seconds to wait for each application's window.
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },

    /// Create a desktop icon that restores a workspace with a double-click.
    Shortcut {
        workspace: String,
        /// Shortcut name (default: the workspace name).
        #[arg(long)]
        name: Option<String>,
        /// Folder for the shortcut (default: your Desktop).
        #[arg(long)]
        folder: Option<PathBuf>,
        /// Replace an existing shortcut.
        #[arg(long)]
        force: bool,
    },

    /// List a workspace's snapshot history.
    Snapshots { workspace: String },

    /// Show a snapshot (latest by default).
    Show {
        workspace: String,
        #[arg(long)]
        version: Option<u32>,
        #[arg(long)]
        json: bool,
    },

    /// Compare two snapshots, or a snapshot with the live environment.
    Diff {
        workspace: String,
        /// Base snapshot version.
        #[arg(long)]
        from: u32,
        /// Snapshot to compare against (default: latest).
        #[arg(long, conflicts_with = "live")]
        to: Option<u32>,
        /// Compare against the live environment instead of a snapshot.
        #[arg(long)]
        live: bool,
        #[arg(long)]
        json: bool,
    },

    /// Permanently delete one snapshot version.
    DeleteSnapshot {
        workspace: String,
        #[arg(long)]
        version: u32,
        /// Confirm the deletion.
        #[arg(long)]
        yes: bool,
    },

    /// Delete old snapshots according to the retention rules. Shows what would
    /// be deleted unless --yes is given. The latest snapshot is always kept.
    Prune {
        /// Workspace to prune (omit with --all).
        workspace: Option<String>,
        #[arg(long, conflicts_with = "workspace")]
        all: bool,
        /// Override retention.keep_last.
        #[arg(long)]
        keep_last: Option<u32>,
        /// Override retention.max_age_days.
        #[arg(long)]
        max_age_days: Option<u32>,
        #[arg(long)]
        yes: bool,
    },

    /// Export workspaces (with full history) to a JSON file.
    Export {
        workspaces: Vec<String>,
        #[arg(long, conflicts_with = "workspaces")]
        all: bool,
        #[arg(long)]
        out: PathBuf,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },

    /// Import workspaces from an export file. Hashes are verified first.
    Import {
        file: PathBuf,
        /// Import under a different name (single-workspace files only).
        #[arg(long = "as")]
        rename: Option<String>,
    },

    /// Write a consistent copy of the whole database.
    Backup {
        /// Destination file (default: backups folder next to the database).
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Verify snapshot hashes and database health (all workspaces by default).
    Verify {
        workspace: Option<String>,
        #[arg(long, requires = "workspace")]
        version: Option<u32>,
    },

    /// Show or create the configuration file.
    #[command(subcommand)]
    Config(ConfigCmd),

    /// Show where ContextFlow keeps its database, config and logs.
    Info,

    /// Debug: list the windows ContextFlow can currently see.
    Windows {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum WorkspaceCmd {
    Create {
        name: String,
        #[arg(long)]
        description: Option<String>,
    },
    List {
        /// Include archived workspaces.
        #[arg(long)]
        all: bool,
    },
    Archive {
        name: String,
    },
    Unarchive {
        name: String,
    },
    /// Permanently delete a workspace and all its snapshots.
    Delete {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the config file location.
    Path,
    /// Print the effective configuration (file merged with defaults).
    Show,
    /// Write a commented starter config file.
    Init {
        #[arg(long)]
        force: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    cf_windows::init_dpi_awareness();

    let config_path = match cli.config.clone() {
        Some(p) => p,
        None => session::default_config_path()?,
    };

    // `config init/path` must work even when the existing file is broken.
    if let Command::Config(cmd @ (ConfigCmd::Init { .. } | ConfigCmd::Path)) = &cli.command {
        return config_cmd(cmd, &config_path, None);
    }

    let cfg = Config::load(&config_path)?;
    let paths = session::resolve_paths(cli.db.clone(), config_path, &cfg)?;
    let _log_guard = logging::init(&cfg.logging, &paths.logs);
    tracing::debug!(db = %redact::home(&paths.db.display().to_string()), "starting");

    if let Command::Config(cmd) = &cli.command {
        return config_cmd(cmd, &paths.config, Some(&cfg));
    }
    if let Command::Windows { json } = cli.command {
        return windows(json);
    }

    let mut db = Database::open(&paths.db)
        .with_context(|| format!("opening database {}", paths.db.display()))?;

    let result = run(cli.command, &mut db, &cfg, &paths, &cli.db);
    if let Err(e) = &result {
        tracing::error!(target: logging::EXIT_TARGET, "{e:#}");
    }
    result
}

fn run(
    command: Command,
    db: &mut Database,
    cfg: &Config,
    paths: &Paths,
    db_flag: &Option<PathBuf>,
) -> Result<()> {
    match command {
        Command::Workspace(cmd) => workspace(db, cmd),
        Command::Capture {
            workspace,
            only,
            repos,
            note,
            no_windows,
            dry_run,
        } => capture_cmd(db, cfg, &workspace, only, repos, note, no_windows, dry_run),
        Command::Restore {
            workspace,
            version,
            only,
            dry_run,
            timeout,
        } => restore_cmd(db, &workspace, version, only, dry_run, timeout),
        Command::Shortcut {
            workspace,
            name,
            folder,
            force,
        } => shortcut_cmd(db, paths, db_flag, &workspace, name, folder, force),
        Command::Snapshots { workspace } => snapshots(db, &workspace),
        Command::Show {
            workspace,
            version,
            json,
        } => show(db, &workspace, version, json),
        Command::Diff {
            workspace,
            from,
            to,
            live,
            json,
        } => diff(db, cfg, &workspace, from, to, live, json),
        Command::DeleteSnapshot {
            workspace,
            version,
            yes,
        } => delete_snapshot(db, &workspace, version, yes),
        Command::Prune {
            workspace,
            all,
            keep_last,
            max_age_days,
            yes,
        } => prune(db, cfg, workspace, all, keep_last, max_age_days, yes),
        Command::Export {
            workspaces,
            all,
            out,
            force,
        } => export(db, workspaces, all, &out, force),
        Command::Import { file, rename } => import(db, &file, rename.as_deref()),
        Command::Backup { out } => backup(db, paths, out),
        Command::Verify { workspace, version } => verify(db, workspace.as_deref(), version),
        Command::Info => info(db, paths),
        Command::Config(_) | Command::Windows { .. } => unreachable!("handled in main"),
    }
}

fn require_yes(yes: bool, what: &str) -> Result<()> {
    if yes {
        return Ok(());
    }
    println!("This will permanently delete {what}. It cannot be undone.");
    bail!("nothing deleted; re-run with --yes to confirm")
}

fn workspace(db: &Database, cmd: WorkspaceCmd) -> Result<()> {
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
                    "{:<32} {:<6} {:>3} snapshot(s)  {}{}",
                    c.name,
                    snaps
                        .first()
                        .map(|s| format!("v{}", s.version))
                        .unwrap_or_else(|| "-".into()),
                    snaps.len(),
                    c.updated_at.format("%Y-%m-%d %H:%M"),
                    if c.is_archived { "  [archived]" } else { "" }
                );
            }
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
        WorkspaceCmd::Delete { name, yes } => {
            let ctx = db.find_context(&name)?;
            let n = db.list_snapshots(ctx.id)?.len();
            require_yes(
                yes,
                &format!("workspace '{}' and its {n} snapshot(s)", ctx.name),
            )?;
            db.delete_context(ctx.id)?;
            println!("Deleted workspace '{}' and {n} snapshot(s)", ctx.name);
        }
    }
    Ok(())
}

fn latest(db: &Database, ctx: &Context) -> Result<Option<Snapshot>> {
    match db.get_snapshot(ctx.id, None) {
        Ok(s) => Ok(Some(s)),
        Err(StorageError::NotFound(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn policy_from(cfg: &Config, keep_last: Option<u32>, max_age_days: Option<u32>) -> RetentionPolicy {
    RetentionPolicy {
        keep_last: keep_last.or(cfg.retention.keep_last),
        max_age: max_age_days
            .or(cfg.retention.max_age_days)
            .map(|d| TimeDelta::days(d as i64)),
    }
}

/// `Code.exe` / `code` / ` CODE ` all mean the application id `code`.
fn normalize_app_names(names: Vec<String>) -> Vec<String> {
    names
        .into_iter()
        .map(|n| {
            let n = n.trim().to_lowercase();
            n.strip_suffix(".exe").map(str::to_string).unwrap_or(n)
        })
        .filter(|n| !n.is_empty())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn capture_cmd(
    db: &mut Database,
    cfg: &Config,
    workspace: &str,
    only: Vec<String>,
    mut repos: Vec<PathBuf>,
    note: Option<String>,
    no_windows: bool,
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
        rules: &cfg.capture,
        git_timeout: cfg.timeouts.git(),
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
        println!("  {:<24} {} window(s)", a.application_id, n);
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

fn restore_cmd(
    db: &mut Database,
    workspace: &str,
    version: Option<u32>,
    only: Vec<String>,
    dry_run: bool,
    timeout: u64,
) -> Result<()> {
    use cf_adapters::RestoreStatus as S;
    let args = RestoreArgs {
        version,
        only: normalize_app_names(only),
        dry_run,
        timeout: Duration::from_secs(timeout.clamp(1, 600)),
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

fn shortcut_cmd(
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

fn fmt_versions(vs: &[u32]) -> String {
    vs.iter()
        .map(|v| format!("v{v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn snapshots(db: &Database, workspace: &str) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    let list = db.list_snapshots(ctx.id)?;
    if list.is_empty() {
        println!("'{}' has no snapshots yet.", ctx.name);
    }
    for s in list {
        println!(
            "v{:<5} {}  {:<9} {}",
            s.version,
            s.created_at.format("%Y-%m-%d %H:%M:%S"),
            s.capture_status.as_str(),
            &s.hash[..12]
        );
    }
    Ok(())
}

fn show(db: &Database, workspace: &str, version: Option<u32>, json: bool) -> Result<()> {
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

fn diff(
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
            rules: &cfg.capture,
            git_timeout: cfg.timeouts.git(),
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

fn delete_snapshot(db: &Database, workspace: &str, version: u32, yes: bool) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    db.get_snapshot(ctx.id, Some(version))?; // fail early if it does not exist
    require_yes(yes, &format!("snapshot v{version} of '{}'", ctx.name))?;
    db.delete_snapshot(ctx.id, version)?;
    println!("Deleted '{}' v{version}", ctx.name);
    Ok(())
}

fn prune(
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

fn export(db: &Database, names: Vec<String>, all: bool, out: &Path, force: bool) -> Result<()> {
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

fn import(db: &mut Database, file: &Path, rename: Option<&str>) -> Result<()> {
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

fn backup(db: &Database, paths: &Paths, out: Option<PathBuf>) -> Result<()> {
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

fn verify(db: &Database, workspace: Option<&str>, version: Option<u32>) -> Result<()> {
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

fn info(db: &Database, paths: &Paths) -> Result<()> {
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

fn config_cmd(cmd: &ConfigCmd, path: &Path, cfg: Option<&Config>) -> Result<()> {
    match cmd {
        ConfigCmd::Path => println!("{}", path.display()),
        ConfigCmd::Show => {
            let cfg = cfg.expect("config loaded for show");
            println!("# effective configuration ({})", path.display());
            print!("{}", cfg.to_toml());
        }
        ConfigCmd::Init { force } => {
            if path.exists() && !force {
                bail!(
                    "{} already exists; use --force to replace it with the template",
                    path.display()
                );
            }
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, cf_config::TEMPLATE)?;
            println!("Wrote {}", path.display());
        }
    }
    Ok(())
}

fn windows(json: bool) -> Result<()> {
    let windows = cf_windows::enumerate_windows()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&windows)?);
        return Ok(());
    }
    for w in windows {
        println!(
            "{:>6}  {:<10} {:>6},{:<6} {:>5}x{:<5} {:<24} {}",
            w.process_id,
            w.display_state.as_str(),
            w.rect.x,
            w.rect.y,
            w.rect.width,
            w.rect.height,
            w.executable_path
                .as_deref()
                .map(capture::application_id)
                .unwrap_or_else(|| "?".into()),
            w.title
        );
    }
    Ok(())
}

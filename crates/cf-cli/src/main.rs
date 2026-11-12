//! `contextflow` — command-line front end to the ContextFlow engine.
//!
//! The CLI is a thin layer: all behaviour lives in the library crates so the
//! Tauri desktop app (Phase 5) can reuse it unchanged.

mod capture;

use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};
use clap::{Parser, Subcommand};

use cf_core::{Context, Snapshot};
use cf_storage::Database;

#[derive(Parser)]
#[command(
    name = "contextflow",
    version,
    about = "Capture your environment. Preserve your progress. Resume exactly where you left off."
)]
struct Cli {
    /// Path to the ContextFlow database (default: per-user data directory).
    #[arg(long, global = true, env = "CONTEXTFLOW_DB")]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create, list and archive workspaces.
    #[command(subcommand)]
    Workspace(WorkspaceCmd),

    /// Capture the current environment as a new snapshot version.
    Capture {
        workspace: String,
        /// Git repository to include (repeatable). Defaults to the repositories
        /// in the workspace's latest snapshot.
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

    /// Verify snapshot integrity (hash check) and database health.
    Verify {
        workspace: String,
        #[arg(long)]
        version: Option<u32>,
    },

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
}

fn default_db_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "ContextFlow", "ContextFlow")
        .context("could not determine the user data directory")?;
    Ok(dirs.data_dir().join("contextflow.db"))
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    cf_windows::init_dpi_awareness();

    let cli = Cli::parse();
    let db_path = match cli.db {
        Some(p) => p,
        None => default_db_path()?,
    };
    let mut db = Database::open(&db_path)
        .with_context(|| format!("opening database {}", db_path.display()))?;

    match cli.command {
        Command::Workspace(cmd) => workspace(&db, cmd),
        Command::Capture {
            workspace,
            repos,
            note,
            no_windows,
            dry_run,
        } => capture_cmd(&mut db, &workspace, repos, note, !no_windows, dry_run),
        Command::Snapshots { workspace } => snapshots(&db, &workspace),
        Command::Show {
            workspace,
            version,
            json,
        } => show(&db, &workspace, version, json),
        Command::Diff {
            workspace,
            from,
            to,
            live,
            json,
        } => diff(&db, &workspace, from, to, live, json),
        Command::Verify { workspace, version } => verify(&db, &workspace, version),
        Command::Windows { json } => windows(json),
    }
}

fn workspace(db: &Database, cmd: WorkspaceCmd) -> Result<()> {
    match cmd {
        WorkspaceCmd::Create { name, description } => {
            let ctx = db.create_context(&name, description.as_deref())?;
            println!("Created workspace '{}' ({})", ctx.name, ctx.id);
        }
        WorkspaceCmd::List { all } => {
            let contexts = db.list_contexts(all)?;
            if contexts.is_empty() {
                println!("No workspaces yet. Create one with: contextflow workspace create <name>");
            }
            for c in contexts {
                let latest = db.list_snapshots(c.id)?.into_iter().next();
                println!(
                    "{:<32} {:<10} {}{}",
                    c.name,
                    latest
                        .map(|s| format!("v{}", s.version))
                        .unwrap_or_else(|| "-".into()),
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
    }
    Ok(())
}

fn latest(db: &Database, ctx: &Context) -> Result<Option<Snapshot>> {
    match db.get_snapshot(ctx.id, None) {
        Ok(s) => Ok(Some(s)),
        Err(cf_storage::StorageError::NotFound(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn capture_cmd(
    db: &mut Database,
    workspace: &str,
    mut repos: Vec<PathBuf>,
    note: Option<String>,
    include_windows: bool,
    dry_run: bool,
) -> Result<()> {
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
        repos: &repos,
        notes: note.into_iter().collect(),
        include_windows,
    });

    println!(
        "Discovered {} application(s), {} window(s), {} monitor(s), {} repositor{}",
        result.payload.applications.len(),
        result.payload.windows.len(),
        result.payload.monitors.len(),
        result.payload.git.len(),
        if result.payload.git.len() == 1 {
            "y"
        } else {
            "ies"
        }
    );
    for a in &result.payload.applications {
        let n = result
            .payload
            .windows
            .iter()
            .filter(|w| w.application_id == a.application_id)
            .count();
        println!("  {:<24} {} window(s)", a.application_id, n);
    }
    for g in &result.payload.git {
        println!(
            "  git {} [{}] {} staged, {} modified, {} untracked",
            g.repository_path,
            g.branch.as_deref().unwrap_or("detached"),
            g.staged,
            g.modified,
            g.untracked
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
    println!(
        "Saved '{}' v{} ({}, {}){}",
        ctx.name,
        snap.version,
        snap.capture_status.as_str(),
        &snap.hash[..12],
        if identical {
            " — identical to previous version"
        } else {
            ""
        }
    );
    Ok(())
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
        "{} v{} — {} ({})",
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
    if !p.git.is_empty() {
        println!("\nRepositories:");
        for g in &p.git {
            println!(
                "  {} [{}] {} — {} staged, {} modified, {} untracked",
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
            repos: &repos,
            notes: latest(db, &ctx)?
                .map(|s| s.payload.notes)
                .unwrap_or_default(),
            include_windows: true,
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

fn verify(db: &Database, workspace: &str, version: Option<u32>) -> Result<()> {
    let ctx = db.find_context(workspace)?;
    let problems = db.integrity_check()?;
    if !problems.is_empty() {
        bail!("database integrity check failed:\n{}", problems.join("\n"));
    }
    let versions: Vec<u32> = match version {
        Some(v) => vec![v],
        None => db
            .list_snapshots(ctx.id)?
            .into_iter()
            .map(|s| s.version)
            .collect(),
    };
    let mut failures = 0;
    for v in versions {
        let r = db.verify_snapshot(ctx.id, Some(v))?;
        if r.ok() {
            println!("v{v}: ok");
        } else {
            failures += 1;
            println!(
                "v{v}: CORRUPT (stored {}, computed {}, payload readable: {})",
                &r.stored_hash[..12],
                &r.computed_hash[..12],
                r.payload_parses
            );
        }
    }
    if failures > 0 {
        bail!("{failures} snapshot(s) failed verification");
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

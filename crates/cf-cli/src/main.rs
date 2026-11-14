//! `contextflow`: command-line front end to the ContextFlow engine.
//!
//! The CLI is a thin layer: all behaviour lives in the library crates so the
//! Tauri desktop app (Phase 5) can reuse it unchanged. This file only defines
//! the command-line arguments and dispatches to `commands::*`.

mod commands;

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::{Parser, Subcommand};

use cf_cli::logging;
use cf_cli::session::{self, Paths};
use cf_config::Config;
use cf_core::redact;
use cf_storage::Database;

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
pub(crate) enum Command {
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
        /// Save minimized windows as normal windows.
        #[arg(long)]
        unminimize: bool,
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
        /// Don't close any programs, whatever the workspaces' cleanup rules say.
        #[arg(long)]
        no_cleanup: bool,
        /// Don't save the workspace you are leaving first.
        #[arg(long)]
        no_save: bool,
    },

    /// Programs a workspace closes when you enter or leave it.
    #[command(subcommand)]
    Cleanup(CleanupCmd),

    /// Leave the active workspace: run its "when leaving" cleanup.
    Leave,

    /// Make restore (and the desktop icon) always use one snapshot version
    /// (default: the latest).
    Pin {
        workspace: String,
        #[arg(long)]
        version: Option<u32>,
    },

    /// Go back to restoring the latest snapshot.
    Unpin { workspace: String },

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
        /// Picture to use as the icon (PNG, JPEG, BMP or ICO). It is kept for
        /// this workspace, and an existing shortcut just gets the new icon.
        #[arg(long, conflicts_with = "default_icon")]
        icon: Option<PathBuf>,
        /// Go back to the standard ContextFlow icon.
        #[arg(long)]
        default_icon: bool,
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

    /// Set up and check the Chrome/Edge extension that saves your open tabs.
    #[command(subcommand)]
    Browser(BrowserCmd),

    /// Show where ContextFlow keeps its database, config and logs.
    Info,

    /// Debug: list the windows ContextFlow can currently see.
    Windows {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum CleanupCmd {
    /// Show a workspace's cleanup rules and what they would close right now.
    Show { workspace: String },
    /// Add a program to a workspace's cleanup list.
    Add {
        workspace: String,
        /// Program name, like `discord` (see `contextflow cleanup apps`).
        process: String,
        /// Close it when the workspace is left, instead of when it is entered.
        #[arg(long)]
        on_exit: bool,
        /// End it if it doesn't close when asked (unsaved work in it is lost).
        #[arg(long)]
        force: bool,
    },
    /// Remove a program from a workspace's cleanup list.
    Remove {
        workspace: String,
        process: String,
        #[arg(long)]
        on_exit: bool,
    },
    /// List the programs running now, with their memory and CPU use.
    Apps,
}

#[derive(Subcommand)]
pub(crate) enum WorkspaceCmd {
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
    /// Change a workspace's name (snapshots, pin and icon are kept).
    Rename {
        name: String,
        new_name: String,
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
        /// Leave the workspace's desktop icon(s) in place. By default they are
        /// moved to the Recycle Bin, since they stop working without the workspace.
        #[arg(long)]
        keep_shortcuts: bool,
    },
    /// Review and approve an imported workspace so it can be restored.
    Trust {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum BrowserCmd {
    /// Register ContextFlow with Chrome and Edge and unpack the extension.
    Install,
    /// Show whether the integration is set up and which tabs are visible now.
    Status,
    /// Remove the registration (remove the extension in the browser itself).
    Uninstall,
}

#[derive(Subcommand)]
pub(crate) enum ConfigCmd {
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
        return commands::config::config_cmd(cmd, &config_path, None);
    }

    let cfg = Config::load(&config_path)?;
    let paths = session::resolve_paths(cli.db.clone(), config_path, &cfg)?;
    let _log_guard = logging::init(&cfg.logging, &paths.logs);
    tracing::debug!(db = %redact::home(&paths.db.display().to_string()), "starting");

    if let Command::Config(cmd) = &cli.command {
        return commands::config::config_cmd(cmd, &paths.config, Some(&cfg));
    }
    if let Command::Windows { json } = cli.command {
        return commands::config::windows(json);
    }
    if let Command::Browser(cmd) = &cli.command {
        return commands::browser::browser_cmd(cmd);
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
        Command::Workspace(cmd) => commands::workspace::workspace(db, paths, cmd),
        Command::Capture {
            workspace,
            only,
            repos,
            note,
            no_windows,
            unminimize,
            dry_run,
        } => commands::capture_cmd::capture_cmd(
            db, cfg, &workspace, only, repos, note, no_windows, unminimize, dry_run,
        ),
        Command::Cleanup(cmd) => commands::cleanup::cleanup_cmd(db, cmd),
        Command::Leave => commands::cleanup::leave(db, cfg),
        Command::Pin { workspace, version } => commands::history::pin(db, &workspace, version),
        Command::Unpin { workspace } => commands::history::unpin(db, &workspace),
        Command::Restore {
            workspace,
            version,
            only,
            dry_run,
            timeout,
            no_cleanup,
            no_save,
        } => commands::restore::restore_cmd(
            db,
            &workspace,
            version,
            only,
            dry_run,
            timeout,
            no_cleanup,
            no_save,
            cfg,
            cfg.browser.max_tabs_per_window,
            cfg.timeouts.restore_overall(),
        ),
        Command::Shortcut {
            workspace,
            name,
            folder,
            force,
            icon,
            default_icon,
        } => commands::restore::shortcut_cmd(
            db,
            paths,
            db_flag,
            &workspace,
            commands::restore::ShortcutOpts {
                name,
                folder,
                force,
                icon,
                default_icon,
            },
        ),
        Command::Snapshots { workspace } => commands::history::snapshots(db, &workspace),
        Command::Show {
            workspace,
            version,
            json,
        } => commands::history::show(db, &workspace, version, json),
        Command::Diff {
            workspace,
            from,
            to,
            live,
            json,
        } => commands::history::diff(db, cfg, &workspace, from, to, live, json),
        Command::DeleteSnapshot {
            workspace,
            version,
            yes,
        } => commands::maintenance::delete_snapshot(db, &workspace, version, yes),
        Command::Prune {
            workspace,
            all,
            keep_last,
            max_age_days,
            yes,
        } => commands::maintenance::prune(db, cfg, workspace, all, keep_last, max_age_days, yes),
        Command::Export {
            workspaces,
            all,
            out,
            force,
        } => commands::maintenance::export(db, workspaces, all, &out, force),
        Command::Import { file, rename } => {
            commands::maintenance::import(db, &file, rename.as_deref())
        }
        Command::Backup { out } => commands::maintenance::backup(db, paths, out),
        Command::Verify { workspace, version } => {
            commands::maintenance::verify(db, workspace.as_deref(), version)
        }
        Command::Info => commands::maintenance::info(db, paths),
        Command::Config(_) | Command::Windows { .. } | Command::Browser(_) => {
            unreachable!("handled in main")
        }
    }
}

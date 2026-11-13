//! `contextflow-launch`: the program a desktop shortcut starts.
//!
//! It is built for the Windows GUI subsystem, so double-clicking it shows no
//! console window. It restores the workspace and stays silent on success; if
//! anything went wrong it shows one message box explaining what.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};

use cf_cli::logging;
use cf_cli::session::{self, RestoreArgs};
use cf_config::Config;
use cf_engine::RestoreRun;
use cf_storage::Database;

const TITLE: &str = "ContextFlow";

struct Args {
    workspace: String,
    db: Option<PathBuf>,
    config: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let (mut workspace, mut db, mut config) = (None, None, None);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--config" => config = it.next().map(PathBuf::from),
            _ if workspace.is_none() => workspace = Some(a),
            other => bail!("unexpected argument '{other}'"),
        }
    }
    match workspace {
        Some(workspace) => Ok(Args {
            workspace,
            db,
            config,
        }),
        None => {
            bail!("no workspace given. This program is started by a ContextFlow desktop shortcut.")
        }
    }
}

fn report(run: &RestoreRun, workspace: &str) -> Option<String> {
    let problems: Vec<String> = run
        .problems()
        .map(|s| format!("• {}: {}", s.application_id, s.message))
        .collect();
    if problems.is_empty() {
        return None;
    }
    Some(format!(
        "'{workspace}' was restored, but some things need your attention:\n\n{}",
        problems.join("\n")
    ))
}

fn run() -> Result<Option<String>> {
    let args = parse_args()?;
    let config_path = match args.config.clone() {
        Some(p) => p,
        None => session::default_config_path()?,
    };
    let cfg = Config::load(&config_path)?;
    let paths = session::resolve_paths(args.db.clone(), config_path, &cfg)?;
    let _guard = logging::init(&cfg.logging, &paths.logs);

    cf_windows::init_dpi_awareness();
    let mut db = Database::open(&paths.db)?;
    let restore_args = RestoreArgs {
        version: None,
        only: Vec::new(),
        dry_run: false,
        timeout: Duration::from_secs(30),
    };
    let (ctx, _snap, run) =
        session::restore_workspace(&mut db, &args.workspace, &restore_args, &mut |m| {
            tracing::info!("{m}")
        })?;
    Ok(report(&run, &ctx.name))
}

fn main() {
    match run() {
        Ok(None) => {}
        Ok(Some(problems)) => cf_windows::message_box(TITLE, &problems, true),
        Err(e) => {
            tracing::error!(target: logging::EXIT_TARGET, "{e:#}");
            cf_windows::message_box(
                TITLE,
                &format!("Could not restore your workspace.\n\n{e:#}"),
                true,
            );
        }
    }
}

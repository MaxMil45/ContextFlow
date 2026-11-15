//! `contextflow-launch`: the program a desktop shortcut starts.
//!
//! It is built for the Windows GUI subsystem, so double-clicking it shows no
//! console window. It restores the workspace and stays silent on success; if
//! anything went wrong it shows one message box explaining what.
//!
//! If the ContextFlow app is running, the restore is handed to it over the
//! per-user pipe; otherwise it runs in this process, exactly as the app
//! would (see `docs/AGENT.md`).

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::PathBuf;

use anyhow::{bail, Result};

use cf_app::service::Via;
use cf_cli::logging;
use cf_cli::session::{self, RestoreBusy};
use cf_config::Config;
use cf_ipc::{RequestBody, ResponseBody, RestoreRequest, RestoreSummary};
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

fn report(summary: &RestoreSummary) -> Option<String> {
    if summary.problems.is_empty() {
        return None;
    }
    let problems: Vec<String> = summary
        .problems
        .iter()
        .map(|p| format!("• {}: {}", p.subject, p.message))
        .collect();
    Some(format!(
        "'{}' was restored, but some things need your attention:\n\n{}",
        summary.workspace,
        problems.join("\n")
    ))
}

fn run() -> Result<Option<String>> {
    let args = parse_args()?;
    let custom_paths = args.db.is_some() || args.config.is_some();
    let config_path = match args.config.clone() {
        Some(p) => p,
        None => session::default_config_path()?,
    };
    let cfg = Config::load(&config_path)?;
    let paths = session::resolve_paths(args.db.clone(), config_path, &cfg)?;
    let _guard = logging::init(&cfg.logging, &paths.logs);

    // Hand it to the running app first; without one, restore here the same way.
    // The app serves only the per-user database, so custom paths stay local.
    let pipe = cf_app::service::pipe_for(custom_paths);
    let request = RequestBody::Restore(RestoreRequest::switch_to(&args.workspace));
    let (reply, via) = cf_app::service::call_or_fallback(pipe.as_deref(), request, |request| {
        cf_windows::init_dpi_awareness();
        let mut db = Database::open(&paths.db)?;
        Ok(cf_app::service::handle(
            &mut db,
            &paths.config,
            request,
            &mut |m| tracing::info!("{m}"),
        ))
    })?;
    tracing::info!(
        "restore handled {}",
        match &via {
            Via::Agent => "by the ContextFlow app".to_string(),
            Via::InProcess(why) => format!("in-process ({why})"),
        }
    );
    match reply {
        ResponseBody::Restored(summary) => Ok(report(&summary)),
        ResponseBody::Busy => Err(RestoreBusy.into()),
        ResponseBody::Error(e) => bail!("{}", e.message),
        other => bail!("unexpected reply from the ContextFlow app: {other:?}"),
    }
}

fn main() {
    match run() {
        Ok(None) => {}
        Ok(Some(problems)) => cf_windows::message_box(TITLE, &problems, true),
        // A restore is already running (a second double-click): do nothing.
        Err(e) if e.downcast_ref::<RestoreBusy>().is_some() => {
            tracing::info!("another restore is already running; this one did nothing");
        }
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

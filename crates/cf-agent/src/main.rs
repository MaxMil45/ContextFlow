//! `contextflow-agent`: the windowless background agent. It serves the
//! front ends over this user's named pipe until asked to shut down.

#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{bail, Context as _, Result};

use cf_agent::Service;
use cf_app::{logging, session};
use cf_config::Config;
use cf_storage::Database;

fn run() -> Result<()> {
    // The agent always uses the per-user config and database: clients with
    // custom ones work in-process instead.
    if let Some(arg) = std::env::args().nth(1) {
        bail!("unexpected argument '{arg}'");
    }
    let config_path = session::default_config_path()?;
    let cfg = Config::load(&config_path)?;
    let paths = session::resolve_paths(None, config_path, &cfg)?;
    let _guard = logging::init(&cfg.logging, &paths.logs);

    cf_windows::init_dpi_awareness();
    let pipe = cf_ipc::default_pipe_name()?;
    let server = cf_ipc::Server::bind(&pipe).context("could not start the agent")?;
    let db = Database::open(&paths.db)?;
    tracing::info!("agent v{} listening", env!("CARGO_PKG_VERSION"));
    server.run(Service::new(paths.config, db))?;
    tracing::info!("agent stopped");
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        tracing::error!(target: logging::EXIT_TARGET, "{e:#}");
        eprintln!("contextflow-agent: {e:#}");
        std::process::exit(1);
    }
}

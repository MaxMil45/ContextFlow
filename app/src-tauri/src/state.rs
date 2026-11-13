//! Process-wide state shared by every command.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use cf_cli::session::{self, Paths};
use cf_config::Config;

/// Commands open their own database connection each time: SQLite in WAL mode
/// allows readers alongside one writer, so a long restore never blocks the
/// dashboard and there is no connection lock to contend on.
pub struct AppState {
    pub paths: Paths,
    pub config: Config,
    /// Cancel flag of the restore in progress, if any.
    pub restore_cancel: Mutex<Option<Arc<AtomicBool>>>,
    /// Keeps the log file writer alive for the life of the process.
    _log_guard: Option<cf_cli::logging::WorkerGuard>,
}

impl AppState {
    /// Resolve the same config, database and log locations as the CLI, honouring
    /// `CONTEXTFLOW_CONFIG` and `CONTEXTFLOW_DB`, so both front ends share data.
    pub fn load() -> Result<Self> {
        let config_path = match std::env::var_os("CONTEXTFLOW_CONFIG") {
            Some(p) => PathBuf::from(p),
            None => session::default_config_path()?,
        };
        let config = Config::load(&config_path)
            .with_context(|| format!("loading config {}", config_path.display()))?;
        let db_override = std::env::var_os("CONTEXTFLOW_DB").map(PathBuf::from);
        let paths = session::resolve_paths(db_override, config_path, &config)?;
        let log_guard = cf_cli::logging::init(&config.logging, &paths.logs);
        Ok(Self {
            paths,
            config,
            restore_cancel: Mutex::new(None),
            _log_guard: log_guard,
        })
    }
}

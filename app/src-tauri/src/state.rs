//! Process-wide state shared by every command.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context as _, Result};
use cf_cli::session::{self, Paths};
use cf_config::Config;

/// Commands open their own database connection each time: SQLite in WAL mode
/// allows readers alongside one writer, so a long restore never blocks the
/// dashboard and there is no connection lock to contend on.
pub struct AppState {
    pub paths: Paths,
    config: RwLock<Config>,
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
            config: RwLock::new(config),
            restore_cancel: Mutex::new(None),
            _log_guard: log_guard,
        })
    }

    /// A copy of the current settings. Settings can change while the app runs
    /// (the Settings screen), so commands read them per call.
    pub fn config(&self) -> Config {
        self.config.read().expect("config lock poisoned").clone()
    }

    /// Write `config` to `config.toml` and use it from now on. The file is
    /// written to a temporary name first, so a failure never leaves a
    /// half-written config.
    pub fn save_config(&self, config: Config) -> Result<()> {
        config.validate()?;
        let path = &self.paths.config;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let text = format!(
            "# Written by the ContextFlow app's Settings. Comments are not kept.\n{}",
            config.to_toml()
        );
        let tmp = path.with_extension("toml.partial");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
        *self.config.write().expect("config lock poisoned") = config;
        Ok(())
    }
}

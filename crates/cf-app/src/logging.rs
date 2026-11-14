//! Logging: warnings to stderr, plus a daily rolling file at the configured level.
//!
//! Never log window titles, full URLs or note contents at info level or above.
//! Use `cf_core::redact` for URLs and paths.

use std::path::Path;

use cf_config::LoggingConfig;
pub use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Log target for the error that ends the program (file only).
pub const EXIT_TARGET: &str = "cf_exit";

/// Keep the returned guard alive for the whole program, or buffered log lines are lost.
pub fn init(cfg: &LoggingConfig, log_dir: &Path) -> Option<WorkerGuard> {
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn"))
                // The final error is printed by main already; only log it to the file.
                .add_directive(
                    format!("{EXIT_TARGET}=off")
                        .parse()
                        .expect("valid directive"),
                ),
        );

    let mut guard = None;
    let file_layer = if cfg.file {
        let appender = std::fs::create_dir_all(log_dir)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                Builder::new()
                    .rotation(Rotation::DAILY)
                    .filename_prefix("contextflow")
                    .filename_suffix("log")
                    .max_log_files(cfg.max_files)
                    .build(log_dir)
                    .map_err(|e| e.to_string())
            });
        match appender {
            Ok(appender) => {
                let (writer, g) = tracing_appender::non_blocking(appender);
                guard = Some(g);
                Some(
                    fmt::layer()
                        .with_ansi(false)
                        .with_writer(writer)
                        .with_filter(EnvFilter::new(&cfg.level)),
                )
            }
            Err(e) => {
                eprintln!(
                    "warning: file logging disabled ({}: {e})",
                    log_dir.display()
                );
                None
            }
        }
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();
    guard
}

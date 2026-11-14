//! Health check and diagnostics report (Settings → Advanced).

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use ts_rs::TS;

use crate::commands::blocking;
use crate::dto::FileResult;
use crate::error::{CmdResult, CommandError, ErrorCode};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct HealthCheck {
    pub name: String,
    /// `ok`, `warn` or `bad`.
    pub status: String,
    pub message: String,
}

fn checks(app: &AppHandle) -> Vec<HealthCheck> {
    let state = app.state::<AppState>();
    let cfg = state.config();
    let mut out: Vec<HealthCheck> =
        cf_app::health::run(&state.paths.db, &state.paths.backups, &cfg)
            .into_iter()
            .map(|c| HealthCheck {
                name: c.name,
                status: c.status.as_str().into(),
                message: c.message,
            })
            .collect();
    let keys = state.hotkeys.lock().map(|k| k.clone()).unwrap_or_default();
    out.push(match keys.summary() {
        Some(problem) => HealthCheck {
            name: "Shortcuts".into(),
            status: "warn".into(),
            message: problem,
        },
        None => HealthCheck {
            name: "Shortcuts".into(),
            status: "ok".into(),
            message: format!(
                "switcher {} · quick capture {}",
                or_off(&cfg.app.switcher_hotkey),
                or_off(&cfg.app.capture_hotkey)
            ),
        },
    });
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    out.push(HealthCheck {
        name: "Start with Windows".into(),
        status: "ok".into(),
        message: if autostart {
            "on: ContextFlow starts in the tray when you sign in".into()
        } else {
            "off: shortcuts and restart recovery work only after you start ContextFlow".into()
        },
    });
    out
}

fn or_off(k: &str) -> String {
    if k.trim().is_empty() {
        "off".into()
    } else {
        k.to_string()
    }
}

/// Run the health check.
#[tauri::command]
pub async fn health_check(app: AppHandle) -> CmdResult<Vec<HealthCheck>> {
    blocking(move || Ok(checks(&app))).await
}

/// Write a plain-text report for a bug report: versions, the health check,
/// settings and the end of today's log. Nothing is sent anywhere; the user
/// picks where it is saved and can read it before sharing it.
#[tauri::command]
pub async fn save_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<FileResult>> {
    let logs = state.paths.logs.clone();
    let cfg = state.config();
    blocking(move || {
        let name = format!(
            "contextflow-diagnostics-{}.txt",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let Some(dest) = app
            .dialog()
            .file()
            .set_title("Save a diagnostics report")
            .set_file_name(name)
            .add_filter("Text", &["txt"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let dest = dest
            .into_path()
            .map_err(|e| CommandError::new(ErrorCode::Conflict, e.to_string()))?;
        let mut text = String::new();
        text.push_str(&format!(
            "ContextFlow {} diagnostics, {}\nWindows ({}), {}\n\n",
            env!("CARGO_PKG_VERSION"),
            chrono::Local::now().to_rfc3339(),
            std::env::consts::ARCH,
            if cfg!(debug_assertions) {
                "debug build"
            } else {
                "release build"
            }
        ));
        text.push_str("== Health check\n");
        for c in checks(&app) {
            text.push_str(&format!("[{}] {}: {}\n", c.status, c.name, c.message));
        }
        text.push_str("\n== Settings (config.toml)\n");
        text.push_str(&cfg.to_toml());
        text.push_str(
            "\n== End of the newest log (logs never contain window titles, addresses or notes)\n",
        );
        text.push_str(&newest_log_tail(&logs, 300));
        std::fs::write(&dest, text)
            .map_err(|e| CommandError::new(ErrorCode::Conflict, e.to_string()))?;
        Ok(Some(FileResult {
            message: "Diagnostics report saved. Read it before you share it.".into(),
            path: dest.display().to_string(),
        }))
    })
    .await
}

fn newest_log_tail(dir: &std::path::Path, lines: usize) -> String {
    let newest = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    let Some(file) = newest else {
        return "(no log file)\n".into();
    };
    let text = std::fs::read_to_string(file.path()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    let mut out = all[start..].join("\n");
    out.push('\n');
    out
}

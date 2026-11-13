//! ContextFlow desktop application.
//!
//! The Rust side is a thin shell: every behaviour lives in the `cf-*` crates
//! and is shared with the CLI. This crate wires those into Tauri commands, the
//! tray and the single-instance guard.

mod commands;
mod dto;
mod error;
mod state;
mod tray;

use tauri::Manager;

pub use error::{CommandError, ErrorCode};

/// Bring the main window to the front (tray click, second launch).
pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn run() {
    // Window placement during restore needs per-monitor DPI awareness.
    cf_windows::init_dpi_awareness();

    let state = match state::AppState::load() {
        Ok(s) => s,
        Err(e) => {
            // Without a config and database location there is nothing to show.
            eprintln!("ContextFlow could not start: {e:#}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        // Registered first so a second launch is caught before anything else runs.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .manage(state)
        .setup(|app| {
            tray::build(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_workspaces,
            commands::create_workspace,
            commands::list_snapshots,
            commands::get_snapshot,
            commands::capture_workspace,
            commands::restore_workspace,
            commands::cancel_restore,
            commands::create_shortcut,
            commands::get_workspace_icon,
            commands::set_workspace_icon,
            commands::reset_workspace_icon,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ContextFlow");
}

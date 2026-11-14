//! ContextFlow desktop application.
//!
//! The Rust side is a thin shell: every behaviour lives in the `cf-*` crates
//! and is shared with the CLI. This crate wires those into Tauri commands, the
//! tray and the single-instance guard.

mod cleanup_commands;
mod commands;
mod dto;
mod error;
mod hotkey;
mod overlay;
mod review;
mod settings;
mod shell;
mod state;

use tauri::{Manager, WindowEvent};

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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        // Start with Windows (opt-in in Settings): quietly, in the tray.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .manage(state)
        .setup(|app| {
            shell::build_tray(app.handle())?;
            // The on-screen confirmation for quick capture, ready (hidden) before it's needed.
            if let Err(e) = overlay::preload(app.handle()) {
                tracing::warn!("could not prepare the capture confirmation: {e}");
            }
            // The quick-capture shortcut. If another program already owns it, say so
            // instead of failing silently.
            let hotkey = app.state::<state::AppState>().config().app.capture_hotkey;
            if let Err(e) = hotkey::register(app.handle(), &hotkey) {
                tracing::warn!("capture shortcut not set up: {e}");
                shell::toast(app.handle(), "Capture shortcut not available", &e);
            }
            // The window is created hidden; show it unless Windows started us at sign-in.
            if !std::env::args().any(|a| a == "--hidden") {
                show_main_window(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // "Close to tray": the window hides and ContextFlow keeps running.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.state::<state::AppState>().config().app.close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
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
            commands::rename_workspace,
            commands::set_workspace_archived,
            commands::get_workspace_shortcuts,
            commands::delete_workspace,
            commands::set_pinned_version,
            commands::delete_snapshot,
            commands::get_trust_review,
            commands::approve_workspace,
            cleanup_commands::list_running_apps,
            cleanup_commands::get_cleanup_config,
            cleanup_commands::set_cleanup_config,
            cleanup_commands::preview_cleanup,
            cleanup_commands::run_cleanup_now,
            cleanup_commands::leave_workspace,
            review::discover_capture,
            review::compare_snapshots,
            settings::get_settings,
            settings::save_settings,
            settings::get_browser_status,
            settings::install_browser_integration,
            settings::remove_browser_integration,
            settings::open_folder,
            settings::backup_now,
            settings::export_workspaces,
            settings::import_workspaces,
            settings::verify_data,
            settings::preview_retention,
            settings::apply_retention,
            shell::refresh_tray_menu,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ContextFlow");
}

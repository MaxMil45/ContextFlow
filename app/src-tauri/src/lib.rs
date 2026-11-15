//! ContextFlow desktop application.
//!
//! The Rust side is a thin shell: every behaviour lives in the `cf-*` crates
//! and is shared with the CLI. This crate wires those into Tauri commands, the
//! tray and the single-instance guard.

mod cleanup_commands;
mod commands;
mod diagnostics;
mod dto;
mod error;
mod flow_commands;
mod hotkey;
mod overlay;
mod pipe;
mod recovery;
mod review;
mod settings;
mod shell;
mod state;
mod switcher;
mod ui;

use tauri::Manager;

pub use error::{CommandError, ErrorCode};

/// Bring the main window to the front (tray click, second launch), creating it
/// if it was closed. See [`ui`].
pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    ui::show_main(app);
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
            // The desktop icon and the CLI hand their restores to this process.
            pipe::serve(app.handle(), &app.state::<state::AppState>().paths);
            // The quick-capture shortcut. If another program already owns it, say so
            // instead of failing silently.
            let keys = app.state::<state::AppState>().config().app;
            let status =
                hotkey::register(app.handle(), &keys.capture_hotkey, &keys.switcher_hotkey);
            if let Some(problem) = status.summary() {
                shell::toast(app.handle(), "A shortcut is not available", &problem);
            }
            // The window is only created when it's wanted: not at all when Windows
            // starts ContextFlow at sign-in, so the tray costs almost nothing.
            if !std::env::args().any(|a| a == "--hidden") {
                show_main_window(app.handle());
            }
            // After a restart, offer to bring back the workspace you were in; and keep
            // that restore point fresh while you work.
            recovery::on_start(app.handle());
            recovery::start_checkpoints(app.handle());
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
            commands::switch_workspace,
            switcher::quick_switch,
            switcher::hide_switcher,
            commands::cancel_restore,
            commands::create_shortcut,
            commands::get_workspace_icon,
            commands::set_workspace_icon,
            commands::reset_workspace_icon,
            commands::update_workspace,
            commands::list_restore_runs,
            commands::set_save_mode,
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
            settings::get_hotkey_status,
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
            ui::take_ui_action,
            flow_commands::get_workspace_commands,
            flow_commands::set_workspace_commands,
            flow_commands::park_current,
            flow_commands::take_resume,
            flow_commands::list_parked,
            flow_commands::list_timeline,
            flow_commands::search_workspaces,
            diagnostics::health_check,
            diagnostics::save_diagnostics,
            overlay::current_toast,
        ])
        .build(tauri::generate_context!())
        .expect("error while starting ContextFlow")
        .run(|app, event| {
            // "Close to tray": closing the window really closes it (freeing its
            // web view), and ContextFlow carries on in the tray. Quit in the tray
            // menu still exits, because that asks with an exit code.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() && app.state::<state::AppState>().config().app.close_to_tray {
                    api.prevent_exit();
                }
            }
        });
}

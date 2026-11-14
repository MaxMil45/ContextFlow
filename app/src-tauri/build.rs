// Every command the UI can call is listed here, so Tauri turns each into a
// permission (`allow-<command>`). Each window then gets only the commands it
// uses (see `capabilities/`): the corner card and the switcher can't reach the
// rest of the app. A command missing here can't be called from any window.
fn main() {
    let commands: &[&str] = &[
        "app_info",
        "list_workspaces",
        "create_workspace",
        "list_snapshots",
        "get_snapshot",
        "capture_workspace",
        "restore_workspace",
        "switch_workspace",
        "quick_switch",
        "hide_switcher",
        "cancel_restore",
        "create_shortcut",
        "get_workspace_icon",
        "set_workspace_icon",
        "reset_workspace_icon",
        "update_workspace",
        "list_restore_runs",
        "set_save_mode",
        "set_workspace_archived",
        "get_workspace_shortcuts",
        "delete_workspace",
        "set_pinned_version",
        "delete_snapshot",
        "get_trust_review",
        "approve_workspace",
        "list_running_apps",
        "get_cleanup_config",
        "set_cleanup_config",
        "preview_cleanup",
        "run_cleanup_now",
        "leave_workspace",
        "discover_capture",
        "compare_snapshots",
        "get_settings",
        "get_hotkey_status",
        "save_settings",
        "get_browser_status",
        "install_browser_integration",
        "remove_browser_integration",
        "open_folder",
        "backup_now",
        "export_workspaces",
        "import_workspaces",
        "verify_data",
        "preview_retention",
        "apply_retention",
        "refresh_tray_menu",
        "take_ui_action",
        "health_check",
        "save_diagnostics",
        "current_toast",
    ];
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("failed to run tauri-build");
}

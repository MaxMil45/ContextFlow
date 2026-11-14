//! The desktop shell: the tray (Open, Capture current, Restore recent),
//! restore notifications, and window behaviour.

use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_notification::NotificationExt as _;

use cf_storage::Database;

use crate::dto::{RestoreReport, RestoreStatus};
use crate::state::AppState;

const TRAY_ID: &str = "main";
const MAX_RECENT: usize = 5;
const RESTORE_PREFIX: &str = "restore:";

/// The workspaces the tray offers to restore: the most recently used ones that
/// have a snapshot and can be restored.
fn recent_workspaces(app: &AppHandle) -> Vec<(String, String)> {
    let state = app.state::<AppState>();
    let Ok(db) = Database::open(&state.paths.db) else {
        return Vec::new();
    };
    let Ok(all) = db.list_contexts(false) else {
        return Vec::new();
    };
    all.into_iter()
        .filter(|c| {
            db.is_trusted(c.id).unwrap_or(false)
                && db.list_snapshots(c.id).is_ok_and(|s| !s.is_empty())
        })
        .take(MAX_RECENT)
        .map(|c| (c.id.to_string(), c.name))
        .collect()
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let open = MenuItem::with_id(app, "open", "Open ContextFlow", true, None::<&str>)?;
    let quick = MenuItem::with_id(app, "quick_capture", "Capture now", true, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", "Capture current…", true, None::<&str>)?;

    let recent = recent_workspaces(app);
    let items = recent
        .iter()
        .map(|(id, name)| {
            // The menu item carries the workspace id, so a rename can't redirect it.
            MenuItem::with_id(
                app,
                format!("{RESTORE_PREFIX}{id}"),
                name,
                true,
                None::<&str>,
            )
        })
        .collect::<tauri::Result<Vec<_>>>()?;
    let none = MenuItem::with_id(app, "none", "No workspaces yet", false, None::<&str>)?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = if items.is_empty() {
        vec![&none]
    } else {
        items.iter().map(|i| i as &dyn IsMenuItem<Wry>).collect()
    };
    let restore = Submenu::with_items(app, "Switch to", true, &refs)?;
    // The workspace a Resume would return to, if one is parked.
    let parked = {
        let state = app.state::<AppState>();
        Database::open(&state.paths.db)
            .ok()
            .and_then(|db| cf_app::park::parked(&db).into_iter().next())
    };
    let resume = MenuItem::with_id(
        app,
        "resume",
        parked
            .as_ref()
            .map_or("Resume (nothing parked)".to_string(), |c| {
                format!("Resume “{}”", c.name)
            }),
        parked.is_some(),
        None::<&str>,
    )?;

    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[
            &open as &dyn IsMenuItem<Wry>,
            &quick,
            &capture,
            &restore,
            &resume,
            &sep,
            &quit,
        ],
    )
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("ContextFlow")
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id.as_ref();
            match id {
                "open" => crate::show_main_window(app),
                "quick_capture" => crate::hotkey::quick_capture(app.clone()),
                "capture" => {
                    // The workspace you're in (else the one used last), worked out here
                    // because the window may still be loading.
                    let state = app.state::<AppState>();
                    let target = cf_storage::Database::open(&state.paths.db)
                        .ok()
                        .and_then(|db| crate::hotkey::target_workspace(&db).ok());
                    match target {
                        Some(ctx) => crate::ui::send(
                            app,
                            crate::ui::UiAction {
                                kind: "capture".into(),
                                workspace_id: ctx.id.to_string(),
                                report: None,
                            },
                        ),
                        None => crate::show_main_window(app),
                    }
                }
                "resume" => {
                    let state = app.state::<AppState>();
                    let next = Database::open(&state.paths.db)
                        .ok()
                        .and_then(|db| cf_app::park::take_resume(&db).ok().flatten());
                    if let Some(ctx) = next {
                        crate::ui::send(
                            app,
                            crate::ui::UiAction {
                                kind: "restore".into(),
                                workspace_id: ctx.id.to_string(),
                                report: None,
                            },
                        );
                    }
                    refresh_tray(app);
                }
                "quit" => {
                    // A clean end: no "pick up where you left off" next time.
                    crate::recovery::on_quit(app);
                    app.exit(0);
                }
                _ => {
                    if let Some(workspace_id) = id.strip_prefix(RESTORE_PREFIX) {
                        crate::ui::send(
                            app,
                            crate::ui::UiAction {
                                kind: "restore".into(),
                                workspace_id: workspace_id.to_string(),
                                report: None,
                            },
                        );
                    }
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Rebuild the "Restore recent" list (after workspaces change).
pub fn refresh_tray(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

#[tauri::command]
pub async fn refresh_tray_menu(app: AppHandle) {
    refresh_tray(&app);
}

/// A Windows notification, whatever the notification setting says: used for
/// answers to something you just asked for, like a quick capture.
pub fn toast(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("could not show a notification: {e}");
    }
}

/// A Windows notification when a restore finishes (if the setting is on).
pub fn notify_restore(app: &AppHandle, report: &RestoreReport) {
    if report.dry_run || !app.state::<AppState>().config().app.notifications {
        return;
    }
    // The window already shows the result; a notification would repeat it.
    let in_front = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
    if in_front {
        return;
    }
    let problems: Vec<&str> = report
        .steps
        .iter()
        .filter(|s| !matches!(s.status, RestoreStatus::Success | RestoreStatus::Skipped))
        .map(|s| s.application_id.as_str())
        .collect();
    let (title, body) = if report.cancelled {
        (
            format!("{} restore cancelled", report.workspace),
            "Programs that hadn't started yet were skipped.".to_string(),
        )
    } else {
        match report.overall {
            RestoreStatus::Success => (
                format!("{} restored", report.workspace),
                format!("Version {} is back on your screen.", report.version),
            ),
            RestoreStatus::Failed => (
                format!("{} could not be restored", report.workspace),
                "Open ContextFlow to see why.".to_string(),
            ),
            _ => (
                format!("{} partly restored", report.workspace),
                format!("Needs your attention: {}.", problems.join(", ")),
            ),
        }
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("could not show a notification: {e}");
    }
}

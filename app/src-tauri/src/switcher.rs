//! The workspace switcher: a small floating picker that opens from anywhere
//! (default `Ctrl+Alt+W`). Type a few letters, press Enter, and ContextFlow
//! saves where you are, puts it away, and brings the other workspace back. The
//! result appears on the same on-screen card quick capture uses.
//!
//! The window is created the first time it is needed and then kept hidden, so
//! ContextFlow doesn't hold another web view open until the shortcut is used.

use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

use std::sync::atomic::{AtomicBool, Ordering};

use crate::commands::{run_restore, Feedback};
use crate::dto::{RestoreRequest, RestoreStatus};
use crate::error::CmdResult;
use crate::hotkey::EVENT_CAPTURE_SAVED;
use crate::state::AppState;

const LABEL: &str = "switcher";
const OPEN_EVENT: &str = "switcher://open";
/// Sent to the main window when a switch needs attention, with its report.
pub const REPORT_EVENT: &str = "switch://report";

/// A switch started from the picker is running (the shortcut then cancels it).
static SWITCHING: AtomicBool = AtomicBool::new(false);
const WIDTH: f64 = 520.0;
const HEIGHT: f64 = 400.0;

fn window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Ok(w);
    }
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("switcher.html".into()))
        .title("ContextFlow switcher")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .build()?;
    // Clicking away closes it, like any picker.
    let handle = w.clone();
    w.on_window_event(move |e| {
        if matches!(e, WindowEvent::Focused(false)) {
            let _ = handle.hide();
        }
    });
    Ok(w)
}

/// Centre the picker, a little above the middle, on the screen the mouse is on.
fn place(app: &AppHandle, w: &WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let area = m.work_area();
    let _ = w.set_size(LogicalSize::new(WIDTH, HEIGHT));
    let width = (WIDTH * scale).round() as i32;
    let height = (HEIGHT * scale).round() as i32;
    let x = area.position.x + (area.size.width as i32 - width) / 2;
    let y = area.position.y + ((area.size.height as i32 - height) as f64 * 0.3) as i32;
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

/// Show the picker, or hide it if it is already open (the shortcut toggles).
/// While a switch from the picker is running, the shortcut cancels it instead.
pub fn toggle(app: &AppHandle) {
    if SWITCHING.load(Ordering::SeqCst) {
        let state = app.state::<AppState>();
        if let Ok(slot) = state.restore_cancel.lock() {
            if let Some(flag) = slot.as_ref() {
                flag.store(true, Ordering::Relaxed);
                crate::overlay::show(
                    app,
                    "saving",
                    "Cancelling…",
                    "Programs already started stay open",
                );
            }
        }
        return;
    }
    let Ok(w) = window(app) else {
        crate::shell::toast(
            app,
            "Switcher not available",
            "The switcher window could not be opened.",
        );
        return;
    };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    place(app, &w);
    let _ = w.show();
    let _ = w.set_focus();
    let _ = app.emit_to(LABEL, OPEN_EVENT, ());
}

#[tauri::command]
pub async fn hide_switcher(app: AppHandle) -> CmdResult<()> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
    Ok(())
}

/// Switch from the picker: returns at once and reports on the on-screen card.
#[tauri::command]
pub async fn quick_switch(
    app: AppHandle,
    state: State<'_, AppState>,
    target: String,
) -> CmdResult<()> {
    let _ = state; // the task below fetches its own handle; this only ensures the state exists
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.hide();
    }
    tauri::async_runtime::spawn(async move {
        crate::overlay::show(
            &app,
            "saving",
            "Switching…",
            "Saving where you are, then bringing it back",
        );
        let request = RestoreRequest {
            workspace: target,
            version: None,
            only: None,
            dry_run: Some(false),
        };
        let state = app.state::<AppState>();
        SWITCHING.store(true, Ordering::SeqCst);
        let result = run_restore(app.clone(), &state, request, Feedback::Card).await;
        SWITCHING.store(false, Ordering::SeqCst);
        match result {
            Ok(report) => {
                let secs = report.duration_ms as f64 / 1000.0;
                let problems = report
                    .steps
                    .iter()
                    .filter(|s| {
                        !matches!(s.status, RestoreStatus::Success | RestoreStatus::Skipped)
                    })
                    .count();
                let mut body = format!("{} apps · {secs:.1} s", report.steps.len());
                if problems > 0 {
                    body.push_str(&format!(" · {problems} need attention"));
                }
                if let Some(v) = report.auto_saved_version {
                    body.push_str(&format!(
                        " · saved “{}” v{v}",
                        report.switched_from.as_deref().unwrap_or("previous")
                    ));
                }
                let title = if report.cancelled {
                    format!("Switch to “{}” cancelled", report.workspace)
                } else {
                    format!("“{}” is ready", report.workspace)
                };
                let kind = if problems > 0 || report.cancelled {
                    "error"
                } else {
                    "saved"
                };
                crate::overlay::show(&app, kind, &title, &body);
                if problems > 0 {
                    // Open the window on the full report, with Retry.
                    crate::show_main_window(&app);
                    let _ = app.emit_to("main", REPORT_EVENT, report.clone());
                }
                let _ = app.emit(EVENT_CAPTURE_SAVED, report.workspace);
            }
            Err(e) => {
                crate::overlay::show(&app, "error", "Couldn't switch", &e.message);
            }
        }
    });
    Ok(())
}

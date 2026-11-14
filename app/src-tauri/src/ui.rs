//! The main window, created only when it is needed.
//!
//! Each window is a WebView2 browser, which costs tens of megabytes even when
//! hidden. So the main window is closed for real when you close it (ContextFlow
//! carries on in the tray), it isn't created at all when Windows starts
//! ContextFlow at sign-in, and it is built again the moment something needs it.
//!
//! Because the window may not exist yet, the tray and the switcher don't send it
//! events directly: they leave a [`UiAction`] here and open the window, and the
//! page picks the action up as soon as it has loaded (or straight away when it is
//! already open).

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use ts_rs::TS;

use crate::dto::RestoreReport;
use crate::error::CmdResult;

pub const MAIN: &str = "main";
/// Tells an open window that an action is waiting.
const PENDING_EVENT: &str = "ui://pending";

/// Something the tray or the switcher asked the window to do.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct UiAction {
    /// `capture` (open the capture review), `restore` (switch to it) or
    /// `report` (show a finished switch's report).
    pub kind: String,
    pub workspace_id: String,
    pub report: Option<RestoreReport>,
}

static PENDING: Mutex<Option<UiAction>> = Mutex::new(None);

fn build_main(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("ContextFlow")
        .inner_size(1100.0, 720.0)
        .min_inner_size(760.0, 520.0)
        .center()
        .visible(false)
        .build()
}

/// Bring the main window to the front, creating it if it was closed. Building
/// a window waits for the event loop, so it happens on its own thread and this
/// returns at once (safe to call from tray and shortcut handlers).
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || match build_main(&app) {
        Ok(w) => {
            let _ = w.show();
            let _ = w.set_focus();
        }
        Err(e) => tracing::warn!("could not open the ContextFlow window: {e}"),
    });
}

/// Ask the window to do `action`, opening it if needed.
pub fn send(app: &AppHandle, action: UiAction) {
    if let Ok(mut slot) = PENDING.lock() {
        *slot = Some(action);
    }
    show_main(app);
    let _ = app.emit_to(MAIN, PENDING_EVENT, ());
}

/// The waiting action, if any (each one is handed out once).
#[tauri::command]
pub async fn take_ui_action() -> CmdResult<Option<UiAction>> {
    Ok(PENDING.lock().ok().and_then(|mut s| s.take()))
}

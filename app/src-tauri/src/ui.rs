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

/// The window called `label`, built with `build` if there is none.
///
/// Only one window is found or built at a time. Tauri checks that a label is
/// free when a build starts but records the window only once it exists, so two
/// callers at once (at start-up: opening the window, and the offer to resume)
/// would each build one with the same label. Closing either then leaves the
/// other unable to reach the app ("failed to acquire webview reference").
///
/// Building waits for the event loop: never call this on the event loop's
/// own thread (tray, shortcut and setup handlers); use a thread of its own.
pub(crate) fn find_or_build(
    app: &AppHandle,
    label: &str,
    build: impl FnOnce(&AppHandle) -> tauri::Result<WebviewWindow>,
) -> tauri::Result<WebviewWindow> {
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    match app.get_webview_window(label) {
        Some(w) => Ok(w),
        None => build(app),
    }
}

/// Bring the main window to the front, creating it if it was closed. This
/// returns at once (safe to call from tray and shortcut handlers).
pub fn show_main(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || match find_or_build(&app, MAIN, build_main) {
        Ok(w) => {
            let _ = w.unminimize();
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

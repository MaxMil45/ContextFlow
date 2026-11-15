//! The on-screen confirmation for quick capture: a small card in the corner of
//! the screen that appears the instant the shortcut is pressed ("Saving…"),
//! becomes "Saved …", and fades away. It never takes focus and lets clicks
//! through, so it doesn't get in the way of what you were doing.
//!
//! The window is created when a card is shown and closed when it fades, so no
//! web view stays in memory between captures. It loads `toast.html`, a tiny page
//! with no framework, which asks for the current card as soon as it loads.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalSize, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub const LABEL: &str = "toast";
const EVENT: &str = "toast://show";

/// Card size in logical pixels.
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 92.0;
/// Gap to the screen corner, in logical pixels.
const MARGIN: f64 = 16.0;
/// How long a result stays on screen; problems stay longer so they can be read.
const LINGER: Duration = Duration::from_millis(3200);
const LINGER_PROBLEM: Duration = Duration::from_millis(9000);

/// Each message gets a number, so an older one never hides a newer one.
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize)]
pub struct Message {
    /// `saving`, `saved` or `error`.
    kind: &'static str,
    title: String,
    body: String,
}

/// The card being shown, for a card window that has only just loaded.
static CURRENT: Mutex<Option<Message>> = Mutex::new(None);

fn build(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("toast.html".into()))
        .title("ContextFlow")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()?;
    // Clicks go to whatever is underneath.
    window.set_ignore_cursor_events(true)?;
    Ok(window)
}

/// The card to show, asked for by `toast.html` when it loads.
#[tauri::command]
pub async fn current_toast() -> crate::error::CmdResult<Option<Message>> {
    Ok(CURRENT.lock().ok().and_then(|c| c.clone()))
}

/// Put the card in the bottom-right corner of the screen the mouse is on.
fn place(app: &AppHandle, window: &WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let _ = window.set_size(LogicalSize::new(WIDTH, HEIGHT));
    let w = (WIDTH * scale).round() as i32;
    let h = (HEIGHT * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32;
    let x = area.position.x + area.size.width as i32 - w - margin;
    let y = area.position.y + area.size.height as i32 - h - margin;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// Show `kind` ("saving", "saved" or "error") on screen. The window is built
/// if needed (on its own thread, so this never blocks a shortcut handler).
/// `saved` and `error` fade away by themselves, and then the window is closed.
pub fn show(app: &AppHandle, kind: &'static str, title: &str, body: &str) -> bool {
    let message = Message {
        kind,
        title: title.to_string(),
        body: body.to_string(),
    };
    if let Ok(mut c) = CURRENT.lock() {
        *c = Some(message.clone());
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        // Progress lines arrive close together, each on its own thread: only
        // one of them may build the card.
        let window = match crate::ui::find_or_build(&app, LABEL, build) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("could not show the on-screen card: {e}");
                if kind != "saving" {
                    crate::shell::toast(&app, &message.title, &message.body);
                }
                return;
            }
        };
        place(&app, &window);
        let _ = window.show();
        let _ = app.emit_to(LABEL, EVENT, message);
        if kind != "saving" {
            let linger = if kind == "error" {
                LINGER_PROBLEM
            } else {
                LINGER
            };
            std::thread::sleep(linger);
            if GENERATION.load(Ordering::SeqCst) == generation {
                if let Ok(mut c) = CURRENT.lock() {
                    c.take();
                }
                // Closed, not hidden: a hidden web view still holds its memory.
                let _ = window.close();
            }
        }
    });
    true
}

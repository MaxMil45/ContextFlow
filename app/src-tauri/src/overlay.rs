//! The on-screen confirmation for quick capture: a small card in the corner of
//! the screen that appears the instant the shortcut is pressed ("Saving…"),
//! becomes "Saved …", and fades away. It never takes focus and lets clicks
//! through, so it doesn't get in the way of what you were doing.
//!
//! The window is created hidden at start-up, so showing it is instant.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub const LABEL: &str = "toast";
const EVENT: &str = "toast://show";

/// Card size in logical pixels.
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 92.0;
/// Gap to the screen corner, in logical pixels.
const MARGIN: f64 = 16.0;
/// How long a result stays on screen.
const LINGER: Duration = Duration::from_millis(3200);

/// Each message gets a number, so an older one never hides a newer one.
static GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Serialize)]
struct Message {
    /// `saving`, `saved` or `error`.
    kind: &'static str,
    title: String,
    body: String,
}

/// Create the (hidden) card window. Call once at start-up.
pub fn preload(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html#toast".into()))
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
    Ok(())
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

/// Show `kind` ("saving", "saved" or "error") on screen. Returns `false` if
/// the card window isn't available, so the caller can fall back to a
/// notification. `saved` and `error` fade away by themselves.
pub fn show(app: &AppHandle, kind: &'static str, title: &str, body: &str) -> bool {
    let Some(window) = app.get_webview_window(LABEL) else {
        return false;
    };
    place(app, &window);
    if window.show().is_err() {
        return false;
    }
    let _ = app.emit_to(
        LABEL,
        EVENT,
        Message {
            kind,
            title: title.to_string(),
            body: body.to_string(),
        },
    );
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if kind != "saving" {
        std::thread::spawn(move || {
            std::thread::sleep(LINGER);
            if GENERATION.load(Ordering::SeqCst) == generation {
                let _ = window.hide();
            }
        });
    }
    true
}

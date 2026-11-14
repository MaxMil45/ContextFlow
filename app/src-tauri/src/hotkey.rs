//! Quick capture: a global key combination (default `Ctrl+Alt+X`) saves the
//! active workspace from anywhere, with no window in the way, like a screen-
//! clipping shortcut. A notification says what was saved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use cf_core::Context;
use cf_storage::Database;

use crate::commands::do_capture;
use crate::dto::CaptureRequest;
use crate::error::{CmdResult, CommandError, ErrorCode};
use crate::state::AppState;

/// Sent to the window after a quick capture, so it can refresh.
pub const EVENT_CAPTURE_SAVED: &str = "capture://saved";

/// Holding the keys down repeats the shortcut; ignore anything this soon after a capture.
const REPEAT_GUARD: Duration = Duration::from_millis(1500);

static BUSY: AtomicBool = AtomicBool::new(false);
static LAST_DONE: Mutex<Option<Instant>> = Mutex::new(None);

/// A hotkey as the user typed it, checked. A shortcut with no modifier would
/// fire while typing, so at least one of Ctrl, Alt, Shift or Win is required.
pub fn parse(hotkey: &str) -> Result<Shortcut, String> {
    let shortcut: Shortcut = hotkey.trim().parse().map_err(|e| {
        format!("“{hotkey}” isn't a key combination ContextFlow understands ({e}). Try something like Ctrl+Alt+X.")
    })?;
    if shortcut.mods.is_empty() {
        return Err(
            "A shortcut needs Ctrl, Alt or Shift as well as a key, or it would trigger while you type."
                .into(),
        );
    }
    Ok(shortcut)
}

/// Whether each global shortcut is working; `Some(message)` when it isn't.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HotkeyStatus {
    pub capture: Option<String>,
    pub switcher: Option<String>,
}

impl HotkeyStatus {
    /// One line for a notification, or `None` when both work.
    pub fn summary(&self) -> Option<String> {
        let parts: Vec<&str> = [self.capture.as_deref(), self.switcher.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

fn register_one(
    app: &AppHandle,
    hotkey: &str,
    what: &str,
    action: fn(&AppHandle),
) -> Option<String> {
    if hotkey.trim().is_empty() {
        return None;
    }
    let shortcut = match parse(hotkey) {
        Ok(s) => s,
        Err(e) => return Some(e),
    };
    app.global_shortcut()
        .on_shortcut(shortcut, move |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                action(app);
            }
        })
        .err()
        .map(|e| {
            tracing::warn!("{what} shortcut {hotkey} not available: {e}");
            format!(
                "{hotkey} ({what}) is already used by another program. Choose a different combination in Settings → General."
            )
        })
}

/// Make `capture` the quick-capture shortcut and `switcher` the workspace-switcher
/// shortcut (an empty one is off). Everything ContextFlow registered before is
/// replaced. Each one is set up on its own, so one being taken never stops the other.
pub fn register(app: &AppHandle, capture: &str, switcher: &str) -> HotkeyStatus {
    if let Err(e) = app.global_shortcut().unregister_all() {
        tracing::warn!("could not release the old shortcuts: {e}");
    }
    let status = HotkeyStatus {
        capture: register_one(app, capture, "quick capture", |app| {
            quick_capture(app.clone())
        }),
        switcher: register_one(app, switcher, "switcher", crate::switcher::toggle),
    };
    if let Ok(mut slot) = app.state::<AppState>().hotkeys.lock() {
        *slot = status.clone();
    }
    status
}

/// The workspace a quick capture saves into: the one you are in; if none (or
/// it is archived), the most recently used one.
pub fn target_workspace(db: &Database) -> CmdResult<Context> {
    if let Some(active) = db.active_context()? {
        if let Ok(ctx) = db.find_context(&active.to_string()) {
            if !ctx.is_archived {
                return Ok(ctx);
            }
        }
    }
    db.list_contexts(false)?.into_iter().next().ok_or_else(|| {
        CommandError::new(
            ErrorCode::NotFound,
            "There is no workspace to save into yet. Create one in ContextFlow first.",
        )
    })
}

/// What a notification should say about a saved snapshot.
fn saved_message(report: &crate::dto::CaptureReport, workspace: &str) -> (String, String) {
    let s = &report.snapshot;
    let windows: u32 = s.applications.iter().map(|a| a.window_count).sum();
    let mut body = format!("{} programs, {} windows", s.applications.len(), windows);
    if report.identical_to_previous {
        body.push_str(" (same as the last version)");
    }
    if !report.warnings.is_empty() {
        body.push_str(&format!(
            ". {} warning(s): open ContextFlow to see them",
            report.warnings.len()
        ));
    }
    (format!("Saved “{workspace}” v{}", s.entry.version), body)
}

/// Save the active workspace now. Runs in the background and reports with a
/// notification; never opens a window.
pub fn quick_capture(app: AppHandle) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return; // one is already running
    }
    if LAST_DONE
        .lock()
        .ok()
        .and_then(|t| *t)
        .is_some_and(|t| t.elapsed() < REPEAT_GUARD)
    {
        BUSY.store(false, Ordering::SeqCst);
        return;
    }
    // Answer the key press at once; the capture itself takes a moment.
    crate::overlay::show(&app, "saving", "Saving your workspace…", "");
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = run(&app);
        if let Ok(mut last) = LAST_DONE.lock() {
            *last = Some(Instant::now());
        }
        BUSY.store(false, Ordering::SeqCst);
        match outcome {
            Ok((workspace, report)) => {
                let (title, body) = saved_message(&report, &workspace);
                if !crate::overlay::show(&app, "saved", &title, &body) {
                    crate::shell::toast(&app, &title, &body);
                }
                let _ = app.emit(EVENT_CAPTURE_SAVED, workspace);
            }
            Err(e) => {
                tracing::warn!("quick capture failed: {}", e.message);
                if !crate::overlay::show(&app, "error", "Couldn't save your workspace", &e.message)
                {
                    crate::shell::toast(&app, "Couldn't save your workspace", &e.message);
                }
            }
        }
    });
}

fn run(app: &AppHandle) -> CmdResult<(String, crate::dto::CaptureReport)> {
    let state = app.state::<AppState>();
    let db = Database::open(&state.paths.db)?;
    let ctx = target_workspace(&db)?;
    drop(db);
    let report = do_capture(
        &state.paths.db,
        &state.config(),
        CaptureRequest {
            workspace: ctx.name.clone(),
            note: None,
            unminimize: None,
            only: None,
            repos: None,
        },
    )?;
    Ok((ctx.name, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_need_a_modifier_and_a_real_key() {
        assert!(parse("Ctrl+Alt+X").is_ok());
        assert!(parse("ctrl+shift+f5").is_ok());
        assert!(parse(" Alt+Shift+1 ").is_ok());
        for bad in ["X", "F5", "Ctrl+Alt", "Ctrl+Alt+Nonsense", "", "+++"] {
            assert!(parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn the_target_is_the_active_workspace_else_the_latest_unarchived() {
        let db = Database::open_in_memory().unwrap();
        assert!(target_workspace(&db).is_err(), "no workspaces yet");

        let a = db.create_context("A", None).unwrap();
        let b = db.create_context("B", None).unwrap();
        // Without an active workspace: the first of the app's recent-first list.
        let first = db.list_contexts(false).unwrap()[0].id;
        assert_eq!(target_workspace(&db).unwrap().id, first);

        db.set_active_context(Some(b.id)).unwrap();
        assert_eq!(
            target_workspace(&db).unwrap().id,
            b.id,
            "the active one wins"
        );
        db.set_active_context(Some(a.id)).unwrap();
        assert_eq!(target_workspace(&db).unwrap().id, a.id);

        // An archived active workspace can't be captured into: fall back to the
        // list, which no longer includes it.
        db.set_archived(a.id, true).unwrap();
        assert_eq!(target_workspace(&db).unwrap().id, b.id);
    }
}

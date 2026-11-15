//! Picking up after a restart.
//!
//! While the desktop app runs it keeps a "session open" mark in the database
//! and clears it when you choose Quit. If Windows restarts, signs you out or
//! ContextFlow crashes, the mark is still there next time it starts, so it
//! offers to bring back the workspace you were in.
//!
//! To make that restore point worth having, the workspace you're in is saved
//! quietly every `checkpoint_minutes` while you work. A checkpoint is an
//! ordinary auto-save: skipped when nothing changed, never for a workspace set
//! to "keep it as I captured it", and never while a restore is running.

use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::state::AppState;

const SESSION_KEY: &str = "session_open";

fn open(app: &AppHandle) -> Option<cf_storage::Database> {
    cf_storage::Database::open(&app.state::<AppState>().paths.db).ok()
}

/// Call once at start-up: offer to resume if the last session didn't end with
/// Quit, then mark this one as open.
pub fn on_start(app: &AppHandle) {
    let Some(db) = open(app) else { return };
    let unclean = db.state_value(SESSION_KEY).ok().flatten().is_some();
    let cfg = app.state::<AppState>().config();
    if unclean && cfg.app.offer_resume {
        if let Some(id) = resumable(&db) {
            tracing::info!("the last session didn't end with Quit: offering to resume");
            crate::ui::send(
                app,
                crate::ui::UiAction {
                    kind: "recover".into(),
                    workspace_id: id,
                    report: None,
                },
            );
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(e) = db.set_state_value(SESSION_KEY, Some(&now)) {
        tracing::warn!("could not record the session start: {e}");
    }
}

/// The workspace to offer: the one you were in, if it can still be restored.
fn resumable(db: &cf_storage::Database) -> Option<String> {
    let active = db.active_context().ok().flatten()?;
    let ctx = db.find_context(&active.to_string()).ok()?;
    let ok = !ctx.is_archived
        && db.is_trusted(ctx.id).unwrap_or(false)
        && db.list_snapshots(ctx.id).is_ok_and(|s| !s.is_empty());
    ok.then(|| ctx.id.to_string())
}

/// Call when the user quits: the session ended cleanly.
pub fn on_quit(app: &AppHandle) {
    if let Some(db) = open(app) {
        let _ = db.set_state_value(SESSION_KEY, None);
    }
}

/// Save the workspace you're in every few minutes, in the background.
pub fn start_checkpoints(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        let minutes = app.state::<AppState>().config().app.checkpoint_minutes;
        // Re-read each round, so a change in Settings takes effect.
        std::thread::sleep(Duration::from_secs(60 * u64::from(minutes.max(1))));
        if minutes == 0 {
            continue;
        }
        checkpoint(&app);
    });
}

fn checkpoint(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Never capture a desktop that is half way through a restore, whoever
    // started it: the window and switcher here, or the desktop icon or CLI
    // (served by this app over the pipe, or running on their own).
    if cf_app::session::restore_running() {
        return;
    }
    let cfg = state.config();
    let Ok(mut db) = cf_storage::Database::open(&state.paths.db) else {
        return;
    };
    if db.active_context().ok().flatten().is_none() {
        return;
    }
    let out = cf_app::autosave::save_active(&mut db, &cfg, None, None, &mut |_| {});
    if let Some(v) = out.saved_version {
        tracing::info!(version = v, "checkpoint saved");
        let _ = tauri::Emitter::emit(app, crate::hotkey::EVENT_CAPTURE_SAVED, out.from);
    }
}

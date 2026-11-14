//! Tauri commands for workspace cleanup ("focus rules"): which programs are
//! running, a workspace's rules, previews and running one side now.

use tauri::State;

use crate::commands::{blocking, open};
use crate::dto::*;
use crate::error::{CmdResult, CommandError, ErrorCode};
use crate::state::AppState;

/// Programs running now with their memory and CPU use, for the program picker.
/// Takes about a second: CPU use is measured over a short window.
#[tauri::command]
pub async fn list_running_apps() -> CmdResult<Vec<RunningApp>> {
    blocking(|| {
        Ok(cf_app::session::running_apps_cached()
            .into_iter()
            .map(Into::into)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn get_cleanup_config(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<CleanupConfig> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        Ok(db.cleanup_config(ctx.id)?.into())
    })
    .await
}

/// Save a workspace's cleanup rules. Critical system programs are refused.
#[tauri::command]
pub async fn set_cleanup_config(
    state: State<'_, AppState>,
    workspace: String,
    config: CleanupConfig,
) -> CmdResult<CleanupConfig> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let config: cf_core::CleanupConfig = config.into();
        for t in config.on_enter.iter().chain(&config.on_exit) {
            let name = cf_core::cleanup::normalize_process_name(&t.process)
                .map_err(|e| CommandError::new(ErrorCode::Conflict, e.to_string()))?;
            if let Some(reason) = cf_cleanup::protected_reason(&format!("{name}.exe"), None) {
                return Err(CommandError::new(
                    ErrorCode::Conflict,
                    format!("'{name}' can't be closed by a workspace: {reason}"),
                ));
            }
        }
        let saved = db.set_cleanup_config(ctx.id, config).map_err(|e| match e {
            cf_storage::StorageError::Core(c) => {
                CommandError::new(ErrorCode::Conflict, c.to_string())
            }
            other => other.into(),
        })?;
        Ok(saved.into())
    })
    .await
}

/// What one side of a workspace's cleanup would do right now.
#[tauri::command]
pub async fn preview_cleanup(
    state: State<'_, AppState>,
    workspace: String,
    phase: CleanupPhase,
    fresh: Option<bool>,
) -> CmdResult<CleanupPlan> {
    let path = state.paths.db.clone();
    let always_on = state.config().capture.normalized_always_on();
    blocking(move || {
        let db = open(&path)?;
        let fresh = fresh.unwrap_or(false);
        Ok(cf_app::session::preview_cleanup_sampled(
            &db,
            &workspace,
            phase.into(),
            &always_on,
            fresh,
        )?
        .into())
    })
    .await
}

/// Run one side of a workspace's cleanup now. The UI shows the preview and asks first.
#[tauri::command]
pub async fn run_cleanup_now(
    state: State<'_, AppState>,
    workspace: String,
    phase: CleanupPhase,
) -> CmdResult<CleanupReport> {
    let path = state.paths.db.clone();
    let always_on = state.config().capture.normalized_always_on();
    blocking(move || {
        let db = open(&path)?;
        let report = cf_app::session::run_cleanup_now(
            &db,
            &workspace,
            phase.into(),
            &always_on,
            &mut |_| {},
        )?;
        Ok(report.into())
    })
    .await
}

/// Leave the active workspace: save it, run its "when leaving" cleanup and clear it.
#[tauri::command]
pub async fn leave_workspace(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    note: Option<String>,
) -> CmdResult<LeaveReport> {
    let path = state.paths.db.clone();
    let cfg = state.config();
    blocking(move || {
        let mut db = open(&path)?;
        let out = cf_app::session::leave_active_workspace(
            &mut db,
            Some(&cfg),
            note.as_deref(),
            &mut |_| {},
        )?;
        if let (true, Some(v), Some(from)) =
            (out.saved.notify, out.saved.saved_version, &out.saved.from)
        {
            crate::shell::toast(
                &app,
                &format!("Saved “{from}”"),
                &format!("Version {v}, saved as you left it."),
            );
        }
        Ok(LeaveReport {
            workspace: out.workspace,
            cleanup: out.cleanup.map(Into::into),
            saved_version: out.saved.saved_version,
            save_note: out.saved.note,
        })
    })
    .await
}

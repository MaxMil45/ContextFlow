//! Tauri commands for start and stop commands, park and resume, and the timeline.

use tauri::{AppHandle, State};

use crate::commands::{blocking, open};
use crate::dto::*;
use crate::error::{CmdResult, CommandError, ErrorCode};
use crate::state::AppState;

#[tauri::command]
pub async fn get_workspace_commands(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<WorkspaceCommandsDto> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        Ok(db.workspace_commands(ctx.id)?.into())
    })
    .await
}

/// Save a workspace's start and stop commands. They never run for a workspace
/// that hasn't been approved, and they are never exported.
#[tauri::command]
pub async fn set_workspace_commands(
    state: State<'_, AppState>,
    workspace: String,
    commands: WorkspaceCommandsDto,
) -> CmdResult<WorkspaceCommandsDto> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        if !db.is_trusted(ctx.id)? {
            return Err(CommandError::new(
                ErrorCode::Conflict,
                "approve this imported workspace before giving it commands",
            ));
        }
        let saved = db.set_workspace_commands(ctx.id, commands.into())?;
        tracing::info!("workspace commands changed from the app");
        Ok(saved.into())
    })
    .await
}

/// Park the workspace you're in, to come back to with Resume. The window then
/// switches to another workspace or leaves, which saves it (with the note).
#[tauri::command]
pub async fn park_current(
    app: AppHandle,
    state: State<'_, AppState>,
    note: Option<String>,
) -> CmdResult<String> {
    let path = state.paths.db.clone();
    let name = blocking(move || {
        let db = open(&path)?;
        let active = db.active_context()?.ok_or_else(|| {
            CommandError::new(
                ErrorCode::Conflict,
                "you aren't in a workspace, so there is nothing to park",
            )
        })?;
        let ctx = db.find_context(&active.to_string())?;
        cf_app::park::park(&db, ctx.id, note.as_deref()).map_err(CommandError::from)?;
        Ok(ctx.name)
    })
    .await?;
    crate::shell::refresh_tray(&app);
    Ok(name)
}

/// Take the workspace to come back to; the window then switches to it.
#[tauri::command]
pub async fn take_resume(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<Option<ParkedWorkspace>> {
    let path = state.paths.db.clone();
    let out = blocking(move || {
        let db = open(&path)?;
        Ok(cf_app::park::take_resume(&db)
            .map_err(CommandError::from)?
            .map(|c| ParkedWorkspace {
                id: c.id.to_string(),
                name: c.name,
            }))
    })
    .await?;
    crate::shell::refresh_tray(&app);
    Ok(out)
}

/// The parked workspaces, the one Resume returns to first.
#[tauri::command]
pub async fn list_parked(state: State<'_, AppState>) -> CmdResult<Vec<ParkedWorkspace>> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        Ok(cf_app::park::parked(&db)
            .into_iter()
            .map(|c| ParkedWorkspace {
                id: c.id.to_string(),
                name: c.name,
            })
            .collect())
    })
    .await
}

/// The most recent switches, restores, leaves, parks and resumes.
#[tauri::command]
pub async fn list_timeline(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> CmdResult<Vec<TimelineEntry>> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        Ok(db
            .recent_events(limit.unwrap_or(200).min(1000))?
            .into_iter()
            .map(|e| TimelineEntry {
                at: e.at.to_rfc3339(),
                kind: e.kind.as_str().into(),
                workspace_id: e.workspace_id.map(|i| i.to_string()),
                workspace: e.workspace,
                from_id: e.from_id.map(|i| i.to_string()),
                from: e.from,
                version: e.version,
                note: e.note,
            })
            .collect())
    })
    .await
}

/// Search what every workspace saved: tabs, folders, notes, repositories, windows.
#[tauri::command]
pub async fn search_workspaces(
    state: State<'_, AppState>,
    query: String,
) -> CmdResult<Vec<SearchHit>> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        Ok(cf_app::search::search(&db, &query)
            .map_err(CommandError::from)?
            .into_iter()
            .map(|h| SearchHit {
                workspace_id: h.workspace_id,
                workspace: h.workspace,
                version: h.version,
                kind: h.kind.into(),
                text: h.text,
                detail: h.detail,
            })
            .collect())
    })
    .await
}

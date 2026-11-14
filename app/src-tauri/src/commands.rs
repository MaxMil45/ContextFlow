//! Tauri commands: thin wrappers over `cf-storage`, `cf-app::capture` and
//! `cf-app::session`. Every command that touches the desktop or the database
//! runs on the blocking pool so the UI thread never waits on it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tauri::{AppHandle, Emitter, State};

use cf_app::capture;
use cf_app::session::RestoreArgs;
use cf_core::{Context, Snapshot, SnapshotSummary};
use cf_storage::{Database, RetentionPolicy, StorageError};

use crate::dto::*;
use crate::error::{CmdResult, CommandError, ErrorCode};
use crate::state::AppState;

/// Event name for restore progress lines.
pub const RESTORE_PROGRESS: &str = "restore://progress";

/// Per-application wait for windows after launch (same as the desktop icon).
const APP_WINDOW_TIMEOUT: Duration = Duration::from_secs(30);

/// Run blocking work off the async runtime and flatten the join error.
pub(crate) async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> CmdResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CommandError::new(ErrorCode::Internal, format!("worker failed: {e}")))?
}

pub(crate) fn open(path: &Path) -> CmdResult<Database> {
    Database::open(path).map_err(Into::into)
}

pub(crate) fn latest(db: &Database, ctx: &Context) -> CmdResult<Option<Snapshot>> {
    match db.get_snapshot(ctx.id, None) {
        Ok(s) => Ok(Some(s)),
        Err(StorageError::NotFound(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn entry_from_summary(s: &SnapshotSummary) -> SnapshotEntry {
    SnapshotEntry {
        auto: false,
        version: s.version,
        created_at: s.created_at.to_rfc3339(),
        short_hash: short_hash(&s.hash),
        capture_status: s.capture_status.into(),
    }
}

fn detail(s: &Snapshot) -> SnapshotDetail {
    let p = &s.payload;
    let applications = p
        .applications
        .iter()
        .map(|a| ApplicationEntry {
            application_id: a.application_id.clone(),
            window_count: p
                .windows
                .iter()
                .filter(|w| w.application_id == a.application_id)
                .count() as u32,
            tab_count: cf_browser::BrowserAppState::from_value(&a.adapter_state)
                .map_or(0, |b| b.tab_count() as u32),
            folder_count: cf_vscode::VsCodeState::from_value(&a.adapter_state)
                .map(|v| v.folder_count() as u32)
                .or_else(|| {
                    cf_vstudio::VisualStudioState::from_value(&a.adapter_state)
                        .map(|v| v.path_count() as u32)
                })
                .unwrap_or(0),
        })
        .collect();
    SnapshotDetail {
        entry: SnapshotEntry {
            auto: false,
            version: s.version,
            created_at: s.created_at.to_rfc3339(),
            short_hash: short_hash(&s.hash),
            capture_status: s.capture_status.into(),
        },
        notes: p.notes.clone(),
        applications,
        monitor_count: p.monitors.len() as u32,
        repositories: p
            .git
            .iter()
            .map(|g| RepositoryEntry {
                path: g.repository_path.clone(),
                branch: g.branch.clone(),
                staged: g.staged,
                modified: g.modified,
                untracked: g.untracked,
            })
            .collect(),
    }
}

fn summarize(db: &Database, ctx: &Context) -> CmdResult<WorkspaceSummary> {
    // `list_snapshots` is newest first.
    let latest = db.list_snapshots(ctx.id)?.first().map(entry_from_summary);
    Ok(WorkspaceSummary {
        id: ctx.id.to_string(),
        name: ctx.name.clone(),
        description: ctx.description.clone(),
        updated_at: ctx.updated_at.to_rfc3339(),
        is_archived: ctx.is_archived,
        is_trusted: db.is_trusted(ctx.id)?,
        pinned_version: db.pinned_version(ctx.id)?,
        latest,
        is_active: db.active_context()? == Some(ctx.id),
        save_mode: db.save_mode(ctx.id)?.into(),
    })
}

fn summary_from_overview(o: &cf_storage::ContextOverview) -> WorkspaceSummary {
    WorkspaceSummary {
        id: o.context.id.to_string(),
        name: o.context.name.clone(),
        description: o.context.description.clone(),
        updated_at: o.context.updated_at.to_rfc3339(),
        is_archived: o.context.is_archived,
        is_trusted: o.is_trusted,
        pinned_version: o.pinned_version,
        latest: o.latest.as_ref().map(entry_from_summary),
        is_active: o.is_active,
        save_mode: o.save_mode.into(),
    }
}

#[tauri::command]
pub async fn app_info(state: State<'_, AppState>) -> CmdResult<AppInfo> {
    let p = &state.paths;
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        database_path: p.db.display().to_string(),
        config_path: p.config.display().to_string(),
        logs_path: p.logs.display().to_string(),
    })
}

#[tauri::command]
pub async fn list_workspaces(
    state: State<'_, AppState>,
    include_archived: Option<bool>,
) -> CmdResult<Vec<WorkspaceSummary>> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        // One query, already newest-activity first (what the dashboard shows).
        let out = db
            .list_overview(include_archived.unwrap_or(false))?
            .iter()
            .map(summary_from_overview)
            .collect::<Vec<_>>();
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn create_workspace(
    state: State<'_, AppState>,
    name: String,
    description: Option<String>,
) -> CmdResult<WorkspaceSummary> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let description = description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty());
        let ctx = db.create_context(name.trim(), description)?;
        summarize(&db, &ctx)
    })
    .await
}

#[tauri::command]
pub async fn list_snapshots(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<Vec<SnapshotEntry>> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        // Newest first, as storage returns them.
        let autos = db.auto_versions(ctx.id)?;
        Ok(db
            .list_snapshots(ctx.id)?
            .iter()
            .map(|s| SnapshotEntry {
                auto: autos.contains(&s.version),
                ..entry_from_summary(s)
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn get_snapshot(
    state: State<'_, AppState>,
    workspace: String,
    version: Option<u32>,
) -> CmdResult<SnapshotDetail> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        Ok(detail(&db.get_snapshot(ctx.id, version)?))
    })
    .await
}

/// Same flow as `contextflow capture`: reuse the previous snapshot's repos,
/// save, then apply auto-prune if configured.
#[tauri::command]
pub async fn capture_workspace(
    state: State<'_, AppState>,
    request: CaptureRequest,
) -> CmdResult<CaptureReport> {
    let path = state.paths.db.clone();
    let cfg = state.config();
    blocking(move || do_capture(&path, &cfg, request)).await
}

/// Save a new snapshot of `request.workspace`: reuse the previous snapshot's
/// repositories (unless some are chosen), capture, save, then apply auto-prune
/// if configured. Used by the Capture button and by the quick-capture hotkey.
pub(crate) fn do_capture(
    path: &Path,
    cfg: &cf_config::Config,
    request: CaptureRequest,
) -> CmdResult<CaptureReport> {
    let mut db = open(path)?;
    let ctx = db.find_context(&request.workspace)?;
    let previous = latest(&db, &ctx)?;
    // Repositories: the ones chosen in the review, else those of the last snapshot.
    let repos: Vec<PathBuf> = match &request.repos {
        Some(chosen) => chosen.iter().map(PathBuf::from).collect(),
        None => previous
            .iter()
            .flat_map(|p| p.payload.git.iter())
            .map(|g| PathBuf::from(&g.repository_path))
            .collect(),
    };
    let only: Vec<String> = request.only.clone().unwrap_or_default();
    if request.only.is_some() && only.is_empty() && repos.is_empty() {
        return Err(CommandError::new(
            ErrorCode::Conflict,
            "choose at least one program or repository to save",
        ));
    }
    let notes = request
        .note
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .into_iter()
        .collect();

    let result = capture::capture(capture::CaptureOptions {
        only: &only,
        repos: &repos,
        notes,
        include_windows: cfg.capture.include_windows,
        unminimize: request.unminimize.unwrap_or(false),
        rules: &cfg.capture,
        git_timeout: cfg.timeouts.git(),
        browser: &cfg.browser,
        browser_state_dir: cf_browser::default_state_dir(),
        vscode_storage: cf_vscode::default_storage_json(),
        visual_studio_settings: cf_vstudio::default_settings_files(),
    });

    let snap = db.create_snapshot(ctx.id, result.payload, result.status)?;
    let identical = previous.as_ref().is_some_and(|p| p.hash == snap.hash);
    tracing::info!(workspace = %ctx.name, version = snap.version, "snapshot saved from the app");

    let auto_pruned = if cfg.retention.auto_prune {
        let policy = RetentionPolicy {
            keep_last: cfg.retention.keep_last,
            max_age: cfg
                .retention
                .max_age_days
                .map(|d| chrono::TimeDelta::days(d as i64)),
        };
        db.prune(ctx.id, &policy, Utc::now())?
    } else {
        Vec::new()
    };

    Ok(CaptureReport {
        snapshot: detail(&snap),
        identical_to_previous: identical,
        warnings: result.warnings,
        excluded_windows: result.excluded_windows as u32,
        minimized_windows: result.minimized_windows as u32,
        pinned_version: db.pinned_version(ctx.id)?.filter(|v| *v != snap.version),
        auto_pruned,
    })
}

/// Clears the in-app cancel slot when the restore ends, however it ends.
struct CancelSlotGuard<'a>(&'a AppState);

impl Drop for CancelSlotGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.0.restore_cancel.lock() {
            slot.take();
        }
    }
}

/// Restore a workspace. Progress lines are emitted as `restore://progress`;
/// [`cancel_restore`] stops it before the next application starts.
#[tauri::command]
pub async fn restore_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    request: RestoreRequest,
) -> CmdResult<RestoreReport> {
    run_restore(app, &state, request, Feedback::Window).await
}

/// Switch to a workspace: save the one you are in (unless turned off), leave it,
/// then restore the target. Every real restore does this now (the desktop icon
/// and the CLI too); the separate command keeps the UI's intent explicit.
#[tauri::command]
pub async fn switch_workspace(
    app: AppHandle,
    state: State<'_, AppState>,
    request: RestoreRequest,
) -> CmdResult<RestoreReport> {
    run_restore(app, &state, request, Feedback::Window).await
}

/// How a restore reports while it runs and when it ends.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Feedback {
    /// From the window: progress lines there, a notification if it isn't in front.
    Window,
    /// From the switcher: progress and the result on the on-screen card.
    Card,
}

pub(crate) async fn run_restore(
    app: AppHandle,
    state: &AppState,
    request: RestoreRequest,
    feedback: Feedback,
) -> CmdResult<RestoreReport> {
    let path = state.paths.db.clone();
    let cfg = state.config();
    let notify_app = app.clone();
    let dry_run = request.dry_run.unwrap_or(false);
    let started = std::time::Instant::now();

    // Register a cancel flag for real restores. The cross-process lock inside
    // `cf_app::session::restore_workspace` still guards against the CLI and
    // the desktop icon; this only gives the UI something to cancel.
    let cancel = Arc::new(AtomicBool::new(false));
    let _guard = if dry_run {
        None
    } else {
        let mut slot = state
            .restore_cancel
            .lock()
            .map_err(|_| CommandError::new(ErrorCode::Internal, "restore state poisoned"))?;
        if slot.is_some() {
            return Err(CommandError::new(
                ErrorCode::RestoreBusy,
                "a restore is already running",
            ));
        }
        *slot = Some(cancel.clone());
        Some(CancelSlotGuard(state))
    };

    let result = blocking(move || {
        let mut db = open(&path)?;
        let workspace = request.workspace.clone();
        let mut log = |m: &str| {
            if feedback == Feedback::Card {
                crate::overlay::show(&app, "saving", "Switching…", m);
            }
            let _ = app.emit(
                RESTORE_PROGRESS,
                RestoreProgress {
                    workspace: workspace.clone(),
                    message: m.to_string(),
                },
            );
        };
        let args = RestoreArgs {
            version: request.version,
            only: request.only.clone().unwrap_or_default(),
            dry_run,
            timeout: APP_WINDOW_TIMEOUT,
            max_tabs_per_window: cfg.browser.max_tabs_per_window,
            overall_timeout: cfg.timeouts.restore_overall(),
            cancel: Some(cancel.clone()),
            skip_cleanup: false,
            // Every real restore of another workspace is a switch, wherever it starts.
            save_before_leaving: Some(cfg.clone()),
        };
        let outcome =
            cf_app::session::restore_workspace(&mut db, &request.workspace, &args, &mut log)?;
        let left = outcome.left;
        // "Save and tell me": say so even if the window isn't open.
        if let (true, Some(v), Some(from)) = (left.notify, left.saved_version, &left.from) {
            crate::shell::toast(
                &app,
                &format!("Saved “{from}”"),
                &format!("Version {v}, saved as you switched away."),
            );
        }
        let (ctx, snap, run) = (outcome.context, outcome.snapshot, outcome.run);
        Ok(RestoreReport {
            workspace_id: ctx.id.to_string(),
            workspace: ctx.name,
            version: snap.version,
            dry_run,
            cancelled: cancel.load(Ordering::Relaxed),
            note: snap.payload.notes.last().cloned(),
            overall: run.overall.into(),
            steps: run
                .steps
                .into_iter()
                .map(|s| RestoreStep {
                    application_id: s.application_id,
                    status: s.status.into(),
                    code: s.code.map(String::from),
                    message: s.message,
                    duration_ms: s.duration_ms,
                })
                .collect(),
            cleanups: outcome.cleanups.into_iter().map(Into::into).collect(),
            duration_ms: started.elapsed().as_millis() as u64,
            saved_at: snap.created_at.to_rfc3339(),
            repositories: detail(&snap).repositories,
            switched_from: left.from,
            auto_saved_version: left.saved_version,
            auto_save_note: left.note,
        })
    })
    .await;
    if let (Feedback::Window, Ok(report)) = (feedback, &result) {
        crate::shell::notify_restore(&notify_app, report);
    }
    result
}

/// Ask the running restore to stop. Returns `false` if none is running.
/// Applications already started are left open; nothing is ever force-closed.
#[tauri::command]
pub async fn cancel_restore(state: State<'_, AppState>) -> CmdResult<bool> {
    let slot = state
        .restore_cancel
        .lock()
        .map_err(|_| CommandError::new(ErrorCode::Internal, "restore state poisoned"))?;
    Ok(match slot.as_ref() {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    })
}

/// Put an icon on the desktop that restores `workspace` with a double-click
/// (same as `contextflow shortcut`). With `overwrite` off, an existing icon of
/// the same name is reported as `already_exists` so the UI can ask first.
#[tauri::command]
pub async fn create_shortcut(
    state: State<'_, AppState>,
    workspace: String,
    overwrite: Option<bool>,
) -> CmdResult<ShortcutCreated> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        if latest(&db, &ctx)?.is_none() {
            return Err(CommandError::new(
                ErrorCode::Conflict,
                format!("'{}' has no snapshot yet; capture it first", ctx.name),
            ));
        }
        // A non-default database or config is remembered in the icon, so it
        // opens the same data the app is showing.
        let db_arg = std::env::var_os("CONTEXTFLOW_DB").map(|_| path.clone());
        let config_arg = std::env::var_os("CONTEXTFLOW_CONFIG")
            .map(PathBuf::from)
            .filter(|p| p.exists());
        let id = ctx.id.to_string();
        let icon = cf_app::icons::custom_icon(&cf_app::icons::icon_dir(&path), &id);
        let created = cf_app::shortcut::create(&cf_app::shortcut::ShortcutSpec {
            workspace_id: &id,
            display_name: &ctx.name,
            folder: None,
            db: db_arg.as_deref(),
            config: config_arg.as_deref(),
            overwrite: overwrite.unwrap_or(false),
            icon: icon.as_deref(),
        })?;
        tracing::info!(workspace = %ctx.name, "desktop shortcut created from the app");
        Ok(ShortcutCreated {
            path: created.display().to_string(),
        })
    })
    .await
}

#[tauri::command]
pub async fn rename_workspace(
    state: State<'_, AppState>,
    workspace: String,
    new_name: String,
) -> CmdResult<WorkspaceSummary> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let renamed = db.rename_context(ctx.id, &new_name)?;
        tracing::info!("workspace renamed from the app");
        summarize(&db, &renamed)
    })
    .await
}

/// Change a workspace's name and description together. Its desktop icons that
/// still carry the old name are renamed to match.
#[tauri::command]
pub async fn update_workspace(
    state: State<'_, AppState>,
    workspace: String,
    name: String,
    description: Option<String>,
) -> CmdResult<WorkspaceUpdate> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let mut icon_error = None;
        let name = name.trim();
        if name != ctx.name {
            db.rename_context(ctx.id, name)?;
            if let Err(e) =
                cf_app::shortcut::rename_for_workspace(&ctx.id.to_string(), &ctx.name, name)
            {
                tracing::warn!("desktop icon not renamed: {e:#}");
                icon_error = Some(format!("{e:#}"));
            }
        }
        let updated = db.set_description(ctx.id, description.as_deref())?;
        tracing::info!("workspace edited from the app");
        Ok(WorkspaceUpdate {
            workspace: summarize(&db, &updated)?,
            icon_error,
        })
    })
    .await
}

/// Choose what happens to a workspace when you switch away from it or leave it.
#[tauri::command]
pub async fn set_save_mode(
    state: State<'_, AppState>,
    workspace: String,
    mode: SaveMode,
) -> CmdResult<WorkspaceSummary> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        db.set_save_mode(ctx.id, mode.into())?;
        summarize(&db, &ctx)
    })
    .await
}

/// The most recent restores of a workspace, newest first, with what went wrong.
#[tauri::command]
pub async fn list_restore_runs(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<Vec<RestoreRunEntry>> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let status = |s: &str| -> RestoreStatus {
            // Stored with `RestoreStatus::as_str`, which is its serde name.
            serde_json::from_value::<cf_core::RestoreStatus>(serde_json::Value::from(s))
                .map(Into::into)
                .unwrap_or(RestoreStatus::Failed)
        };
        Ok(db
            .recent_restore_runs(ctx.id, 10)?
            .into_iter()
            .map(|r| RestoreRunEntry {
                started_at: r.started_at.to_rfc3339(),
                version: r.version,
                overall: status(&r.overall_status),
                steps: r.steps as u32,
                problems: r
                    .problems
                    .into_iter()
                    .map(|(app, st, message)| RestoreProblem {
                        application_id: app,
                        status: status(&st),
                        message,
                    })
                    .collect(),
            })
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn set_workspace_archived(
    state: State<'_, AppState>,
    workspace: String,
    archived: bool,
) -> CmdResult<WorkspaceSummary> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        db.set_archived(ctx.id, archived)?;
        summarize(&db, &db.find_context(&ctx.id.to_string())?)
    })
    .await
}

/// Desktop icons that restore this workspace, for the delete dialog.
#[tauri::command]
pub async fn get_workspace_shortcuts(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<Vec<String>> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        Ok(
            cf_app::shortcut::find_for_workspace(&ctx.id.to_string(), None)?
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        )
    })
    .await
}

/// Permanently delete a workspace and all its snapshots, and (if asked) move
/// its desktop icons to the Recycle Bin. The UI asks first.
#[tauri::command]
pub async fn delete_workspace(
    state: State<'_, AppState>,
    workspace: String,
    delete_shortcuts: Option<bool>,
) -> CmdResult<DeleteOutcome> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let id = ctx.id.to_string();
        // Look first: after the delete the id is gone, but the icons stay.
        let mut lookup_error = None;
        let shortcuts = if delete_shortcuts.unwrap_or(false) {
            cf_app::shortcut::find_for_workspace(&id, None).unwrap_or_else(|e| {
                lookup_error = Some(format!("{e:#}"));
                Vec::new()
            })
        } else {
            Vec::new()
        };
        db.delete_context(ctx.id)?;
        // The picture kept for its desktop icon is of no use any more.
        cf_app::icons::clear_custom_icon(&cf_app::icons::icon_dir(&path), &id);
        tracing::info!(workspace = %ctx.name, "workspace deleted from the app");
        // The workspace is gone either way; a problem with the icons is reported, not fatal.
        let shortcut_error = match cf_app::shortcut::recycle(&shortcuts) {
            Ok(()) => lookup_error,
            Err(e) => Some(format!("{e:#}")),
        };
        Ok(DeleteOutcome {
            shortcuts_removed: if shortcut_error.is_some() {
                Vec::new()
            } else {
                shortcuts.iter().map(|p| p.display().to_string()).collect()
            },
            shortcut_error,
        })
    })
    .await
}

/// Make restore (and the desktop icon) always use `version`; `None` goes back
/// to the latest snapshot.
#[tauri::command]
pub async fn set_pinned_version(
    state: State<'_, AppState>,
    workspace: String,
    version: Option<u32>,
) -> CmdResult<WorkspaceSummary> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        if let Some(v) = version {
            // Fail with "not found" now rather than at the next restore.
            db.get_snapshot(ctx.id, Some(v))?;
        }
        db.set_pinned_version(ctx.id, version)?;
        summarize(&db, &ctx)
    })
    .await
}

/// Permanently delete one snapshot. A pinned version is refused by storage.
#[tauri::command]
pub async fn delete_snapshot(
    state: State<'_, AppState>,
    workspace: String,
    version: u32,
) -> CmdResult<()> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        db.delete_snapshot(ctx.id, version)?;
        Ok(())
    })
    .await
}

/// Every program the workspace could start, for the approval dialog.
#[tauri::command]
pub async fn get_trust_review(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<TrustReview> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let mut programs: std::collections::BTreeMap<(String, String, String), Vec<u32>> =
            Default::default();
        for s in db.list_snapshots(ctx.id)? {
            let snapshot = db.get_snapshot(ctx.id, Some(s.version))?;
            for a in &snapshot.payload.applications {
                programs
                    .entry((
                        a.application_id.clone(),
                        a.executable_path.clone(),
                        a.launch_arguments.join(" "),
                    ))
                    .or_default()
                    .push(s.version);
            }
        }
        Ok(TrustReview {
            workspace: ctx.name,
            programs: programs
                .into_iter()
                .map(
                    |((application_id, executable_path, arguments), versions)| ProgramEntry {
                        application_id,
                        executable_path,
                        arguments,
                        versions,
                    },
                )
                .collect(),
        })
    })
    .await
}

/// Approve an imported workspace for restoring. Only called from the approval
/// dialog, after the user has seen the list from [`get_trust_review`].
#[tauri::command]
pub async fn approve_workspace(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<WorkspaceSummary> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        db.set_trusted(ctx.id, true)?;
        tracing::info!(workspace = %ctx.name, "workspace approved from the app");
        summarize(&db, &ctx)
    })
    .await
}

fn icon_info(icon: Option<&Path>) -> CmdResult<WorkspaceIcon> {
    use base64::Engine as _;
    let Some(icon) = icon else {
        return Ok(WorkspaceIcon {
            custom: false,
            data_url: None,
        });
    };
    let bytes = std::fs::read(icon)
        .map_err(|e| CommandError::new(ErrorCode::Internal, format!("reading the icon: {e}")))?;
    Ok(WorkspaceIcon {
        custom: true,
        data_url: Some(format!(
            "data:image/x-icon;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )),
    })
}

/// Set (`picture = Some`) or reset (`None`) a workspace's icon, and change it
/// on its desktop shortcut if there is one.
fn apply_icon(db_path: &Path, workspace: &str, picture: Option<&[u8]>) -> CmdResult<IconUpdate> {
    let db = open(db_path)?;
    let ctx = db.find_context(workspace)?;
    let id = ctx.id.to_string();
    let dir = cf_app::icons::icon_dir(db_path);
    let icon = match picture {
        Some(bytes) => Some(
            cf_app::icons::set_custom_icon(&dir, &id, bytes)
                .map_err(|e| CommandError::new(ErrorCode::Conflict, format!("{e:#}")))?,
        ),
        None => {
            cf_app::icons::clear_custom_icon(&dir, &id);
            None
        }
    };
    let shortcuts = cf_app::shortcut::update_icon(&id, None, icon.as_deref())?;
    Ok(IconUpdate {
        icon: icon_info(icon.as_deref())?,
        shortcut_path: shortcuts.first().map(|p| p.display().to_string()),
    })
}

#[tauri::command]
pub async fn get_workspace_icon(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<WorkspaceIcon> {
    let path = state.paths.db.clone();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let icon = cf_app::icons::custom_icon(&cf_app::icons::icon_dir(&path), &ctx.id.to_string());
        icon_info(icon.as_deref())
    })
    .await
}

/// The picture arrives as the raw request body (no JSON number arrays for a
/// multi-megabyte photo); the workspace name, URI-encoded, in `x-workspace`.
#[tauri::command]
pub async fn set_workspace_icon(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> CmdResult<IconUpdate> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(CommandError::new(
            ErrorCode::Internal,
            "expected the picture as raw bytes",
        ));
    };
    let workspace = request
        .headers()
        .get("x-workspace")
        .and_then(|v| v.to_str().ok())
        .map(percent_decode)
        .ok_or_else(|| CommandError::new(ErrorCode::Internal, "missing x-workspace header"))?;
    let bytes = bytes.clone();
    let path = state.paths.db.clone();
    blocking(move || apply_icon(&path, &workspace, Some(&bytes))).await
}

#[tauri::command]
pub async fn reset_workspace_icon(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<IconUpdate> {
    let path = state.paths.db.clone();
    blocking(move || apply_icon(&path, &workspace, None)).await
}

/// Undo `encodeURIComponent` (headers must be ASCII; names may not be).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn workspace_summary_reports_the_newest_snapshot_as_latest() {
        let mut db = cf_storage::Database::open_in_memory().unwrap();
        let ctx = db.create_context("W", None).unwrap();
        for n in 1..=3 {
            let mut payload = cf_core::SnapshotPayload::default();
            payload.notes.push(format!("note {n}"));
            db.create_snapshot(ctx.id, payload, cf_core::CaptureStatus::Complete)
                .unwrap();
        }
        let summary = super::summarize(&db, &ctx).unwrap();
        assert_eq!(summary.latest.unwrap().version, 3);
    }

    #[test]
    fn percent_decoding_undoes_encode_uri_component() {
        use super::percent_decode;
        assert_eq!(percent_decode("Chess"), "Chess");
        assert_eq!(
            percent_decode("Uni%20Dev%3A%20%C3%A9t%C3%A9"),
            "Uni Dev: été"
        );
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%2"), "%2");
    }
}

//! Workspaces, snapshots, capture, restore, icons and approval.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::CleanupReport;
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct WorkspaceSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// RFC 3339.
    pub updated_at: String,
    pub is_archived: bool,
    /// Imported workspaces must be approved (CLI: `workspace trust`) before restore.
    pub is_trusted: bool,
    pub pinned_version: Option<u32>,
    pub latest: Option<SnapshotEntry>,
    /// The workspace last restored: the one you are "in".
    pub is_active: bool,
    /// What happens to it when you switch away from it or leave it.
    pub save_mode: SaveMode,
}

/// What happens to a workspace when you switch away from it or leave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum SaveMode {
    /// Save its changes.
    Auto,
    /// Save its changes and show a notification.
    Notify,
    /// Never save automatically ("one and done"): it always restores as captured.
    Fixed,
}

impl From<cf_storage::SaveMode> for SaveMode {
    fn from(m: cf_storage::SaveMode) -> Self {
        match m {
            cf_storage::SaveMode::Auto => Self::Auto,
            cf_storage::SaveMode::Notify => Self::Notify,
            cf_storage::SaveMode::Fixed => Self::Fixed,
        }
    }
}

impl From<SaveMode> for cf_storage::SaveMode {
    fn from(m: SaveMode) -> Self {
        match m {
            SaveMode::Auto => Self::Auto,
            SaveMode::Notify => Self::Notify,
            SaveMode::Fixed => Self::Fixed,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct SnapshotEntry {
    /// Saved automatically when switching away or leaving.
    #[serde(default)]
    pub auto: bool,
    pub version: u32,
    /// RFC 3339.
    pub created_at: String,
    /// First 12 hex digits of the snapshot hash.
    pub short_hash: String,
    pub capture_status: CaptureStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum CaptureStatus {
    Complete,
    Partial,
    Failed,
}

impl From<cf_core::CaptureStatus> for CaptureStatus {
    fn from(s: cf_core::CaptureStatus) -> Self {
        match s {
            cf_core::CaptureStatus::Complete => Self::Complete,
            cf_core::CaptureStatus::Partial => Self::Partial,
            cf_core::CaptureStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct SnapshotDetail {
    #[serde(flatten)]
    #[ts(flatten)]
    pub entry: SnapshotEntry,
    pub notes: Vec<String>,
    pub applications: Vec<ApplicationEntry>,
    pub monitor_count: u32,
    pub repositories: Vec<RepositoryEntry>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct ApplicationEntry {
    pub application_id: String,
    pub window_count: u32,
    /// Browser tabs saved (Chrome/Edge with the extension), else 0.
    pub tab_count: u32,
    /// VS Code folders saved, else 0.
    pub folder_count: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RepositoryEntry {
    pub path: String,
    pub branch: Option<String>,
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CaptureRequest {
    pub workspace: String,
    #[ts(optional)]
    pub note: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub unminimize: Option<bool>,
    /// Save only these programs (from the capture review). Default: all.
    #[serde(default)]
    #[ts(optional)]
    pub only: Option<Vec<String>>,
    /// Git repositories to record (full paths). Default: those of the last snapshot.
    #[serde(default)]
    #[ts(optional)]
    pub repos: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CaptureReport {
    pub snapshot: SnapshotDetail,
    /// The new snapshot hashes the same as the previous one.
    pub identical_to_previous: bool,
    pub warnings: Vec<String>,
    pub excluded_windows: u32,
    pub minimized_windows: u32,
    /// Restore still uses this version, not the new one.
    pub pinned_version: Option<u32>,
    pub auto_pruned: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RestoreRequest {
    pub workspace: String,
    /// Default: the pinned version, else the latest.
    #[ts(optional)]
    pub version: Option<u32>,
    #[serde(default)]
    #[ts(optional)]
    pub dry_run: Option<bool>,
    /// Restore only these programs (retry what failed). Cleanup is skipped.
    #[serde(default)]
    #[ts(optional)]
    pub only: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum RestoreStatus {
    Success,
    PartialSuccess,
    Failed,
    Skipped,
    RequiresUserIntervention,
}

impl From<cf_core::RestoreStatus> for RestoreStatus {
    fn from(s: cf_core::RestoreStatus) -> Self {
        use cf_core::RestoreStatus as S;
        match s {
            S::Success => Self::Success,
            S::PartialSuccess => Self::PartialSuccess,
            S::Failed => Self::Failed,
            S::Skipped => Self::Skipped,
            S::RequiresUserIntervention => Self::RequiresUserIntervention,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RestoreStep {
    pub application_id: String,
    pub status: RestoreStatus,
    pub code: Option<String>,
    pub message: String,
    #[ts(type = "number")]
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RestoreReport {
    pub workspace: String,
    pub workspace_id: String,
    pub version: u32,
    pub dry_run: bool,
    pub cancelled: bool,
    /// The snapshot's last progress note, shown as the "resume card".
    pub note: Option<String>,
    pub overall: RestoreStatus,
    pub steps: Vec<RestoreStep>,
    /// Programs closed before the restore (leaving the old workspace, entering this one).
    pub cleanups: Vec<CleanupReport>,
    /// Seconds-level measure of "keystroke to ready": the whole operation, in ms.
    #[ts(type = "number")]
    pub duration_ms: u64,
    /// When the restored snapshot was saved (RFC 3339), for "where you left off".
    pub saved_at: String,
    /// Git state saved in the restored snapshot.
    pub repositories: Vec<RepositoryEntry>,
    /// Set by a switch: the workspace that was left.
    pub switched_from: Option<String>,
    /// Set by a switch: the version number saved for the workspace that was left.
    pub auto_saved_version: Option<u32>,
    /// Why the workspace that was left was not saved, when it wasn't.
    pub auto_save_note: Option<String>,
}

/// Payload of the `restore://progress` event.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RestoreProgress {
    pub workspace: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct AppInfo {
    pub version: String,
    pub database_path: String,
    pub config_path: String,
    pub logs_path: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct ShortcutCreated {
    /// Full path of the `.lnk` file.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct WorkspaceIcon {
    /// `false` = the standard ContextFlow icon.
    pub custom: bool,
    /// `data:image/x-icon;base64,...` of the custom icon, for previews.
    pub data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct IconUpdate {
    pub icon: WorkspaceIcon,
    /// The desktop shortcut whose icon was changed too, if there is one.
    pub shortcut_path: Option<String>,
}

/// One program a workspace could start, across all of its snapshots.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct ProgramEntry {
    pub application_id: String,
    pub executable_path: String,
    pub arguments: String,
    /// Snapshot versions that contain it.
    pub versions: Vec<u32>,
}

/// What approving an imported workspace would allow. Approval covers every
/// version (`restore` can pick any), so all of them are listed.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct TrustReview {
    pub workspace: String,
    pub programs: Vec<ProgramEntry>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct DeleteOutcome {
    /// Desktop icons moved to the Recycle Bin.
    pub shortcuts_removed: Vec<String>,
    /// Set when the workspace is deleted but its icons could not be removed.
    pub shortcut_error: Option<String>,
}

/// One past restore of a workspace.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RestoreRunEntry {
    /// RFC 3339.
    pub started_at: String,
    pub version: u32,
    pub overall: RestoreStatus,
    pub steps: u32,
    pub problems: Vec<RestoreProblem>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RestoreProblem {
    pub application_id: String,
    pub status: RestoreStatus,
    pub message: Option<String>,
}

/// Result of editing a workspace's name or description.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct WorkspaceUpdate {
    pub workspace: WorkspaceSummary,
    /// Set when its desktop icon could not be renamed to match.
    pub icon_error: Option<String>,
}

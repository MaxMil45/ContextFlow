//! Data sent to the UI. These are deliberately separate from the `cf-core`
//! model: the domain crates stay free of UI concerns, and the IPC contract can
//! evolve without touching stored snapshots. TypeScript types are generated
//! from these structs into `app/src/bindings/` by `cargo test -p contextflow-app`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

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
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct SnapshotEntry {
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
    pub version: u32,
    pub dry_run: bool,
    pub cancelled: bool,
    /// The snapshot's last progress note, shown as the "resume card".
    pub note: Option<String>,
    pub overall: RestoreStatus,
    pub steps: Vec<RestoreStep>,
    /// Programs closed before the restore (leaving the old workspace, entering this one).
    pub cleanups: Vec<CleanupReport>,
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

// ----------------------------------------------------------------- cleanup

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum CloseMode {
    /// Ask the program to close; leave it if it stays open.
    Graceful,
    /// Ask first, then end it if it is still running (unsaved work is lost).
    Force,
}

impl From<cf_core::CloseMode> for CloseMode {
    fn from(m: cf_core::CloseMode) -> Self {
        match m {
            cf_core::CloseMode::Graceful => Self::Graceful,
            cf_core::CloseMode::Force => Self::Force,
        }
    }
}

impl From<CloseMode> for cf_core::CloseMode {
    fn from(m: CloseMode) -> Self {
        match m {
            CloseMode::Graceful => Self::Graceful,
            CloseMode::Force => Self::Force,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CleanupTarget {
    /// Program name without `.exe`, e.g. `discord`.
    pub process: String,
    pub mode: CloseMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CleanupConfig {
    pub on_enter: Vec<CleanupTarget>,
    pub on_exit: Vec<CleanupTarget>,
}

impl From<cf_core::CleanupConfig> for CleanupConfig {
    fn from(c: cf_core::CleanupConfig) -> Self {
        let conv = |v: Vec<cf_core::CleanupTarget>| {
            v.into_iter()
                .map(|t| CleanupTarget {
                    process: t.process,
                    mode: t.mode.into(),
                })
                .collect()
        };
        Self {
            on_enter: conv(c.on_enter),
            on_exit: conv(c.on_exit),
        }
    }
}

impl From<CleanupConfig> for cf_core::CleanupConfig {
    fn from(c: CleanupConfig) -> Self {
        let conv = |v: Vec<CleanupTarget>| {
            v.into_iter()
                .map(|t| cf_core::CleanupTarget {
                    process: t.process,
                    mode: t.mode.into(),
                })
                .collect()
        };
        Self {
            on_enter: conv(c.on_enter),
            on_exit: conv(c.on_exit),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum CleanupPhase {
    Entering,
    Leaving,
}

impl From<CleanupPhase> for cf_cli::session::CleanupPhase {
    fn from(p: CleanupPhase) -> Self {
        match p {
            CleanupPhase::Entering => Self::Entering,
            CleanupPhase::Leaving => Self::Leaving,
        }
    }
}

impl From<cf_cli::session::CleanupPhase> for CleanupPhase {
    fn from(p: cf_cli::session::CleanupPhase) -> Self {
        match p {
            cf_cli::session::CleanupPhase::Entering => Self::Entering,
            cf_cli::session::CleanupPhase::Leaving => Self::Leaving,
        }
    }
}

/// A program that is running now.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RunningApp {
    pub process: String,
    pub display_name: String,
    pub process_count: u32,
    #[ts(type = "number")]
    pub ram_bytes: u64,
    pub cpu_percent: f32,
    /// Why cleanup would refuse to close it, if it would.
    pub protected: Option<String>,
}

impl From<cf_cleanup::RunningApp> for RunningApp {
    fn from(a: cf_cleanup::RunningApp) -> Self {
        Self {
            process: a.process,
            display_name: a.display_name,
            process_count: a.process_count as u32,
            ram_bytes: a.ram_bytes,
            cpu_percent: a.cpu_percent,
            protected: a.protected,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum PlannedAction {
    Close {
        #[serde(rename = "processCount")]
        #[ts(rename = "processCount")]
        process_count: u32,
        #[serde(rename = "ramBytes")]
        #[ts(rename = "ramBytes", type = "number")]
        ram_bytes: u64,
        #[serde(rename = "cpuPercent")]
        #[ts(rename = "cpuPercent")]
        cpu_percent: f32,
    },
    NotRunning,
    Protected {
        reason: String,
    },
    KeptByWorkspace,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct PlanItem {
    pub process: String,
    pub display_name: String,
    pub mode: CloseMode,
    pub action: PlannedAction,
}

/// What a cleanup would do, and what it would give back.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CleanupPlan {
    pub items: Vec<PlanItem>,
    #[ts(type = "number")]
    pub ram_bytes: u64,
    pub cpu_percent: f32,
}

impl From<cf_cleanup::CleanupPlan> for CleanupPlan {
    fn from(p: cf_cleanup::CleanupPlan) -> Self {
        Self {
            ram_bytes: p.ram_bytes,
            cpu_percent: p.cpu_percent,
            items: p
                .items
                .into_iter()
                .map(|i| PlanItem {
                    process: i.process,
                    display_name: i.display_name,
                    mode: i.mode.into(),
                    action: match i.action {
                        cf_cleanup::PlannedAction::Close {
                            process_count,
                            ram_bytes,
                            cpu_percent,
                        } => PlannedAction::Close {
                            process_count: process_count as u32,
                            ram_bytes,
                            cpu_percent,
                        },
                        cf_cleanup::PlannedAction::NotRunning => PlannedAction::NotRunning,
                        cf_cleanup::PlannedAction::Protected { reason } => {
                            PlannedAction::Protected { reason }
                        }
                        cf_cleanup::PlannedAction::KeptByWorkspace => {
                            PlannedAction::KeptByWorkspace
                        }
                    },
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum CleanupOutcome {
    Closed,
    Terminated,
    StillRunning,
    NotRunning,
    Protected,
    KeptByWorkspace,
    Failed,
}

impl From<cf_cleanup::Outcome> for CleanupOutcome {
    fn from(o: cf_cleanup::Outcome) -> Self {
        use cf_cleanup::Outcome as O;
        match o {
            O::Closed => Self::Closed,
            O::Terminated => Self::Terminated,
            O::StillRunning => Self::StillRunning,
            O::NotRunning => Self::NotRunning,
            O::Protected => Self::Protected,
            O::KeptByWorkspace => Self::KeptByWorkspace,
            O::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CleanupResult {
    pub process: String,
    pub mode: CloseMode,
    pub outcome: CleanupOutcome,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CleanupReport {
    pub phase: CleanupPhase,
    /// The workspace whose rules were applied.
    pub workspace: String,
    pub results: Vec<CleanupResult>,
}

impl From<cf_cli::session::CleanupReport> for CleanupReport {
    fn from(r: cf_cli::session::CleanupReport) -> Self {
        Self {
            phase: r.phase.into(),
            workspace: r.workspace,
            results: r
                .results
                .into_iter()
                .map(|t| CleanupResult {
                    process: t.process,
                    mode: t.mode.into(),
                    outcome: t.outcome.into(),
                    message: t.message,
                })
                .collect(),
        }
    }
}

pub use crate::dto_more::*;

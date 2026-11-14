//! Workspace cleanup ("focus rules"): running programs, plans and results.

use serde::{Deserialize, Serialize};
use ts_rs::TS;
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

impl From<CleanupPhase> for cf_app::session::CleanupPhase {
    fn from(p: CleanupPhase) -> Self {
        match p {
            CleanupPhase::Entering => Self::Entering,
            CleanupPhase::Leaving => Self::Leaving,
        }
    }
}

impl From<cf_app::session::CleanupPhase> for CleanupPhase {
    fn from(p: cf_app::session::CleanupPhase) -> Self {
        match p {
            cf_app::session::CleanupPhase::Entering => Self::Entering,
            cf_app::session::CleanupPhase::Leaving => Self::Leaving,
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
    /// The executable files running under this name.
    pub executables: Vec<String>,
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
            executables: a.executables,
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
    /// Executable files the rule matches right now.
    pub executables: Vec<String>,
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
                    executables: i.executables,
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

impl From<cf_app::session::CleanupReport> for CleanupReport {
    fn from(r: cf_app::session::CleanupReport) -> Self {
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

/// The result of leaving a workspace without entering another.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct LeaveReport {
    /// The workspace that was left; `null` when none was active.
    pub workspace: Option<String>,
    pub cleanup: Option<CleanupReport>,
    /// The version saved before leaving, if one was.
    pub saved_version: Option<u32>,
    /// Why nothing was saved, when it wasn't.
    pub save_note: Option<String>,
}

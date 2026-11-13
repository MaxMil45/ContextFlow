use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Version of the [`SnapshotPayload`] layout. Bump when the payload shape changes
/// in a way that older readers cannot understand, and add a migration path.
pub const PAYLOAD_SCHEMA_VERSION: u32 = 1;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

id_type!(
    /// Stable identity of a workspace ("context").
    ContextId
);
id_type!(
    /// Stable identity of one immutable snapshot.
    SnapshotId
);

/// A named, user-owned workspace. Its history lives in [`Snapshot`]s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub id: ContextId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub is_archived: bool,
}

/// Outcome of the capture that produced a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStatus {
    /// Every requested component was captured.
    Complete,
    /// Some components failed; the snapshot is still usable.
    Partial,
    /// Nothing useful was captured (kept for diagnostics).
    Failed,
}

impl CaptureStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

impl FromStr for CaptureStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "complete" => Ok(Self::Complete),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown capture status '{other}'")),
        }
    }
}

/// An immutable, versioned record of a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub context_id: ContextId,
    pub version: u32,
    pub created_at: DateTime<Utc>,
    /// SHA-256 (hex) of the canonical JSON encoding of `payload`.
    pub hash: String,
    pub capture_status: CaptureStatus,
    pub payload: SnapshotPayload,
}

/// Snapshot header without the payload, for listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotSummary {
    pub id: SnapshotId,
    pub version: u32,
    pub created_at: DateTime<Utc>,
    pub hash: String,
    pub capture_status: CaptureStatus,
}

/// Everything captured about a workspace at one point in time.
///
/// The payload is the unit that is hashed and stored. It must never contain
/// credentials, cookies or tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotPayload {
    pub schema_version: u32,
    #[serde(default)]
    pub applications: Vec<ApplicationState>,
    #[serde(default)]
    pub windows: Vec<WindowState>,
    #[serde(default)]
    pub monitors: Vec<MonitorInfo>,
    #[serde(default)]
    pub git: Vec<GitState>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// Free-form capture metadata (ContextFlow version, capture duration, ...).
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl Default for SnapshotPayload {
    fn default() -> Self {
        Self {
            schema_version: PAYLOAD_SCHEMA_VERSION,
            applications: Vec::new(),
            windows: Vec::new(),
            monitors: Vec::new(),
            git: Vec::new(),
            notes: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

/// A launchable application that belongs to the workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplicationState {
    /// Stable, human-readable identity, e.g. `code` or `windowsterminal`.
    pub application_id: String,
    pub executable_path: String,
    #[serde(default)]
    pub launch_arguments: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    /// Adapter responsible for application-specific state, if any.
    #[serde(default)]
    pub adapter_id: Option<String>,
    /// Opaque, adapter-versioned state.
    #[serde(default)]
    pub adapter_state: serde_json::Value,
}

/// A rectangle in virtual-screen physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayState {
    Normal,
    Maximized,
    Minimized,
}

impl DisplayState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Maximized => "maximized",
            Self::Minimized => "minimized",
        }
    }
}

/// Geometry and identity of a top-level window. Window handles are *not*
/// stored: they are temporary OS references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub application_id: String,
    pub title: String,
    #[serde(default)]
    pub window_class: Option<String>,
    #[serde(default)]
    pub monitor_id: Option<String>,
    pub rect: Rect,
    pub display_state: DisplayState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// Best available stable identifier for the display.
    pub id: String,
    pub primary: bool,
    pub rect: Rect,
    pub work_area: Rect,
    #[serde(default)]
    pub dpi: Option<u32>,
}

/// Read-only Git metadata. ContextFlow never mutates repositories.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GitState {
    pub repository_path: String,
    /// `None` means detached HEAD.
    #[serde(default)]
    pub branch: Option<String>,
    /// `None` means the repository has no commits yet.
    #[serde(default)]
    pub head_commit: Option<String>,
    #[serde(default)]
    pub staged: u32,
    #[serde(default)]
    pub modified: u32,
    #[serde(default)]
    pub untracked: u32,
    #[serde(default)]
    pub conflicted: u32,
    #[serde(default)]
    pub ahead: Option<u32>,
    #[serde(default)]
    pub behind: Option<u32>,
}

/// Outcome of one restoration task (spec §4.4, stage 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreStatus {
    Success,
    PartialSuccess,
    Failed,
    Skipped,
    RequiresUserIntervention,
}

impl RestoreStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialSuccess => "partial_success",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::RequiresUserIntervention => "requires_user_intervention",
        }
    }
}

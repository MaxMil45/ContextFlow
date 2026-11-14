//! More data sent to the UI: settings, browser setup, data maintenance,
//! capture review and comparing snapshots. Re-exported from `dto`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ----------------------------------------------------------------- settings

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CaptureSettings {
    /// Save windows and their positions.
    pub include_windows: bool,
    /// Program names never captured, e.g. `discord`.
    pub excluded_processes: Vec<String>,
    /// Windows whose title contains one of these are skipped.
    pub excluded_title_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct BrowserSettings {
    /// Save and reopen Chrome/Edge tabs (needs the browser extension).
    pub capture_tabs: bool,
    /// Tabs whose address contains one of these are never saved.
    pub excluded_url_patterns: Vec<String>,
    pub max_tabs_per_window: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RetentionSettings {
    /// Keep at least this many of the newest snapshots.
    pub keep_last: Option<u32>,
    /// Keep snapshots younger than this many days.
    pub max_age_days: Option<u32>,
    /// Apply the rules after every capture.
    pub auto_prune: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct TimeoutSettings {
    /// Longest a git command may take, in milliseconds.
    #[ts(type = "number")]
    pub git_ms: u64,
    /// Longest a whole restore may take, in seconds.
    #[ts(type = "number")]
    pub restore_overall_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct AppSettings {
    /// A Windows notification when a restore finishes.
    pub notifications: bool,
    /// Closing the window keeps ContextFlow running in the tray.
    pub close_to_tray: bool,
    /// Start ContextFlow, hidden in the tray, when you sign in to Windows.
    pub start_with_windows: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct Settings {
    pub capture: CaptureSettings,
    pub browser: BrowserSettings,
    pub retention: RetentionSettings,
    pub timeouts: TimeoutSettings,
    pub app: AppSettings,
    /// `trace`, `debug`, `info`, `warn` or `error`.
    pub log_level: String,
}

// ----------------------------------------------------------- browser setup

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct BrowserLink {
    pub name: String,
    /// Chrome/Edge can find the ContextFlow host.
    pub registered: bool,
    /// The extension is loaded and talking to ContextFlow right now.
    pub connected: bool,
    pub windows: u32,
    pub tabs: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct BrowserStatus {
    pub browsers: Vec<BrowserLink>,
    pub manifest_present: bool,
    pub extension_unpacked: bool,
    /// Where to "Load unpacked" from.
    pub extension_dir: Option<String>,
}

// --------------------------------------------------------- data maintenance

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct FileResult {
    /// What was written or read.
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct ImportedWorkspace {
    pub name: String,
    pub snapshots: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct VerifyFailure {
    pub workspace: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct VerifyResult {
    pub ok: bool,
    pub database_problems: Vec<String>,
    pub snapshots_checked: u32,
    pub failures: Vec<VerifyFailure>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct PruneItem {
    pub workspace: String,
    pub versions: Vec<u32>,
}

/// Snapshots the retention rules would delete (or just deleted).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct RetentionResult {
    pub items: Vec<PruneItem>,
}

// ------------------------------------------------------------ capture review

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct DiscoveredApp {
    pub application_id: String,
    pub window_count: u32,
    /// Titles of its windows, so you can recognise what you're saving.
    pub window_titles: Vec<String>,
    pub tab_count: u32,
    pub folder_count: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct DiscoveredRepo {
    pub path: String,
    pub branch: Option<String>,
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
}

/// What a capture would save right now, to choose from before saving.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CaptureDiscovery {
    pub apps: Vec<DiscoveredApp>,
    pub repositories: Vec<DiscoveredRepo>,
    pub monitor_count: u32,
    pub warnings: Vec<String>,
    pub excluded_windows: u32,
    pub minimized_windows: u32,
}

// ------------------------------------------------------------------ compare

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../src/bindings/")]
pub enum DiffStatus {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct DiffEntry {
    pub status: DiffStatus,
    pub title: String,
    /// What exactly changed, one line each.
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct DiffGroup {
    /// `applications`, `windows`, `monitors`, `repositories` or `notes`.
    pub category: String,
    pub label: String,
    pub entries: Vec<DiffEntry>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/bindings/")]
pub struct CompareResult {
    pub workspace: String,
    /// "v2".
    pub from: String,
    /// "v3", or "your screen right now".
    pub to: String,
    pub is_empty: bool,
    /// Plain-language advice before restoring, shown first.
    pub recommendations: Vec<String>,
    pub groups: Vec<DiffGroup>,
}

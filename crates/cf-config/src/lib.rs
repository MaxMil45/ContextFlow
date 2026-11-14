//! ContextFlow configuration.
//!
//! Configuration lives in `config.toml` in the per-user config directory. A
//! missing file means "all defaults". Unknown keys are rejected so typos are
//! reported instead of silently ignored, and every value is range-checked.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

/// `Spotify.exe ` -> `spotify`.
fn normalize_process(p: &str) -> String {
    let p = p.trim().to_lowercase();
    p.strip_suffix(".exe").map(str::to_string).unwrap_or(p)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub storage: StorageConfig,
    pub capture: CaptureConfig,
    pub retention: RetentionConfig,
    pub timeouts: TimeoutConfig,
    pub logging: LoggingConfig,
    pub browser: BrowserConfig,
    pub app: AppConfig,
}

/// Desktop app behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    /// Show a Windows notification when a restore finishes.
    pub notifications: bool,
    /// Closing the window keeps ContextFlow running in the tray.
    pub close_to_tray: bool,
    /// A key combination that saves the active workspace from anywhere, without
    /// opening the app, like `Ctrl+Alt+X`. Empty switches it off.
    pub capture_hotkey: String,
    /// A key combination that opens the workspace switcher from anywhere, like
    /// `Ctrl+Alt+W`. Empty switches it off.
    pub switcher_hotkey: String,
    /// Save the workspace you are leaving before switching to another one.
    pub auto_save_on_switch: bool,
    /// After Windows restarts (or ContextFlow ends without Quit), offer to bring
    /// back the workspace you were in.
    pub offer_resume: bool,
    /// Save the workspace you're in every this many minutes while you work, so a
    /// restart or crash loses little. 0 switches it off.
    pub checkpoint_minutes: u32,
    /// Restore the layout saved for the screens connected now (docked, laptop...).
    pub match_display_setup: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            notifications: true,
            close_to_tray: false,
            capture_hotkey: "Ctrl+Alt+X".into(),
            switcher_hotkey: "Ctrl+Alt+W".into(),
            auto_save_on_switch: true,
            offer_resume: true,
            checkpoint_minutes: 20,
            match_display_setup: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    /// Overrides the default database location.
    pub database_path: Option<PathBuf>,
    /// Where `contextflow backup` writes by default.
    pub backup_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptureConfig {
    pub include_windows: bool,
    /// Process names (with or without `.exe`, case-insensitive) never captured.
    pub excluded_processes: Vec<String>,
    /// Windows whose title contains any of these (case-insensitive) are skipped.
    pub excluded_title_patterns: Vec<String>,
    /// Programs that belong to every workspace (music, chat): never saved into
    /// one, so a restore never moves them, and never closed by Focus rules.
    pub always_on: Vec<String>,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            include_windows: true,
            excluded_processes: Vec::new(),
            excluded_title_patterns: Vec::new(),
            always_on: Vec::new(),
        }
    }
}

impl CaptureConfig {
    /// Excluded process names normalised to lower-case stems (`Code.exe` -> `code`).
    /// Programs never captured: the excluded ones and the always-on ones.
    pub fn normalized_excluded_processes(&self) -> Vec<String> {
        self.excluded_processes
            .iter()
            .chain(&self.always_on)
            .map(|p| normalize_process(p))
            .collect()
    }

    /// The always-on programs, as Focus rules name them (`spotify`).
    pub fn normalized_always_on(&self) -> Vec<String> {
        self.always_on
            .iter()
            .map(|p| normalize_process(p))
            .collect()
    }

    pub fn title_is_excluded(&self, title: &str) -> bool {
        let title = title.to_lowercase();
        self.excluded_title_patterns
            .iter()
            .any(|p| title.contains(&p.to_lowercase()))
    }
}

/// A snapshot is pruned only when it breaks *every* configured rule, and the
/// latest snapshot of a workspace is never pruned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionConfig {
    /// Keep at least this many newest snapshots per workspace.
    pub keep_last: Option<u32>,
    /// Keep snapshots newer than this many days.
    pub max_age_days: Option<u32>,
    /// Prune automatically after each capture. Off by default.
    pub auto_prune: bool,
}

impl RetentionConfig {
    pub fn is_configured(&self) -> bool {
        self.keep_last.is_some() || self.max_age_days.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TimeoutConfig {
    /// Per-command timeout for git, in milliseconds.
    pub git_ms: u64,
    /// Longest a whole restore may take, in seconds.
    pub restore_overall_secs: u64,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            git_ms: 10_000,
            restore_overall_secs: 120,
        }
    }
}

impl TimeoutConfig {
    pub fn git(&self) -> Duration {
        Duration::from_millis(self.git_ms)
    }

    pub fn restore_overall(&self) -> Duration {
        Duration::from_secs(self.restore_overall_secs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    /// trace | debug | info | warn | error
    pub level: String,
    /// Write a daily rolling log file in the data directory.
    pub file: bool,
    /// How many daily log files to keep.
    pub max_files: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            file: true,
            max_files: 7,
        }
    }
}

/// Browser tab saving (needs the ContextFlow browser extension).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BrowserConfig {
    /// Save open tabs when capturing a browser.
    pub capture_tabs: bool,
    /// Tabs whose address contains any of these (case-insensitive) are never
    /// saved, e.g. "mybank.com" or "token=".
    pub excluded_url_patterns: Vec<String>,
    /// Most tabs reopened per browser window on restore.
    pub max_tabs_per_window: usize,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            capture_tabs: true,
            excluded_url_patterns: Vec::new(),
            max_tabs_per_window: 50,
        }
    }
}

const LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error"];

impl Config {
    /// Load from `path`. A missing file yields the defaults.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(ConfigError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        Self::parse(&text).map_err(|e| match e {
            ConfigError::Parse { message, .. } => ConfigError::Parse {
                path: path.to_path_buf(),
                message,
            },
            other => other,
        })
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let cfg: Config = toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: PathBuf::from("config.toml"),
            message: e.to_string().trim().to_string(),
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |m: String| Err(ConfigError::Invalid(m));
        if self.retention.keep_last == Some(0) {
            return invalid("retention.keep_last must be at least 1".into());
        }
        if self.retention.max_age_days == Some(0) {
            return invalid("retention.max_age_days must be at least 1".into());
        }
        if self.retention.auto_prune && !self.retention.is_configured() {
            return invalid(
                "retention.auto_prune needs retention.keep_last or retention.max_age_days".into(),
            );
        }
        if !(100..=600_000).contains(&self.timeouts.git_ms) {
            return invalid("timeouts.git_ms must be between 100 and 600000".into());
        }
        if !(10..=1800).contains(&self.timeouts.restore_overall_secs) {
            return invalid("timeouts.restore_overall_secs must be between 10 and 1800".into());
        }
        if !LEVELS.contains(&self.logging.level.as_str()) {
            return invalid(format!(
                "logging.level must be one of {}",
                LEVELS.join(", ")
            ));
        }
        if !(1..=200).contains(&self.browser.max_tabs_per_window) {
            return invalid("browser.max_tabs_per_window must be between 1 and 200".into());
        }
        if self
            .browser
            .excluded_url_patterns
            .iter()
            .any(|s| s.trim().is_empty())
        {
            return invalid("browser.excluded_url_patterns entries must not be empty".into());
        }
        if self.app.capture_hotkey.chars().count() > 64
            || self.app.capture_hotkey.chars().any(char::is_control)
        {
            return invalid(
                "app.capture_hotkey must be a key combination such as Ctrl+Alt+X".into(),
            );
        }
        if self.app.switcher_hotkey.chars().count() > 64
            || self.app.switcher_hotkey.chars().any(char::is_control)
        {
            return invalid(
                "app.switcher_hotkey must be a key combination such as Ctrl+Alt+W".into(),
            );
        }
        if !self.app.capture_hotkey.trim().is_empty()
            && self
                .app
                .capture_hotkey
                .trim()
                .eq_ignore_ascii_case(self.app.switcher_hotkey.trim())
        {
            return invalid("app.capture_hotkey and app.switcher_hotkey must be different".into());
        }
        if self.app.checkpoint_minutes > 240 {
            return invalid("app.checkpoint_minutes must be between 0 (off) and 240".into());
        }
        if !(1..=365).contains(&self.logging.max_files) {
            return invalid("logging.max_files must be between 1 and 365".into());
        }
        if self
            .capture
            .excluded_processes
            .iter()
            .chain(&self.capture.excluded_title_patterns)
            .any(|s| s.trim().is_empty())
        {
            return invalid("capture exclusion entries must not be empty".into());
        }
        Ok(())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("config always serialises")
    }
}

/// Commented starter file written by `contextflow config init`.
pub const TEMPLATE: &str = r#"# ContextFlow configuration
# Every setting is optional; the values shown are the defaults.

[storage]
# database_path = 'C:\Users\you\ContextFlow\contextflow.db'
# backup_dir = 'D:\Backups\ContextFlow'

[capture]
include_windows = true
# Process names to never capture (".exe" optional, case-insensitive).
excluded_processes = []
# Skip windows whose title contains any of these (case-insensitive).
excluded_title_patterns = []

[retention]
# A snapshot is pruned only when it breaks every rule set here.
# The latest snapshot of a workspace is never pruned.
# keep_last = 50
# max_age_days = 90
auto_prune = false

[timeouts]
git_ms = 10000
# Longest a whole restore may take, in seconds.
restore_overall_secs = 120

[browser]
# Save open tabs when capturing Chrome/Edge (needs `contextflow browser install`).
capture_tabs = true
# Tabs whose address contains any of these are never saved (case-insensitive).
excluded_url_patterns = []
# Most tabs reopened per browser window.
max_tabs_per_window = 50

[app]
# Show a Windows notification when a restore finishes.
notifications = true
# Closing the window keeps ContextFlow running in the tray.
close_to_tray = false
# Saves the active workspace from anywhere, without opening the app.
# Needs at least one of Ctrl, Alt, Shift. Use "" to switch it off.
capture_hotkey = "Ctrl+Alt+X"
# Opens the workspace switcher from anywhere. Use "" to switch it off.
switcher_hotkey = "Ctrl+Alt+W"
# Save the workspace you are leaving before switching to another one.
auto_save_on_switch = true
# After a restart, offer to bring back the workspace you were in.
offer_resume = true
# Save the workspace you're in every N minutes while you work (0 = off).
checkpoint_minutes = 20
# Restore the layout saved for the screens connected now (docked, laptop only...).
match_display_setup = true

[logging]
level = "info"
file = true
max_files = 7
"#;

/// The per-user directories ContextFlow stores its data and config in.
/// Shared by the CLI and the desktop launcher so they always agree.
pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("dev", "ContextFlow", "ContextFlow")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_equals_defaults() {
        assert_eq!(Config::parse(TEMPLATE).unwrap(), Config::default());
    }

    #[test]
    fn the_capture_hotkey_defaults_on_and_can_be_switched_off() {
        assert_eq!(Config::default().app.capture_hotkey, "Ctrl+Alt+X");
        let off = Config::parse(
            "[app]
capture_hotkey = \"\"
",
        )
        .unwrap();
        assert_eq!(off.app.capture_hotkey, "");
        assert!(off.validate().is_ok());
        let mut bad = Config::default();
        bad.app.capture_hotkey = "x".repeat(65);
        assert!(bad.validate().is_err());
    }

    #[test]
    fn the_switcher_defaults_on_with_auto_save_and_cannot_clash_with_capture() {
        let cfg = Config::default();
        assert_eq!(cfg.app.switcher_hotkey, "Ctrl+Alt+W");
        assert!(cfg.app.auto_save_on_switch);
        assert!(cfg.validate().is_ok());

        let mut same = Config::default();
        same.app.switcher_hotkey = " ctrl+alt+x ".into();
        assert!(same.validate().is_err(), "two actions on one shortcut");

        // Turning the capture shortcut off frees the combination.
        let mut off = Config::default();
        off.app.capture_hotkey = String::new();
        off.app.switcher_hotkey = "Ctrl+Alt+X".into();
        assert!(off.validate().is_ok());

        let mut bad = Config::default();
        bad.app.switcher_hotkey = "x".repeat(65);
        assert!(bad.validate().is_err());
    }

    #[test]
    fn always_on_apps_are_never_captured_and_checkpoints_are_bounded() {
        let mut cfg = Config::default();
        cfg.capture.always_on = vec![" Spotify.exe ".into()];
        assert_eq!(
            cfg.capture.normalized_always_on(),
            vec!["spotify".to_string()]
        );
        assert!(cfg
            .capture
            .normalized_excluded_processes()
            .contains(&"spotify".to_string()));
        assert_eq!(cfg.app.checkpoint_minutes, 20);
        assert!(cfg.app.offer_resume && cfg.app.match_display_setup);
        cfg.app.checkpoint_minutes = 241;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn empty_file_is_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn missing_file_is_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load(&dir.path().join("nope.toml")).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn overrides_apply() {
        let cfg = Config::parse(
            r#"
            [capture]
            excluded_processes = ["Spotify.exe", " Discord "]
            excluded_title_patterns = ["Private"]
            [retention]
            keep_last = 10
            auto_prune = true
            [timeouts]
            git_ms = 2500
            "#,
        )
        .unwrap();
        assert_eq!(
            cfg.capture.normalized_excluded_processes(),
            vec!["spotify", "discord"]
        );
        assert!(cfg.capture.title_is_excluded("my private notes"));
        assert!(!cfg.capture.title_is_excluded("public"));
        assert_eq!(cfg.retention.keep_last, Some(10));
        assert_eq!(cfg.timeouts.git(), Duration::from_millis(2500));
        assert!(cfg.logging.file, "untouched sections keep defaults");
    }

    #[test]
    fn unknown_keys_report_line_numbers() {
        let err = Config::parse("[capture]\ninclude_windowz = true\n").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("include_windowz"), "{msg}");
        assert!(msg.contains("line 2"), "{msg}");
    }

    #[test]
    fn invalid_values_are_rejected() {
        for bad in [
            "[retention]\nkeep_last = 0",
            "[retention]\nmax_age_days = 0",
            "[retention]\nauto_prune = true",
            "[timeouts]\ngit_ms = 5",
            "[logging]\nlevel = \"loud\"",
            "[logging]\nmax_files = 0",
            "[browser]\nmax_tabs_per_window = 0",
            "[browser]\nmax_tabs_per_window = 500",
            "[browser]\nexcluded_url_patterns = [\" \"]",
            "[capture]\nexcluded_processes = [\"  \"]",
        ] {
            assert!(
                matches!(Config::parse(bad), Err(ConfigError::Invalid(_))),
                "{bad}"
            );
        }
        assert!(matches!(
            Config::parse("[timeouts]\ngit_ms = \"fast\""),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn round_trips_through_toml() {
        let mut cfg = Config::default();
        cfg.retention.keep_last = Some(5);
        cfg.storage.database_path = Some(PathBuf::from(r"C:\data\cf.db"));
        assert_eq!(Config::parse(&cfg.to_toml()).unwrap(), cfg);
    }
}

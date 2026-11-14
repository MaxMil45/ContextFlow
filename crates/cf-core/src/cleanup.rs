//! Workspace cleanup rules: which programs to close when a workspace is
//! entered or left. Pure data and validation; closing them is `cf-cleanup`.

use serde::{Deserialize, Serialize};

use crate::{CoreError, Result};

/// Most programs one list may hold.
pub const MAX_TARGETS: usize = 50;
const MAX_PROCESS_NAME_CHARS: usize = 64;

/// How a program is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseMode {
    /// Ask the program to close, as its window's close button does, and wait.
    /// If it stays open (many apps just hide to the tray), it is left running.
    #[default]
    Graceful,
    /// Ask first and wait a few seconds; if it is still running, end it.
    /// Unsaved work in it is lost.
    Force,
}

/// One program in a cleanup list, named like an application id: the
/// executable's file name without `.exe`, in lower case (`discord`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupTarget {
    pub process: String,
    #[serde(default)]
    pub mode: CloseMode,
}

/// A workspace's cleanup rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupConfig {
    /// Closed when the workspace is entered (restored).
    #[serde(default)]
    pub on_enter: Vec<CleanupTarget>,
    /// Closed when the workspace is left (another one is restored, or the
    /// user leaves it).
    #[serde(default)]
    pub on_exit: Vec<CleanupTarget>,
}

fn invalid(msg: impl Into<String>) -> CoreError {
    CoreError::InvalidCleanup(msg.into())
}

/// `Discord.exe` -> `discord`. Rejects anything that isn't a plain program name.
pub fn normalize_process_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower).trim();
    if stem.is_empty() {
        return Err(invalid("a program name must not be empty"));
    }
    if stem.chars().count() > MAX_PROCESS_NAME_CHARS {
        return Err(invalid(format!(
            "'{trimmed}' is too long for a program name"
        )));
    }
    if stem.chars().any(|c| {
        c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err(invalid(format!(
            "'{trimmed}' is not a program name (use just the name, like discord)"
        )));
    }
    Ok(stem.to_string())
}

fn normalize_list(list: Vec<CleanupTarget>, what: &str) -> Result<Vec<CleanupTarget>> {
    if list.len() > MAX_TARGETS {
        return Err(invalid(format!(
            "{what}: at most {MAX_TARGETS} programs per list"
        )));
    }
    let mut out: Vec<CleanupTarget> = Vec::with_capacity(list.len());
    for t in list {
        let process = normalize_process_name(&t.process)?;
        // The same program twice keeps the first entry.
        if !out.iter().any(|o| o.process == process) {
            out.push(CleanupTarget {
                process,
                mode: t.mode,
            });
        }
    }
    Ok(out)
}

impl CleanupConfig {
    /// Validated, with names normalised and duplicates removed.
    pub fn normalized(self) -> Result<Self> {
        Ok(Self {
            on_enter: normalize_list(self.on_enter, "when entering")?,
            on_exit: normalize_list(self.on_exit, "when leaving")?,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.on_enter.is_empty() && self.on_exit.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, mode: CloseMode) -> CleanupTarget {
        CleanupTarget {
            process: name.into(),
            mode,
        }
    }

    #[test]
    fn names_are_normalised_like_application_ids() {
        assert_eq!(normalize_process_name(" Discord.EXE ").unwrap(), "discord");
        assert_eq!(normalize_process_name("steam").unwrap(), "steam");
        for bad in ["", "  ", ".exe", r"C:\x\a.exe", "a/b", "we*ird", "a\nb"] {
            assert!(normalize_process_name(bad).is_err(), "{bad:?}");
        }
        assert!(normalize_process_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn duplicates_collapse_and_the_first_mode_wins() {
        let cfg = CleanupConfig {
            on_enter: vec![
                target("Discord.exe", CloseMode::Force),
                target("discord", CloseMode::Graceful),
                target("steam", CloseMode::Graceful),
            ],
            on_exit: vec![],
        }
        .normalized()
        .unwrap();
        assert_eq!(
            cfg.on_enter,
            vec![
                target("discord", CloseMode::Force),
                target("steam", CloseMode::Graceful)
            ]
        );
    }

    #[test]
    fn lists_are_capped() {
        let many = (0..=MAX_TARGETS)
            .map(|i| target(&format!("app{i}"), CloseMode::Graceful))
            .collect();
        assert!(CleanupConfig {
            on_enter: many,
            on_exit: vec![]
        }
        .normalized()
        .is_err());
    }

    #[test]
    fn a_missing_mode_means_graceful_and_missing_lists_are_empty() {
        let cfg: CleanupConfig = serde_json::from_str(r#"{"on_enter":[{"process":"x"}]}"#).unwrap();
        assert_eq!(cfg.on_enter[0].mode, CloseMode::Graceful);
        assert!(cfg.on_exit.is_empty());
    }
}

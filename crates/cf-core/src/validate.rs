//! Validation applied before anything is persisted.

use crate::{CoreError, Result, SnapshotPayload, PAYLOAD_SCHEMA_VERSION};

pub const MAX_NAME_CHARS: usize = 100;
pub const MAX_NOTE_BYTES: usize = 64 * 1024;
/// Largest adapter-owned state (e.g. a browser's tabs) per application.
pub const MAX_ADAPTER_STATE_BYTES: usize = 2 * 1024 * 1024;
/// Generous bound on window dimensions; anything larger is corrupt data.
pub const MAX_WINDOW_DIMENSION: i32 = 100_000;

/// Validate and normalise a workspace name. Returns the trimmed name.
pub fn context_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(CoreError::InvalidName("name must not be empty".into()));
    }
    if trimmed.chars().count() > MAX_NAME_CHARS {
        return Err(CoreError::InvalidName(format!(
            "name must be at most {MAX_NAME_CHARS} characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(CoreError::InvalidName(
            "name must not contain control characters".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn invalid(msg: impl Into<String>) -> CoreError {
    CoreError::InvalidPayload(msg.into())
}

fn check_path(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(invalid(format!("{field} must not be empty")));
    }
    if value.contains('\0') {
        return Err(invalid(format!("{field} must not contain NUL characters")));
    }
    Ok(())
}

/// Reject payloads that are structurally unsafe or obviously corrupt.
pub fn payload(p: &SnapshotPayload) -> Result<()> {
    if p.schema_version != PAYLOAD_SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported schema_version {} (expected {PAYLOAD_SCHEMA_VERSION})",
            p.schema_version
        )));
    }

    for app in &p.applications {
        if app.application_id.trim().is_empty() {
            return Err(invalid("application_id must not be empty"));
        }
        check_path("executable_path", &app.executable_path)?;
        if let Some(dir) = &app.working_directory {
            check_path("working_directory", dir)?;
        }
        let state_bytes = serde_json::to_vec(&app.adapter_state).map_or(0, |v| v.len());
        if state_bytes > MAX_ADAPTER_STATE_BYTES {
            return Err(invalid(format!(
                "state saved for '{}' is too large ({state_bytes} bytes)",
                app.application_id
            )));
        }
        if app.launch_arguments.iter().any(|a| a.contains('\0')) {
            return Err(invalid("launch arguments must not contain NUL characters"));
        }
    }

    for w in &p.windows {
        if w.application_id.trim().is_empty() {
            return Err(invalid("window application_id must not be empty"));
        }
        let r = w.rect;
        if r.width <= 0 || r.height <= 0 {
            return Err(invalid(format!(
                "window '{}' has non-positive size {}x{}",
                w.title, r.width, r.height
            )));
        }
        if r.width > MAX_WINDOW_DIMENSION || r.height > MAX_WINDOW_DIMENSION {
            return Err(invalid(format!(
                "window '{}' has implausible size",
                w.title
            )));
        }
    }

    for m in &p.monitors {
        if m.id.trim().is_empty() {
            return Err(invalid("monitor id must not be empty"));
        }
    }

    for g in &p.git {
        check_path("repository_path", &g.repository_path)?;
        if let Some(head) = &g.head_commit {
            let ok_len = head.len() == 40 || head.len() == 64; // SHA-1 or SHA-256 repos
            if !ok_len || !head.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(invalid(format!("invalid head commit '{head}'")));
            }
        }
    }

    for note in &p.notes {
        if note.len() > MAX_NOTE_BYTES {
            return Err(invalid(format!(
                "notes must be at most {MAX_NOTE_BYTES} bytes"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DisplayState, GitState, Rect, WindowState};

    #[test]
    fn names_are_trimmed_and_checked() {
        assert_eq!(context_name("  Uni Dev  ").unwrap(), "Uni Dev");
        assert!(context_name("   ").is_err());
        assert!(context_name("bad\nname").is_err());
        assert!(context_name(&"x".repeat(101)).is_err());
    }

    #[test]
    fn default_payload_is_valid() {
        payload(&SnapshotPayload::default()).unwrap();
    }

    #[test]
    fn rejects_zero_sized_window() {
        let mut p = SnapshotPayload::default();
        p.windows.push(WindowState {
            application_id: "code".into(),
            title: "x".into(),
            window_class: None,
            monitor_id: None,
            rect: Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 10,
            },
            display_state: DisplayState::Normal,
        });
        assert!(payload(&p).is_err());
    }

    #[test]
    fn rejects_bad_commit_hash() {
        let mut p = SnapshotPayload::default();
        p.git.push(GitState {
            repository_path: "C:/repo".into(),
            head_commit: Some("not-a-sha".into()),
            ..Default::default()
        });
        assert!(payload(&p).is_err());
    }

    #[test]
    fn rejects_oversized_adapter_state() {
        let mut p = SnapshotPayload::default();
        p.applications.push(crate::ApplicationState {
            application_id: "chrome".into(),
            executable_path: "C:/chrome.exe".into(),
            launch_arguments: vec![],
            working_directory: None,
            adapter_id: Some("browser".into()),
            adapter_state: serde_json::json!({ "blob": "x".repeat(MAX_ADAPTER_STATE_BYTES) }),
        });
        assert!(payload(&p).is_err());
    }

    #[test]
    fn rejects_future_schema() {
        let p = SnapshotPayload {
            schema_version: 99,
            ..Default::default()
        };
        assert!(payload(&p).is_err());
    }
}

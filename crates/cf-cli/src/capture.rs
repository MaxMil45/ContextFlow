//! Builds a [`SnapshotPayload`] from the live environment.
//!
//! Capture is strictly non-destructive: it only reads window, monitor and Git
//! state. Failures in one component downgrade the snapshot to `Partial` rather
//! than aborting the whole capture.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use cf_core::{ApplicationState, CaptureStatus, SnapshotPayload, WindowState};

/// Shell / system processes whose windows are never part of a workspace.
const EXCLUDED_PROCESSES: &[&str] = &[
    "textinputhost",
    "searchhost",
    "shellexperiencehost",
    "startmenuexperiencehost",
    "lockapp",
    "systemsettings",
    "contextflow",
];

pub struct CaptureOptions<'a> {
    pub repos: &'a [PathBuf],
    pub notes: Vec<String>,
    pub include_windows: bool,
}

pub struct CaptureResult {
    pub payload: SnapshotPayload,
    pub status: CaptureStatus,
    pub warnings: Vec<String>,
}

pub fn application_id(executable_path: &str) -> String {
    Path::new(executable_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "unknown".into())
}

pub fn capture(opts: CaptureOptions<'_>) -> CaptureResult {
    let started = Instant::now();
    let mut payload = SnapshotPayload {
        notes: opts.notes,
        ..Default::default()
    };
    let mut warnings = Vec::new();

    if opts.include_windows {
        match cf_windows::enumerate_monitors() {
            Ok(m) => payload.monitors = m,
            Err(e) => warnings.push(format!("monitors: {e}")),
        }
        match cf_windows::enumerate_windows() {
            Ok(windows) => {
                let own_pid = std::process::id();
                let mut apps: BTreeMap<String, ApplicationState> = BTreeMap::new();
                for w in windows {
                    if w.process_id == own_pid || w.rect.width <= 0 || w.rect.height <= 0 {
                        continue;
                    }
                    let Some(exe) = w.executable_path.clone() else {
                        warnings.push(format!(
                            "window '{}' skipped: process {} is not accessible (elevated?)",
                            w.title, w.process_id
                        ));
                        continue;
                    };
                    let app_id = application_id(&exe);
                    if EXCLUDED_PROCESSES.contains(&app_id.as_str()) {
                        continue;
                    }
                    apps.entry(app_id.clone())
                        .or_insert_with(|| ApplicationState {
                            application_id: app_id.clone(),
                            executable_path: exe,
                            launch_arguments: Vec::new(),
                            working_directory: None,
                            adapter_id: None,
                            adapter_state: serde_json::Value::Null,
                        });
                    payload.windows.push(WindowState {
                        application_id: app_id,
                        title: w.title,
                        window_class: Some(w.class_name),
                        monitor_id: w.monitor_id,
                        rect: w.rect,
                        display_state: w.display_state,
                    });
                }
                payload.applications = apps.into_values().collect();
            }
            Err(e) => warnings.push(format!("windows: {e}")),
        }
    }

    for repo in opts.repos {
        match cf_git::inspect(repo) {
            Ok(state) => payload.git.push(state),
            Err(e) => warnings.push(format!("git {}: {e}", repo.display())),
        }
    }

    payload.metadata.insert(
        "contextflow_version".into(),
        env!("CARGO_PKG_VERSION").into(),
    );
    payload.metadata.insert(
        "capture_duration_ms".into(),
        (started.elapsed().as_millis() as u64).into(),
    );

    let status = if warnings.iter().any(|w| !w.starts_with("window '")) {
        CaptureStatus::Partial
    } else {
        CaptureStatus::Complete
    };
    CaptureResult {
        payload,
        status,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_ids_are_lowercase_stems() {
        assert_eq!(
            application_id(r"C:\Program Files\Microsoft VS Code\Code.exe"),
            "code"
        );
        assert_eq!(application_id(r"C:\Windows\explorer.exe"), "explorer");
    }
}

//! Builds a [`SnapshotPayload`] from the live environment.
//!
//! Capture is strictly non-destructive: it only reads window, monitor and Git
//! state. Failures in one component downgrade the snapshot to `Partial` rather
//! than aborting the whole capture.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cf_config::CaptureConfig;
use cf_core::{ApplicationState, CaptureStatus, SnapshotPayload, WindowState};

/// Shell / system processes whose windows are never part of a workspace.
const BUILTIN_EXCLUDED_PROCESSES: &[&str] = &[
    "textinputhost",
    "searchhost",
    "shellexperiencehost",
    "startmenuexperiencehost",
    "lockapp",
    "applicationframehost",
    "systemsettings",
    "contextflow",
];

pub struct CaptureOptions<'a> {
    /// Capture only these application ids (empty = every application).
    pub only: &'a [String],
    /// Optional extra: Git repositories to record alongside the applications.
    pub repos: &'a [PathBuf],
    pub notes: Vec<String>,
    pub include_windows: bool,
    pub rules: &'a CaptureConfig,
    pub git_timeout: Duration,
}

pub struct CaptureResult {
    pub payload: SnapshotPayload,
    pub status: CaptureStatus,
    pub warnings: Vec<String>,
    /// Windows skipped because of configured exclusion rules.
    pub excluded_windows: usize,
}

pub use cf_engine::application_id;

pub fn capture(opts: CaptureOptions<'_>) -> CaptureResult {
    let started = Instant::now();
    let mut payload = SnapshotPayload {
        notes: opts.notes,
        ..Default::default()
    };
    let mut warnings = Vec::new();
    let mut component_failed = false;
    let mut excluded_windows = 0;

    let user_excluded = opts.rules.normalized_excluded_processes();
    let is_excluded_process = |id: &str| {
        BUILTIN_EXCLUDED_PROCESSES.contains(&id) || user_excluded.iter().any(|e| e == id)
    };

    if opts.include_windows {
        match cf_windows::enumerate_monitors() {
            Ok(m) => payload.monitors = m,
            Err(e) => {
                component_failed = true;
                warnings.push(format!("monitors: {e}"));
            }
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
                            "a window of process {} was skipped: the process is not accessible (probably running as administrator)",
                            w.process_id
                        ));
                        continue;
                    };
                    let app_id = application_id(&exe);
                    if is_excluded_process(&app_id)
                        || (!opts.only.is_empty() && !opts.only.contains(&app_id))
                    {
                        continue;
                    }
                    if opts.rules.title_is_excluded(&w.title) {
                        excluded_windows += 1;
                        continue;
                    }
                    apps.entry(app_id.clone()).or_insert_with(|| {
                        // Microsoft Store apps are started by package id; save it
                        // now so restore still works after the app updates.
                        let aumid = cf_windows::packaged_app_id(&exe);
                        ApplicationState {
                            application_id: app_id.clone(),
                            executable_path: exe,
                            launch_arguments: Vec::new(),
                            working_directory: None,
                            adapter_id: aumid.as_ref().map(|_| "packaged-app".to_string()),
                            adapter_state: aumid
                                .map(|id| serde_json::json!({ "aumid": id }))
                                .unwrap_or(serde_json::Value::Null),
                        }
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
            Err(e) => {
                component_failed = true;
                warnings.push(format!("windows: {e}"));
            }
        }
    }

    for repo in opts.repos {
        match cf_git::inspect_with_timeout(repo, opts.git_timeout) {
            Ok(state) => payload.git.push(state),
            Err(e) => {
                component_failed = true;
                warnings.push(format!("git {}: {e}", repo.display()));
            }
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

    let status = if component_failed {
        CaptureStatus::Partial
    } else {
        CaptureStatus::Complete
    };
    tracing::info!(
        applications = payload.applications.len(),
        windows = payload.windows.len(),
        monitors = payload.monitors.len(),
        repositories = payload.git.len(),
        excluded_windows,
        status = status.as_str(),
        duration_ms = started.elapsed().as_millis() as u64,
        "capture finished"
    );
    CaptureResult {
        payload,
        status,
        warnings,
        excluded_windows,
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

    #[test]
    fn excluded_processes_never_appear() {
        // Exclude every process: the capture must then contain no windows,
        // whatever happens to be open on this machine.
        let rules = CaptureConfig {
            excluded_title_patterns: vec!["".into()],
            ..Default::default()
        };
        let r = capture(CaptureOptions {
            only: &[],
            repos: &[],
            notes: vec![],
            include_windows: true,
            rules: &rules,
            git_timeout: Duration::from_secs(5),
        });
        assert!(r.payload.windows.is_empty());
        assert!(r.payload.applications.is_empty());
    }
}

//! Saving the workspace you are leaving, so coming back finds it as you left it.
//!
//! Every way of switching (the app, the switcher, the tray, a desktop icon, the
//! CLI) runs this before the leave rules close anything. It is careful about
//! what it saves:
//!
//! * nothing, if none of the workspace's saved programs are open (an emptied
//!   desktop must never overwrite the newest snapshot with an empty one);
//! * nothing, if the layout is the same as the newest snapshot (so switching
//!   back and forth doesn't fill the history with copies);
//! * otherwise a snapshot marked as an auto-save, which keeps the last note
//!   and repositories, and which retention counts separately from the
//!   snapshots you saved on purpose.

use std::collections::HashSet;
use std::path::PathBuf;

use cf_config::Config;
use cf_core::{ContextId, SnapshotPayload};
use cf_storage::{Database, SaveMode, SnapshotOrigin};
use serde_json::Value;

use crate::capture;

/// What happened to the workspace being left.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoSave {
    /// The workspace that was left, if there was one.
    pub from: Option<String>,
    /// The version saved for it, if one was.
    pub saved_version: Option<u32>,
    /// Why nothing was saved, in words for the user.
    pub note: Option<String>,
    /// The workspace asks to be told when it is saved automatically.
    pub notify: bool,
}

/// Whether a capture should become an auto-save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Save,
    /// None of the programs the workspace saved before are open.
    NothingOpen,
    /// The layout is the same as the newest snapshot.
    Unchanged,
}

/// The parts of a payload that matter for restoring it. Window titles change
/// constantly (a new tab, an unsaved-file marker), and capture metadata always
/// differs, so neither counts as a change.
fn layout(p: &SnapshotPayload) -> Value {
    let mut apps: Vec<Value> = p
        .applications
        .iter()
        .map(|a| {
            serde_json::json!([
                a.application_id,
                a.executable_path,
                a.launch_arguments,
                a.adapter_state
            ])
        })
        .collect();
    apps.sort_by_key(|v| v.to_string());
    let mut windows: Vec<Value> = p
        .windows
        .iter()
        .map(|w| {
            serde_json::json!([
                w.application_id,
                w.monitor_id,
                w.rect.x,
                w.rect.y,
                w.rect.width,
                w.rect.height,
                w.display_state.as_str()
            ])
        })
        .collect();
    windows.sort_by_key(|v| v.to_string());
    let mut git: Vec<Value> = p
        .git
        .iter()
        .map(|g| {
            serde_json::json!([
                g.repository_path,
                g.branch,
                g.head_commit,
                g.staged,
                g.modified,
                g.untracked
            ])
        })
        .collect();
    git.sort_by_key(|v| v.to_string());
    serde_json::json!([apps, windows, git])
}

/// Decide what to do with `captured`, given the newest snapshot (if any).
pub fn decide(previous: Option<&SnapshotPayload>, captured: &SnapshotPayload) -> Decision {
    let Some(previous) = previous else {
        return if captured.applications.is_empty() {
            Decision::NothingOpen
        } else {
            Decision::Save
        };
    };
    let expected: HashSet<&str> = previous
        .applications
        .iter()
        .map(|a| a.application_id.as_str())
        .collect();
    let any_open = captured
        .applications
        .iter()
        .any(|a| expected.contains(a.application_id.as_str()));
    if !expected.is_empty() && !any_open {
        return Decision::NothingOpen;
    }
    if layout(previous) == layout(captured) {
        return Decision::Unchanged;
    }
    Decision::Save
}

/// Save the active workspace before leaving it for `target` (or just leaving
/// it, with `None`). Never fails: problems are reported in [`AutoSave::note`].
/// `leave_note` is a line the user wrote as they left ("finish the migration
/// test"): it replaces the carried-over note, and a new note is a reason to save
/// even when the layout hasn't changed.
pub fn save_active(
    db: &mut Database,
    cfg: &Config,
    target: Option<ContextId>,
    leave_note: Option<&str>,
    log: &mut dyn FnMut(&str),
) -> AutoSave {
    let mut out = AutoSave::default();
    let Ok(Some(active)) = db.active_context() else {
        return out;
    };
    if target == Some(active) {
        return out;
    }
    let Ok(prev) = db.find_context(&active.to_string()) else {
        return out;
    };
    out.from = Some(prev.name.clone());
    if !cfg.app.auto_save_on_switch {
        out.note = Some("Saving before switching is turned off.".into());
        return out;
    }
    if prev.is_archived {
        out.note = Some("It is archived, so it was not saved.".into());
        return out;
    }
    match db.save_mode(prev.id).unwrap_or_default() {
        SaveMode::Fixed => {
            out.note =
                Some("It is set to always restore as you captured it, so it was not saved.".into());
            return out;
        }
        SaveMode::Notify => out.notify = true,
        SaveMode::Auto => {}
    }
    let previous = db.get_snapshot(prev.id, None).ok();
    let repos: Vec<PathBuf> = previous
        .iter()
        .flat_map(|s| s.payload.git.iter())
        .map(|g| PathBuf::from(&g.repository_path))
        .collect();
    // A note written as you leave wins; otherwise keep the last one, so "where
    // you left off" still says what you were doing.
    let previous_note = previous
        .as_ref()
        .and_then(|s| s.payload.notes.last().cloned());
    let new_note = leave_note
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().take(500).collect::<String>());
    let note_changed = new_note.is_some() && new_note != previous_note;
    let notes: Vec<String> = new_note.or(previous_note).into_iter().collect();

    log(&format!("Saving “{}”", prev.name));
    let result = capture::capture(capture::CaptureOptions {
        only: &[],
        repos: &repos,
        notes,
        include_windows: cfg.capture.include_windows,
        unminimize: false,
        rules: &cfg.capture,
        git_timeout: cfg.timeouts.git(),
        browser: &cfg.browser,
        browser_state_dir: cf_browser::default_state_dir(),
        vscode_storage: cf_vscode::default_storage_json(),
        visual_studio_settings: cf_vstudio::default_settings_files(),
        explorer_folders: true,
        jetbrains_recent: cf_jetbrains::default_recent_files(),
    });

    let decision = match decide(previous.as_ref().map(|s| &s.payload), &result.payload) {
        Decision::Unchanged if note_changed => Decision::Save,
        d => d,
    };
    match decision {
        Decision::NothingOpen => {
            out.note = Some("None of its programs were open, so the last save was kept.".into());
        }
        Decision::Unchanged => {
            out.note = Some("Nothing had changed since its last save.".into());
        }
        Decision::Save => {
            match db.create_snapshot_as(
                prev.id,
                result.payload,
                result.status,
                SnapshotOrigin::Auto,
            ) {
                Ok(snap) => {
                    out.saved_version = Some(snap.version);
                    tracing::info!(workspace = %prev.name, version = snap.version, "auto-saved before leaving");
                    // Auto-saves have their own small allowance; the user's own rules
                    // (if any) apply to the rest.
                    let policy = crate::maintenance::retention_policy(cfg).unwrap_or_default();
                    if let Err(e) = db.prune(prev.id, &policy, chrono::Utc::now()) {
                        tracing::warn!("pruning after an auto-save failed: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!("auto-save before leaving failed: {e}");
                    out.note = Some(format!("It could not be saved: {e}."));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_core::{ApplicationState, DisplayState, Rect, WindowState};

    fn app(id: &str) -> ApplicationState {
        ApplicationState {
            application_id: id.into(),
            executable_path: format!("C:\\{id}.exe"),
            launch_arguments: Vec::new(),
            working_directory: None,
            adapter_id: None,
            adapter_state: Value::Null,
        }
    }

    fn window(id: &str, x: i32, title: &str) -> WindowState {
        WindowState {
            application_id: id.into(),
            title: title.into(),
            window_class: None,
            monitor_id: Some("M1".into()),
            rect: Rect {
                x,
                y: 0,
                width: 800,
                height: 600,
            },
            display_state: DisplayState::Normal,
        }
    }

    fn payload(apps: &[&str], windows: Vec<WindowState>) -> SnapshotPayload {
        SnapshotPayload {
            applications: apps.iter().map(|a| app(a)).collect(),
            windows,
            ..Default::default()
        }
    }

    #[test]
    fn an_emptied_desktop_never_replaces_the_last_save() {
        let before = payload(&["code", "chrome"], vec![window("code", 0, "a")]);
        let now = payload(&["spotify"], vec![window("spotify", 0, "music")]);
        assert_eq!(decide(Some(&before), &now), Decision::NothingOpen);
        assert_eq!(
            decide(Some(&before), &payload(&[], vec![])),
            Decision::NothingOpen
        );
    }

    #[test]
    fn title_and_metadata_changes_are_not_a_new_layout() {
        let before = payload(&["code"], vec![window("code", 0, "main.rs - ContextFlow")]);
        let mut now = payload(
            &["code"],
            vec![window("code", 0, "● main.rs - ContextFlow")],
        );
        now.metadata.insert("capture_ms".into(), Value::from(123));
        assert_eq!(decide(Some(&before), &now), Decision::Unchanged);
    }

    #[test]
    fn a_moved_window_or_new_program_is_saved() {
        let before = payload(&["code"], vec![window("code", 0, "a")]);
        let moved = payload(&["code"], vec![window("code", 400, "a")]);
        assert_eq!(decide(Some(&before), &moved), Decision::Save);
        let added = payload(&["code", "chrome"], vec![window("code", 0, "a")]);
        assert_eq!(decide(Some(&before), &added), Decision::Save);
    }

    #[test]
    fn a_first_save_needs_something_open() {
        assert_eq!(decide(None, &payload(&[], vec![])), Decision::NothingOpen);
        assert_eq!(decide(None, &payload(&["code"], vec![])), Decision::Save);
    }

    #[test]
    fn a_workspace_kept_as_captured_is_never_auto_saved() {
        let mut db = Database::open_in_memory().unwrap();
        let chess = db.create_context("Chess", None).unwrap();
        let focus = db.create_context("Focus", None).unwrap();
        db.set_active_context(Some(chess.id)).unwrap();
        db.set_save_mode(chess.id, SaveMode::Fixed).unwrap();
        let out = save_active(
            &mut db,
            &Config::default(),
            Some(focus.id),
            Some("note"),
            &mut |_| {},
        );
        assert_eq!(out.from.as_deref(), Some("Chess"));
        assert_eq!(out.saved_version, None);
        assert!(out.note.unwrap().contains("always restore"));
        assert!(db.list_snapshots(chess.id).unwrap().is_empty());
    }

    #[test]
    fn switching_to_the_same_workspace_saves_nothing() {
        let mut db = Database::open_in_memory().unwrap();
        let a = db.create_context("A", None).unwrap();
        db.set_active_context(Some(a.id)).unwrap();
        assert_eq!(
            save_active(&mut db, &Config::default(), Some(a.id), None, &mut |_| {}),
            AutoSave::default()
        );
    }
}

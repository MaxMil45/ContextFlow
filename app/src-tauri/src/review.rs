//! Capture review (what a capture would save, to choose from) and comparing
//! two snapshots, or a snapshot with the screen right now.

use std::path::PathBuf;

use tauri::State;

use cf_app::capture;
use cf_core::{Rect, Snapshot, SnapshotPayload};
use cf_diff::{FieldChange, ItemStatus};

use crate::commands::{blocking, latest, open};
use crate::dto::*;
use crate::error::{CmdResult, CommandError, ErrorCode};
use crate::state::AppState;

/// Run a capture without saving anything.
pub(crate) fn capture_now(
    cfg: &cf_config::Config,
    repos: &[PathBuf],
    notes: Vec<String>,
) -> capture::CaptureResult {
    capture::capture(capture::CaptureOptions {
        only: &[],
        repos,
        notes,
        include_windows: cfg.capture.include_windows,
        unminimize: false,
        rules: &cfg.capture,
        git_timeout: cfg.timeouts.git(),
        browser: &cfg.browser,
        browser_state_dir: cf_browser::default_state_dir(),
        detect: cf_detect::Sources::installed(),
    })
}

/// What a capture would save right now: the programs, their windows, tabs and
/// folders, and the Git repositories from the last snapshot. Nothing is saved.
#[tauri::command]
pub async fn discover_capture(
    state: State<'_, AppState>,
    workspace: String,
) -> CmdResult<CaptureDiscovery> {
    let path = state.paths.db.clone();
    let cfg = state.config();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let repos: Vec<PathBuf> = latest(&db, &ctx)?
            .iter()
            .flat_map(|s| s.payload.git.iter())
            .map(|g| PathBuf::from(&g.repository_path))
            .collect();
        let r = capture_now(&cfg, &repos, Vec::new());
        let p = &r.payload;
        let apps = p
            .applications
            .iter()
            .map(|a| {
                let titles: Vec<String> = p
                    .windows
                    .iter()
                    .filter(|w| w.application_id == a.application_id)
                    .map(|w| w.title.clone())
                    .collect();
                DiscoveredApp {
                    application_id: a.application_id.clone(),
                    window_count: titles.len() as u32,
                    window_titles: titles,
                    tab_count: cf_browser::BrowserAppState::from_value(&a.adapter_state)
                        .map_or(0, |b| b.tab_count() as u32),
                    folder_count: cf_detect::saved(a.adapter_id.as_deref(), &a.adapter_state)
                        .map_or(0, |(_, s)| s.path_count() as u32),
                }
            })
            .collect();
        Ok(CaptureDiscovery {
            apps,
            repositories: p
                .git
                .iter()
                .map(|g| DiscoveredRepo {
                    path: g.repository_path.clone(),
                    branch: g.branch.clone(),
                    staged: g.staged,
                    modified: g.modified,
                    untracked: g.untracked,
                })
                .collect(),
            monitor_count: p.monitors.len() as u32,
            warnings: r.warnings,
            excluded_windows: r.excluded_windows as u32,
            minimized_windows: r.minimized_windows as u32,
        })
    })
    .await
}

// ------------------------------------------------------------------ compare

fn status(s: ItemStatus) -> DiffStatus {
    match s {
        ItemStatus::Added => DiffStatus::Added,
        ItemStatus::Removed => DiffStatus::Removed,
        ItemStatus::Changed => DiffStatus::Changed,
    }
}

fn arrow<T: std::fmt::Display>(label: &str, c: &FieldChange<T>) -> String {
    format!("{label}: {} → {}", c.before, c.after)
}

fn opt(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "none".into())
}

fn rect(r: &Rect) -> String {
    format!("{}×{} at {}, {}", r.width, r.height, r.x, r.y)
}

fn group(category: &str, label: &str, entries: Vec<DiffEntry>) -> Option<DiffGroup> {
    (!entries.is_empty()).then(|| DiffGroup {
        category: category.into(),
        label: label.into(),
        entries,
    })
}

/// Turn `cf-diff`'s report into groups the UI can show: by category, in a fixed order.
pub(crate) fn to_groups(report: &cf_diff::DiffReport) -> Vec<DiffGroup> {
    let mut groups = Vec::new();

    let mut apps = Vec::new();
    for a in &report.applications.added {
        apps.push(DiffEntry {
            status: DiffStatus::Added,
            title: a.clone(),
            details: vec![],
        });
    }
    for a in &report.applications.removed {
        apps.push(DiffEntry {
            status: DiffStatus::Removed,
            title: a.clone(),
            details: vec![],
        });
    }
    for c in &report.applications.executable_changed {
        apps.push(DiffEntry {
            status: DiffStatus::Changed,
            title: c.application_id.clone(),
            details: vec![arrow("program", &c.change)],
        });
    }
    groups.extend(group("applications", "Applications", apps));

    let windows = report
        .windows
        .iter()
        .map(|w| {
            let mut details = Vec::new();
            if let Some(c) = &w.title_change {
                details.push(arrow("title", c));
            }
            if let Some(c) = &w.rect {
                details.push(format!(
                    "size and position: {} → {}",
                    rect(&c.before),
                    rect(&c.after)
                ));
            }
            if let Some(c) = &w.display_state {
                details.push(format!("state: {:?} → {:?}", c.before, c.after).to_lowercase());
            }
            if let Some(c) = &w.monitor {
                details.push(format!("monitor: {} → {}", opt(&c.before), opt(&c.after)));
            }
            DiffEntry {
                status: status(w.status),
                title: format!("{} · {}", w.application_id, w.title),
                details,
            }
        })
        .collect();
    groups.extend(group("windows", "Windows", windows));

    let mut monitors = Vec::new();
    for m in &report.monitors.added {
        monitors.push(DiffEntry {
            status: DiffStatus::Added,
            title: m.clone(),
            details: vec![],
        });
    }
    for m in &report.monitors.removed {
        monitors.push(DiffEntry {
            status: DiffStatus::Removed,
            title: m.clone(),
            details: vec![],
        });
    }
    groups.extend(group("monitors", "Monitors", monitors));

    let repos = report
        .git
        .iter()
        .map(|g| {
            let mut details = Vec::new();
            if let Some(c) = &g.branch {
                details.push(format!("branch: {} → {}", opt(&c.before), opt(&c.after)));
            }
            if let Some(c) = &g.head_commit {
                let short = |s: &Option<String>| {
                    s.as_deref()
                        .map_or_else(|| "none".into(), |s| s.chars().take(8).collect::<String>())
                };
                details.push(format!(
                    "commit: {} → {}",
                    short(&c.before),
                    short(&c.after)
                ));
            }
            for (label, c) in [
                ("staged", &g.staged),
                ("modified", &g.modified),
                ("untracked", &g.untracked),
            ] {
                if let Some(c) = c {
                    details.push(arrow(label, c));
                }
            }
            DiffEntry {
                status: status(g.status),
                title: g.repository_path.clone(),
                details,
            }
        })
        .collect();
    groups.extend(group("repositories", "Git repositories", repos));

    if let Some(n) = &report.notes {
        let last = |v: &Vec<String>| v.last().cloned().unwrap_or_else(|| "(none)".into());
        groups.extend(group(
            "notes",
            "Progress note",
            vec![DiffEntry {
                status: DiffStatus::Changed,
                title: "Note".into(),
                details: vec![format!("“{}” → “{}”", last(&n.before), last(&n.after))],
            }],
        ));
    }
    groups
}

fn compare_payloads(
    workspace: &str,
    from_label: String,
    to_label: String,
    before: &SnapshotPayload,
    after: &SnapshotPayload,
) -> CompareResult {
    let report = cf_diff::diff(before, after);
    CompareResult {
        workspace: workspace.to_string(),
        from: from_label,
        to: to_label,
        is_empty: report.is_empty(),
        recommendations: report.notices.clone(),
        groups: to_groups(&report),
    }
}

/// Compare snapshot `from` with `to`, or with the screen right now when `to`
/// is `None` ("what has changed since I saved this?").
#[tauri::command]
pub async fn compare_snapshots(
    state: State<'_, AppState>,
    workspace: String,
    from: u32,
    to: Option<u32>,
) -> CmdResult<CompareResult> {
    let path = state.paths.db.clone();
    let cfg = state.config();
    blocking(move || {
        let db = open(&path)?;
        let ctx = db.find_context(&workspace)?;
        let base: Snapshot = db.get_snapshot(ctx.id, Some(from))?;
        match to {
            Some(v) => {
                if v == from {
                    return Err(CommandError::new(
                        ErrorCode::Conflict,
                        "choose two different versions to compare",
                    ));
                }
                let other = db.get_snapshot(ctx.id, Some(v))?;
                Ok(compare_payloads(
                    &ctx.name,
                    format!("v{from}"),
                    format!("v{v}"),
                    &base.payload,
                    &other.payload,
                ))
            }
            None => {
                let repos: Vec<PathBuf> = base
                    .payload
                    .git
                    .iter()
                    .map(|g| PathBuf::from(&g.repository_path))
                    .collect();
                let notes = latest(&db, &ctx)?
                    .map(|s| s.payload.notes)
                    .unwrap_or_default();
                let live = capture_now(&cfg, &repos, notes);
                Ok(compare_payloads(
                    &ctx.name,
                    format!("v{from}"),
                    "your screen right now".into(),
                    &base.payload,
                    &live.payload,
                ))
            }
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_core::{DisplayState, WindowState};

    fn window(app: &str, title: &str, x: i32) -> WindowState {
        WindowState {
            application_id: app.into(),
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

    #[test]
    fn a_comparison_is_grouped_by_category_with_readable_details() {
        let mut a = SnapshotPayload::default();
        a.windows.push(window("code", "main.rs", 0));
        a.notes.push("before".into());
        let mut b = SnapshotPayload::default();
        b.windows.push(window("code", "main.rs", 100));
        b.windows.push(window("chrome", "Docs", 0));
        b.notes.push("after".into());

        let r = compare_payloads("W", "v1".into(), "v2".into(), &a, &b);
        assert!(!r.is_empty);
        let cats: Vec<_> = r.groups.iter().map(|g| g.category.as_str()).collect();
        assert_eq!(
            cats,
            vec!["windows", "notes"],
            "fixed order, empty groups left out"
        );
        let windows = &r.groups[0].entries;
        assert!(windows
            .iter()
            .any(|e| e.status == DiffStatus::Added && e.title.contains("chrome")));
        let moved = windows
            .iter()
            .find(|e| e.status == DiffStatus::Changed)
            .unwrap();
        assert!(
            moved.details[0].contains("800×600 at 0, 0 → 800×600 at 100, 0"),
            "{:?}",
            moved.details
        );
        assert!(r.groups[1].entries[0].details[0].contains("“before” → “after”"));
    }

    #[test]
    fn identical_snapshots_compare_as_empty() {
        let a = SnapshotPayload::default();
        let r = compare_payloads("W", "v1".into(), "v2".into(), &a, &a);
        assert!(r.is_empty && r.groups.is_empty() && r.recommendations.is_empty());
    }
}

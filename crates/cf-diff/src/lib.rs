//! Context Diff: a deterministic, explainable comparison of two workspace
//! states (two snapshots, or a snapshot and the live environment).
//!
//! Output ordering is fully determined by the inputs (BTreeMap keys and input
//! order), so the same pair of payloads always yields the same report.

use std::collections::{BTreeMap, BTreeSet};

use cf_core::{DisplayState, GitState, Rect, SnapshotPayload, WindowState};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldChange<T> {
    pub before: T,
    pub after: T,
}

fn change<T: PartialEq + Clone>(before: &T, after: &T) -> Option<FieldChange<T>> {
    (before != after).then(|| FieldChange {
        before: before.clone(),
        after: after.clone(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SetDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl SetDiff {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutableChange {
    pub application_id: String,
    pub change: FieldChange<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ApplicationDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub executable_changed: Vec<ExecutableChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitRepoDiff {
    pub repository_path: String,
    pub status: ItemStatus,
    pub branch: Option<FieldChange<Option<String>>>,
    pub head_commit: Option<FieldChange<Option<String>>>,
    pub staged: Option<FieldChange<u32>>,
    pub modified: Option<FieldChange<u32>>,
    pub untracked: Option<FieldChange<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WindowDiff {
    pub application_id: String,
    pub title: String,
    pub status: ItemStatus,
    pub title_change: Option<FieldChange<String>>,
    pub rect: Option<FieldChange<Rect>>,
    pub display_state: Option<FieldChange<DisplayState>>,
    pub monitor: Option<FieldChange<Option<String>>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DiffReport {
    pub applications: ApplicationDiff,
    pub monitors: SetDiff,
    pub git: Vec<GitRepoDiff>,
    pub windows: Vec<WindowDiff>,
    pub notes: Option<FieldChange<Vec<String>>>,
    /// Plain-language consequences and safe-restore guidance.
    pub notices: Vec<String>,
}

impl DiffReport {
    pub fn is_empty(&self) -> bool {
        self.applications.added.is_empty()
            && self.applications.removed.is_empty()
            && self.applications.executable_changed.is_empty()
            && self.monitors.is_empty()
            && self.git.is_empty()
            && self.windows.is_empty()
            && self.notes.is_none()
    }

    /// Human-readable summary, one change per line.
    pub fn summary_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for a in &self.applications.added {
            out.push(format!("+ application {a}"));
        }
        for a in &self.applications.removed {
            out.push(format!("- application {a}"));
        }
        for c in &self.applications.executable_changed {
            out.push(format!(
                "~ application {}: executable {} -> {}",
                c.application_id, c.change.before, c.change.after
            ));
        }
        for m in &self.monitors.added {
            out.push(format!("+ monitor {m}"));
        }
        for m in &self.monitors.removed {
            out.push(format!("- monitor {m}"));
        }
        for g in &self.git {
            match g.status {
                ItemStatus::Added => out.push(format!("+ repository {}", g.repository_path)),
                ItemStatus::Removed => out.push(format!("- repository {}", g.repository_path)),
                ItemStatus::Changed => {
                    let mut parts = Vec::new();
                    if let Some(b) = &g.branch {
                        parts.push(format!("branch {} -> {}", opt(&b.before), opt(&b.after)));
                    }
                    if let Some(h) = &g.head_commit {
                        parts.push(format!("HEAD {} -> {}", short(&h.before), short(&h.after)));
                    }
                    for (label, c) in [
                        ("staged", &g.staged),
                        ("modified", &g.modified),
                        ("untracked", &g.untracked),
                    ] {
                        if let Some(c) = c {
                            parts.push(format!("{label} {} -> {}", c.before, c.after));
                        }
                    }
                    out.push(format!(
                        "~ repository {}: {}",
                        g.repository_path,
                        parts.join(", ")
                    ));
                }
            }
        }
        for w in &self.windows {
            let sign = match w.status {
                ItemStatus::Added => "+",
                ItemStatus::Removed => "-",
                ItemStatus::Changed => "~",
            };
            let mut line = format!("{sign} window [{}] {}", w.application_id, w.title);
            if let Some(r) = &w.rect {
                line.push_str(&format!(
                    " moved {},{} {}x{} -> {},{} {}x{}",
                    r.before.x,
                    r.before.y,
                    r.before.width,
                    r.before.height,
                    r.after.x,
                    r.after.y,
                    r.after.width,
                    r.after.height
                ));
            }
            if let Some(s) = &w.display_state {
                line.push_str(&format!(" {} -> {}", s.before.as_str(), s.after.as_str()));
            }
            if let Some(t) = &w.title_change {
                line.push_str(&format!(" (was \"{}\")", t.before));
            }
            out.push(line);
        }
        if self.notes.is_some() {
            out.push("~ progress notes changed".into());
        }
        out
    }
}

fn opt(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("(detached)")
}

fn short(v: &Option<String>) -> String {
    v.as_deref()
        .map(|s| s.chars().take(10).collect())
        .unwrap_or_else(|| "(none)".into())
}

/// Case- and separator-insensitive path key (Windows paths are case-insensitive).
fn path_key(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// Compare `before` (usually a saved snapshot) with `after` (a newer snapshot
/// or the live environment).
pub fn diff(before: &SnapshotPayload, after: &SnapshotPayload) -> DiffReport {
    let mut report = DiffReport {
        applications: diff_applications(before, after),
        monitors: diff_monitors(before, after),
        git: diff_git(&before.git, &after.git),
        windows: diff_windows(&before.windows, &after.windows),
        notes: change(&before.notes, &after.notes),
        notices: Vec::new(),
    };
    report.notices = notices(&report);
    report
}

fn diff_applications(before: &SnapshotPayload, after: &SnapshotPayload) -> ApplicationDiff {
    let index = |p: &SnapshotPayload| -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        for a in &p.applications {
            m.entry(a.application_id.clone())
                .or_insert_with(|| a.executable_path.clone());
        }
        m
    };
    let (b, a) = (index(before), index(after));
    let mut out = ApplicationDiff::default();
    for (id, exe) in &b {
        match a.get(id) {
            None => out.removed.push(id.clone()),
            Some(new_exe) if path_key(new_exe) != path_key(exe) => {
                out.executable_changed.push(ExecutableChange {
                    application_id: id.clone(),
                    change: FieldChange {
                        before: exe.clone(),
                        after: new_exe.clone(),
                    },
                })
            }
            Some(_) => {}
        }
    }
    out.added = a
        .keys()
        .filter(|id| !b.contains_key(*id))
        .cloned()
        .collect();
    out
}

fn diff_monitors(before: &SnapshotPayload, after: &SnapshotPayload) -> SetDiff {
    let b: BTreeSet<&str> = before.monitors.iter().map(|m| m.id.as_str()).collect();
    let a: BTreeSet<&str> = after.monitors.iter().map(|m| m.id.as_str()).collect();
    SetDiff {
        added: a.difference(&b).map(|s| s.to_string()).collect(),
        removed: b.difference(&a).map(|s| s.to_string()).collect(),
    }
}

fn diff_git(before: &[GitState], after: &[GitState]) -> Vec<GitRepoDiff> {
    let index = |g: &[GitState]| -> BTreeMap<String, GitState> {
        g.iter()
            .map(|s| (path_key(&s.repository_path), s.clone()))
            .collect()
    };
    let (b, a) = (index(before), index(after));
    let keys: BTreeSet<&String> = b.keys().chain(a.keys()).collect();
    let mut out = Vec::new();
    for key in keys {
        let entry = |path: &str, status| GitRepoDiff {
            repository_path: path.to_string(),
            status,
            branch: None,
            head_commit: None,
            staged: None,
            modified: None,
            untracked: None,
        };
        match (b.get(key), a.get(key)) {
            (Some(old), None) => out.push(entry(&old.repository_path, ItemStatus::Removed)),
            (None, Some(new)) => out.push(entry(&new.repository_path, ItemStatus::Added)),
            (Some(old), Some(new)) => {
                let d = GitRepoDiff {
                    branch: change(&old.branch, &new.branch),
                    head_commit: change(&old.head_commit, &new.head_commit),
                    staged: change(&old.staged, &new.staged),
                    modified: change(&old.modified, &new.modified),
                    untracked: change(&old.untracked, &new.untracked),
                    ..entry(&new.repository_path, ItemStatus::Changed)
                };
                let changed = d.branch.is_some()
                    || d.head_commit.is_some()
                    || d.staged.is_some()
                    || d.modified.is_some()
                    || d.untracked.is_some();
                if changed {
                    out.push(d);
                }
            }
            (None, None) => unreachable!(),
        }
    }
    out
}

/// Windows are matched per application: first by identical title, then the
/// remainder pairwise in capture order.
fn diff_windows(before: &[WindowState], after: &[WindowState]) -> Vec<WindowDiff> {
    let group = |ws: &[WindowState]| -> BTreeMap<String, Vec<WindowState>> {
        let mut m: BTreeMap<String, Vec<WindowState>> = BTreeMap::new();
        for w in ws {
            m.entry(w.application_id.clone())
                .or_default()
                .push(w.clone());
        }
        m
    };
    let (b, a) = (group(before), group(after));
    let apps: BTreeSet<&String> = b.keys().chain(a.keys()).collect();
    let empty = Vec::new();
    let mut out = Vec::new();

    for app in apps {
        let olds = b.get(app).unwrap_or(&empty);
        let news = a.get(app).unwrap_or(&empty);
        let mut used = vec![false; news.len()];
        let mut pairs: Vec<(&WindowState, &WindowState)> = Vec::new();
        let mut unmatched = Vec::new();

        for o in olds {
            match (0..news.len()).find(|&j| !used[j] && news[j].title == o.title) {
                Some(j) => {
                    used[j] = true;
                    pairs.push((o, &news[j]));
                }
                None => unmatched.push(o),
            }
        }
        let mut remaining = (0..news.len())
            .filter(|&j| !used[j])
            .collect::<Vec<_>>()
            .into_iter();
        let mut removed = Vec::new();
        for o in unmatched {
            match remaining.next() {
                Some(j) => pairs.push((o, &news[j])),
                None => removed.push(o),
            }
        }

        for (o, n) in pairs {
            let d = WindowDiff {
                application_id: app.clone(),
                title: n.title.clone(),
                status: ItemStatus::Changed,
                title_change: change(&o.title, &n.title),
                rect: change(&o.rect, &n.rect),
                display_state: change(&o.display_state, &n.display_state),
                monitor: change(&o.monitor_id, &n.monitor_id),
            };
            if d.title_change.is_some()
                || d.rect.is_some()
                || d.display_state.is_some()
                || d.monitor.is_some()
            {
                out.push(d);
            }
        }
        let simple = |w: &WindowState, status| WindowDiff {
            application_id: app.clone(),
            title: w.title.clone(),
            status,
            title_change: None,
            rect: None,
            display_state: None,
            monitor: None,
        };
        for o in removed {
            out.push(simple(o, ItemStatus::Removed));
        }
        for j in remaining {
            out.push(simple(&news[j], ItemStatus::Added));
        }
    }
    out
}

fn notices(r: &DiffReport) -> Vec<String> {
    let mut out = Vec::new();
    for g in &r.git {
        match g.status {
            ItemStatus::Removed => out.push(format!(
                "Repository {} could not be found or inspected; it will be skipped.",
                g.repository_path
            )),
            ItemStatus::Changed => {
                if let Some(b) = &g.branch {
                    out.push(format!(
                        "Repository {} is on branch {} (snapshot: {}). ContextFlow never switches branches; check out {} yourself if you need it.",
                        g.repository_path,
                        opt(&b.after),
                        opt(&b.before),
                        opt(&b.before)
                    ));
                }
                if g.head_commit.is_some() && g.branch.is_none() {
                    out.push(format!(
                        "Repository {} has moved to a different commit since the snapshot. Your newer work is kept as is.",
                        g.repository_path
                    ));
                }
            }
            ItemStatus::Added => {}
        }
    }
    for m in &r.monitors.removed {
        out.push(format!(
            "Monitor {m} is no longer connected; its windows will be moved onto an available display."
        ));
    }
    for a in &r.applications.removed {
        out.push(format!(
            "Application {a} is missing from the current environment."
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_core::{ApplicationState, MonitorInfo};

    fn app(id: &str, exe: &str) -> ApplicationState {
        ApplicationState {
            application_id: id.into(),
            executable_path: exe.into(),
            launch_arguments: vec![],
            working_directory: None,
            adapter_id: None,
            adapter_state: serde_json::Value::Null,
        }
    }

    fn win(app: &str, title: &str, x: i32) -> WindowState {
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

    fn mon(id: &str) -> MonitorInfo {
        let r = Rect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        MonitorInfo {
            id: id.into(),
            primary: true,
            rect: r,
            work_area: r,
            dpi: None,
        }
    }

    fn base() -> SnapshotPayload {
        SnapshotPayload {
            applications: vec![
                app("code", r"C:\VSCode\Code.exe"),
                app("chrome", r"C:\Chrome\chrome.exe"),
            ],
            windows: vec![win("code", "main.rs - cf", 0), win("chrome", "Docs", 900)],
            monitors: vec![mon("M1"), mon("M2")],
            git: vec![GitState {
                repository_path: r"C:\src\cf".into(),
                branch: Some("main".into()),
                head_commit: Some("a".repeat(40)),
                modified: 2,
                ..Default::default()
            }],
            notes: vec!["finish storage layer".into()],
            ..Default::default()
        }
    }

    #[test]
    fn identical_payloads_produce_empty_report() {
        let r = diff(&base(), &base());
        assert!(r.is_empty(), "{r:#?}");
        assert!(r.notices.is_empty());
    }

    #[test]
    fn detects_spec_example_divergence() {
        let before = base();
        let mut after = base();
        after.git[0].branch = Some("feature/database".into());
        after.git[0].head_commit = Some("b".repeat(40));
        after.git[0].modified = 5;
        after.git[0].repository_path = r"c:/SRC/cf/".into(); // same repo, different spelling
        after.applications.retain(|a| a.application_id != "chrome");
        after.applications.push(app("wt", r"C:\WT\wt.exe"));
        after.monitors.retain(|m| m.id != "M2");
        after.notes.push("db schema done".into());

        let r = diff(&before, &after);
        assert_eq!(r.git.len(), 1);
        let g = &r.git[0];
        assert_eq!(g.status, ItemStatus::Changed);
        assert_eq!(
            g.branch.as_ref().unwrap().after.as_deref(),
            Some("feature/database")
        );
        assert_eq!(
            g.modified,
            Some(FieldChange {
                before: 2,
                after: 5
            })
        );
        assert_eq!(r.applications.added, vec!["wt"]);
        assert_eq!(r.applications.removed, vec!["chrome"]);
        assert_eq!(r.monitors.removed, vec!["M2"]);
        assert!(r.notes.is_some());
        assert!(r
            .notices
            .iter()
            .any(|n| n.contains("never switches branches")));
        assert!(r.notices.iter().any(|n| n.contains("M2")));
    }

    #[test]
    fn windows_match_by_title_then_order() {
        let before = SnapshotPayload {
            windows: vec![
                win("code", "a.rs", 0),
                win("code", "b.rs", 100),
                win("code", "c.rs", 200),
            ],
            ..Default::default()
        };
        let after = SnapshotPayload {
            windows: vec![win("code", "b.rs", 150), win("code", "z.rs", 0)],
            ..Default::default()
        };
        let r = diff(&before, &after);
        // b.rs matched by title and moved; a.rs paired with z.rs (retitled); c.rs removed.
        assert_eq!(r.windows.len(), 3);
        let moved = r.windows.iter().find(|w| w.title == "b.rs").unwrap();
        assert_eq!(moved.rect.as_ref().unwrap().after.x, 150);
        let retitled = r.windows.iter().find(|w| w.title == "z.rs").unwrap();
        assert_eq!(retitled.title_change.as_ref().unwrap().before, "a.rs");
        assert_eq!(
            r.windows
                .iter()
                .filter(|w| w.status == ItemStatus::Removed)
                .count(),
            1
        );
    }

    #[test]
    fn output_is_deterministic() {
        let mut after = base();
        after.git[0].branch = None;
        let a = diff(&base(), &after);
        let b = diff(&base(), &after);
        assert_eq!(a, b);
        assert_eq!(a.summary_lines(), b.summary_lines());
        assert!(a.summary_lines()[0].contains("(detached)"));
    }
}

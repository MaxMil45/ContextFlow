//! Search across what your workspaces saved: "which workspace had that tab,
//! that folder, that note?". Looks at the newest snapshot of every workspace
//! that isn't archived. Everything stays local; nothing is indexed elsewhere.

use cf_core::SnapshotPayload;
use cf_storage::Database;

/// Most results returned.
pub const MAX_HITS: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub workspace_id: String,
    pub workspace: String,
    pub version: u32,
    /// `tab`, `folder`, `note`, `repository`, `window` or `app`.
    pub kind: &'static str,
    pub text: String,
    /// More detail (a tab's address, a folder's path), when there is some.
    pub detail: Option<String>,
}

/// Every searchable thing in one snapshot: (kind, text, detail).
pub fn items(p: &SnapshotPayload) -> Vec<(&'static str, String, Option<String>)> {
    let mut out = Vec::new();
    for note in &p.notes {
        out.push(("note", note.clone(), None));
    }
    for a in &p.applications {
        out.push(("app", a.application_id.clone(), None));
        if let Some(b) = cf_browser::BrowserAppState::from_value(&a.adapter_state) {
            for t in b.windows.iter().flat_map(|w| &w.tabs) {
                out.push(("tab", t.title.clone(), Some(t.url.clone())));
            }
        }
        if let Some(v) = cf_vscode::VsCodeState::from_value(&a.adapter_state) {
            for w in &v.windows {
                if let Some(path) = w.folder_uri.as_deref().and_then(cf_vscode::folder_path) {
                    out.push((
                        "folder",
                        w.root_name.clone().unwrap_or_default(),
                        Some(path),
                    ));
                }
            }
        }
        if let Some(v) = cf_vstudio::VisualStudioState::from_value(&a.adapter_state) {
            for w in v.windows.iter().filter(|w| w.path.is_some()) {
                out.push(("folder", w.name.clone().unwrap_or_default(), w.path.clone()));
            }
        }
        if let Some(j) = cf_jetbrains::JetBrainsState::from_value(&a.adapter_state) {
            for w in j.windows.iter().filter(|w| w.path.is_some()) {
                out.push(("folder", w.name.clone().unwrap_or_default(), w.path.clone()));
            }
        }
        if let Some(e) = cf_explorer::ExplorerState::from_value(&a.adapter_state) {
            for w in e.windows.iter().filter(|w| w.path.is_some()) {
                out.push(("folder", w.name.clone().unwrap_or_default(), w.path.clone()));
            }
        }
    }
    for g in &p.git {
        out.push((
            "repository",
            g.branch.clone().unwrap_or_else(|| "detached".into()),
            Some(g.repository_path.clone()),
        ));
    }
    for w in &p.windows {
        out.push(("window", w.title.clone(), None));
    }
    out
}

/// Whether every word of `query` appears in `text` or `detail`.
fn matches(words: &[String], text: &str, detail: Option<&str>) -> bool {
    let hay = format!("{} {}", text, detail.unwrap_or("")).to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// Search the newest snapshot of every workspace. Notes, tabs and folders come
/// before window titles and app names, and duplicates are dropped.
pub fn search(db: &Database, query: &str) -> anyhow::Result<Vec<Hit>> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() || query.trim().chars().count() < 2 {
        return Ok(Vec::new());
    }
    let rank = |k: &str| {
        ["note", "tab", "folder", "repository", "window", "app"]
            .iter()
            .position(|x| *x == k)
    };
    let mut hits = Vec::new();
    for ctx in db.list_contexts(false)? {
        let Ok(snap) = db.get_snapshot(ctx.id, None) else {
            continue;
        };
        for (kind, text, detail) in items(&snap.payload) {
            if matches(&words, &text, detail.as_deref())
                && !hits.iter().any(|h: &Hit| {
                    h.workspace_id == ctx.id.to_string() && h.kind == kind && h.text == text
                })
            {
                hits.push(Hit {
                    workspace_id: ctx.id.to_string(),
                    workspace: ctx.name.clone(),
                    version: snap.version,
                    kind,
                    text,
                    detail,
                });
            }
        }
    }
    hits.sort_by_key(|h| rank(h.kind));
    hits.truncate(MAX_HITS);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_core::{ApplicationState, CaptureStatus};

    #[test]
    fn tabs_notes_and_folders_are_found_across_workspaces() {
        let mut db = Database::open_in_memory().unwrap();
        let a = db.create_context("Thesis", None).unwrap();
        let b = db.create_context("Chess", None).unwrap();
        let browser = cf_browser::BrowserAppState {
            schema_version: cf_browser::saved::STATE_SCHEMA,
            browser: "chrome".into(),
            windows: vec![cf_browser::SavedWindow {
                window_title: "x".into(),
                tabs: vec![cf_browser::SavedTab {
                    url: "https://www.chess.com/play".into(),
                    title: "Play Chess Online".into(),
                    pinned: false,
                }],
                active_tab: Some(0),
            }],
        };
        let chrome = ApplicationState {
            application_id: "chrome".into(),
            executable_path: "C:\\chrome.exe".into(),
            launch_arguments: vec![],
            working_directory: None,
            adapter_id: Some("browser".into()),
            adapter_state: browser.to_value(),
        };
        let thesis = SnapshotPayload {
            notes: vec!["finish the methods chapter".into()],
            ..Default::default()
        };
        let chess = SnapshotPayload {
            applications: vec![chrome],
            ..Default::default()
        };
        db.create_snapshot(a.id, thesis, CaptureStatus::Complete)
            .unwrap();
        db.create_snapshot(b.id, chess, CaptureStatus::Complete)
            .unwrap();

        let hits = search(&db, "chess.com").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].workspace.as_str(), hits[0].kind), ("Chess", "tab"));
        let hits = search(&db, "METHODS chapter").unwrap();
        assert_eq!(
            (hits[0].workspace.as_str(), hits[0].kind),
            ("Thesis", "note")
        );
        assert!(
            search(&db, "c").unwrap().is_empty(),
            "one letter is too short"
        );
    }
}

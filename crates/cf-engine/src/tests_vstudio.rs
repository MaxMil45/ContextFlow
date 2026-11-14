// Visual Studio solution restore tests. Included at the end of tests.rs, so they
// share its helpers (`Fake`, `app`, `r`, `monitor`, `open_window`, ...).

use cf_vstudio::{VisualStudioState, VsWindow};

const SLN: &str = r"C:\work\CorpusAudit\CorpusAudit.sln";
const FOLDER: &str = r"C:\work\Notes";
const SLN_TITLE: &str = "CorpusAudit - README.md - Microsoft Visual Studio";
const FOLDER_TITLE: &str = "Notes - Microsoft Visual Studio";

fn studio_app(windows: Vec<VsWindow>) -> ApplicationState {
    let mut a = app("devenv");
    a.adapter_id = Some("visualstudio".into());
    a.adapter_state = VisualStudioState {
        schema_version: 1,
        windows,
    }
    .to_value();
    a
}

fn studio_window(title: &str, name: &str, path: Option<&str>) -> VsWindow {
    VsWindow {
        window_title: title.into(),
        name: Some(name.into()),
        path: path.map(String::from),
    }
}

fn studio_payload() -> SnapshotPayload {
    SnapshotPayload {
        applications: vec![studio_app(vec![
            studio_window(SLN_TITLE, "CorpusAudit", Some(SLN)),
            studio_window(FOLDER_TITLE, "Notes", Some(FOLDER)),
        ])],
        windows: vec![
            saved_window("devenv", SLN_TITLE, r(0, 0, 1200, 900), "M1"),
            saved_window("devenv", FOLDER_TITLE, r(1200, 0, 700, 900), "M1"),
        ],
        ..Default::default()
    }
}

#[test]
fn closed_visual_studio_reopens_each_solution_in_place() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        browser_windows: RefCell::new(vec![
            open_window(21, "devenv", "x"),
            open_window(22, "devenv", "y"),
        ]),
        ..Default::default()
    };
    let result = run(&d, &studio_payload(), &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(
        *d.app_window_opens.borrow(),
        vec![vec![SLN.to_string()], vec![FOLDER.to_string()]],
        "devenv.exe <path> for each saved window"
    );
    let placed = d.placed.borrow();
    assert!(placed.contains(&(21, r(0, 0, 1200, 900), DisplayState::Normal)));
    assert!(placed.contains(&(22, r(1200, 0, 700, 900), DisplayState::Normal)));
    assert!(
        result.steps[0].message.contains("solution(s) or folder(s)"),
        "{}",
        result.steps[0].message
    );
}

#[test]
fn running_visual_studio_only_opens_what_is_not_open() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        // The solution is already open (its document changed since: still the same solution).
        open: RefCell::new(vec![open_window(1, "devenv", "CorpusAudit - Program.cs - Microsoft Visual Studio")]),
        browser_windows: RefCell::new(vec![open_window(2, "devenv", "Notes")]),
        ..Default::default()
    };
    run(&d, &studio_payload(), &fast());
    assert_eq!(*d.app_window_opens.borrow(), vec![vec![FOLDER.to_string()]]);
}

#[test]
fn unsafe_saved_paths_are_never_put_on_a_command_line() {
    let payload = SnapshotPayload {
        applications: vec![studio_app(vec![studio_window(SLN_TITLE, "CorpusAudit", Some("--evil"))])],
        windows: vec![saved_window("devenv", SLN_TITLE, r(0, 0, 900, 800), "M1")],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        ..Default::default()
    };
    run(&d, &payload, &fast());
    assert!(d.app_window_opens.borrow().is_empty());
}

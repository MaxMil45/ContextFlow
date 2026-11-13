// VS Code folder restore tests. Included at the end of tests.rs, so they share
// its helpers (`Fake`, `app`, `r`, `monitor`, `open_window`, ...).

use cf_vscode::{VsCodeState, VsCodeWindow};

const ALPHA: &str = "file:///c%3A/work/Alpha";
const BETA: &str = "file:///c%3A/work/Beta";
const ALPHA_TITLE: &str = "main.rs - Alpha - Visual Studio Code";
const BETA_TITLE: &str = "Beta - Visual Studio Code";

fn vs_window(title: &str, root: &str, uri: Option<&str>) -> VsCodeWindow {
    VsCodeWindow {
        window_title: title.into(),
        root_name: Some(root.into()),
        folder_uri: uri.map(String::from),
    }
}

fn code_app(windows: Vec<VsCodeWindow>) -> ApplicationState {
    let mut a = app("code");
    a.adapter_id = Some("vscode".into());
    a.adapter_state = VsCodeState {
        schema_version: 1,
        windows,
    }
    .to_value();
    a
}

/// Two VS Code windows, each with its own folder and position.
fn code_payload() -> SnapshotPayload {
    SnapshotPayload {
        applications: vec![code_app(vec![
            vs_window(ALPHA_TITLE, "Alpha", Some(ALPHA)),
            vs_window(BETA_TITLE, "Beta", Some(BETA)),
        ])],
        windows: vec![
            saved_window("code", ALPHA_TITLE, r(0, 0, 900, 800), "M1"),
            saved_window("code", BETA_TITLE, r(1000, 0, 800, 700), "M1"),
        ],
        ..Default::default()
    }
}

fn folder_args(uri: &str) -> Vec<String> {
    vec!["--new-window".into(), "--folder-uri".into(), uri.into()]
}

#[test]
fn closed_vscode_reopens_each_folder_in_place() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        // The windows appear in the order they are opened.
        browser_windows: RefCell::new(vec![
            open_window(11, "code", "x"),
            open_window(12, "code", "y"),
        ]),
        ..Default::default()
    };
    let result = run(&d, &code_payload(), &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(
        *d.app_window_opens.borrow(),
        vec![folder_args(ALPHA), folder_args(BETA)]
    );
    // Each new window went to *its own* saved position.
    let placed = d.placed.borrow();
    assert!(placed.contains(&(11, r(0, 0, 900, 800), DisplayState::Normal)));
    assert!(placed.contains(&(12, r(1000, 0, 800, 700), DisplayState::Normal)));
    assert!(
        result.steps[0]
            .message
            .contains("2 folder(s) opened in 2 window(s)"),
        "{}",
        result.steps[0].message
    );
    assert!(d.launched.borrow().is_empty(), "not started by a bare launch");
}

#[test]
fn running_vscode_only_opens_folders_that_are_not_open() {
    // Alpha is already open; Beta is not.
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![open_window(1, "code", ALPHA_TITLE)]),
        browser_windows: RefCell::new(vec![open_window(2, "code", "Beta")]),
        ..Default::default()
    };
    let result = run(&d, &code_payload(), &fast());
    assert_eq!(*d.app_window_opens.borrow(), vec![folder_args(BETA)]);
    assert!(
        result.steps[0]
            .message
            .contains("added 1 missing folder(s) in 1 new window(s)"),
        "{}",
        result.steps[0].message
    );
    let placed = d.placed.borrow();
    // The new window is placed once (by the folder stage), the old one repositioned.
    assert_eq!(placed.iter().filter(|(h, ..)| *h == 2).count(), 1);
    assert!(placed.iter().any(|(h, ..)| *h == 1));
    drop(placed);

    // With both open, a second restore opens nothing.
    let d2 = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![
            open_window(1, "code", ALPHA_TITLE),
            open_window(2, "code", BETA_TITLE),
        ]),
        ..Default::default()
    };
    run(&d2, &code_payload(), &fast());
    assert!(d2.app_window_opens.borrow().is_empty());
}

#[test]
fn unsafe_folder_uris_are_never_passed_to_the_command_line() {
    let p = SnapshotPayload {
        applications: vec![code_app(vec![
            vs_window("a - Visual Studio Code", "a", Some("--extensionDevelopmentPath=x")),
            vs_window("b - Visual Studio Code", "b", Some("file:///c:/a b")),
            vs_window("c - Visual Studio Code", "c", Some("https://example.com/x")),
            // One safe folder keeps the app on the folder path.
            vs_window("Safe - Visual Studio Code", "Safe", Some("file:///c%3A/work/Safe")),
        ])],
        windows: vec![],
        ..Default::default()
    };
    let d = Fake {
        browser_windows: RefCell::new(vec![open_window(1, "code", "Safe")]),
        ..Default::default()
    };
    run(&d, &p, &fast());
    let opens = d.app_window_opens.borrow();
    assert_eq!(*opens, vec![folder_args("file:///c%3A/work/Safe")]);
    for args in opens.iter() {
        assert!(
            args.iter()
                .all(|a| !a.contains("extensionDevelopmentPath") && !a.contains("example.com")),
            "{args:?}"
        );
    }
}

#[test]
fn vscode_without_saved_folders_behaves_like_any_other_app() {
    let mut a = app("code");
    a.adapter_id = None;
    let p = SnapshotPayload {
        applications: vec![a],
        windows: vec![saved_window("code", "x", r(0, 0, 500, 500), "M1")],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        on_launch: vec![("code".into(), vec![open_window(1, "code", "x")])],
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(*d.launched.borrow(), vec!["code".to_string()]);
    assert!(d.app_window_opens.borrow().is_empty());
}

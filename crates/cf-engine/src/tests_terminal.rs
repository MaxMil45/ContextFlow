// Windows Terminal restore tests. Included at the end of tests.rs, so they
// share its helpers (`Fake`, `app`, `r`, `monitor`, `open_window`, ...).

use cf_detect::{State as TerminalState, Window as TerminalWindow};

fn terminal_payload() -> SnapshotPayload {
    let window = |path: &str, tabs: &[&str]| TerminalWindow {
        window_title: "PowerShell".into(),
        name: None,
        path: Some(path.into()),
        tabs: tabs.iter().map(|t| t.to_string()).collect(),
    };
    let mut a = app("windowsterminal");
    a.adapter_id = Some("terminal".into());
    a.adapter_state = TerminalState {
        schema_version: 1,
        windows: vec![
            window(r"C:\work\api", &[r"C:\work\web"]),
            window(r"C:\notes", &[]),
        ],
    }
    .to_value();
    SnapshotPayload {
        applications: vec![a],
        windows: vec![
            saved_window("windowsterminal", "PowerShell", r(0, 0, 900, 800), "M1"),
            saved_window("windowsterminal", "PowerShell", r(1000, 0, 800, 700), "M1"),
        ],
        ..Default::default()
    }
}

fn wt_args(folders: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = ["-w", "new", "-d", folders[0]].map(String::from).into();
    for f in &folders[1..] {
        args.extend([";", "new-tab", "-d", f].map(String::from));
    }
    args
}

#[test]
fn closed_terminal_reopens_each_window_with_a_tab_per_folder_in_place() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        browser_windows: RefCell::new(vec![
            open_window(21, "windowsterminal", "x"),
            open_window(22, "windowsterminal", "y"),
        ]),
        ..Default::default()
    };
    let result = run(&d, &terminal_payload(), &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(
        *d.app_window_opens.borrow(),
        vec![
            wt_args(&[r"C:\work\api", r"C:\work\web"]),
            wt_args(&[r"C:\notes"])
        ]
    );
    let placed = d.placed.borrow();
    assert!(placed.contains(&(21, r(0, 0, 900, 800), DisplayState::Normal)));
    assert!(placed.contains(&(22, r(1000, 0, 800, 700), DisplayState::Normal)));
}

#[test]
fn a_running_terminal_with_enough_windows_gets_no_duplicates() {
    // One window is open (the titles say nothing about folders): only the
    // second saved window is missing.
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![open_window(1, "windowsterminal", "PowerShell")]),
        browser_windows: RefCell::new(vec![open_window(2, "windowsterminal", "y")]),
        ..Default::default()
    };
    run(&d, &terminal_payload(), &fast());
    assert_eq!(*d.app_window_opens.borrow(), vec![wt_args(&[r"C:\notes"])]);

    let d2 = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![
            open_window(1, "windowsterminal", "PowerShell"),
            open_window(2, "windowsterminal", "PowerShell"),
        ]),
        ..Default::default()
    };
    run(&d2, &terminal_payload(), &fast());
    assert!(d2.app_window_opens.borrow().is_empty());
}

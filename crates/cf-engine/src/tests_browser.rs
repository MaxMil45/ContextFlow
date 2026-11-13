// Browser tab restore tests. Included at the end of tests.rs, so they share its
// helpers (`Fake`, `app`, `r`, `monitor`, `open_window`, ...).

use cf_browser::{BrowserAppState, SavedTab, SavedWindow};

fn tab(url: &str) -> SavedTab {
    SavedTab {
        url: url.into(),
        title: url.into(),
        pinned: false,
    }
}

fn browser_app(windows: Vec<SavedWindow>) -> ApplicationState {
    let mut a = app("chrome");
    a.adapter_id = Some("browser".into());
    a.adapter_state = BrowserAppState {
        schema_version: 1,
        browser: "chrome".into(),
        windows,
    }
    .to_value();
    a
}

fn saved_tabs_window(title: &str, urls: &[&str]) -> SavedWindow {
    SavedWindow {
        window_title: title.into(),
        tabs: urls.iter().map(|u| tab(u)).collect(),
        active_tab: Some(0),
    }
}

/// Two saved Chrome windows with different tabs and positions.
fn chrome_payload() -> SnapshotPayload {
    SnapshotPayload {
        applications: vec![browser_app(vec![
            saved_tabs_window(
                "Drive - Google Chrome",
                &[
                    "https://drive.google.com/drive/x",
                    "https://mail.google.com",
                ],
            ),
            saved_tabs_window("Docs - Google Chrome", &["https://docs.rs"]),
        ])],
        windows: vec![
            saved_window("chrome", "Drive - Google Chrome", r(0, 0, 900, 800), "M1"),
            saved_window("chrome", "Docs - Google Chrome", r(1000, 0, 800, 700), "M1"),
        ],
        ..Default::default()
    }
}

#[test]
fn closed_browser_is_started_with_each_window_and_tabs_and_placed() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        // The windows appear in the order they are opened.
        browser_windows: RefCell::new(vec![
            open_window(11, "chrome", "New Tab"),
            open_window(12, "chrome", "New Tab"),
        ]),
        ..Default::default()
    };
    let result = run(&d, &chrome_payload(), &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(
        *d.browser_opens.borrow(),
        vec![
            vec![
                "https://drive.google.com/drive/x".to_string(),
                "https://mail.google.com".to_string()
            ],
            vec!["https://docs.rs".to_string()],
        ]
    );
    // Each new window went to *its own* saved position, whatever its title.
    let placed = d.placed.borrow();
    assert!(placed.contains(&(11, r(0, 0, 900, 800), DisplayState::Normal)));
    assert!(placed.contains(&(12, r(1000, 0, 800, 700), DisplayState::Normal)));
    assert!(result.steps[0]
        .message
        .contains("3 tab(s) opened in 2 window(s)"));
    assert!(
        d.launched.borrow().is_empty(),
        "opened via tabs, not a bare launch"
    );
}

#[test]
fn running_browser_only_gets_missing_tabs_and_never_duplicates() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![open_window(1, "chrome", "Docs - Google Chrome")]),
        // Docs and Mail are already open.
        live_urls: Some(vec![
            "https://docs.rs".into(),
            "https://mail.google.com".into(),
        ]),
        browser_windows: RefCell::new(vec![open_window(2, "chrome", "Drive")]),
        ..Default::default()
    };
    let result = run(&d, &chrome_payload(), &fast());
    // Only the Drive tab was missing, so one new window with one tab.
    assert_eq!(
        *d.browser_opens.borrow(),
        vec![vec!["https://drive.google.com/drive/x".to_string()]]
    );
    assert!(
        result.steps[0]
            .message
            .contains("added 1 missing tab(s) in 1 new window(s)"),
        "{}",
        result.steps[0].message
    );
    // The new window is placed once, by the tab stage (not re-paired by the generic one).
    let placed = d.placed.borrow();
    assert_eq!(placed.iter().filter(|(h, ..)| *h == 2).count(), 1);
    // The pre-existing window was repositioned as usual.
    assert!(placed.iter().any(|(h, ..)| *h == 1));
    drop(placed);

    // A second run, now that everything is open, opens nothing.
    let d2 = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![
            open_window(1, "chrome", "Docs - Google Chrome"),
            open_window(2, "chrome", "Drive - Google Chrome"),
        ]),
        live_urls: Some(vec![
            "https://docs.rs".into(),
            "https://mail.google.com".into(),
            "https://drive.google.com/drive/x".into(),
        ]),
        ..Default::default()
    };
    run(&d2, &chrome_payload(), &fast());
    assert!(d2.browser_opens.borrow().is_empty());
}

#[test]
fn running_browser_without_the_extension_is_left_alone_and_says_why() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![open_window(1, "chrome", "Docs - Google Chrome")]),
        live_urls: None,
        ..Default::default()
    };
    let result = run(&d, &chrome_payload(), &fast());
    assert!(d.browser_opens.borrow().is_empty());
    assert!(result.steps[0].message.contains("extension isn't connected"));
    assert!(
        d.placed.borrow().iter().any(|(h, ..)| *h == 1),
        "still repositioned"
    );
}

#[test]
fn browser_window_that_never_appears_is_reported() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        // Nothing ever shows up.
        ..Default::default()
    };
    let result = run(&d, &chrome_payload(), &fast());
    let s = &result.steps[0];
    assert_eq!(s.status, RestoreStatus::Failed, "{s:#?}");
    assert!(s.message.contains("did not appear"));
}

#[test]
fn only_plain_web_addresses_are_opened_and_limits_apply() {
    let mut urls: Vec<String> = (0..70).map(|i| format!("https://a.com/{i}")).collect();
    urls.insert(0, "chrome://settings".into());
    urls.insert(1, "--remote-debugging-port=1".into());
    urls.insert(2, "https://a.com/0".into()); // duplicate of a later one
    let refs: Vec<&str> = urls.iter().map(String::as_str).collect();
    let p = SnapshotPayload {
        applications: vec![browser_app(vec![saved_tabs_window(
            "W - Google Chrome",
            &refs,
        )])],
        windows: vec![saved_window(
            "chrome",
            "W - Google Chrome",
            r(0, 0, 800, 600),
            "M1",
        )],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        browser_windows: RefCell::new(vec![open_window(1, "chrome", "x")]),
        ..Default::default()
    };
    let opts = RestoreOptions {
        max_tabs_per_window: 10,
        ..fast()
    };
    let result = run(&d, &p, &opts);
    let opened = d.browser_opens.borrow()[0].clone();
    assert_eq!(opened.len(), 10, "capped");
    assert!(
        opened.iter().all(|u| u.starts_with("https://")),
        "{opened:?}"
    );
    assert_eq!(
        opened.iter().filter(|u| *u == "https://a.com/0").count(),
        1,
        "deduplicated"
    );
    assert!(result.steps[0].message.contains("only the first 10"));
}

#[test]
fn dry_run_describes_tabs_without_opening_anything() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        ..Default::default()
    };
    let opts = RestoreOptions {
        dry_run: true,
        ..fast()
    };
    let result = run(&d, &chrome_payload(), &opts);
    assert!(result.steps[0].message.contains("3 tab(s) in 2 window(s)"));
    assert!(d.browser_opens.borrow().is_empty());
}

#[test]
fn browser_without_saved_tabs_behaves_like_any_other_app() {
    let mut a = app("chrome");
    a.adapter_id = None;
    let p = SnapshotPayload {
        applications: vec![a],
        windows: vec![saved_window("chrome", "x", r(0, 0, 500, 500), "M1")],
        ..Default::default()
    };
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        on_launch: vec![("chrome".into(), vec![open_window(1, "chrome", "x")])],
        ..Default::default()
    };
    let result = run(&d, &p, &fast());
    assert_eq!(result.overall, RestoreStatus::Success);
    assert_eq!(*d.launched.borrow(), vec!["chrome".to_string()]);
    assert!(d.browser_opens.borrow().is_empty());
}

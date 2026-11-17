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
    // The new window is placed by the tab stage only: at once, then once more
    // in the same spot (not re-paired by the generic step).
    let placed = d.placed.borrow();
    let new_window: Vec<_> = placed.iter().filter(|(h, ..)| *h == 2).collect();
    assert_eq!(new_window.len(), 2, "{placed:?}");
    assert_eq!(new_window[0], new_window[1]);
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

/// The "Chess" report: Chrome is already running with one window whose title
/// has changed since capture; the other saved window (YouTube, left screen) is
/// reopened for its missing tab. Each must end up on its own screen.
#[test]
fn reopened_window_keeps_its_slot_and_running_window_gets_the_other() {
    let mut payload = SnapshotPayload {
        applications: vec![browser_app(vec![
            saved_tabs_window("YouTube - Google Chrome", &["https://www.youtube.com/"]),
            saved_tabs_window(
                "Home - Chess.com - Google Chrome",
                &["https://www.chess.com/home"],
            ),
        ])],
        windows: vec![
            saved_window("chrome", "YouTube - Google Chrome", r(100, 100, 1200, 900), "LEFT"),
            saved_window(
                "chrome",
                "Home - Chess.com - Google Chrome",
                r(2700, 100, 900, 900),
                "RIGHT",
            ),
        ],
        ..Default::default()
    };
    for w in &mut payload.windows {
        w.display_state = DisplayState::Maximized;
    }
    let d = Fake {
        monitors: vec![
            monitor("LEFT", true, r(0, 0, 2560, 1440)),
            monitor("RIGHT", false, r(2560, 0, 1920, 1080)),
        ],
        // The open window's title no longer matches what was saved.
        open: RefCell::new(vec![open_window(1, "chrome", "Live game - Chess.com - Google Chrome")]),
        live_urls: Some(vec!["https://www.chess.com/game/live/1".into(), "https://www.chess.com/home".into()]),
        browser_windows: RefCell::new(vec![open_window(2, "chrome", "YouTube - Google Chrome")]),
        ..Default::default()
    };
    let result = run(&d, &payload, &fast());
    let placed = d.placed.borrow();
    // The reopened window is placed at once and once more shortly after,
    // always in its own slot; the running one gets the other.
    assert_eq!(
        *placed,
        vec![
            (2, r(100, 100, 1200, 900), DisplayState::Maximized),
            (1, r(2700, 100, 900, 900), DisplayState::Maximized),
            (2, r(100, 100, 1200, 900), DisplayState::Maximized),
        ],
        "{result:#?}"
    );
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert!(
        !result.steps[0].message.contains("only opened"),
        "{}",
        result.steps[0].message
    );
}

/// Chrome is open with an unrelated window, and both saved windows have to be
/// added. Every saved position goes to a new window, yet the report must still
/// say Chrome was already running, not that it was started.
#[test]
fn a_running_browser_that_got_all_its_windows_added_is_not_called_started() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![open_window(1, "chrome", "Spider - Google Chrome")]),
        live_urls: Some(vec!["https://example.com/spider".into()]),
        browser_windows: RefCell::new(vec![
            open_window(2, "chrome", "Drive"),
            open_window(3, "chrome", "Docs"),
        ]),
        ..Default::default()
    };
    let result = run(&d, &chrome_payload(), &fast());
    let message = &result.steps[0].message;
    assert!(
        message.starts_with("already running; added 3 missing tab(s) in 2 new window(s)"),
        "{message}"
    );
    assert!(d.launched.borrow().is_empty(), "Chrome was not started");
}

/// A Chrome window with two pinned tabs (mail, chat) and an ordinary one.
fn pinned_payload() -> SnapshotPayload {
    let mut window = saved_tabs_window(
        "Mail - Google Chrome",
        &[
            "https://mail.example.com/",
            "https://chat.example.com/",
            "https://docs.example.com/",
        ],
    );
    window.tabs[0].pinned = true;
    window.tabs[1].pinned = true;
    SnapshotPayload {
        applications: vec![browser_app(vec![window])],
        windows: vec![saved_window(
            "chrome",
            "Mail - Google Chrome",
            r(0, 0, 900, 800),
            "M1",
        )],
        ..Default::default()
    }
}

#[test]
fn pinned_tabs_are_asked_for_before_their_window_opens() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        browser_windows: RefCell::new(vec![open_window(11, "chrome", "x")]),
        ..Default::default()
    };
    let result = run(&d, &pinned_payload(), &fast());
    assert_eq!(result.overall, RestoreStatus::Success, "{result:#?}");
    assert_eq!(
        *d.pin_requests.borrow(),
        vec![(
            "chrome".to_string(),
            vec![
                "https://mail.example.com/".to_string(),
                "https://chat.example.com/".to_string()
            ]
        )]
    );
    // Asked first, so the request is waiting when the extension connects.
    let events = d.events.borrow();
    let pin = events.iter().position(|e| e == "pin:2").unwrap();
    let open = events.iter().position(|e| e == "open:3").unwrap();
    assert!(pin < open, "{events:?}");
}

#[test]
fn only_pinned_tabs_that_are_being_reopened_are_asked_for() {
    // Mail is already open: it is not reopened, so it is not asked for (the
    // user's own tab is never pinned by ContextFlow).
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        open: RefCell::new(vec![open_window(1, "chrome", "Mail - Google Chrome")]),
        live_urls: Some(vec!["https://mail.example.com".into()]),
        browser_windows: RefCell::new(vec![open_window(12, "chrome", "y")]),
        ..Default::default()
    };
    run(&d, &pinned_payload(), &fast());
    assert_eq!(
        *d.pin_requests.borrow(),
        vec![(
            "chrome".to_string(),
            vec!["https://chat.example.com/".to_string()]
        )]
    );
}

#[test]
fn a_window_without_pinned_tabs_asks_for_nothing() {
    let d = Fake {
        monitors: vec![monitor("M1", true, r(0, 0, 1920, 1080))],
        browser_windows: RefCell::new(vec![
            open_window(11, "chrome", "x"),
            open_window(12, "chrome", "y"),
        ]),
        ..Default::default()
    };
    run(&d, &chrome_payload(), &fast());
    assert!(d.pin_requests.borrow().is_empty());
}

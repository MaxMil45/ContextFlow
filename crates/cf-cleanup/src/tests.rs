use std::cell::RefCell;

use super::*;

fn proc(pid: u32, name: &str, ram_mb: u64) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent_pid: 1,
        name: name.into(),
        path: Some(format!(r"C:\Program Files\App\{name}")),
        session_id: Some(1),
        ram_bytes: ram_mb * 1024 * 1024,
        cpu_time_100ns: 0,
    }
}

fn target(process: &str, mode: CloseMode) -> CleanupTarget {
    CleanupTarget {
        process: process.into(),
        mode,
    }
}

/// A scripted system. `closes_on_request` programs exit when asked; others
/// ignore the request (they hide to the tray). Time passes only by sleeping.
#[derive(Default)]
struct Fake {
    procs: RefCell<Vec<ProcessInfo>>,
    closes_on_request: HashSet<String>,
    /// Programs with no window to ask.
    windowless: HashSet<String>,
    unkillable: HashSet<u32>,
    protected: HashSet<u32>,
    asked: RefCell<Vec<u32>>,
    terminated: RefCell<Vec<u32>>,
    /// Extra CPU time each process gains per sleep, in 100 ns ticks.
    cpu_per_sleep: HashMap<u32, u64>,
}

impl Fake {
    fn with(procs: Vec<ProcessInfo>) -> Self {
        Self {
            procs: RefCell::new(procs),
            ..Default::default()
        }
    }
}

impl Processes for Fake {
    fn list(&self) -> Vec<ProcessInfo> {
        self.procs.borrow().clone()
    }
    fn request_close(&self, pid: u32) -> usize {
        self.asked.borrow_mut().push(pid);
        let name = stem(
            &self
                .procs
                .borrow()
                .iter()
                .find(|p| p.pid == pid)
                .map(|p| p.name.clone())
                .unwrap_or_default(),
        );
        if self.windowless.contains(&name) {
            return 0;
        }
        if self.closes_on_request.contains(&name) {
            self.procs.borrow_mut().retain(|p| p.pid != pid);
        }
        1
    }
    fn terminate(&self, pid: u32) -> Result<(), String> {
        if self.unkillable.contains(&pid) {
            return Err("access is denied".into());
        }
        self.terminated.borrow_mut().push(pid);
        self.procs.borrow_mut().retain(|p| p.pid != pid);
        Ok(())
    }
    fn own_session(&self) -> Option<u32> {
        Some(1)
    }
    fn protected_pids(&self) -> HashSet<u32> {
        self.protected.clone()
    }
    fn logical_cpus(&self) -> usize {
        4
    }
    fn sleep(&self, _d: Duration) {
        for p in self.procs.borrow_mut().iter_mut() {
            p.cpu_time_100ns += self.cpu_per_sleep.get(&p.pid).copied().unwrap_or(0);
        }
    }
}

fn fast() -> RunOptions {
    RunOptions {
        close_wait: Duration::from_millis(20),
        poll: Duration::from_millis(1),
    }
}

// ----------------------------------------------------------------- protection

#[test]
fn system_programs_are_protected_by_name_and_by_folder() {
    for name in [
        "csrss.exe",
        "SVCHOST.EXE",
        "explorer.exe",
        "lsass",
        "MsMpEng.exe",
    ] {
        assert!(protected_reason(name, None).is_some(), "{name}");
    }
    assert!(protected_reason("anything.exe", Some(r"C:\Windows\System32\anything.exe")).is_some());
    assert!(protected_reason("tool.exe", Some(r"C:\Windows\Tools\tool.exe")).is_some());
    assert!(protected_reason("contextflow-app.exe", None).is_some());
    // Ordinary apps, including ones with "windows" in a longer name or folder.
    for (name, path) in [
        (
            "discord.exe",
            r"C:\Users\me\AppData\Local\Discord\Discord.exe",
        ),
        ("steam.exe", r"C:\Program Files (x86)\Steam\steam.exe"),
        ("x.exe", r"C:\Program Files\WindowsApps\Foo\x.exe"),
        ("y.exe", r"C:\WindowsTools\y.exe"),
    ] {
        assert_eq!(protected_reason(name, Some(path)), None, "{name}");
    }
}

// -------------------------------------------------------------------- listing

#[test]
fn running_apps_group_by_program_and_add_up_memory() {
    let d = Fake::with(vec![
        proc(10, "Discord.exe", 300),
        proc(11, "Discord.exe", 100),
        proc(12, "steam.exe", 150),
        proc(13, "csrss.exe", 5),
    ]);
    let apps = running_apps(&d, Duration::from_millis(1));
    let discord = apps.iter().find(|a| a.process == "discord").unwrap();
    assert_eq!(discord.process_count, 2);
    assert_eq!(discord.ram_bytes, 400 * 1024 * 1024);
    assert_eq!(discord.protected, None);
    // Biggest first; system programs are listed but marked.
    assert_eq!(apps[0].process, "discord");
    assert!(apps
        .iter()
        .find(|a| a.process == "csrss")
        .unwrap()
        .protected
        .is_some());
}

#[test]
fn programs_cleanup_must_not_touch_are_left_out_of_the_list() {
    let mut other_session = proc(20, "other.exe", 10);
    other_session.session_id = Some(0);
    let mut hidden = proc(21, "hidden.exe", 10);
    hidden.path = None; // can't be inspected: elevated or protected
    let mut d = Fake::with(vec![
        proc(1, "self.exe", 10),
        proc(22, "parent.exe", 10),
        proc(23, "ok.exe", 10),
        other_session,
        hidden,
    ]);
    d.protected = HashSet::from([22]);
    let names: Vec<_> = running_apps(&d, Duration::from_millis(1))
        .into_iter()
        .map(|a| a.process)
        .collect();
    assert!(names.contains(&"ok".to_string()));
    for gone in ["parent", "other", "hidden"] {
        assert!(
            !names.contains(&gone.to_string()),
            "{gone} must not be listed"
        );
    }
}

#[test]
fn cpu_is_measured_over_the_sampling_window() {
    let mut d = Fake::with(vec![proc(10, "busy.exe", 10), proc(11, "idle1.exe", 10)]);
    // 4 CPUs: 1 core fully busy for the window is 25% of the machine. The fake
    // adds ticks per sleep and the real clock barely moves, so use a window
    // long enough to measure.
    d.cpu_per_sleep.insert(10, 10_000_000); // 1 s of CPU time...
    let apps = running_apps(&d, Duration::from_millis(50)); // ...in ~50 ms of wall time
    let busy = apps.iter().find(|a| a.process == "busy").unwrap();
    let idle = apps.iter().find(|a| a.process == "idle1").unwrap();
    assert!(busy.cpu_percent > 100.0, "{}", busy.cpu_percent); // far above idle
    assert_eq!(idle.cpu_percent, 0.0);
}

// ----------------------------------------------------------------------- plan

#[test]
fn the_plan_adds_up_what_would_be_given_back() {
    let d = Fake::with(vec![
        proc(10, "Discord.exe", 400),
        proc(12, "steam.exe", 150),
        proc(30, "docker.exe", 900),
    ]);
    let apps = running_apps(&d, Duration::from_millis(1));
    let targets = [
        target("discord", CloseMode::Graceful),
        target("steam", CloseMode::Force),
        target("epicgameslauncher", CloseMode::Graceful), // not running
        target("explorer", CloseMode::Force),             // protected
        target("docker", CloseMode::Graceful),            // the workspace uses it
    ];
    let keep = HashSet::from(["docker".to_string()]);
    let plan = plan(&targets, &keep, &apps);

    assert_eq!(plan.closing(), 2);
    assert_eq!(plan.ram_bytes, 550 * 1024 * 1024);
    let action = |p: &str| {
        plan.items
            .iter()
            .find(|i| i.process == p)
            .unwrap()
            .action
            .clone()
    };
    assert!(matches!(
        action("discord"),
        PlannedAction::Close {
            process_count: 1,
            ..
        }
    ));
    assert_eq!(action("epicgameslauncher"), PlannedAction::NotRunning);
    assert!(matches!(
        action("explorer"),
        PlannedAction::Protected { .. }
    ));
    assert_eq!(action("docker"), PlannedAction::KeptByWorkspace);
}

// ------------------------------------------------------------------------ run

#[test]
fn a_program_that_closes_when_asked_is_not_ended() {
    let mut d = Fake::with(vec![proc(10, "Discord.exe", 100)]);
    d.closes_on_request.insert("discord".into());
    let r = run(
        &d,
        &[target("discord", CloseMode::Force)],
        &HashSet::new(),
        &fast(),
    );
    assert_eq!(r[0].outcome, Outcome::Closed);
    assert!(d.terminated.borrow().is_empty(), "force mode asks first");
}

#[test]
fn graceful_mode_never_ends_a_program_that_stays_open() {
    let d = Fake::with(vec![proc(10, "Steam.exe", 100)]); // hides to the tray
    let r = run(
        &d,
        &[target("steam", CloseMode::Graceful)],
        &HashSet::new(),
        &fast(),
    );
    assert_eq!(r[0].outcome, Outcome::StillRunning);
    assert!(r[0].message.contains("force"), "{}", r[0].message);
    assert!(d.terminated.borrow().is_empty());
    assert_eq!(*d.asked.borrow(), vec![10]);
}

#[test]
fn force_mode_ends_a_program_that_ignores_the_request() {
    let d = Fake::with(vec![proc(10, "Steam.exe", 100), proc(11, "steam.exe", 50)]);
    let r = run(
        &d,
        &[target("steam", CloseMode::Force)],
        &HashSet::new(),
        &fast(),
    );
    assert_eq!(r[0].outcome, Outcome::Terminated);
    assert_eq!(*d.terminated.borrow(), vec![10, 11]);
}

#[test]
fn force_mode_does_not_wait_for_a_program_with_no_window() {
    let mut d = Fake::with(vec![proc(10, "daemon.exe", 100)]);
    d.windowless.insert("daemon".into());
    let slow = RunOptions {
        close_wait: Duration::from_secs(30),
        poll: Duration::from_millis(1),
    };
    let t = Instant::now();
    let r = run(
        &d,
        &[target("daemon", CloseMode::Force)],
        &HashSet::new(),
        &slow,
    );
    assert_eq!(r[0].outcome, Outcome::Terminated);
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "must not sit out the 30 s wait"
    );
}

#[test]
fn graceful_mode_explains_a_program_with_no_window() {
    let mut d = Fake::with(vec![proc(10, "daemon.exe", 100)]);
    d.windowless.insert("daemon".into());
    let r = run(
        &d,
        &[target("daemon", CloseMode::Graceful)],
        &HashSet::new(),
        &fast(),
    );
    assert_eq!(r[0].outcome, Outcome::StillRunning);
    assert!(r[0].message.contains("no window"), "{}", r[0].message);
}

#[test]
fn an_unkillable_program_is_reported_and_the_rest_still_run() {
    let mut d = Fake::with(vec![proc(10, "stuck.exe", 100), proc(11, "other.exe", 100)]);
    d.unkillable.insert(10);
    d.closes_on_request.insert("other".into());
    let r = run(
        &d,
        &[
            target("stuck", CloseMode::Force),
            target("other", CloseMode::Graceful),
        ],
        &HashSet::new(),
        &fast(),
    );
    assert_eq!(r[0].outcome, Outcome::Failed);
    assert!(
        r[0].message.contains("access is denied"),
        "{}",
        r[0].message
    );
    assert_eq!(r[1].outcome, Outcome::Closed);
}

#[test]
fn protected_kept_and_missing_programs_are_never_touched() {
    let d = Fake::with(vec![
        proc(1, "explorer.exe", 100),
        proc(2, "docker.exe", 100),
        proc(3, "mine.exe", 100),
    ]);
    let keep = HashSet::from(["docker".to_string()]);
    let r = run(
        &d,
        &[
            target("explorer", CloseMode::Force),
            target("docker", CloseMode::Force),
            target("nothere", CloseMode::Force),
        ],
        &keep,
        &fast(),
    );
    let outcomes: Vec<_> = r.iter().map(|x| x.outcome).collect();
    assert_eq!(
        outcomes,
        vec![
            Outcome::Protected,
            Outcome::KeptByWorkspace,
            Outcome::NotRunning
        ]
    );
    assert!(d.asked.borrow().is_empty() && d.terminated.borrow().is_empty());
}

#[test]
fn a_program_that_is_running_contextflow_is_protected_as_a_whole() {
    // Pid 7 started ContextFlow; its sibling process 8 has the same name.
    let mut d = Fake::with(vec![proc(7, "host.exe", 100), proc(8, "host.exe", 100)]);
    d.protected.insert(7);

    // Not offered as closable...
    let apps = running_apps(&d, Duration::from_millis(1));
    let host = apps.iter().find(|a| a.process == "host").unwrap();
    assert_eq!(host.protected.as_deref(), Some("it is running ContextFlow"));
    assert_eq!(host.process_count, 1, "the parent itself isn't counted");

    // ...and refused when listed anyway, without asking or ending anything.
    let r = run(
        &d,
        &[target("host", CloseMode::Force)],
        &HashSet::new(),
        &fast(),
    );
    assert_eq!(r[0].outcome, Outcome::Protected);
    assert!(d.asked.borrow().is_empty() && d.terminated.borrow().is_empty());
}

// ------------------------------------------------------ closing all at once

/// Several programs hide to the tray instead of exiting. Each used to get its
/// own full wait, one after another; now they are asked together and waited
/// for together.
#[test]
fn programs_that_ignore_the_request_share_one_wait() {
    let d = Fake::with(vec![
        proc(10, "Discord.exe", 100),
        proc(11, "Steam.exe", 100),
        proc(12, "Epic.exe", 100),
    ]); // none of them closes when asked
    let opts = RunOptions {
        close_wait: Duration::from_millis(300),
        poll: Duration::from_millis(1),
    };
    let targets = [
        target("discord", CloseMode::Graceful),
        target("steam", CloseMode::Graceful),
        target("epic", CloseMode::Graceful),
    ];
    let began = Instant::now();
    let r = run(&d, &targets, &HashSet::new(), &opts);
    let took = began.elapsed();
    assert!(
        r.iter().all(|x| x.outcome == Outcome::StillRunning),
        "{r:?}"
    );
    // One wait, not three (which would take 900 ms).
    assert!(took < Duration::from_millis(600), "took {took:?}");
}

#[test]
fn every_program_is_asked_before_any_is_ended_and_results_keep_their_order() {
    let mut d = Fake::with(vec![
        proc(10, "stubborn.exe", 100),
        proc(11, "polite.exe", 100),
        proc(12, "tray.exe", 100),
    ]);
    d.closes_on_request.insert("polite".into());
    let r = run(
        &d,
        &[
            target("stubborn", CloseMode::Force),
            target("polite", CloseMode::Graceful),
            target("missing", CloseMode::Graceful),
            target("tray", CloseMode::Graceful),
        ],
        &HashSet::new(),
        &fast(),
    );
    let outcomes: Vec<Outcome> = r.iter().map(|x| x.outcome).collect();
    assert_eq!(
        outcomes,
        [
            Outcome::Terminated,
            Outcome::Closed,
            Outcome::NotRunning,
            Outcome::StillRunning
        ],
        "{r:?}"
    );
    let names: Vec<&str> = r.iter().map(|x| x.process.as_str()).collect();
    assert_eq!(names, ["stubborn", "polite", "missing", "tray"]);
    // All three that were running were asked, and only the force-mode one ended.
    assert_eq!(*d.asked.borrow(), vec![10, 11, 12]);
    assert_eq!(*d.terminated.borrow(), vec![10]);
}

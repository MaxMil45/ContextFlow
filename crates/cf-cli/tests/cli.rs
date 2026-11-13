//! Integration tests for the `contextflow` binary.
//!
//! Safety rules for every test here:
//! * each test gets its own temporary database and config (never the real ones);
//! * `capture` always passes `--no-windows`, so nothing reads the real desktop
//!   and results are deterministic;
//! * `restore` is only ever run with `--dry-run`;
//! * `shortcut` and `browser install` are never run.

use std::path::Path;
use std::process::{Command, Output};

/// Command for the binary, pointed at databases and config inside `dir`.
fn cmd(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_contextflow"));
    c.env("CONTEXTFLOW_DB", dir.join("cf.db"))
        .env("CONTEXTFLOW_CONFIG", dir.join("none.toml"));
    c
}

fn cf(dir: &Path, args: &[&str]) -> Output {
    cmd(dir).args(args).output().expect("run contextflow")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// Run and require success, returning stdout.
fn ok(dir: &Path, args: &[&str]) -> String {
    let o = cf(dir, args);
    assert!(
        o.status.success(),
        "`contextflow {}` failed\nstdout: {}\nstderr: {}",
        args.join(" "),
        stdout(&o),
        stderr(&o)
    );
    stdout(&o)
}

/// Run and require failure, returning stderr.
fn fails(dir: &Path, args: &[&str]) -> String {
    let o = cf(dir, args);
    assert!(
        !o.status.success(),
        "`contextflow {}` should have failed\nstdout: {}",
        args.join(" "),
        stdout(&o)
    );
    stderr(&o)
}

fn capture(dir: &Path, workspace: &str, note: &str) -> String {
    ok(dir, &["capture", workspace, "--no-windows", "--note", note])
}

fn snapshot_lines(dir: &Path, workspace: &str) -> Vec<String> {
    ok(dir, &["snapshots", workspace])
        .lines()
        .filter(|l| l.starts_with('v'))
        .map(String::from)
        .collect()
}

#[test]
fn workspace_create_list_and_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    assert!(ok(d, &["workspace", "create", "A"]).contains("Created workspace 'A'"));

    let err = fails(d, &["workspace", "create", "a"]);
    assert!(err.contains("already exists"), "{err}");

    let list = ok(d, &["workspace", "list"]);
    assert!(list.lines().any(|l| l.starts_with('A')), "{list}");
    assert_eq!(list.lines().count(), 1, "only one workspace: {list}");
}

#[test]
fn capture_versions_and_identical_note() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    ok(d, &["workspace", "create", "A"]);

    let first = capture(d, "A", "x");
    assert!(first.contains("Saved 'A' v1"), "{first}");
    let second = capture(d, "A", "x");
    assert!(second.contains("Saved 'A' v2"), "{second}");
    assert!(
        second.contains("identical to the previous version"),
        "{second}"
    );
}

#[test]
fn delete_snapshot_requires_yes_and_never_reuses_versions() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    ok(d, &["workspace", "create", "A"]);
    capture(d, "A", "one");
    capture(d, "A", "two");

    let err = fails(d, &["delete-snapshot", "A", "--version", "2"]);
    assert!(err.contains("--yes"), "{err}");
    assert_eq!(
        snapshot_lines(d, "A").len(),
        2,
        "nothing deleted without --yes"
    );

    ok(d, &["delete-snapshot", "A", "--version", "2", "--yes"]);
    let next = capture(d, "A", "three");
    assert!(
        next.contains("Saved 'A' v3"),
        "version 2 must not be reused: {next}"
    );
}

#[test]
fn prune_is_dry_run_without_yes() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    ok(d, &["workspace", "create", "A"]);
    for note in ["a", "b", "c", "d"] {
        capture(d, "A", note);
    }
    assert_eq!(snapshot_lines(d, "A").len(), 4);

    let preview = ok(d, &["prune", "A", "--keep-last", "2"]);
    assert!(preview.contains("would delete"), "{preview}");
    assert_eq!(snapshot_lines(d, "A").len(), 4, "dry run deletes nothing");

    ok(d, &["prune", "A", "--keep-last", "2", "--yes"]);
    let left = snapshot_lines(d, "A");
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(
        left[0].starts_with("v4") && left[1].starts_with("v3"),
        "{left:?}"
    );
}

#[test]
fn export_import_round_trip_and_tamper() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    ok(d, &["workspace", "create", "A"]);
    capture(d, "A", "first-note");
    capture(d, "A", "tamper-me");

    let file = d.join("a.json");
    let file_arg = file.to_str().unwrap();
    ok(d, &["export", "A", "--out", file_arg]);

    let err = fails(d, &["import", file_arg]);
    assert!(err.contains("already exists"), "{err}");

    ok(d, &["import", file_arg, "--as", "B"]);
    assert_eq!(snapshot_lines(d, "B").len(), 2, "history came across");

    // Edit a note inside the export, as an attacker or a bad disk might.
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("tamper-me"));
    let bad = d.join("bad.json");
    std::fs::write(&bad, text.replace("tamper-me", "tamper-xx")).unwrap();

    let err = fails(d, &["import", bad.to_str().unwrap(), "--as", "C"]);
    assert!(err.contains("hash check"), "{err}");
    let list = ok(d, &["workspace", "list"]);
    assert!(
        !list.lines().any(|l| l.starts_with('C')),
        "tampered import must not create a workspace: {list}"
    );
}

#[test]
fn verify_reports_ok() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    ok(d, &["workspace", "create", "A"]);
    capture(d, "A", "one");
    capture(d, "A", "two");
    let out = ok(d, &["verify"]);
    assert!(out.contains("database: ok"), "{out}");
}

#[test]
fn bad_config_reports_line() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let config = d.join("bad.toml");
    std::fs::write(&config, "[capture]\ninclude_windowz = true\n").unwrap();

    let out = cmd(d)
        .env("CONTEXTFLOW_CONFIG", &config)
        .arg("info")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("line 2"), "{err}");
    assert!(err.contains("include_windowz"), "{err}");
}

#[test]
fn restore_dry_run_on_empty_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    ok(d, &["workspace", "create", "A"]);
    capture(d, "A", "nothing open");
    let out = ok(d, &["restore", "A", "--dry-run"]);
    assert!(out.contains("nothing to restore"), "{out}");
}

#[test]
fn imported_workspace_needs_approval() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    ok(d, &["workspace", "create", "A"]);
    capture(d, "A", "one");
    let file = d.join("a.json");
    let file_arg = file.to_str().unwrap();
    ok(d, &["export", "A", "--out", file_arg]);
    ok(d, &["import", file_arg, "--as", "B"]);

    let list = ok(d, &["workspace", "list"]);
    assert!(list.contains("[needs approval]"), "{list}");

    // A real restore is refused; a dry run only reads, so it is allowed.
    let err = fails(d, &["restore", "B"]);
    assert!(err.contains("has not been approved"), "{err}");
    assert!(ok(d, &["restore", "B", "--dry-run"]).contains("nothing to restore"));

    // Approval needs an explicit --yes.
    let err = fails(d, &["workspace", "trust", "B"]);
    assert!(err.contains("nothing approved"), "{err}");
    fails(d, &["restore", "B"]);

    ok(d, &["workspace", "trust", "B", "--yes"]);
    assert!(ok(d, &["restore", "B"]).contains("nothing to restore"));
    assert!(ok(d, &["workspace", "trust", "B"]).contains("already approved"));
    assert!(!ok(d, &["workspace", "list"]).contains("[needs approval]"));
}

#[test]
fn pin_controls_which_version_restore_uses() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    ok(d, &["workspace", "create", "A"]);
    capture(d, "A", "one");
    capture(d, "A", "two");

    // Unpinned, restore uses the latest.
    let latest = ok(d, &["restore", "A", "--dry-run"]);
    assert!(latest.contains("v2") && latest.contains("two"), "{latest}");

    assert!(ok(d, &["pin", "A", "--version", "1"]).contains("pinned to v1"));
    let pinned = ok(d, &["restore", "A", "--dry-run"]);
    assert!(pinned.contains("v1") && pinned.contains("one"), "{pinned}");
    assert!(ok(d, &["snapshots", "A"]).contains("(pinned)"));

    // A later capture does not change what restore uses, and says so.
    let third = capture(d, "A", "three");
    assert!(third.contains("still use v1"), "{third}");
    let still = ok(d, &["restore", "A", "--dry-run"]);
    assert!(still.contains("v1") && still.contains("one"), "{still}");

    // A pinned snapshot cannot be deleted until it is unpinned.
    let err = fails(d, &["delete-snapshot", "A", "--version", "1", "--yes"]);
    assert!(err.contains("pinned"), "{err}");

    assert!(ok(d, &["unpin", "A"]).contains("latest snapshot again"));
    let unpinned = ok(d, &["restore", "A", "--dry-run"]);
    assert!(unpinned.contains("v3"), "{unpinned}");
}

//! Crash-safety test (PLAN WP 1.5).
//!
//! The parent test repeatedly spawns this same test binary as a child that
//! writes snapshots in a tight loop, kills it at varying moments, and then
//! checks the database: no partial snapshots, no corruption, no gaps.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use cf_core::{CaptureStatus, DisplayState, GitState, Rect, SnapshotPayload, WindowState};
use cf_storage::Database;

const CHILD_ENV: &str = "CF_CRASH_CHILD_DB";
const WINDOWS_PER_SNAPSHOT: usize = 25;

fn heavy_payload(i: u64) -> SnapshotPayload {
    let mut p = SnapshotPayload::default();
    for w in 0..WINDOWS_PER_SNAPSHOT {
        p.windows.push(WindowState {
            application_id: format!("app{}", w % 5),
            title: format!("window {w} of snapshot {i}"),
            window_class: Some("Class".into()),
            monitor_id: Some("M1".into()),
            rect: Rect {
                x: w as i32,
                y: 0,
                width: 640,
                height: 480,
            },
            display_state: DisplayState::Normal,
        });
    }
    p.git.push(GitState {
        repository_path: format!(r"C:\src\repo{i}"),
        ..Default::default()
    });
    p.notes.push(format!("iteration {i}"));
    p
}

/// Child mode: write snapshots until killed. A no-op in a normal test run.
#[test]
fn crash_child() {
    let Ok(path) = std::env::var(CHILD_ENV) else {
        return;
    };
    let mut db = Database::open(Path::new(&path)).unwrap();
    let ctx = match db.find_context("crash") {
        Ok(c) => c,
        Err(_) => db.create_context("crash", None).unwrap(),
    };
    let mut i = 0;
    loop {
        db.create_snapshot(ctx.id, heavy_payload(i), CaptureStatus::Complete)
            .unwrap();
        i += 1;
    }
}

#[test]
fn killing_a_writer_never_leaves_partial_snapshots() {
    let iterations: u64 = std::env::var("CF_CRASH_ITERATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(25);
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("crash.db");
    let exe = std::env::current_exe().unwrap();

    for i in 0..iterations {
        let mut child = Command::new(&exe)
            .args(["crash_child", "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, &db_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // Vary the kill point so it lands in different phases of a write.
        std::thread::sleep(Duration::from_millis(40 + (i * 37) % 160));
        child.kill().ok();
        child.wait().unwrap();

        let db = Database::open(&db_path).unwrap();
        assert!(
            db.integrity_check().unwrap().is_empty(),
            "iteration {i}: sqlite integrity check failed"
        );
        let Ok(ctx) = db.find_context("crash") else {
            continue; // killed before the workspace was created
        };
        let snaps = db.list_snapshots(ctx.id).unwrap();
        let n = snaps.len() as u32;
        // Versions are contiguous 1..=n: no gaps, no half-written versions.
        let mut vs: Vec<u32> = snaps.iter().map(|s| s.version).collect();
        vs.sort_unstable();
        assert_eq!(vs, (1..=n).collect::<Vec<_>>(), "iteration {i}");

        for s in &snaps {
            assert!(
                db.verify_snapshot(ctx.id, Some(s.version)).unwrap().ok(),
                "iteration {i}: v{} corrupt",
                s.version
            );
        }
        // Every snapshot has all of its projection rows (nothing half-inserted).
        let partial: i64 = db
            .raw_connection()
            .query_row(
                "SELECT COUNT(*) FROM context_snapshots s
                 WHERE (SELECT COUNT(*) FROM snapshot_windows w WHERE w.snapshot_id = s.id) != ?1
                    OR (SELECT COUNT(*) FROM snapshot_notes n WHERE n.snapshot_id = s.id) != 1",
                [WINDOWS_PER_SNAPSHOT as i64],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(partial, 0, "iteration {i}: partial snapshot found");
    }

    // Guard against a vacuous pass (e.g. the child never managed to write).
    let db = Database::open(&db_path).unwrap();
    let ctx = db
        .find_context("crash")
        .expect("child never created the workspace");
    let written = db.list_snapshots(ctx.id).unwrap().len();
    assert!(written > 0, "child never wrote a snapshot");
    eprintln!("crash test: {iterations} kills, {written} snapshots survived intact");
}

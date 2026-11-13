//! Retention, deletion, migrations, export/import and backup.

use chrono::{TimeDelta, Utc};
use rusqlite::Connection;

use cf_core::{hash, CaptureStatus, DisplayState, Rect, SnapshotPayload, WindowState};
use cf_storage::{Database, ExportFile, RetentionPolicy, StorageError};

fn payload(n: u32) -> SnapshotPayload {
    let mut p = SnapshotPayload::default();
    p.notes.push(format!("note {n}"));
    p.windows.push(WindowState {
        application_id: "code".into(),
        title: format!("file{n}.rs"),
        window_class: None,
        monitor_id: None,
        rect: Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        },
        display_state: DisplayState::Normal,
    });
    p
}

fn db_with_versions(n: u32) -> (Database, cf_core::ContextId) {
    let mut db = Database::open_in_memory().unwrap();
    let ctx = db.create_context("W", None).unwrap();
    for i in 1..=n {
        db.create_snapshot(ctx.id, payload(i), CaptureStatus::Complete)
            .unwrap();
    }
    (db, ctx.id)
}

fn versions(db: &Database, id: cf_core::ContextId) -> Vec<u32> {
    db.list_snapshots(id)
        .unwrap()
        .into_iter()
        .map(|s| s.version)
        .collect()
}

#[test]
fn deleting_the_latest_never_reuses_its_version() {
    let (mut db, id) = db_with_versions(3);
    db.delete_snapshot(id, 3).unwrap();
    let s = db
        .create_snapshot(id, payload(99), CaptureStatus::Complete)
        .unwrap();
    assert_eq!(s.version, 4);
    assert_eq!(versions(&db, id), vec![4, 2, 1]);
    assert!(matches!(
        db.delete_snapshot(id, 3),
        Err(StorageError::NotFound(_))
    ));
}

#[test]
fn deleting_cascades_to_projections() {
    let (db, id) = db_with_versions(2);
    db.delete_snapshot(id, 1).unwrap();
    let count = |table: &str| -> i64 {
        db.raw_connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count("snapshot_windows"), 1);
    assert_eq!(count("snapshot_notes"), 1);

    db.delete_context(id).unwrap();
    assert_eq!(count("context_snapshots"), 0);
    assert_eq!(count("snapshot_windows"), 0);
    assert!(db.find_context("W").is_err());
}

#[test]
fn keep_last_retention() {
    let (mut db, id) = db_with_versions(5);
    let policy = RetentionPolicy {
        keep_last: Some(2),
        max_age: None,
    };
    let pruned = db.prune(id, &policy, Utc::now()).unwrap();
    assert_eq!(pruned, vec![3, 2, 1]);
    assert_eq!(versions(&db, id), vec![5, 4]);
    // Idempotent.
    assert!(db.prune(id, &policy, Utc::now()).unwrap().is_empty());
}

#[test]
fn latest_is_never_pruned_even_when_everything_is_old() {
    let (mut db, id) = db_with_versions(3);
    let policy = RetentionPolicy {
        keep_last: Some(1),
        max_age: Some(TimeDelta::days(1)),
    };
    let far_future = Utc::now() + TimeDelta::days(365);
    db.prune(id, &policy, far_future).unwrap();
    assert_eq!(versions(&db, id), vec![3]);
}

#[test]
fn both_rules_must_be_broken_to_prune() {
    let (db, id) = db_with_versions(4);
    let policy = RetentionPolicy {
        keep_last: Some(1),
        max_age: Some(TimeDelta::days(30)),
    };
    // Everything is outside keep_last but nothing is older than 30 days.
    assert!(db
        .retention_candidates(id, &policy, Utc::now())
        .unwrap()
        .is_empty());
    // Thirty-one days later, the old ones qualify.
    let later = Utc::now() + TimeDelta::days(31);
    let c: Vec<u32> = db
        .retention_candidates(id, &policy, later)
        .unwrap()
        .into_iter()
        .map(|s| s.version)
        .collect();
    assert_eq!(c, vec![3, 2, 1]);
    // No rules -> nothing pruned.
    assert!(db
        .retention_candidates(id, &RetentionPolicy::default(), later)
        .unwrap()
        .is_empty());
}

#[test]
fn upgrading_a_v1_database_backfills_version_counter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        let json = hash::canonical_json(&payload(1)).unwrap();
        let h = hash::sha256_hex(json.as_bytes());
        conn.execute(
            "INSERT INTO contexts VALUES ('00000000-0000-4000-8000-000000000001', 'Old', NULL,
             '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z', 0)",
            [],
        )
        .unwrap();
        for v in [1, 7] {
            conn.execute(
                "INSERT INTO context_snapshots VALUES (?1, '00000000-0000-4000-8000-000000000001',
                 ?2, '2026-01-01T00:00:00.000Z', ?3, 'complete', ?4)",
                rusqlite::params![
                    format!("00000000-0000-4000-8000-00000000010{v}"),
                    v,
                    h,
                    json
                ],
            )
            .unwrap();
        }
    }
    let mut db = Database::open(&path).unwrap();
    assert_eq!(
        db.schema_version().unwrap(),
        Database::latest_schema_version()
    );
    let ctx = db.find_context("old").unwrap();
    let next = db
        .create_snapshot(ctx.id, payload(2), CaptureStatus::Complete)
        .unwrap();
    assert_eq!(next.version, 8);
    assert!(db.verify_snapshot(ctx.id, Some(1)).unwrap().ok());
}

#[test]
fn export_import_round_trip_preserves_history_and_hashes() {
    let (src, id) = db_with_versions(3);
    src.delete_snapshot(id, 2).unwrap();
    let exported = src.export_context(id).unwrap();
    let file = ExportFile::new(vec![exported], "test");
    let json = serde_json::to_string_pretty(&file).unwrap();

    let parsed: ExportFile = serde_json::from_str(&json).unwrap();
    parsed.check_header().unwrap();
    let mut dst = Database::open_in_memory().unwrap();
    let ctx = dst.import_workspace(&parsed.workspaces[0], None).unwrap();

    assert_eq!(versions(&dst, ctx.id), vec![3, 1]);
    for v in [1, 3] {
        let a = src.get_snapshot(id, Some(v)).unwrap();
        let b = dst.get_snapshot(ctx.id, Some(v)).unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.payload, b.payload);
        assert_eq!(a.created_at, b.created_at);
        assert!(dst.verify_snapshot(ctx.id, Some(v)).unwrap().ok());
    }
    // The version counter came across too: the next capture is v4, not v2.
    let s = dst
        .create_snapshot(ctx.id, payload(4), CaptureStatus::Complete)
        .unwrap();
    assert_eq!(s.version, 4);

    // Importing again clashes on the name, unless renamed.
    assert!(matches!(
        dst.import_workspace(&parsed.workspaces[0], None),
        Err(StorageError::Conflict(_))
    ));
    let copy = dst
        .import_workspace(&parsed.workspaces[0], Some("W copy"))
        .unwrap();
    assert_eq!(versions(&dst, copy.id), vec![3, 1]);
}

#[test]
fn tampered_import_writes_nothing() {
    let (src, id) = db_with_versions(2);
    let mut ws = src.export_context(id).unwrap();
    ws.snapshots[1].payload_json = ws.snapshots[1].payload_json.replace("note 2", "note X");

    let mut dst = Database::open_in_memory().unwrap();
    let err = dst.import_workspace(&ws, None).unwrap_err();
    assert!(matches!(err, StorageError::Integrity(_)), "{err}");
    assert!(dst.list_contexts(true).unwrap().is_empty());

    let file = ExportFile {
        format: "something-else".into(),
        ..ExportFile::new(vec![], "test")
    };
    assert!(file.check_header().is_err());
}

#[test]
fn backup_is_a_complete_verified_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (db, id) = db_with_versions(3);
    let dest = dir.path().join("backups").join("cf-backup.db");
    db.backup_to(&dest).unwrap();
    assert!(matches!(
        db.backup_to(&dest),
        Err(StorageError::Conflict(_))
    ));

    let copy = Database::open(&dest).unwrap();
    assert!(copy.integrity_check().unwrap().is_empty());
    let ctx = copy.find_context("W").unwrap();
    assert_eq!(versions(&copy, ctx.id), vec![3, 2, 1]);
    assert_eq!(
        copy.get_snapshot(ctx.id, None).unwrap().hash,
        db.get_snapshot(id, None).unwrap().hash
    );
}

#[test]
fn restore_runs_are_recorded_and_deleted_with_their_snapshot() {
    use cf_storage::RestoreStepRecord;
    let (mut db, id) = db_with_versions(1);
    let snap = db.get_snapshot(id, None).unwrap();
    let step = |app: &str, status: &str| RestoreStepRecord {
        component_type: "application".into(),
        component_identifier: app.into(),
        status: status.into(),
        error_code: (status == "failed").then(|| "EXE_NOT_FOUND".to_string()),
        error_message: None,
        duration_ms: 12,
    };
    let now = Utc::now();
    db.record_restore_run(
        snap.id,
        now,
        now,
        "partial_success",
        &[step("code", "success"), step("chrome", "failed")],
    )
    .unwrap();
    let runs = db.restore_runs(snap.id).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].overall_status, "partial_success");

    db.delete_snapshot(id, 1).unwrap();
    let steps: i64 = db
        .raw_connection()
        .query_row("SELECT COUNT(*) FROM restore_steps", [], |r| r.get(0))
        .unwrap();
    assert_eq!(steps, 0, "history is removed with its snapshot");
}

#[test]
fn imported_workspaces_start_untrusted() {
    let (mut db, id) = db_with_versions(1);
    assert!(
        db.is_trusted(id).unwrap(),
        "workspaces made here are trusted"
    );

    let exported = db.export_context(id).unwrap();
    let copy = db.import_workspace(&exported, Some("copy")).unwrap();
    assert!(!db.is_trusted(copy.id).unwrap(), "imported ones are not");
    assert!(db.is_trusted(id).unwrap(), "the original is unaffected");

    db.set_trusted(copy.id, true).unwrap();
    assert!(db.is_trusted(copy.id).unwrap());

    let missing = cf_core::ContextId::new();
    assert!(matches!(
        db.is_trusted(missing),
        Err(StorageError::NotFound(_))
    ));
    assert!(matches!(
        db.set_trusted(missing, true),
        Err(StorageError::NotFound(_))
    ));
}

#[test]
fn pinned_version_is_protected() {
    let (mut db, id) = db_with_versions(3);
    assert_eq!(db.pinned_version(id).unwrap(), None);

    db.set_pinned_version(id, Some(1)).unwrap();
    assert_eq!(db.pinned_version(id).unwrap(), Some(1));

    // A pinned snapshot cannot be deleted...
    assert!(matches!(
        db.delete_snapshot(id, 1),
        Err(StorageError::Conflict(_))
    ));
    // ...and pruning leaves it alone, keeping the latest too.
    let policy = RetentionPolicy {
        keep_last: Some(1),
        max_age: None,
    };
    db.prune(id, &policy, Utc::now()).unwrap();
    assert_eq!(versions(&db, id), vec![3, 1]);

    // Pinning something that does not exist is refused and changes nothing.
    assert!(matches!(
        db.set_pinned_version(id, Some(9)),
        Err(StorageError::NotFound(_))
    ));
    assert_eq!(db.pinned_version(id).unwrap(), Some(1));

    // Unpinned, it can be deleted again.
    db.set_pinned_version(id, None).unwrap();
    db.delete_snapshot(id, 1).unwrap();
    assert_eq!(versions(&db, id), vec![3]);
}

#[test]
fn restore_history_is_capped_per_snapshot() {
    use cf_storage::RestoreStepRecord;
    use chrono::SecondsFormat;

    let (mut db, id) = db_with_versions(2);
    let first = db.get_snapshot(id, Some(1)).unwrap();
    let second = db.get_snapshot(id, Some(2)).unwrap();
    let step = RestoreStepRecord {
        component_type: "application".into(),
        component_identifier: "code".into(),
        status: "success".into(),
        error_code: None,
        error_message: None,
        duration_ms: 1,
    };

    // 55 runs of snapshot 1, each a second after the last; 3 of snapshot 2.
    let base = Utc::now();
    for i in 0..55 {
        let at = base + TimeDelta::seconds(i);
        db.record_restore_run(first.id, at, at, "success", std::slice::from_ref(&step))
            .unwrap();
    }
    for i in 0..3 {
        let at = base + TimeDelta::seconds(i);
        db.record_restore_run(second.id, at, at, "success", std::slice::from_ref(&step))
            .unwrap();
    }

    let count = |sql: &str| -> i64 {
        db.raw_connection()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    };
    // Each snapshot is capped on its own: 50 + 3, not 50 in total.
    assert_eq!(count("SELECT COUNT(*) FROM restore_runs"), 53);
    assert_eq!(db.restore_runs(second.id).unwrap().len(), 3);
    // The steps of deleted runs are gone too.
    assert_eq!(count("SELECT COUNT(*) FROM restore_steps"), 53);

    // The oldest surviving run of snapshot 1 is the 6th one recorded.
    let oldest: String = db
        .raw_connection()
        .query_row(
            "SELECT started_at FROM restore_runs WHERE snapshot_id = ?1
             ORDER BY started_at ASC LIMIT 1",
            [first.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let sixth = (base + TimeDelta::seconds(5)).to_rfc3339_opts(SecondsFormat::Millis, true);
    assert_eq!(oldest, sixth);
}

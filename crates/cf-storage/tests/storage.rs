use cf_core::{CaptureStatus, DisplayState, GitState, Rect, SnapshotPayload, WindowState};
use cf_storage::{Database, StorageError};

fn sample_payload(note: &str) -> SnapshotPayload {
    let mut p = SnapshotPayload::default();
    p.windows.push(WindowState {
        application_id: "code".into(),
        title: "main.rs - ContextFlow - Visual Studio Code".into(),
        window_class: Some("Chrome_WidgetWin_1".into()),
        monitor_id: Some(r"\\.\DISPLAY1".into()),
        rect: Rect {
            x: 0,
            y: 0,
            width: 1280,
            height: 1000,
        },
        display_state: DisplayState::Normal,
    });
    p.git.push(GitState {
        repository_path: r"C:\src\contextflow".into(),
        branch: Some("main".into()),
        head_commit: Some("a".repeat(40)),
        modified: 2,
        ..Default::default()
    });
    p.notes.push(note.into());
    p
}

#[test]
fn workspaces_are_unique_case_insensitively() {
    let db = Database::open_in_memory().unwrap();
    db.create_context("University Development", None).unwrap();
    let err = db
        .create_context("university development", None)
        .unwrap_err();
    assert!(matches!(err, StorageError::Conflict(_)));
    assert!(db.create_context("   ", None).is_err());
    assert_eq!(db.list_contexts(false).unwrap().len(), 1);
}

#[test]
fn snapshots_are_versioned_per_workspace() {
    let mut db = Database::open_in_memory().unwrap();
    let a = db.create_context("A", None).unwrap();
    let b = db.create_context("B", Some("second")).unwrap();

    let s1 = db
        .create_snapshot(a.id, sample_payload("one"), CaptureStatus::Complete)
        .unwrap();
    let s2 = db
        .create_snapshot(a.id, sample_payload("two"), CaptureStatus::Partial)
        .unwrap();
    let t1 = db
        .create_snapshot(b.id, sample_payload("one"), CaptureStatus::Complete)
        .unwrap();

    assert_eq!((s1.version, s2.version, t1.version), (1, 2, 1));
    // Identical payloads produce identical hashes, across workspaces.
    assert_eq!(s1.hash, t1.hash);
    assert_ne!(s1.hash, s2.hash);

    let listed = db.list_snapshots(a.id).unwrap();
    assert_eq!(
        listed.iter().map(|s| s.version).collect::<Vec<_>>(),
        vec![2, 1]
    );

    let latest = db.get_snapshot(a.id, None).unwrap();
    assert_eq!(latest.version, 2);
    assert_eq!(latest.payload.notes, vec!["two".to_string()]);
    assert_eq!(db.get_snapshot(a.id, Some(1)).unwrap(), s1);
}

#[test]
fn snapshots_cannot_be_updated() {
    let mut db = Database::open_in_memory().unwrap();
    let ctx = db.create_context("A", None).unwrap();
    db.create_snapshot(ctx.id, sample_payload("x"), CaptureStatus::Complete)
        .unwrap();

    let conn = db.raw_connection();
    for sql in [
        "UPDATE context_snapshots SET payload_json = '{}'",
        "UPDATE snapshot_windows SET x = 5",
        "UPDATE snapshot_git_states SET branch_name = 'evil'",
        "UPDATE snapshot_notes SET note_content = 'changed'",
    ] {
        let err = conn.execute(sql, []).unwrap_err();
        assert!(err.to_string().contains("immutable"), "{sql}: {err}");
    }
    assert!(db.verify_snapshot(ctx.id, None).unwrap().ok());
}

#[test]
fn tampering_is_detected() {
    let mut db = Database::open_in_memory().unwrap();
    let ctx = db.create_context("A", None).unwrap();
    db.create_snapshot(ctx.id, sample_payload("x"), CaptureStatus::Complete)
        .unwrap();

    // Simulate out-of-band corruption by bypassing the immutability trigger.
    let conn = db.raw_connection();
    conn.execute_batch(
        "DROP TRIGGER trg_snapshots_immutable;
         UPDATE context_snapshots SET payload_json = replace(payload_json, 'main', 'evil');",
    )
    .unwrap();
    let report = db.verify_snapshot(ctx.id, Some(1)).unwrap();
    assert!(!report.ok());
    assert!(report.payload_parses);
}

#[test]
fn invalid_payloads_are_rejected_without_side_effects() {
    let mut db = Database::open_in_memory().unwrap();
    let ctx = db.create_context("A", None).unwrap();
    let mut bad = sample_payload("x");
    bad.windows[0].rect.width = -1;
    assert!(matches!(
        db.create_snapshot(ctx.id, bad, CaptureStatus::Complete),
        Err(StorageError::Core(_))
    ));
    assert!(db.list_snapshots(ctx.id).unwrap().is_empty());
}

#[test]
fn archived_workspaces_reject_capture() {
    let mut db = Database::open_in_memory().unwrap();
    let ctx = db.create_context("A", None).unwrap();
    db.set_archived(ctx.id, true).unwrap();
    assert!(matches!(
        db.create_snapshot(ctx.id, sample_payload("x"), CaptureStatus::Complete),
        Err(StorageError::Conflict(_))
    ));
    assert!(db.list_contexts(false).unwrap().is_empty());
    assert_eq!(db.list_contexts(true).unwrap().len(), 1);
}

#[test]
fn data_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("contextflow.db");
    let hash = {
        let mut db = Database::open(&path).unwrap();
        let ctx = db.create_context("Persistent", None).unwrap();
        db.create_snapshot(ctx.id, sample_payload("keep me"), CaptureStatus::Complete)
            .unwrap()
            .hash
    };
    let db = Database::open(&path).unwrap();
    assert_eq!(
        db.schema_version().unwrap(),
        Database::latest_schema_version()
    );
    let ctx = db.find_context("persistent").unwrap();
    let snap = db.get_snapshot(ctx.id, None).unwrap();
    assert_eq!(snap.hash, hash);
    assert_eq!(snap.payload.notes, vec!["keep me".to_string()]);
    assert!(db.integrity_check().unwrap().is_empty());
}

#[test]
fn newer_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cf.db");
    {
        let db = Database::open(&path).unwrap();
        db.raw_connection()
            .pragma_update(None, "user_version", 999)
            .unwrap();
    }
    assert!(matches!(
        Database::open(&path),
        Err(StorageError::UnsupportedSchema { found: 999, .. })
    ));
}

-- ContextFlow schema v1.
--
-- context_snapshots.payload_json is the canonical, hashed source of truth.
-- The snapshot_* tables are query-friendly projections written in the same
-- transaction. All snapshot rows are immutable: UPDATE is rejected by triggers.
-- DELETE is permitted only for retention policies and explicit user deletion.

CREATE TABLE contexts (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    is_archived INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0, 1))
);

CREATE TABLE context_snapshots (
    id             TEXT PRIMARY KEY,
    context_id     TEXT NOT NULL REFERENCES contexts(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL CHECK (version_number > 0),
    created_at     TEXT NOT NULL,
    snapshot_hash  TEXT NOT NULL,
    capture_status TEXT NOT NULL CHECK (capture_status IN ('complete', 'partial', 'failed')),
    payload_json   TEXT NOT NULL,
    UNIQUE (context_id, version_number)
);
CREATE INDEX idx_snapshots_context ON context_snapshots(context_id, version_number DESC);
CREATE INDEX idx_snapshots_hash ON context_snapshots(snapshot_hash);

CREATE TABLE snapshot_applications (
    id                    INTEGER PRIMARY KEY,
    snapshot_id           TEXT NOT NULL REFERENCES context_snapshots(id) ON DELETE CASCADE,
    application_id        TEXT NOT NULL,
    executable_path       TEXT NOT NULL,
    launch_arguments_json TEXT NOT NULL,
    working_directory     TEXT,
    adapter_id            TEXT,
    adapter_state_json    TEXT NOT NULL
);
CREATE INDEX idx_snapshot_applications ON snapshot_applications(snapshot_id);

CREATE TABLE snapshot_windows (
    id                 INTEGER PRIMARY KEY,
    snapshot_id        TEXT NOT NULL REFERENCES context_snapshots(id) ON DELETE CASCADE,
    application_id     TEXT NOT NULL,
    title              TEXT NOT NULL,
    window_class       TEXT,
    monitor_identifier TEXT,
    x                  INTEGER NOT NULL,
    y                  INTEGER NOT NULL,
    width              INTEGER NOT NULL CHECK (width > 0),
    height             INTEGER NOT NULL CHECK (height > 0),
    display_state      TEXT NOT NULL CHECK (display_state IN ('normal', 'maximized', 'minimized'))
);
CREATE INDEX idx_snapshot_windows ON snapshot_windows(snapshot_id);

CREATE TABLE snapshot_git_states (
    id                INTEGER PRIMARY KEY,
    snapshot_id       TEXT NOT NULL REFERENCES context_snapshots(id) ON DELETE CASCADE,
    repository_path   TEXT NOT NULL,
    branch_name       TEXT,
    head_commit       TEXT,
    staged_changes    INTEGER NOT NULL,
    modified_changes  INTEGER NOT NULL,
    untracked_changes INTEGER NOT NULL
);
CREATE INDEX idx_snapshot_git_states ON snapshot_git_states(snapshot_id);

-- Populated from Phase 4 (file metadata comparison).
CREATE TABLE snapshot_files (
    id                     INTEGER PRIMARY KEY,
    snapshot_id            TEXT NOT NULL REFERENCES context_snapshots(id) ON DELETE CASCADE,
    project_path           TEXT NOT NULL,
    file_path              TEXT NOT NULL,
    modification_timestamp TEXT,
    file_size              INTEGER,
    content_hash           TEXT
);
CREATE INDEX idx_snapshot_files ON snapshot_files(snapshot_id);

CREATE TABLE snapshot_notes (
    id           INTEGER PRIMARY KEY,
    snapshot_id  TEXT NOT NULL REFERENCES context_snapshots(id) ON DELETE CASCADE,
    note_content TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_snapshot_notes ON snapshot_notes(snapshot_id);

-- Populated from Phase 2 (restoration engine).
CREATE TABLE restore_runs (
    id             TEXT PRIMARY KEY,
    snapshot_id    TEXT NOT NULL REFERENCES context_snapshots(id) ON DELETE CASCADE,
    started_at     TEXT NOT NULL,
    completed_at   TEXT,
    overall_status TEXT NOT NULL
);

CREATE TABLE restore_steps (
    id                   INTEGER PRIMARY KEY,
    restore_run_id       TEXT NOT NULL REFERENCES restore_runs(id) ON DELETE CASCADE,
    component_type       TEXT NOT NULL,
    component_identifier TEXT NOT NULL,
    status               TEXT NOT NULL,
    error_code           TEXT,
    error_message        TEXT,
    duration_ms          INTEGER
);
CREATE INDEX idx_restore_steps ON restore_steps(restore_run_id);

-- Immutability enforcement.
CREATE TRIGGER trg_snapshots_immutable BEFORE UPDATE ON context_snapshots
BEGIN SELECT RAISE(ABORT, 'snapshots are immutable'); END;
CREATE TRIGGER trg_snapshot_applications_immutable BEFORE UPDATE ON snapshot_applications
BEGIN SELECT RAISE(ABORT, 'snapshots are immutable'); END;
CREATE TRIGGER trg_snapshot_windows_immutable BEFORE UPDATE ON snapshot_windows
BEGIN SELECT RAISE(ABORT, 'snapshots are immutable'); END;
CREATE TRIGGER trg_snapshot_git_states_immutable BEFORE UPDATE ON snapshot_git_states
BEGIN SELECT RAISE(ABORT, 'snapshots are immutable'); END;
CREATE TRIGGER trg_snapshot_files_immutable BEFORE UPDATE ON snapshot_files
BEGIN SELECT RAISE(ABORT, 'snapshots are immutable'); END;
CREATE TRIGGER trg_snapshot_notes_immutable BEFORE UPDATE ON snapshot_notes
BEGIN SELECT RAISE(ABORT, 'snapshots are immutable'); END;

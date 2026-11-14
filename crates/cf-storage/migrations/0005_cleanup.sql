-- Per-workspace cleanup rules (JSON; NULL = none) and a small key/value table,
-- used to remember which workspace is active.
ALTER TABLE contexts ADD COLUMN cleanup_json TEXT;

CREATE TABLE app_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

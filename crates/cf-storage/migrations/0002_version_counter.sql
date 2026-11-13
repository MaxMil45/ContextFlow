-- Schema v2: per-workspace version counter.
--
-- Version numbers are allocated from contexts.last_version instead of
-- MAX(version_number), so deleting the newest snapshot can never cause a
-- version number to be reused for different content.

ALTER TABLE contexts ADD COLUMN last_version INTEGER NOT NULL DEFAULT 0 CHECK (last_version >= 0);

UPDATE contexts SET last_version = COALESCE(
    (SELECT MAX(version_number) FROM context_snapshots s WHERE s.context_id = contexts.id),
    0
);

CREATE INDEX idx_snapshots_created ON context_snapshots(context_id, created_at);

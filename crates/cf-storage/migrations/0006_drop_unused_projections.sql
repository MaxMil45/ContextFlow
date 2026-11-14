-- The snapshot_* projection tables were written on every capture but never read:
-- the hashed payload in context_snapshots is the single source of truth.
-- Dropping the tables also drops their immutability triggers.
DROP TABLE IF EXISTS snapshot_applications;
DROP TABLE IF EXISTS snapshot_windows;
DROP TABLE IF EXISTS snapshot_git_states;
DROP TABLE IF EXISTS snapshot_files;
DROP TABLE IF EXISTS snapshot_notes;

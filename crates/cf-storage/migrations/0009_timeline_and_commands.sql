-- The timeline: every switch, restore, leave, park and resume, newest first.
-- Kept to the most recent 1000 entries.
CREATE TABLE context_events (
    id              INTEGER PRIMARY KEY,
    at              TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK (kind IN ('switch', 'restore', 'leave', 'park', 'resume')),
    context_id      TEXT REFERENCES contexts(id) ON DELETE CASCADE,
    from_context_id TEXT REFERENCES contexts(id) ON DELETE SET NULL,
    version         INTEGER,
    note            TEXT
);
CREATE INDEX idx_context_events_at ON context_events(at DESC);

-- A workspace's start and stop commands (JSON). Local only: never exported.
ALTER TABLE contexts ADD COLUMN commands_json TEXT;

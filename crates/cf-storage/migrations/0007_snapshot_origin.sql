-- Where a snapshot came from: 'manual' (Capture, quick capture, CLI) or 'auto'
-- (saved automatically when switching away from or leaving a workspace).
-- Retention keeps a separate, small allowance of auto-saves so they never push
-- out versions the user saved on purpose.
ALTER TABLE context_snapshots ADD COLUMN origin TEXT NOT NULL DEFAULT 'manual'
    CHECK (origin IN ('manual', 'auto'));

-- What happens to a workspace when you switch away from it or leave it:
-- 'auto'   save its changes (the default),
-- 'notify' save its changes and say so,
-- 'fixed'  never save automatically: it always restores exactly as captured.
ALTER TABLE contexts ADD COLUMN save_mode TEXT NOT NULL DEFAULT 'auto'
    CHECK (save_mode IN ('auto', 'notify', 'fixed'));

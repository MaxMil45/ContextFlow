ALTER TABLE contexts ADD COLUMN is_trusted INTEGER NOT NULL DEFAULT 1 CHECK (is_trusted IN (0, 1));

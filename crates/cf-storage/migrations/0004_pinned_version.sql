ALTER TABLE contexts ADD COLUMN pinned_version INTEGER CHECK (pinned_version IS NULL OR pinned_version > 0);

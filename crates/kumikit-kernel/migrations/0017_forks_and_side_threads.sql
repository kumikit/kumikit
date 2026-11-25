-- Forks and side threads (M7 step 6).
ALTER TABLE threads ADD COLUMN forked_from_json TEXT;
ALTER TABLE threads ADD COLUMN parent_thread TEXT;
ALTER TABLE threads ADD COLUMN side INTEGER NOT NULL DEFAULT 0;
ALTER TABLE threads ADD COLUMN parent_seen_seq INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_threads_parent
    ON threads(parent_thread)
    WHERE parent_thread IS NOT NULL;

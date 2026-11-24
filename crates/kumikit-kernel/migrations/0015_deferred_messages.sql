-- Schema 15: deferred messages (wake-ups, ADR 0027).

ALTER TABLE queued_messages ADD COLUMN trigger_json TEXT;
ALTER TABLE queued_messages ADD COLUMN next_at TEXT;
ALTER TABLE queued_messages ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE queued_messages ADD COLUMN wait_thread TEXT;
ALTER TABLE queued_messages ADD COLUMN deferred_entry TEXT;

CREATE INDEX queued_messages_next_at
    ON queued_messages (next_at)
    WHERE next_at IS NOT NULL;

CREATE INDEX queued_messages_wait_thread
    ON queued_messages (wait_thread)
    WHERE wait_thread IS NOT NULL;

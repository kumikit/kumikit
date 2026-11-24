-- Schema 13: the durable thread queue (ADR 0027). Rows are derived from `queue.*` and
-- `turn.started` events and written in the same transaction as the event.
-- Migrations are forward-only; never edit a migration that has shipped, add a new one.

CREATE TABLE queued_messages (
    id       TEXT PRIMARY KEY,
    thread   TEXT NOT NULL REFERENCES threads (id),
    -- The turn id the message will run as.
    turn     TEXT NOT NULL,
    -- Order within the thread; lower runs first. Not dense.
    position INTEGER NOT NULL,
    -- `UserInput` as JSON.
    input    TEXT NOT NULL,
    -- The plugin that sent it, if one did.
    sender   TEXT,
    created  TEXT NOT NULL,
    updated  TEXT NOT NULL
) STRICT;

CREATE INDEX queued_messages_by_thread ON queued_messages (thread, position);

-- Why the queue is held (`stopped`, `failed`, `restarted`), or NULL.
ALTER TABLE threads ADD COLUMN queue_hold TEXT;

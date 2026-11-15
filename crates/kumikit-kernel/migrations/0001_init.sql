-- Schema 1: workspaces, threads, turns, the event log and approvals.
-- Migrations are forward-only; never edit a migration that has shipped, add a new one.

CREATE TABLE workspaces (
    id      TEXT PRIMARY KEY,
    name    TEXT NOT NULL,
    root    TEXT NOT NULL UNIQUE,
    created TEXT NOT NULL,
    closed  INTEGER NOT NULL DEFAULT 0
) STRICT;

CREATE TABLE threads (
    id               TEXT PRIMARY KEY,
    workspace        TEXT NOT NULL REFERENCES workspaces (id),
    title            TEXT,
    provider         TEXT NOT NULL,
    model            TEXT NOT NULL,
    -- Derived from the log, written in the same transaction as the event that changes it.
    status           TEXT NOT NULL,
    archived         INTEGER NOT NULL DEFAULT 0,
    created          TEXT NOT NULL,
    updated          TEXT NOT NULL,
    -- Last assigned seq; 0 before the first event.
    head             INTEGER NOT NULL DEFAULT 0,
    provider_session TEXT
) STRICT;

CREATE INDEX threads_by_workspace ON threads (workspace, updated);

CREATE TABLE events (
    thread TEXT NOT NULL REFERENCES threads (id),
    seq    INTEGER NOT NULL,
    time   TEXT NOT NULL,
    kind   TEXT NOT NULL,
    data   TEXT NOT NULL,
    PRIMARY KEY (thread, seq)
) STRICT, WITHOUT ROWID;

CREATE TABLE turns (
    id          TEXT PRIMARY KEY,
    thread      TEXT NOT NULL REFERENCES threads (id),
    -- seq of the turn.started event; orders turns within a thread.
    started_seq INTEGER NOT NULL,
    started     TEXT NOT NULL,
    ended       TEXT,
    outcome     TEXT
) STRICT;

CREATE INDEX turns_by_thread ON turns (thread, started_seq);

CREATE TABLE approvals (
    id         TEXT PRIMARY KEY,
    thread     TEXT NOT NULL REFERENCES threads (id),
    turn       TEXT NOT NULL REFERENCES turns (id),
    kind       TEXT NOT NULL,
    summary    TEXT NOT NULL,
    detail     TEXT,
    status     TEXT NOT NULL,
    decision   TEXT,
    decided_by TEXT
) STRICT;

CREATE INDEX approvals_by_thread ON approvals (thread, status);

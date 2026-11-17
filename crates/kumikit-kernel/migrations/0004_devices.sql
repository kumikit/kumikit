-- Schema 4: paired devices (docs/spec/03-kernel.md#storage). Only the SHA-256 of a
-- device's token is kept; the token itself is never stored.

CREATE TABLE devices (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created    TEXT NOT NULL,
    last_seen  TEXT
) STRICT;

-- Schedules (docs/spec/03-kernel.md#schedules; ADR 0024). Times are RFC 3339 in UTC.
CREATE TABLE schedules (
    plugin       TEXT NOT NULL,
    key          TEXT NOT NULL,
    cron         TEXT NOT NULL,
    zone         TEXT,               -- IANA name; NULL for the core's local zone
    description  TEXT NOT NULL,
    missed       TEXT NOT NULL,      -- skip | once
    enabled      INTEGER NOT NULL,
    source       TEXT NOT NULL,      -- manifest | plugin
    next_run     TEXT,
    last_run     TEXT,
    last_outcome TEXT,               -- ran | failed | missed | skipped
    PRIMARY KEY (plugin, key)
) STRICT, WITHOUT ROWID;

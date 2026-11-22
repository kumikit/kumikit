-- Schema 10: core-level key-value configuration (such as Kumikit home folder).
-- Migrations are forward-only; never edit a migration that has shipped, add a new one.

CREATE TABLE core_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT, WITHOUT ROWID;

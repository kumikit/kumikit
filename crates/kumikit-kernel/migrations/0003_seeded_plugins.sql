-- Schema 3: core plugins preinstalled from a bundled directory (`--bundled-plugins`).
-- A row stays after the plugin is removed, so a removed plugin is not seeded again.

CREATE TABLE seeded_plugins (
    id      TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    seeded  TEXT NOT NULL
) STRICT;

-- Schema 2: installed plugins and their key-value stores.
-- Migrations are forward-only; never edit a migration that has shipped, add a new one.

CREATE TABLE plugins (
    id        TEXT PRIMARY KEY,
    version   TEXT NOT NULL,
    -- installed | enabled | disabled
    state     TEXT NOT NULL,
    -- SHA-256 of the package file, hex.
    sha256    TEXT NOT NULL,
    -- Granted permissions as JSON; NULL until the user grants them.
    granted   TEXT,
    installed TEXT NOT NULL
) STRICT;

-- One namespace per plugin. Rows are deleted with the plugin's data on uninstall.
CREATE TABLE plugin_kv (
    plugin TEXT NOT NULL,
    key    TEXT NOT NULL,
    value  BLOB NOT NULL,
    PRIMARY KEY (plugin, key)
) STRICT, WITHOUT ROWID;

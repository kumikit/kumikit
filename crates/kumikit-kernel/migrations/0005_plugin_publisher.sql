-- Schema 5: where each plugin came from and the key that signed it (10-distribution,
-- Install; ADR 0018). A later version must be signed by `publisher_key`.

-- The signing key's id (`ed25519:…`); NULL for an unsigned package.
ALTER TABLE plugins ADD COLUMN publisher_key TEXT;
-- `file`, `bundled`, or `registry+<index URL>`.
ALTER TABLE plugins ADD COLUMN source TEXT NOT NULL DEFAULT 'file';

UPDATE plugins SET source = 'bundled' WHERE id IN (SELECT id FROM seeded_plugins);

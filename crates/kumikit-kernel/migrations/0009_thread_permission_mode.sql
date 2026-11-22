-- Schema 9: per-thread permission mode (`standard`, `accept_edits`, `plan`, `full_access`).
-- NULL means `standard`.

ALTER TABLE threads ADD COLUMN permission_mode TEXT;

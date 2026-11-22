-- Schema 11: optional per-thread Git worktree path so worktree threads stay under their parent workspace.
-- Migrations are forward-only; never edit a migration that has shipped, add a new one.

ALTER TABLE threads ADD COLUMN worktree_path TEXT;

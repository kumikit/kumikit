-- Per-turn author attribution and pending provider handoff (ADR 0029).

ALTER TABLE turns ADD COLUMN provider TEXT;
ALTER TABLE turns ADD COLUMN model TEXT;
ALTER TABLE turns ADD COLUMN purpose TEXT;

ALTER TABLE threads ADD COLUMN pending_handoff TEXT;

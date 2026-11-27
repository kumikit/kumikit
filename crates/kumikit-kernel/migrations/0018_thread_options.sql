-- Schema 18: per-thread model settings (ADR 0033), a JSON object of option id to value.
-- NULL means none chosen.

ALTER TABLE threads ADD COLUMN options_json TEXT;

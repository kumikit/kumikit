-- Token usage per turn and each provider's latest plan limits (ADR 0035).

-- One row per turn that reported usage, derived from its last `usage` event in the same
-- transaction. Not tied to the thread: deleting a thread keeps its counts, so usage
-- statistics do not shrink when the user tidies the sidebar. No text is stored.
CREATE TABLE usage_records (
    turn                TEXT PRIMARY KEY,
    thread              TEXT NOT NULL,
    provider            TEXT NOT NULL,
    model               TEXT NOT NULL,
    -- When the turn started (RFC 3339 UTC); statistics count a turn on this day.
    started             TEXT NOT NULL,
    input_tokens        INTEGER NOT NULL,
    output_tokens       INTEGER NOT NULL,
    cached_input_tokens INTEGER NOT NULL DEFAULT 0,
    cost                REAL,
    -- The `models` breakdown of the event as JSON, when it had one.
    models              TEXT
) STRICT;

CREATE INDEX usage_records_by_started ON usage_records (started);

-- The latest plan limits each provider reported; `limits` is NULL when the provider said
-- it has none.
CREATE TABLE provider_limits (
    provider TEXT PRIMARY KEY,
    limits   TEXT,
    observed TEXT NOT NULL
) STRICT;

-- Backfill from the existing logs: each turn's last `usage` event. Copies of events in
-- forked threads have no turn row of their own, so they are not counted twice.
INSERT INTO usage_records
    (turn, thread, provider, model, started, input_tokens, output_tokens, cost)
SELECT t.id,
       t.thread,
       coalesce(t.provider, th.provider),
       coalesce(t.model, th.model),
       t.started,
       coalesce(json_extract(e.data, '$.inputTokens'), 0),
       coalesce(json_extract(e.data, '$.outputTokens'), 0),
       json_extract(e.data, '$.cost')
FROM turns t
JOIN threads th ON th.id = t.thread
JOIN events e ON e.thread = t.thread AND e.seq = (
    SELECT max(u.seq) FROM events u
    WHERE u.thread = t.thread
      AND u.kind = 'usage'
      AND json_extract(u.data, '$.turn') = t.id
);

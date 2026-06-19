-- Per-turn token usage and cost recorded under a session. Session-level totals are derived by
-- summing every recorded turn, so the cumulative figure is never reset to a single turn's value.

CREATE TABLE usage (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    input_tokens  INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    total_tokens  INTEGER NOT NULL,
    cost          REAL,
    created_at    TEXT NOT NULL
);

CREATE INDEX idx_usage_session ON usage(session_id, created_at, id);

-- Per-session todo items. The list is ordered by `position` (0-based) and replaced wholesale by a
-- `todo` tool write, so the stored rows always reflect the latest list the model committed for the
-- session. Cascades with its owning session so a deleted session takes its todos with it.

CREATE TABLE todos (
    id         TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    content    TEXT NOT NULL,
    status     TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_todos_session ON todos(session_id, position);

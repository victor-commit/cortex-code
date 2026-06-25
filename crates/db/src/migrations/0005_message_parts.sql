-- Rich persisted message parts keyed by message id and stable part id.
-- `part_json` stores the discriminated protocol::MessagePart payload.

CREATE TABLE message_parts (
    id         TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    ordinal    INTEGER NOT NULL,
    part_json  TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_message_parts_message
    ON message_parts(message_id, ordinal, id);

CREATE INDEX idx_message_parts_session
    ON message_parts(session_id, message_id, ordinal);

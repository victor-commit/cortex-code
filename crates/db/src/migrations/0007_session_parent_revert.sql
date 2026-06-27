-- Session forking and revert state.
-- `parent_id` records the session a fork was created from, so child sessions can be listed.
-- NULL means a top-level (non-forked) session.
ALTER TABLE sessions ADD COLUMN parent_id TEXT;

-- `reverted_message_id` records the message a session is currently reverted to (file changes
-- undone via the shadow checkpoint store). NULL means the session is not in a reverted state.
ALTER TABLE sessions ADD COLUMN reverted_message_id TEXT;

CREATE INDEX idx_sessions_parent ON sessions(parent_id);

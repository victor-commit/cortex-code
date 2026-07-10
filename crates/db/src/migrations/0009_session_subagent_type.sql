-- Immutable task-child persona metadata.
--
-- Existing task children recorded their persona only in a mutable title. Do not backfill this
-- column: a legacy title cannot prove the child was created for the persona it claims. Those
-- rows intentionally remain NULL and task authorization fails closed.
ALTER TABLE sessions ADD COLUMN subagent_type TEXT CHECK (
    subagent_type IS NULL
    OR (
        length(trim(subagent_type)) > 0
        AND subagent_type = lower(trim(subagent_type))
    )
);

-- A task persona is declared only when the child row is created. Neither task rows nor generic
-- forks may gain, clear, or change a persona later through mutable display/session operations.
CREATE TRIGGER sessions_subagent_type_immutable
BEFORE UPDATE OF subagent_type ON sessions
WHEN OLD.subagent_type IS NOT NEW.subagent_type
BEGIN
    SELECT RAISE(ABORT, 'subagent type is immutable');
END;

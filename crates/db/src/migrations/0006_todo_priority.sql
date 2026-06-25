-- Add todo priority while preserving existing rows.
--
-- Existing todos default to `medium` so older sessions continue to deserialize with the new
-- protocol shape.

ALTER TABLE todos
ADD COLUMN priority TEXT NOT NULL DEFAULT 'medium';

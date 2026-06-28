-- Multi-directory project model, durable account records, and session-to-project links.
--
-- A `project` groups one or more directories under one stable id, so sessions, search, and config
-- can span every directory of a single logical project instead of keying identity to one cwd hash.
-- An `account` is a durable mapped credential record for a provider/plugin auth identity; it never
-- holds a raw secret (the 0600 auth store remains the sole secret sink). Sessions gain a nullable
-- `project_id` so they retain project identity across restarts.

CREATE TABLE projects (
    id         TEXT PRIMARY KEY,
    name       TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE project_directories (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path       TEXT NOT NULL,
    position   INTEGER NOT NULL,
    is_primary INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    PRIMARY KEY (project_id, path)
);

-- A directory belongs to at most one project, so a session opened from any registered directory
-- resolves to exactly one project (no cwd-hash collisions across a project's directories).
CREATE UNIQUE INDEX idx_project_directories_path ON project_directories(path);

CREATE TABLE accounts (
    id          TEXT PRIMARY KEY,
    provider    TEXT NOT NULL UNIQUE,
    label       TEXT,
    auth_method TEXT,
    source      TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

ALTER TABLE sessions ADD COLUMN project_id TEXT;
CREATE INDEX idx_sessions_project ON sessions(project_id);

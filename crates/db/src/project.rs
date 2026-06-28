//! The projects repository: a project groups one or more directories under one stable id.
//!
//! Where the legacy model keyed project identity to a single working-directory hash, a [`Project`]
//! can span several directories. Each directory belongs to at most one project (a `UNIQUE` index on
//! the directory path), so a session opened from any registered directory resolves to exactly one
//! project rather than minting a fresh cwd-hash identity. The directory list is ordered, with the
//! first registered directory marked primary.
//!
//! Paths are stored verbatim (a filesystem path is not a secret, and exact bytes are required to
//! match a directory back to its project); callers redact at the output boundary as elsewhere.

use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use crate::error::{DbError, DbResult};

/// A persisted project: a named group of one or more directories under a stable id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Stable project identifier. Callers derive it from the primary directory's workspace key so
    /// the same primary directory always resolves to the same project across processes.
    pub id: String,
    /// Optional human-facing name (typically the primary directory's basename).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Every directory registered to this project, ordered with the primary directory first.
    pub directories: Vec<String>,
    /// Creation timestamp (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// Last-update timestamp (UTC), advanced when a directory is added or removed.
    pub updated_at: String,
}

/// Read/write access to the `projects`/`project_directories` tables over a borrowed connection.
pub struct ProjectRepo<'a> {
    conn: &'a Connection,
}

impl<'a> ProjectRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Ensure a project with the given stable `id` exists whose primary directory is `primary_dir`,
    /// returning the persisted record.
    ///
    /// Idempotent and race-tolerant (`INSERT OR IGNORE`): two processes ensuring the same id
    /// converge on one row, and an existing project's metadata/directories are left intact. The
    /// primary directory is registered (position 0, `is_primary = 1`) on first creation.
    pub fn ensure(&self, id: &str, name: Option<&str>, primary_dir: &str) -> DbResult<Project> {
        self.conn.execute(
            "INSERT OR IGNORE INTO projects (id, name, created_at, updated_at)
             VALUES (?1, ?2, datetime('now'), datetime('now'))",
            params![id, name],
        )?;
        // The primary directory belongs to this project; if another project already claims it, that
        // is a real conflict the caller should see rather than a silent re-home.
        if let Some(owner) = self.project_for_directory(primary_dir)? {
            if owner != id {
                return Err(DbError::Operation {
                    message: format!(
                        "directory `{primary_dir}` already belongs to project `{owner}`"
                    ),
                });
            }
        } else {
            self.insert_directory(id, primary_dir, true)?;
        }
        self.require(id)
    }

    /// Register an additional directory with an existing project, returning the updated record.
    ///
    /// Errors with [`DbError::NotFound`] when the project does not exist and with
    /// [`DbError::Operation`] when the directory already belongs to a different project (the
    /// `UNIQUE` path index keeps a directory in exactly one project). Re-adding a directory already
    /// in this project is a no-op.
    pub fn add_directory(&self, project_id: &str, dir: &str) -> DbResult<Project> {
        self.require(project_id)?;
        match self.project_for_directory(dir)? {
            Some(owner) if owner == project_id => {}
            Some(owner) => {
                return Err(DbError::Operation {
                    message: format!("directory `{dir}` already belongs to project `{owner}`"),
                });
            }
            None => {
                self.insert_directory(project_id, dir, false)?;
                self.touch(project_id)?;
            }
        }
        self.require(project_id)
    }

    /// Remove a non-primary directory from a project, returning whether a row was removed.
    ///
    /// The primary directory cannot be removed (it anchors the project's identity); attempting to
    /// remove it, or an unregistered directory, is a clean `false`.
    pub fn remove_directory(&self, project_id: &str, dir: &str) -> DbResult<bool> {
        let affected = self.conn.execute(
            "DELETE FROM project_directories \
             WHERE project_id = ?1 AND path = ?2 AND is_primary = 0",
            params![project_id, dir],
        )?;
        if affected > 0 {
            self.touch(project_id)?;
        }
        Ok(affected > 0)
    }

    /// The id of the project that owns `dir`, or `None` when no project claims it.
    pub fn project_for_directory(&self, dir: &str) -> DbResult<Option<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT project_id FROM project_directories WHERE path = ?1")?;
        let mut rows = statement.query_map(params![dir], |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// The project that owns `dir` (with its full directory list), or `None`.
    pub fn find_by_directory(&self, dir: &str) -> DbResult<Option<Project>> {
        match self.project_for_directory(dir)? {
            Some(id) => self.get(&id),
            None => Ok(None),
        }
    }

    /// Fetch one project (with its ordered directories) by id, or `None`.
    pub fn get(&self, id: &str) -> DbResult<Option<Project>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, name, created_at, updated_at FROM projects WHERE id = ?1")?;
        let mut rows = statement.query_map(params![id], project_row)?;
        match rows.next() {
            Some(row) => {
                let mut project = row?;
                project.directories = self.directories(id)?;
                Ok(Some(project))
            }
            None => Ok(None),
        }
    }

    /// Fetch one project by id, erroring with [`DbError::NotFound`] if it is absent.
    pub fn require(&self, id: &str) -> DbResult<Project> {
        self.get(id)?.ok_or_else(|| DbError::NotFound {
            kind: "project",
            id: id.to_owned(),
        })
    }

    /// List every project (each with its ordered directories), newest-first.
    pub fn list(&self) -> DbResult<Vec<Project>> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, created_at, updated_at FROM projects \
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = statement.query_map([], project_row)?;
        let mut projects = Vec::new();
        for row in rows {
            let mut project = row?;
            project.directories = self.directories(&project.id)?;
            projects.push(project);
        }
        Ok(projects)
    }

    /// The directories registered to a project, ordered (primary first, then by registration).
    pub fn directories(&self, project_id: &str) -> DbResult<Vec<String>> {
        let mut statement = self.conn.prepare(
            "SELECT path FROM project_directories WHERE project_id = ?1 \
             ORDER BY is_primary DESC, position ASC",
        )?;
        let rows = statement.query_map(params![project_id], |row| row.get::<_, String>(0))?;
        let mut directories = Vec::new();
        for row in rows {
            directories.push(row?);
        }
        Ok(directories)
    }

    fn insert_directory(&self, project_id: &str, dir: &str, is_primary: bool) -> DbResult<()> {
        let position: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM project_directories WHERE project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO project_directories \
             (project_id, path, position, is_primary, created_at) \
             VALUES (?1, ?2, ?3, ?4, datetime('now'))",
            params![project_id, dir, position, i64::from(is_primary)],
        )?;
        Ok(())
    }

    fn touch(&self, id: &str) -> DbResult<()> {
        self.conn.execute(
            "UPDATE projects SET updated_at = datetime('now') WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }
}

fn project_row(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get("id")?,
        name: row.get("name")?,
        directories: Vec::new(),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

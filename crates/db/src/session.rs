//! The sessions repository: durable conversation records keyed by a stable id.

use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use crate::error::{DbError, DbResult};
use crate::id::random_id;

/// A persisted session: a top-level conversation that messages hang off of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Stable, opaque identifier (prefixed `ses_`), reusable across process invocations.
    pub id: String,
    /// Optional human-facing title.
    pub title: Option<String>,
    /// Creation timestamp (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// Last-update timestamp (UTC), advanced when a message is appended.
    pub updated_at: String,
}

impl Session {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

/// Read/write access to the `sessions` table over a borrowed connection.
pub struct SessionRepo<'a> {
    conn: &'a Connection,
}

impl<'a> SessionRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Create a new session with an optional title, returning the persisted record.
    ///
    /// The title is scrubbed of API-key-shaped secrets (replaced by the canonical
    /// [`redaction::PLACEHOLDER`]) before it is written, so a secret pasted into a title never
    /// lands in the database file; non-secret titles are stored verbatim.
    pub fn create(&self, title: Option<&str>) -> DbResult<Session> {
        let title = title.map(redaction::redact);
        let id = random_id(self.conn, "ses")?;
        let session = self.conn.query_row(
            "INSERT INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, ?2, datetime('now'), datetime('now'))
             RETURNING id, title, created_at, updated_at",
            params![id, title],
            Session::from_row,
        )?;
        Ok(session)
    }

    /// Fetch a session by id, or `None` if it does not exist.
    pub fn get(&self, id: &str) -> DbResult<Option<Session>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, title, created_at, updated_at FROM sessions WHERE id = ?1")?;
        let mut rows = statement.query_map(params![id], Session::from_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Fetch a session by id, erroring with [`DbError::NotFound`] if it is absent.
    pub fn require(&self, id: &str) -> DbResult<Session> {
        self.get(id)?.ok_or_else(|| DbError::NotFound {
            kind: "session",
            id: id.to_owned(),
        })
    }

    /// List sessions newest-first (by creation time, then id for a stable tiebreak).
    pub fn list(&self) -> DbResult<Vec<Session>> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, created_at, updated_at FROM sessions \
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = statement.query_map([], Session::from_row)?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }
}

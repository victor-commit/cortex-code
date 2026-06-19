//! The messages repository: ordered, content-bearing records stored under a session.

use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use crate::error::{DbError, DbResult};
use crate::id::random_id;

/// A persisted message belonging to a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Stable, opaque identifier (prefixed `msg_`).
    pub id: String,
    /// The owning session's id.
    pub session_id: String,
    /// The message role (e.g. `user`, `assistant`, `system`).
    pub role: String,
    /// The message content, stored and retrieved verbatim.
    pub content: String,
    /// Creation timestamp (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
}

impl Message {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            session_id: row.get("session_id")?,
            role: row.get("role")?,
            content: row.get("content")?,
            created_at: row.get("created_at")?,
        })
    }
}

/// Read/write access to the `messages` table over a borrowed connection.
pub struct MessageRepo<'a> {
    conn: &'a Connection,
}

impl<'a> MessageRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Append a message to a session, returning the persisted record.
    ///
    /// The content is scrubbed of API-key-shaped secrets (replaced by the canonical
    /// [`redaction::PLACEHOLDER`]) before it is written, so the raw secret never lands in the
    /// database file; non-secret text is stored verbatim.
    ///
    /// Errors with [`DbError::NotFound`] if the session does not exist, so the caller gets a
    /// clear diagnostic rather than an opaque foreign-key constraint failure.
    pub fn create(&self, session_id: &str, role: &str, content: &str) -> DbResult<Message> {
        let session_exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id = ?1)",
            params![session_id],
            |row| row.get(0),
        )?;
        if !session_exists {
            return Err(DbError::NotFound {
                kind: "session",
                id: session_id.to_owned(),
            });
        }

        let content = redaction::redact(content);
        let id = random_id(self.conn, "msg")?;
        let message = self.conn.query_row(
            "INSERT INTO messages (id, session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             RETURNING id, session_id, role, content, created_at",
            params![id, session_id, role, content],
            Message::from_row,
        )?;
        self.conn.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
            params![session_id],
        )?;
        Ok(message)
    }

    /// List the messages under a session in chronological order (oldest first).
    pub fn list_for_session(&self, session_id: &str) -> DbResult<Vec<Message>> {
        let mut statement = self.conn.prepare(
            "SELECT id, session_id, role, content, created_at FROM messages \
             WHERE session_id = ?1 ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map(params![session_id], Message::from_row)?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        Ok(messages)
    }
}

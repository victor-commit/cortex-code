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
    /// The model chosen for this session (e.g. via the TUI `/models` picker), or `None` when no
    /// per-session choice was made (the configured/default model then governs). Honored on a
    /// headless `cortex run --session <id>` resume without an explicit `--model`.
    pub model: Option<String>,
    /// The session this one was forked from, or `None` for a top-level session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// The message id this session is currently reverted to (its file changes undone via the
    /// shadow checkpoint store), or `None` when the session is not in a reverted state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reverted_message_id: Option<String>,
}

impl Session {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            model: row.get("model")?,
            parent_id: row.get("parent_id")?,
            reverted_message_id: row.get("reverted_message_id")?,
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
             RETURNING id, title, created_at, updated_at, model, parent_id, reverted_message_id",
            params![id, title],
            Session::from_row,
        )?;
        Ok(session)
    }

    /// Get a session by id, creating it with that exact id (and optional title) when it does not
    /// yet exist, returning the persisted record.
    ///
    /// Unlike [`create`](Self::create), the caller supplies the id, so an arbitrary, externally
    /// chosen identifier (e.g. a user-provided `--session` value) becomes a durable session that
    /// can be resumed across process invocations. The insert is idempotent and race-tolerant
    /// (`INSERT OR IGNORE`): two processes ensuring the same id converge on the same row rather
    /// than one failing on a primary-key conflict, and an existing session is left untouched.
    ///
    /// Any title is scrubbed of API-key-shaped secrets before it is written, matching
    /// [`create`](Self::create).
    pub fn ensure(&self, id: &str, title: Option<&str>) -> DbResult<Session> {
        let title = title.map(redaction::redact);
        self.conn.execute(
            "INSERT OR IGNORE INTO sessions (id, title, created_at, updated_at)
             VALUES (?1, ?2, datetime('now'), datetime('now'))",
            params![id, title],
        )?;
        self.require(id)
    }

    /// The most recently active session, or `None` when there are none.
    ///
    /// "Most recent" tracks the last activity rather than creation order: it is the session holding
    /// the newest message (messages are appended with a monotonically increasing `rowid`), so a
    /// `--continue` run attaches to the conversation most recently written to. Because the message
    /// `rowid` is monotonic this is robust to the second-granular timestamps (two operations in the
    /// same clock second still order correctly). Sessions with no messages fall back to creation
    /// order (their own `rowid`), and sort behind any session that has activity.
    pub fn most_recent(&self) -> DbResult<Option<Session>> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, created_at, updated_at, model, parent_id, reverted_message_id \
             FROM sessions AS s \
             ORDER BY (SELECT MAX(rowid) FROM messages WHERE session_id = s.id) DESC, \
             s.rowid DESC LIMIT 1",
        )?;
        let mut rows = statement.query_map([], Session::from_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Fetch a session by id, or `None` if it does not exist.
    pub fn get(&self, id: &str) -> DbResult<Option<Session>> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, created_at, updated_at, model, parent_id, reverted_message_id \
             FROM sessions WHERE id = ?1",
        )?;
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

    /// Rename a session (set or clear its title), returning the updated record.
    ///
    /// The title is scrubbed of API-key-shaped secrets (replaced by the canonical
    /// [`redaction::PLACEHOLDER`]) before it is written, matching [`create`](Self::create), so a
    /// secret pasted into a rename never lands in the database file. Passing `None` clears the
    /// title. Errors with [`DbError::NotFound`] when no session has the given id.
    pub fn set_title(&self, id: &str, title: Option<&str>) -> DbResult<Session> {
        let title = title.map(redaction::redact);
        let affected = self.conn.execute(
            "UPDATE sessions SET title = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![id, title],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound {
                kind: "session",
                id: id.to_owned(),
            });
        }
        self.require(id)
    }

    /// Set (or clear) the model chosen for a session, returning the updated record.
    ///
    /// This is the durable seam behind the TUI `/models` picker: a model selected for a session is
    /// persisted here so a later headless `cortex run --session <id>` resume honors it without an
    /// explicit `--model`. Passing `None` clears the per-session choice (the configured/default
    /// model then governs). Errors with [`DbError::NotFound`] when no session has the given id.
    ///
    /// A model id is not secret, so it is stored verbatim (unlike a title, which is redacted).
    pub fn set_model(&self, id: &str, model: Option<&str>) -> DbResult<Session> {
        let affected = self.conn.execute(
            "UPDATE sessions SET model = ?2, updated_at = datetime('now') WHERE id = ?1",
            params![id, model],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound {
                kind: "session",
                id: id.to_owned(),
            });
        }
        self.require(id)
    }

    /// Delete a session and (via the `ON DELETE CASCADE` foreign key on `messages`) every message
    /// stored under it, returning whether a session row was removed.
    ///
    /// Deleting an unknown id is not an error: it returns `false` so a caller can treat the
    /// "already gone" case idempotently.
    pub fn delete(&self, id: &str) -> DbResult<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(affected > 0)
    }

    /// List sessions newest-first (by creation time, then id for a stable tiebreak).
    pub fn list(&self) -> DbResult<Vec<Session>> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, created_at, updated_at, model, parent_id, reverted_message_id \
             FROM sessions \
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = statement.query_map([], Session::from_row)?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    /// Create a new session forked from `parent_id`, with an optional title.
    ///
    /// The new session has its own opaque id and records `parent_id` so it is listable as a child
    /// of the parent. The caller is responsible for copying any transcript into the child. The
    /// title is scrubbed of API-key-shaped secrets before it is written, matching
    /// [`create`](Self::create).
    pub fn create_child(&self, parent_id: &str, title: Option<&str>) -> DbResult<Session> {
        let title = title.map(redaction::redact);
        let id = random_id(self.conn, "ses")?;
        let session = self.conn.query_row(
            "INSERT INTO sessions (id, title, parent_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, datetime('now'), datetime('now'))
             RETURNING id, title, created_at, updated_at, model, parent_id, reverted_message_id",
            params![id, title, parent_id],
            Session::from_row,
        )?;
        Ok(session)
    }

    /// List the child sessions forked from `parent_id`, newest-first.
    pub fn children(&self, parent_id: &str) -> DbResult<Vec<Session>> {
        let mut statement = self.conn.prepare(
            "SELECT id, title, created_at, updated_at, model, parent_id, reverted_message_id \
             FROM sessions WHERE parent_id = ?1 \
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = statement.query_map(params![parent_id], Session::from_row)?;
        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(row?);
        }
        Ok(sessions)
    }

    /// Set (or clear) the message id a session is reverted to, returning the updated record.
    ///
    /// Passing `None` clears the reverted state. A message id is not secret, so it is stored
    /// verbatim. Errors with [`DbError::NotFound`] when no session has the given id.
    pub fn set_reverted_message(&self, id: &str, message_id: Option<&str>) -> DbResult<Session> {
        let affected = self.conn.execute(
            "UPDATE sessions SET reverted_message_id = ?2, updated_at = datetime('now') \
             WHERE id = ?1",
            params![id, message_id],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound {
                kind: "session",
                id: id.to_owned(),
            });
        }
        self.require(id)
    }
}

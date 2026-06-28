//! The messages repository: ordered, content-bearing records stored under a session.

use std::collections::HashMap;

use protocol::MessagePart;
use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use crate::error::{DbError, DbResult};
use crate::id::{prefix, random_id};

/// A persisted message belonging to a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Stable, opaque identifier (prefixed `msg_`).
    pub id: String,
    /// The owning session's id.
    pub session_id: String,
    /// The message role (e.g. `user`, `assistant`, `system`).
    pub role: String,
    /// The message content, stored and retrieved verbatim.
    pub content: String,
    /// Rich typed parts persisted for this message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MessagePart>,
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
            parts: Vec::new(),
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
        let mut parts = Vec::new();
        if !content.trim().is_empty() {
            parts.push(MessagePart::text(content.to_owned()));
        }
        self.create_with_parts(session_id, role, content, &parts)
    }

    /// Append a message with explicit rich parts.
    ///
    /// `content` remains the compatibility text projection used by history replay surfaces, while
    /// `parts` preserves the typed representation for server/SSE/ACP/SDK-style consumers.
    pub fn create_with_parts(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
        parts: &[MessagePart],
    ) -> DbResult<Message> {
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
        let id = random_id(self.conn, prefix::MESSAGE)?;
        let mut message = self.conn.query_row(
            "INSERT INTO messages (id, session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             RETURNING id, session_id, role, content, created_at",
            params![id, session_id, role, content],
            Message::from_row,
        )?;
        self.insert_parts(session_id, &message.id, parts)?;
        message.parts = self.list_parts_for_message(&message.id)?;
        self.conn.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
            params![session_id],
        )?;
        Ok(message)
    }

    /// List the messages under a session in chronological order (oldest first).
    /// Ordering is by `created_at`, then by `rowid` so messages written within the same clock
    /// second (timestamps are second-granular) are returned in true insertion order rather than by
    /// the random hex id. This keeps a turn's `user` prompt ahead of its `assistant` reply when
    /// projecting durable history back into a conversation.
    pub fn list_for_session(&self, session_id: &str) -> DbResult<Vec<Message>> {
        let mut statement = self.conn.prepare(
            "SELECT id, session_id, role, content, created_at FROM messages \
             WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = statement.query_map(params![session_id], Message::from_row)?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row?);
        }
        let mut parts_by_message = self.list_parts_for_session(session_id)?;
        for message in &mut messages {
            message.parts = parts_by_message.remove(&message.id).unwrap_or_default();
        }
        Ok(messages)
    }

    /// Fetch one message (with its rich parts) by id, or `None` if it does not exist.
    pub fn get(&self, message_id: &str) -> DbResult<Option<Message>> {
        let mut statement = self.conn.prepare(
            "SELECT id, session_id, role, content, created_at FROM messages WHERE id = ?1",
        )?;
        let mut rows = statement.query_map(params![message_id], Message::from_row)?;
        match rows.next() {
            Some(row) => {
                let mut message = row?;
                message.parts = self.list_parts_for_message(&message.id)?;
                Ok(Some(message))
            }
            None => Ok(None),
        }
    }

    /// Fetch one message by id, erroring with [`DbError::NotFound`] if it is absent.
    pub fn require(&self, message_id: &str) -> DbResult<Message> {
        self.get(message_id)?.ok_or_else(|| DbError::NotFound {
            kind: "message",
            id: message_id.to_owned(),
        })
    }

    /// Delete one message and (via `ON DELETE CASCADE`) its parts, returning whether a row was
    /// removed. Deleting an unknown id is not an error: it returns `false`.
    pub fn delete(&self, message_id: &str) -> DbResult<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
        Ok(affected > 0)
    }

    /// Fetch one rich message part by its stable part id, or `None` if it does not exist.
    pub fn part(&self, part_id: &str) -> DbResult<Option<MessagePart>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, part_json, created_at FROM message_parts WHERE id = ?1")?;
        let mut rows = statement.query_map(params![part_id], |row| {
            let id: String = row.get("id")?;
            let part_json: String = row.get("part_json")?;
            let created_at: String = row.get("created_at")?;
            Ok((id, part_json, created_at))
        })?;
        match rows.next() {
            Some(row) => {
                let (id, part_json, created_at) = row?;
                let mut part: MessagePart =
                    serde_json::from_str(&part_json).map_err(|err| DbError::Operation {
                        message: format!("failed to deserialize message part {id}: {err}"),
                    })?;
                part.id = id;
                part.created_at = Some(created_at);
                Ok(Some(part))
            }
            None => Ok(None),
        }
    }

    /// Replace the stored body of one rich message part, preserving its id, ordinal, and creation
    /// timestamp. Returns the persisted part. Errors with [`DbError::NotFound`] when the part id is
    /// unknown.
    pub fn update_part(&self, part_id: &str, part: &MessagePart) -> DbResult<MessagePart> {
        let mut payload = part.clone();
        payload.id = part_id.to_owned();
        payload.created_at = None;
        let serialized = serde_json::to_string(&payload).map_err(|err| DbError::Operation {
            message: format!("failed to serialize message part: {err}"),
        })?;
        let redacted = redaction::redact(&serialized);
        let affected = self.conn.execute(
            "UPDATE message_parts SET part_json = ?2 WHERE id = ?1",
            params![part_id, redacted],
        )?;
        if affected == 0 {
            return Err(DbError::NotFound {
                kind: "message_part",
                id: part_id.to_owned(),
            });
        }
        self.part(part_id)?.ok_or_else(|| DbError::NotFound {
            kind: "message_part",
            id: part_id.to_owned(),
        })
    }

    /// Delete one rich message part by id, returning whether a row was removed. Deleting an unknown
    /// id is not an error: it returns `false`.
    pub fn delete_part(&self, part_id: &str) -> DbResult<bool> {
        let affected = self
            .conn
            .execute("DELETE FROM message_parts WHERE id = ?1", params![part_id])?;
        Ok(affected > 0)
    }

    fn insert_parts(
        &self,
        session_id: &str,
        message_id: &str,
        parts: &[MessagePart],
    ) -> DbResult<()> {
        for (ordinal, part) in parts.iter().enumerate() {
            let mut payload = part.clone();
            let part_id = if payload.id.trim().is_empty() {
                format!("{message_id}_p{:04}", ordinal + 1)
            } else {
                payload.id.clone()
            };
            let created_at = payload.created_at.clone();
            payload.id = part_id.clone();
            payload.created_at = None;
            let serialized = serde_json::to_string(&payload).map_err(|err| DbError::Operation {
                message: format!("failed to serialize message part: {err}"),
            })?;
            let redacted = redaction::redact(&serialized);
            self.conn.execute(
                "INSERT INTO message_parts (id, message_id, session_id, ordinal, part_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, COALESCE(?6, datetime('now')))",
                params![
                    part_id,
                    message_id,
                    session_id,
                    ordinal as i64,
                    redacted,
                    created_at
                ],
            )?;
        }
        Ok(())
    }

    fn list_parts_for_session(
        &self,
        session_id: &str,
    ) -> DbResult<HashMap<String, Vec<MessagePart>>> {
        let mut statement = self.conn.prepare(
            "SELECT id, message_id, part_json, created_at FROM message_parts \
             WHERE session_id = ?1 ORDER BY ordinal ASC, rowid ASC",
        )?;
        let rows = statement.query_map(params![session_id], |row| {
            let id: String = row.get("id")?;
            let message_id: String = row.get("message_id")?;
            let part_json: String = row.get("part_json")?;
            let created_at: String = row.get("created_at")?;
            Ok((id, message_id, part_json, created_at))
        })?;
        let mut grouped: HashMap<String, Vec<MessagePart>> = HashMap::new();
        for row in rows {
            let (id, message_id, part_json, created_at) = row?;
            let mut part: MessagePart =
                serde_json::from_str(&part_json).map_err(|err| DbError::Operation {
                    message: format!("failed to deserialize message part {id}: {err}"),
                })?;
            part.id = id;
            part.created_at = Some(created_at);
            grouped.entry(message_id).or_default().push(part);
        }
        Ok(grouped)
    }

    fn list_parts_for_message(&self, message_id: &str) -> DbResult<Vec<MessagePart>> {
        let mut statement = self.conn.prepare(
            "SELECT id, part_json, created_at FROM message_parts \
             WHERE message_id = ?1 ORDER BY ordinal ASC, rowid ASC",
        )?;
        let rows = statement.query_map(params![message_id], |row| {
            let id: String = row.get("id")?;
            let part_json: String = row.get("part_json")?;
            let created_at: String = row.get("created_at")?;
            Ok((id, part_json, created_at))
        })?;
        let mut parts = Vec::new();
        for row in rows {
            let (id, part_json, created_at) = row?;
            let mut part: MessagePart =
                serde_json::from_str(&part_json).map_err(|err| DbError::Operation {
                    message: format!("failed to deserialize message part {id}: {err}"),
                })?;
            part.id = id;
            part.created_at = Some(created_at);
            parts.push(part);
        }
        Ok(parts)
    }
}

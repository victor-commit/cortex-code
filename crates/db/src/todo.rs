//! The todos repository: a session's ordered todo list, replaced wholesale on each write.
//!
//! A `todo` tool call commits the complete list it wants the session to hold, so persistence is a
//! whole-list replace ([`TodoRepo::replace_for_session`]) rather than per-item mutation: the prior
//! rows are cleared and the new ones inserted in order inside one transaction. Reads
//! ([`TodoRepo::list_for_session`]) return the items in their stored order. Todos are scoped to
//! their session, so one session never sees another's list and a fresh session reports an empty
//! list.

use protocol::{TodoItem, TodoStatus};
use rusqlite::{Connection, Row, params};

use crate::error::{DbError, DbResult};
use crate::id::random_id;

/// Read/write access to the `todos` table over a borrowed connection.
pub struct TodoRepo<'a> {
    conn: &'a Connection,
}

impl<'a> TodoRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Replace a session's entire todo list with `items`, in order.
    ///
    /// The prior list is cleared and the supplied items are inserted (position 0..N) inside one
    /// transaction, so a concurrent reader sees either the old list or the new one, never a partial
    /// mix. Each item's content is scrubbed of API-key-shaped secrets before it is written. An
    /// empty `items` clears the list.
    ///
    /// # Errors
    /// Returns [`DbError::NotFound`] if the session does not exist, so the caller gets a clear
    /// diagnostic rather than an opaque foreign-key failure.
    pub fn replace_for_session(&self, session_id: &str, items: &[TodoItem]) -> DbResult<()> {
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

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> DbResult<()> {
            self.conn.execute(
                "DELETE FROM todos WHERE session_id = ?1",
                params![session_id],
            )?;

            for (position, item) in items.iter().enumerate() {
                let id = random_id(self.conn, "tod")?;
                let content = redaction::redact(&item.content);
                self.conn.execute(
                    "INSERT INTO todos (id, session_id, position, content, status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
                    params![
                        id,
                        session_id,
                        position as i64,
                        content,
                        item.status.as_str(),
                    ],
                )?;
            }
            self.conn.execute(
                "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
                params![session_id],
            )?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    /// List a session's todo items in their stored order (position ascending). A session with no
    /// todos returns an empty list (not an error).
    pub fn list_for_session(&self, session_id: &str) -> DbResult<Vec<TodoItem>> {
        let mut statement = self.conn.prepare(
            "SELECT content, status FROM todos WHERE session_id = ?1 ORDER BY position ASC, rowid ASC",
        )?;
        let rows = statement.query_map(params![session_id], todo_from_row)?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }
}

/// Build a [`TodoItem`] from a `(content, status)` row, defaulting an unrecognized stored status to
/// [`TodoStatus::Pending`] rather than failing the read.
fn todo_from_row(row: &Row<'_>) -> rusqlite::Result<TodoItem> {
    let content: String = row.get("content")?;
    let status: String = row.get("status")?;
    Ok(TodoItem::new(
        content,
        TodoStatus::parse(&status).unwrap_or(TodoStatus::Pending),
    ))
}

#[cfg(test)]
mod tests {
    use crate::Database;
    use protocol::{TodoItem, TodoStatus};

    fn item(content: &str, status: TodoStatus) -> TodoItem {
        TodoItem::new(content, status)
    }

    #[test]
    fn replace_then_list_returns_items_in_order() {
        let db = Database::open_in_memory().expect("open");
        let session = db.sessions().create(None).expect("session");

        let items = vec![
            item("first", TodoStatus::Pending),
            item("second", TodoStatus::InProgress),
            item("third", TodoStatus::Pending),
        ];
        db.todos()
            .replace_for_session(&session.id, &items)
            .expect("replace");

        let stored = db.todos().list_for_session(&session.id).expect("list");
        assert_eq!(stored, items);
    }

    #[test]
    fn replace_overwrites_the_prior_list_and_keeps_untouched_statuses() {
        let db = Database::open_in_memory().expect("open");
        let session = db.sessions().create(None).expect("session");

        let first = vec![
            item("a", TodoStatus::Pending),
            item("b", TodoStatus::Pending),
            item("c", TodoStatus::Pending),
        ];
        db.todos()
            .replace_for_session(&session.id, &first)
            .expect("replace");

        // The model marks item 1 completed and re-commits the full list; items 2-3 keep their
        // prior statuses.
        let updated = vec![
            item("a", TodoStatus::Completed),
            item("b", TodoStatus::Pending),
            item("c", TodoStatus::Pending),
        ];
        db.todos()
            .replace_for_session(&session.id, &updated)
            .expect("replace");

        let stored = db.todos().list_for_session(&session.id).expect("list");
        assert_eq!(stored.len(), 3);
        assert_eq!(stored[0].status, TodoStatus::Completed);
        assert_eq!(stored[1].status, TodoStatus::Pending);
        assert_eq!(stored[2].status, TodoStatus::Pending);
    }

    #[test]
    fn todos_are_isolated_per_session() {
        let db = Database::open_in_memory().expect("open");
        let session_a = db.sessions().create(None).expect("session a");
        let session_b = db.sessions().create(None).expect("session b");

        db.todos()
            .replace_for_session(&session_a.id, &[item("only in a", TodoStatus::Pending)])
            .expect("replace a");

        assert_eq!(
            db.todos()
                .list_for_session(&session_a.id)
                .expect("list a")
                .len(),
            1
        );
        assert!(
            db.todos()
                .list_for_session(&session_b.id)
                .expect("list b")
                .is_empty(),
            "a fresh session reports an empty todo list"
        );
    }

    #[test]
    fn empty_replace_clears_the_list() {
        let db = Database::open_in_memory().expect("open");
        let session = db.sessions().create(None).expect("session");
        db.todos()
            .replace_for_session(&session.id, &[item("x", TodoStatus::Pending)])
            .expect("replace");
        db.todos()
            .replace_for_session(&session.id, &[])
            .expect("clear");
        assert!(
            db.todos()
                .list_for_session(&session.id)
                .expect("list")
                .is_empty()
        );
    }

    #[test]
    fn replace_for_a_missing_session_is_a_clear_not_found() {
        let db = Database::open_in_memory().expect("open");
        let err = db
            .todos()
            .replace_for_session("ses_missing", &[item("x", TodoStatus::Pending)])
            .expect_err("missing session must error");
        assert!(matches!(
            err,
            crate::DbError::NotFound {
                kind: "session",
                ..
            }
        ));
    }

    #[test]
    fn todo_content_is_redacted_before_storage() {
        let db = Database::open_in_memory().expect("open");
        let session = db.sessions().create(None).expect("session");
        let secret = format!("sk-{}-{}", "or-v1", "a".repeat(48));
        db.todos()
            .replace_for_session(
                &session.id,
                &[item(&format!("use key {secret}"), TodoStatus::Pending)],
            )
            .expect("replace");
        let stored = db.todos().list_for_session(&session.id).expect("list");
        assert!(
            !stored[0].content.contains(&secret),
            "the raw secret must not be persisted"
        );
    }
}

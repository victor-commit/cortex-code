//! Bridge the `tools` crate's [`TodoStore`] seam to the `db` per-session todos repository.
//!
//! The `todo` tool persists its per-session list through the [`tools::TodoStore`] trait without
//! depending on SQLite; this adapter implements that trait over the durable [`db::Database`]. It
//! holds the resolved database path and opens a short-lived connection per operation, so the store
//! is `Send + Sync` (shared across the turn's tool calls) and writes land in the same on-disk
//! database the rest of the run uses, durable across separate `cortex run` invocations.

use std::path::PathBuf;

use db::Database;
use protocol::TodoItem;
use tools::TodoStore;

/// A [`TodoStore`] backed by the durable SQLite database at a resolved path.
pub struct DbTodoStore {
    path: PathBuf,
}

impl DbTodoStore {
    /// Build a store that reads and writes todos in the database at `path`.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Open a fresh connection to the backing database, mapping any failure to a message string.
    fn open(&self) -> Result<Database, String> {
        Database::open(&self.path).map_err(|err| err.to_string())
    }
}

impl TodoStore for DbTodoStore {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        self.open()?
            .todos()
            .list_for_session(session_id)
            .map_err(|err| err.to_string())
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        self.open()?
            .todos()
            .replace_for_session(session_id, items)
            .map_err(|err| err.to_string())
    }
}

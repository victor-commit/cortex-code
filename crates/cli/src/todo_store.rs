//! Bridge the `tools` crate's [`TodoStore`] seam to the `db` per-session todos repository.
//!
//! The `todo` tool persists its per-session list through the [`tools::TodoStore`] trait without
//! depending on SQLite; this adapter implements that trait over the durable [`db::Database`]. It
//! opens one connection up front and reuses it for every todo read/write in the session, avoiding
//! repeated open/migrate overhead on each tool call.

use std::path::PathBuf;
use std::sync::Mutex;

use db::Database;
use protocol::TodoItem;
use tools::TodoStore;

/// A [`TodoStore`] backed by the durable SQLite database at a resolved path.
pub struct DbTodoStore {
    db: Mutex<Result<Database, String>>,
}

impl DbTodoStore {
    /// Build a store that reads and writes todos in the database at `path`.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            db: Mutex::new(Database::open(&path).map_err(|err| err.to_string())),
        }
    }

    /// Borrow the shared connection for one repository operation.
    fn with_db<T>(&self, f: impl FnOnce(&Database) -> Result<T, String>) -> Result<T, String> {
        let guard = self
            .db
            .lock()
            .map_err(|_| "todo store connection lock poisoned".to_owned())?;
        match guard.as_ref() {
            Ok(db) => f(db),
            Err(err) => Err(err.clone()),
        }
    }
}

impl TodoStore for DbTodoStore {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        self.with_db(|db| {
            db.todos()
                .list_for_session(session_id)
                .map_err(|err| err.to_string())
        })
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        self.with_db(|db| {
            db.todos()
                .replace_for_session(session_id, items)
                .map_err(|err| err.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use protocol::{TodoItem, TodoStatus};

    use super::*;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-db-todo-store-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn list_and_replace_round_trip_through_the_shared_connection() {
        let dir = temp_dir("round-trip");
        let db_path = dir.join("cortex.db");
        let db = Database::open(&db_path).expect("open db");
        let session = db.sessions().create(None).expect("create session");
        drop(db);

        let store = DbTodoStore::new(db_path);
        let items = vec![
            TodoItem::new("first", TodoStatus::Pending),
            TodoItem::new("second", TodoStatus::InProgress),
        ];
        store.replace(&session.id, &items).expect("replace todos");
        assert_eq!(store.list(&session.id).expect("list todos"), items);

        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn operations_continue_after_db_file_is_unlinked_without_reopening() {
        let dir = temp_dir("reuse");
        let db_path = dir.join("cortex.db");
        let db = Database::open(&db_path).expect("open db");
        let session = db.sessions().create(None).expect("create session");
        drop(db);

        let store = DbTodoStore::new(db_path.clone());
        store
            .replace(&session.id, &[TodoItem::new("v1", TodoStatus::Pending)])
            .expect("initial replace");

        std::fs::remove_file(&db_path).expect("unlink database file");
        store
            .replace(&session.id, &[TodoItem::new("v2", TodoStatus::Completed)])
            .expect("replace through reused connection");
        let listed = store.list(&session.id).expect("list after unlink");
        assert_eq!(listed, vec![TodoItem::new("v2", TodoStatus::Completed)]);

        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}

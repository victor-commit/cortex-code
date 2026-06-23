//! SQLite (rusqlite, bundled) repositories and migrations for sessions and messages.
//!
//! [`Database::open`] opens (creating it and any parent directories if needed) the SQLite file
//! at a given path and applies all pending schema migrations automatically. Opening is:
//!
//! - **self-initializing** — a nonexistent path becomes a valid SQLite database with the full
//!   schema and a `schema_migrations` version table.
//! - **idempotent** — re-opening an already-migrated database neither re-runs nor fails
//!   migrations, and leaves existing rows intact.
//! - **concurrency-safe** — a busy timeout plus a serialized (`BEGIN IMMEDIATE`) migration
//!   transaction means two processes opening the same fresh path both succeed without
//!   corruption or double-applied migrations.
//!
//! Failures (an unwritable path, a non-SQLite junk file) surface as a typed [`DbError`] rather
//! than a panic, so the caller can emit a clear diagnostic and a non-zero exit.

mod error;
mod id;
mod message;
mod migrations;
mod path;
mod session;
mod todo;
mod usage;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;

pub use error::{DbError, DbResult};
pub use message::{Message, MessageRepo};
pub use migrations::{MIGRATIONS_TABLE, latest_version};
pub use path::{DB_FILE_NAME, DbPathInputs, ENV_DATA_DIR, ENV_DB_PATH, default_location_hint};
pub use session::{Session, SessionRepo};
pub use todo::TodoRepo;
pub use usage::{UsageRecord, UsageRepo, UsageTotals};

/// How long an open/migrate operation waits on a database lock before giving up, generous
/// enough to let a concurrent first-use migration commit first.
const BUSY_TIMEOUT: Duration = Duration::from_millis(15_000);

/// An open, migrated SQLite database, the entry point to the session and message repositories.
#[derive(Debug)]
pub struct Database {
    conn: Connection,
    path: PathBuf,
}

impl Database {
    /// Open (and, if absent, create and migrate) the database at `path`.
    ///
    /// # Errors
    /// Returns [`DbError::NotADatabase`] if the file is not a valid SQLite database,
    /// [`DbError::Open`] if the path is unwritable or its parent cannot be created, and
    /// [`DbError::Migrate`] if a migration fails.
    pub fn open(path: impl AsRef<Path>) -> DbResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|err| DbError::open_io(path, &err))?;
            }
        }

        let mut conn = Connection::open(path).map_err(|err| DbError::from_open(path, err))?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|err| DbError::from_open(path, err))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|err| DbError::from_open(path, err))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|err| DbError::from_open(path, err))?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(|err| DbError::from_open(path, err))?;

        migrations::apply(&mut conn).map_err(|err| DbError::from_open(path, err))?;

        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// Open an ephemeral in-memory database (migrated), for tests and transient use.
    ///
    /// # Errors
    /// Returns [`DbError::Migrate`] if migrations fail to apply.
    pub fn open_in_memory() -> DbResult<Self> {
        let mut conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", true)?;
        migrations::apply(&mut conn).map_err(|err| DbError::Migrate {
            message: err.to_string(),
        })?;
        Ok(Self {
            conn,
            path: PathBuf::from(":memory:"),
        })
    }

    /// The filesystem path this database was opened from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The currently applied schema version.
    ///
    /// # Errors
    /// Returns [`DbError`] if the version cannot be read.
    pub fn schema_version(&self) -> DbResult<i64> {
        Ok(migrations::current_version(&self.conn)?)
    }

    /// Run SQLite's `PRAGMA integrity_check` and return its single-row result.
    ///
    /// A healthy, uncorrupted database returns the literal `"ok"`; any other value enumerates the
    /// corruption found. This lets callers (and the durability tests) assert that the on-disk
    /// database survived concurrent writers or a hard kill (`SIGKILL`) without corruption.
    ///
    /// # Errors
    /// Returns [`DbError`] if the pragma cannot be evaluated.
    pub fn integrity_check(&self) -> DbResult<String> {
        let result = self
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))?;
        Ok(result)
    }

    /// The sessions repository.
    #[must_use]
    pub fn sessions(&self) -> SessionRepo<'_> {
        SessionRepo::new(&self.conn)
    }

    /// The messages repository.
    #[must_use]
    pub fn messages(&self) -> MessageRepo<'_> {
        MessageRepo::new(&self.conn)
    }

    /// The usage repository.
    #[must_use]
    pub fn usage(&self) -> UsageRepo<'_> {
        UsageRepo::new(&self.conn)
    }

    /// The todos repository.
    #[must_use]
    pub fn todos(&self) -> TodoRepo<'_> {
        TodoRepo::new(&self.conn)
    }
}

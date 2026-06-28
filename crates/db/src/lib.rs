//! SQLite (rusqlite, bundled) repositories and migrations for sessions and messages.
//!
//! [`Database::open`] opens (creating it and any parent directories if needed) the SQLite file
//! at a given path and applies all pending schema migrations automatically. Opening is:
//!
//! - **self-initializing** — a nonexistent path becomes a valid SQLite database with the full
//!   schema and a `schema_migrations` version table.
//! - **idempotent** — re-opening an already-migrated database neither re-runs nor fails
//!   migrations, and leaves existing rows intact.
//! - **concurrency-safe** — a busy timeout, a serialized (`BEGIN IMMEDIATE`) migration
//!   transaction, and a bounded open retry mean several processes opening the same fresh path all
//!   succeed without corruption or double-applied migrations, even though the one-time
//!   `journal_mode=WAL` switch needs a brief exclusive lock that SQLite does not route through the
//!   busy handler.
//!
//! Failures (an unwritable path, a non-SQLite junk file) surface as a typed [`DbError`] rather
//! than a panic, so the caller can emit a clear diagnostic and a non-zero exit.

mod account;
mod error;
mod id;
mod message;
mod migrations;
mod path;
mod project;
mod session;
mod todo;
mod usage;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::Connection;

pub use account::{Account, AccountRepo};
pub use error::{DbError, DbResult};
pub use id::prefix;
pub use message::{Message, MessageRepo};
pub use migrations::{MIGRATIONS_TABLE, latest_version};
pub use path::{DB_FILE_NAME, DbPathInputs, ENV_DATA_DIR, ENV_DB_PATH, default_location_hint};
pub use project::{Project, ProjectRepo};
pub use session::{Session, SessionRepo};
pub use todo::TodoRepo;
pub use usage::{UsageRecord, UsageRepo, UsageTotals};

/// How long an open/migrate operation waits on a database lock before giving up, generous
/// enough to let a concurrent first-use migration commit first.
const BUSY_TIMEOUT: Duration = Duration::from_millis(15_000);

/// Bounded wall-clock budget for retrying an open that hits transient busy/locked contention the
/// [`BUSY_TIMEOUT`] itself cannot absorb. The one-time `journal_mode=WAL` switch needs a brief
/// EXCLUSIVE lock that SQLite returns `SQLITE_BUSY`/`SQLITE_LOCKED` for *immediately*, without
/// invoking the busy handler, so several processes first-using the same fresh database race on it.
/// Past this budget a persistent busy surfaces as a clear [`DbError::Busy`] rather than a hang.
const OPEN_BUSY_RETRY_BUDGET: Duration = Duration::from_millis(8_000);

/// The initial backoff between open retries; it doubles per attempt up to [`OPEN_BUSY_RETRY_CAP`].
const OPEN_BUSY_RETRY_BASE_DELAY: Duration = Duration::from_millis(5);

/// The maximum backoff between open retries, so a long contention window stays responsive.
const OPEN_BUSY_RETRY_CAP: Duration = Duration::from_millis(200);

/// The backoff before the `attempt`-th open retry: an exponential ramp from
/// [`OPEN_BUSY_RETRY_BASE_DELAY`], capped at [`OPEN_BUSY_RETRY_CAP`], plus a small randomized jitter
/// so concurrent first-use openers do not retry in lockstep (a thundering herd on the exclusive
/// lock the WAL switch needs).
fn open_retry_backoff(attempt: u32) -> Duration {
    let base_ms = OPEN_BUSY_RETRY_BASE_DELAY.as_millis() as u64;
    let cap_ms = OPEN_BUSY_RETRY_CAP.as_millis() as u64;
    let exponent = attempt.saturating_sub(1).min(6);
    let scaled_ms = (base_ms << exponent).min(cap_ms);
    let jitter_ms = open_retry_jitter() % (scaled_ms / 2 + 1);
    Duration::from_millis(scaled_ms + jitter_ms)
}

/// A cheap, dependency-free jitter source: the subsecond nanoseconds of the wall clock, which
/// differ between concurrent openers and so desynchronize their retry wakeups without pulling in a
/// random-number generator.
fn open_retry_jitter() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| u64::from(elapsed.subsec_nanos()))
}

/// An open, migrated SQLite database, the entry point to the session and message repositories.
#[derive(Debug)]
pub struct Database {
    conn: Connection,
    path: PathBuf,
}

impl Database {
    /// Open (and, if absent, create and migrate) the database at `path`.
    ///
    /// Opening is idempotent and safe under concurrent first use: a transient `SQLITE_BUSY`/
    /// `SQLITE_LOCKED` from the one-time `journal_mode=WAL` switch (which SQLite does not route
    /// through the busy handler) is retried on a fresh connection within a bounded budget
    /// ([`OPEN_BUSY_RETRY_BUDGET`]). The retry only re-attempts the idempotent open/migrate path,
    /// so it never masks a lost write or partial data — a write that contends at runtime is still
    /// governed solely by the busy timeout. A busy condition that persists past the budget surfaces
    /// as a clear [`DbError::Busy`] rather than a hang.
    ///
    /// # Errors
    /// Returns [`DbError::NotADatabase`] if the file is not a valid SQLite database,
    /// [`DbError::Open`] if the path is unwritable or its parent cannot be created,
    /// [`DbError::Migrate`] if a migration fails, and [`DbError::Busy`] if lock contention does not
    /// clear within the bounded retry budget.
    pub fn open(path: impl AsRef<Path>) -> DbResult<Self> {
        Self::open_tuned(path.as_ref(), BUSY_TIMEOUT, OPEN_BUSY_RETRY_BUDGET)
    }

    /// [`open`](Self::open) with an explicit per-attempt busy timeout and overall retry budget, so
    /// tests can prove the bounded failure path (a persistent busy surfaces as [`DbError::Busy`]
    /// rather than hanging) without waiting out the full production windows. A single attempt can
    /// itself block for up to `busy_timeout` on a long-held lock, so the worst-case open time is
    /// roughly `busy_timeout + retry_budget`; both are bounded.
    fn open_tuned(path: &Path, busy_timeout: Duration, retry_budget: Duration) -> DbResult<Self> {
        let start = Instant::now();
        let mut attempt: u32 = 0;
        loop {
            match Self::open_once(path, busy_timeout) {
                Ok(db) => return Ok(db),
                Err(DbError::Busy { .. }) if start.elapsed() < retry_budget => {
                    attempt += 1;
                    std::thread::sleep(open_retry_backoff(attempt));
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// A single open attempt: create the parent directory if needed, open the connection, apply the
    /// connection pragmas, and run any pending migrations. The whole sequence is idempotent, so the
    /// retry in [`open_tuned`](Self::open_tuned) can safely re-run it on a brand-new connection.
    fn open_once(path: &Path, busy_timeout: Duration) -> DbResult<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|err| DbError::open_io(path, &err))?;
            }
        }

        let mut conn = Connection::open(path).map_err(|err| DbError::from_open(path, err))?;
        conn.busy_timeout(busy_timeout)
            .map_err(|err| DbError::from_open(path, err))?;

        // WAL is a persistent property of the database file: once any connection establishes it,
        // every later open reads it back, so the switch (and its contended exclusive lock) is only
        // ever taken on a truly fresh database. Re-issuing it would force the same exclusive lock
        // on every concurrent open and needlessly multiply the first-use contention.
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(|err| DbError::from_open(path, err))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            conn.pragma_update(None, "journal_mode", "WAL")
                .map_err(|err| DbError::from_open(path, err))?;
        }

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

    /// Open an existing database read-only, for diagnostics that must not mutate it.
    ///
    /// Unlike [`open`](Self::open) this neither creates the file nor runs migrations nor switches
    /// the journal mode, so it issues no writes and leaves the on-disk database (and its checksum)
    /// untouched. It is the backing open for `cortex debug db`.
    ///
    /// # Errors
    /// Returns [`DbError`] if the database does not exist or cannot be opened read-only.
    pub fn open_read_only(path: impl AsRef<Path>) -> DbResult<Self> {
        let path = path.as_ref();
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)
            .map_err(|err| DbError::from_open(path, err))?;
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|err| DbError::from_open(path, err))?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
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

    /// Read-only row counts for the core tables, for diagnostics (`cortex debug db`).
    ///
    /// Issues only `SELECT count(*)` queries and never writes, so it does not change the database
    /// (its file checksum is unaffected on an already-migrated database).
    ///
    /// # Errors
    /// Returns [`DbError`] if a count query cannot be evaluated.
    pub fn table_counts(&self) -> DbResult<Vec<(String, i64)>> {
        const TABLES: &[&str] = &["sessions", "messages", "message_parts", "todos", "usage"];
        let mut counts = Vec::with_capacity(TABLES.len());
        for table in TABLES {
            let count: i64 =
                self.conn
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                        row.get(0)
                    })?;
            counts.push(((*table).to_owned(), count));
        }
        Ok(counts)
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

    /// The projects repository.
    #[must_use]
    pub fn projects(&self) -> ProjectRepo<'_> {
        ProjectRepo::new(&self.conn)
    }

    /// The accounts repository.
    #[must_use]
    pub fn accounts(&self) -> AccountRepo<'_> {
        AccountRepo::new(&self.conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The backoff stays bounded: it never exceeds the cap plus its (half-cap) jitter ceiling, no
    // matter how many attempts elapse, so a long contention window cannot produce an unbounded
    // sleep.
    #[test]
    fn open_retry_backoff_is_bounded_and_grows() {
        let max = OPEN_BUSY_RETRY_CAP + OPEN_BUSY_RETRY_CAP / 2;
        let mut last = Duration::ZERO;
        for attempt in 1..=64 {
            let delay = open_retry_backoff(attempt);
            assert!(
                delay <= max,
                "attempt {attempt} backoff {delay:?} exceeded the bound {max:?}"
            );
            // The non-jitter ramp is non-decreasing up to the cap; sample the first few attempts.
            if attempt <= 6 {
                assert!(
                    delay >= last || delay >= OPEN_BUSY_RETRY_CAP,
                    "attempt {attempt} backoff {delay:?} regressed below {last:?}"
                );
            }
            last = delay;
        }
    }

    // A busy condition that never clears must surface as a clear `Busy` within a bounded window —
    // proving the policy is bounded and fails fast rather than hanging. A blocker connection holds
    // an EXCLUSIVE write lock on a fresh, NON-WAL database, so every open attempt's pragmas/WAL
    // switch keep losing the lock until the per-attempt busy timeout elapses; the bounded retry
    // budget then gives up with a clear error. Small tuned windows keep the test fast.
    #[test]
    fn open_surfaces_persistent_busy_within_a_bounded_window() {
        let dir = std::env::temp_dir().join(format!(
            "cortex-db-busy-bound-{}-{}",
            std::process::id(),
            open_retry_jitter()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("cortex.db");

        // Create the file in default (delete) journal mode and hold a write transaction open: the
        // EXCLUSIVE lock persists for the lifetime of `blocker`, so no other connection can take
        // the lock its pragmas/migration need.
        let blocker = Connection::open(&path).expect("open blocker");
        blocker
            .busy_timeout(Duration::from_millis(50))
            .expect("blocker busy timeout");
        blocker
            .execute_batch("BEGIN EXCLUSIVE")
            .expect("hold exclusive lock");

        let busy_timeout = Duration::from_millis(40);
        let retry_budget = Duration::from_millis(150);
        let start = Instant::now();
        let result = Database::open_tuned(&path, busy_timeout, retry_budget);
        let elapsed = start.elapsed();

        assert!(
            matches!(result, Err(DbError::Busy { .. })),
            "persistent lock contention must surface as a clear Busy error, got: {result:?}"
        );
        // Worst case is roughly one final busy_timeout after the budget elapses; allow generous
        // slack but assert it cannot hang.
        let ceiling = busy_timeout + retry_budget * 4;
        assert!(
            elapsed < ceiling,
            "the bounded retry must fail fast, not hang (elapsed {elapsed:?}, ceiling {ceiling:?})"
        );

        drop(blocker);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn table_counts_report_core_tables_and_are_read_only() {
        let db = Database::open_in_memory().expect("memory db");
        let session = db.sessions().create(None).expect("create session");
        db.messages()
            .create(&session.id, "user", "hi")
            .expect("create message");

        let counts = db.table_counts().expect("counts");
        let map: std::collections::HashMap<&str, i64> =
            counts.iter().map(|(name, n)| (name.as_str(), *n)).collect();
        assert_eq!(map.get("sessions"), Some(&1));
        assert_eq!(map.get("messages"), Some(&1));
        assert!(map.contains_key("message_parts"));
        assert!(map.contains_key("todos"));
        assert!(map.contains_key("usage"));

        // A second call returns identical counts: counting does not mutate the tables.
        let again = db.table_counts().expect("counts again");
        assert_eq!(counts, again);
    }
}

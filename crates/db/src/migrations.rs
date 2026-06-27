//! Explicit, ordered schema migrations applied inside a single serialized transaction.
//!
//! Migrations are tracked in a `schema_migrations` table. Application runs inside a
//! `BEGIN IMMEDIATE` transaction so that, combined with a busy timeout on the connection,
//! two processes opening the same fresh database serialize: the first applies the schema and
//! commits; the second waits, re-reads the now-current version, and finds nothing to do. This
//! makes migrations idempotent across restarts and safe under concurrent first-use.

use rusqlite::{Connection, TransactionBehavior, params};

/// The migration/version table that records which migrations have been applied.
pub const MIGRATIONS_TABLE: &str = "schema_migrations";

/// An ordered migration: a monotonically increasing version and the SQL that establishes it.
struct Migration {
    version: i64,
    sql: &'static str,
}

/// The ordered list of migrations. Each runs exactly once, in version order. Append new
/// migrations with the next version; never edit or renumber a shipped migration.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("migrations/0002_usage.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("migrations/0003_todos.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("migrations/0004_session_model.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("migrations/0005_message_parts.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("migrations/0006_todo_priority.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("migrations/0007_session_parent_revert.sql"),
    },
];

/// The highest migration version known to this build.
#[must_use]
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map_or(0, |migration| migration.version)
}

/// Apply every pending migration to `conn`, leaving the database at [`latest_version`].
///
/// Idempotent: re-running against an already-migrated database is a no-op that neither re-runs
/// nor fails a migration. Concurrency-safe when the connection has a busy timeout set.
pub fn apply(conn: &mut Connection) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (
            version    INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );"
    ))?;

    let current: i64 = tx.query_row(
        &format!("SELECT COALESCE(MAX(version), 0) FROM {MIGRATIONS_TABLE}"),
        [],
        |row| row.get(0),
    )?;

    for migration in MIGRATIONS {
        if migration.version > current {
            tx.execute_batch(migration.sql)?;
            tx.execute(
                &format!(
                    "INSERT INTO {MIGRATIONS_TABLE} (version, applied_at) \
                     VALUES (?1, datetime('now'))"
                ),
                params![migration.version],
            )?;
        }
    }

    tx.commit()
}

/// The currently applied schema version (the max recorded migration, or `0` if none).
pub fn current_version(conn: &Connection) -> rusqlite::Result<i64> {
    let table_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        params![MIGRATIONS_TABLE],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(0);
    }
    conn.query_row(
        &format!("SELECT COALESCE(MAX(version), 0) FROM {MIGRATIONS_TABLE}"),
        [],
        |row| row.get(0),
    )
}

//! The persistence error type and its mapping from raw `rusqlite` failures.

use std::fmt;
use std::path::{Path, PathBuf};

/// A persistence-layer failure, classified so callers can render a clear, actionable
/// diagnostic without leaking a raw backtrace.
#[derive(Debug)]
pub enum DbError {
    /// The file at this path exists but is not a valid SQLite database (e.g. junk bytes).
    NotADatabase { path: PathBuf },
    /// The database could not be opened or created: the path is unwritable, a parent
    /// directory cannot be created, or the file is locked beyond the retry window.
    Open { path: PathBuf, message: String },
    /// A schema migration failed to apply.
    Migrate { message: String },
    /// A read/write against an open database failed.
    Operation { message: String },
    /// The database (or a table it needs) is held by another connection and the busy timeout
    /// elapsed before the lock cleared. Distinguished from a generic [`DbError::Operation`] so a
    /// caller can surface a clear "busy, retry shortly" diagnostic for honest write contention
    /// (e.g. two `cortex run` processes on the same session) rather than a raw lock error.
    Busy { message: String },
    /// A referenced entity does not exist (e.g. a message stored under an unknown session).
    NotFound { kind: &'static str, id: String },
}

impl DbError {
    /// Build an [`DbError::Open`] from an I/O failure while preparing the DB path's parent.
    pub(crate) fn open_io(path: &Path, source: &std::io::Error) -> Self {
        Self::Open {
            path: path.to_path_buf(),
            message: source.to_string(),
        }
    }

    /// Classify a `rusqlite` failure observed while opening or migrating the database at
    /// `path`. A "not a database" code becomes [`DbError::NotADatabase`]; a busy/locked code
    /// becomes [`DbError::Busy`]; an open/access code becomes [`DbError::Open`]; anything else is
    /// treated as a migration failure.
    pub(crate) fn from_open(path: &Path, source: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(failure, _) = &source {
            match failure.code {
                rusqlite::ErrorCode::NotADatabase => {
                    return Self::NotADatabase {
                        path: path.to_path_buf(),
                    };
                }
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                    return Self::Busy {
                        message: source.to_string(),
                    };
                }
                rusqlite::ErrorCode::CannotOpen
                | rusqlite::ErrorCode::PermissionDenied
                | rusqlite::ErrorCode::ReadOnly => {
                    return Self::Open {
                        path: path.to_path_buf(),
                        message: source.to_string(),
                    };
                }
                _ => {}
            }
        }
        Self::Migrate {
            message: source.to_string(),
        }
    }
}

/// Whether a raw `rusqlite` failure is a busy/locked contention error (`SQLITE_BUSY` /
/// `SQLITE_LOCKED`), which the busy timeout could not resolve within its window.
fn is_busy_or_locked(source: &rusqlite::Error) -> bool {
    matches!(
        source,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADatabase { path } => write!(
                formatter,
                "the file at {} is not a valid SQLite database",
                path.display()
            ),
            Self::Open { path, message } => {
                write!(
                    formatter,
                    "cannot open database at {}: {message}",
                    path.display()
                )
            }
            Self::Migrate { message } => write!(formatter, "schema migration failed: {message}"),
            Self::Operation { message } => {
                write!(formatter, "database operation failed: {message}")
            }
            Self::Busy { .. } => write!(
                formatter,
                "the database is busy: another cortex process is holding the session; \
                 retry in a moment"
            ),
            Self::NotFound { kind, id } => write!(formatter, "no such {kind}: {id}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(source: rusqlite::Error) -> Self {
        if is_busy_or_locked(&source) {
            Self::Busy {
                message: source.to_string(),
            }
        } else {
            Self::Operation {
                message: source.to_string(),
            }
        }
    }
}

/// A `Result` specialized to [`DbError`].
pub type DbResult<T> = Result<T, DbError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The `SQLITE_BUSY` primary result code.
    const SQLITE_BUSY: std::os::raw::c_int = 5;
    /// The `SQLITE_LOCKED` primary result code.
    const SQLITE_LOCKED: std::os::raw::c_int = 6;
    /// The `SQLITE_CONSTRAINT` primary result code (a non-contention operation failure).
    const SQLITE_CONSTRAINT: std::os::raw::c_int = 19;

    fn sqlite_failure(primary_code: std::os::raw::c_int) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(primary_code),
            Some("database is locked".to_owned()),
        )
    }

    // A busy/locked contention failure is classified as `Busy`, not a generic `Operation`, so the
    // caller can render a clear "retry shortly" diagnostic for honest write contention rather than
    // a raw lock error. The rendered message deliberately does not echo "database is locked".
    #[test]
    fn busy_and_locked_codes_classify_as_busy() {
        for code in [SQLITE_BUSY, SQLITE_LOCKED] {
            let err = DbError::from(sqlite_failure(code));
            assert!(
                matches!(err, DbError::Busy { .. }),
                "code {code} must classify as Busy, got: {err:?}"
            );
            let rendered = err.to_string();
            assert!(
                rendered.contains("busy") && rendered.contains("retry"),
                "the busy diagnostic must read as a transient, retryable condition: {rendered}"
            );
            assert!(
                !rendered.contains("database is locked"),
                "the busy diagnostic must not surface the raw lock error text: {rendered}"
            );
        }
    }

    // A non-contention SQLite failure (e.g. a constraint violation) stays a generic `Operation`
    // error, so the `Busy` class is reserved strictly for lock contention.
    #[test]
    fn non_contention_failures_stay_operation_errors() {
        let err = DbError::from(sqlite_failure(SQLITE_CONSTRAINT));
        assert!(
            matches!(err, DbError::Operation { .. }),
            "a constraint failure must not be misclassified as Busy, got: {err:?}"
        );
    }

    // The open-time classifier also routes a busy/locked code to `Busy` (a concurrent first-use
    // migration that out-waited the busy timeout), while a genuine cannot-open stays `Open`.
    #[test]
    fn open_time_busy_is_classified_as_busy() {
        let busy = DbError::from_open(Path::new("/tmp/x.db"), sqlite_failure(SQLITE_BUSY));
        assert!(matches!(busy, DbError::Busy { .. }), "got: {busy:?}");
    }
}

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
    /// `path`. A "not a database" code becomes [`DbError::NotADatabase`]; an open/access/lock
    /// code becomes [`DbError::Open`]; anything else is treated as a migration failure.
    pub(crate) fn from_open(path: &Path, source: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(failure, _) = &source {
            match failure.code {
                rusqlite::ErrorCode::NotADatabase => {
                    return Self::NotADatabase {
                        path: path.to_path_buf(),
                    };
                }
                rusqlite::ErrorCode::CannotOpen
                | rusqlite::ErrorCode::PermissionDenied
                | rusqlite::ErrorCode::ReadOnly
                | rusqlite::ErrorCode::DatabaseBusy
                | rusqlite::ErrorCode::DatabaseLocked => {
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
            Self::NotFound { kind, id } => write!(formatter, "no such {kind}: {id}"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(source: rusqlite::Error) -> Self {
        Self::Operation {
            message: source.to_string(),
        }
    }
}

/// A `Result` specialized to [`DbError`].
pub type DbResult<T> = Result<T, DbError>;

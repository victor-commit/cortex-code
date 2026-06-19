//! Opaque identifier generation backed by SQLite's CSPRNG.
//!
//! Using `randomblob` keeps id generation dependency-free and collision-resistant (128 bits of
//! entropy) without pulling in a separate RNG crate.

use rusqlite::Connection;

use crate::error::DbResult;

/// Generate a prefixed, 128-bit random hex identifier (e.g. `ses_<32 hex chars>`).
pub(crate) fn random_id(conn: &Connection, prefix: &str) -> DbResult<String> {
    let hex: String = conn.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?;
    Ok(format!("{prefix}_{hex}"))
}

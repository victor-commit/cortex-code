//! The usage repository: per-turn token usage and cost recorded under a session, plus the
//! session-level cumulative totals derived by summing every recorded turn.
//!
//! A turn's usage is appended with [`UsageRepo::record`]; the session-wide figure consumers report
//! is [`UsageRepo::session_totals`], which sums every recorded turn so the cumulative value grows
//! monotonically across turns and is never reset to a single turn's value.

use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use crate::error::{DbError, DbResult};
use crate::id::{prefix, random_id};

/// A persisted per-turn usage record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Stable, opaque identifier (prefixed `usg_`).
    pub id: String,
    /// The owning session's id.
    pub session_id: String,
    /// Prompt/input tokens consumed by the turn.
    pub input_tokens: u64,
    /// Completion/output tokens produced by the turn.
    pub output_tokens: u64,
    /// Total tokens (input + output) reported for the turn.
    pub total_tokens: u64,
    /// Monetary cost of the turn when the provider reported it.
    pub cost: Option<f64>,
    /// Creation timestamp (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
}

impl UsageRecord {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            session_id: row.get("session_id")?,
            input_tokens: row.get::<_, i64>("input_tokens")? as u64,
            output_tokens: row.get::<_, i64>("output_tokens")? as u64,
            total_tokens: row.get::<_, i64>("total_tokens")? as u64,
            cost: row.get("cost")?,
            created_at: row.get("created_at")?,
        })
    }
}

/// Cumulative usage across every recorded turn in a session.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UsageTotals {
    /// Sum of every recorded turn's input tokens.
    pub input_tokens: u64,
    /// Sum of every recorded turn's output tokens.
    pub output_tokens: u64,
    /// Sum of every recorded turn's total tokens.
    pub total_tokens: u64,
    /// Sum of every recorded turn's cost, or `None` when no turn reported a cost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// Read/write access to the `usage` table over a borrowed connection.
pub struct UsageRepo<'a> {
    conn: &'a Connection,
}

impl<'a> UsageRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Record a turn's token usage (and optional cost) under a session, returning the persisted
    /// record.
    ///
    /// Errors with [`DbError::NotFound`] if the session does not exist, so the caller gets a clear
    /// diagnostic rather than an opaque foreign-key constraint failure.
    pub fn record(
        &self,
        session_id: &str,
        input_tokens: u64,
        output_tokens: u64,
        total_tokens: u64,
        cost: Option<f64>,
    ) -> DbResult<UsageRecord> {
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

        let id = random_id(self.conn, prefix::USAGE)?;
        let record = self.conn.query_row(
            "INSERT INTO usage \
               (id, session_id, input_tokens, output_tokens, total_tokens, cost, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
             RETURNING id, session_id, input_tokens, output_tokens, total_tokens, cost, created_at",
            params![
                id,
                session_id,
                input_tokens as i64,
                output_tokens as i64,
                total_tokens as i64,
                cost,
            ],
            UsageRecord::from_row,
        )?;
        Ok(record)
    }

    /// The session's cumulative usage across every recorded turn.
    ///
    /// Summing all recorded turns means the total grows monotonically as turns accumulate and is
    /// never reset to a single turn's value. A session with no recorded usage yields all-zero token
    /// counts and a `None` cost.
    pub fn session_totals(&self, session_id: &str) -> DbResult<UsageTotals> {
        let totals = self.conn.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0), \
                    COALESCE(SUM(total_tokens), 0), SUM(cost) \
             FROM usage WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok(UsageTotals {
                    input_tokens: row.get::<_, i64>(0)? as u64,
                    output_tokens: row.get::<_, i64>(1)? as u64,
                    total_tokens: row.get::<_, i64>(2)? as u64,
                    cost: row.get::<_, Option<f64>>(3)?,
                })
            },
        )?;
        Ok(totals)
    }
}

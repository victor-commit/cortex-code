//! The accounts repository: durable mapped credential records for provider/plugin auth.
//!
//! An [`Account`] records the *existence and metadata* of a provider/plugin auth identity (its
//! provider id, a label, the auth method, and the resolution source) so the set of authenticated
//! providers is durable and inspectable across restarts. It deliberately holds **no raw secret**:
//! the 0600 auth store (`providers::store_credential`) remains the sole sink for the actual key, so
//! the database file never contains a credential. Metadata is redacted defensively before it is
//! written, matching the persistence redaction invariant for every other repository.

use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use crate::error::{DbError, DbResult};
use crate::id::{prefix, random_id};

/// A persisted account: provider/plugin auth metadata, never a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Stable, opaque identifier (prefixed `acc_`).
    pub id: String,
    /// The provider/plugin id this account authenticates (unique).
    pub provider: String,
    /// Optional human-facing label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The auth method used (e.g. `api`, `oauth`, or a plugin method id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    /// How the credential was supplied (e.g. `flag`, `env:NAME`, `stdin`, `plugin:<id>:<method>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Creation timestamp (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// Last-update timestamp (UTC), refreshed when the credential is re-stored.
    pub updated_at: String,
}

/// Read/write access to the `accounts` table over a borrowed connection.
pub struct AccountRepo<'a> {
    conn: &'a Connection,
}

impl<'a> AccountRepo<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Record (or refresh) the account for `provider`, returning the persisted record.
    ///
    /// Keyed by provider: re-authenticating an existing provider updates its metadata and
    /// `updated_at` while preserving the original `created_at` and id. All metadata is scrubbed of
    /// API-key-shaped secrets before it is written, so the database file never gains a raw key.
    pub fn upsert(
        &self,
        provider: &str,
        label: Option<&str>,
        auth_method: Option<&str>,
        source: Option<&str>,
    ) -> DbResult<Account> {
        let provider = redaction::redact(provider);
        let label = label.map(redaction::redact);
        let auth_method = auth_method.map(redaction::redact);
        let source = source.map(redaction::redact);
        let id = random_id(self.conn, prefix::ACCOUNT)?;
        self.conn.execute(
            "INSERT INTO accounts (id, provider, label, auth_method, source, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))
             ON CONFLICT(provider) DO UPDATE SET
                 label = excluded.label,
                 auth_method = excluded.auth_method,
                 source = excluded.source,
                 updated_at = datetime('now')",
            params![id, provider, label, auth_method, source],
        )?;
        self.get_by_provider(&provider)?.ok_or(DbError::NotFound {
            kind: "account",
            id: provider,
        })
    }

    /// Fetch the account for a provider, or `None` if none is recorded.
    pub fn get_by_provider(&self, provider: &str) -> DbResult<Option<Account>> {
        let mut statement = self.conn.prepare(
            "SELECT id, provider, label, auth_method, source, created_at, updated_at \
             FROM accounts WHERE provider = ?1",
        )?;
        let mut rows = statement.query_map(params![provider], account_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// List every recorded account, ordered by provider.
    pub fn list(&self) -> DbResult<Vec<Account>> {
        let mut statement = self.conn.prepare(
            "SELECT id, provider, label, auth_method, source, created_at, updated_at \
             FROM accounts ORDER BY provider ASC",
        )?;
        let rows = statement.query_map([], account_row)?;
        let mut accounts = Vec::new();
        for row in rows {
            accounts.push(row?);
        }
        Ok(accounts)
    }

    /// Remove the account for a provider, returning whether a row was removed. Removing an absent
    /// provider is a clean `false`.
    pub fn remove(&self, provider: &str) -> DbResult<bool> {
        let affected = self.conn.execute(
            "DELETE FROM accounts WHERE provider = ?1",
            params![provider],
        )?;
        Ok(affected > 0)
    }
}

fn account_row(row: &Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get("id")?,
        provider: row.get("provider")?,
        label: row.get("label")?,
        auth_method: row.get("auth_method")?,
        source: row.get("source")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

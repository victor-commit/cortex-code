//! Best-effort durable account records that mirror the local credential store.
//!
//! The 0600 auth store (`providers::store_credential`) remains the sole sink for the actual secret;
//! an [`db::Account`] holds only provider/account *metadata* (provider id, label, auth method,
//! resolution source) so the set of authenticated providers is durable and inspectable across
//! restarts. Recording is best-effort: a database that cannot be opened degrades to no record
//! rather than failing an otherwise-successful login, because the credential store is the source of
//! truth either way.

use db::{Account, Database, DbPathInputs};
use serde::Serialize;

fn open() -> Option<Database> {
    let path = DbPathInputs::from_process().resolve()?;
    Database::open(&path).ok()
}

/// Record (or refresh) the durable account record for a provider. Best-effort.
pub fn record_account(provider: &str, label: Option<&str>, method: Option<&str>, source: &str) {
    if let Some(db) = open() {
        let _ = db.accounts().upsert(provider, label, method, Some(source));
    }
}

/// Forget the durable account record for a provider, mirroring credential removal. Best-effort.
pub fn forget_account(provider: &str) {
    if let Some(db) = open() {
        let _ = db.accounts().remove(provider);
    }
}

/// List the durable account records as redacted output documents. Best-effort (empty on failure).
pub fn list_accounts() -> Vec<AccountDocument> {
    let Some(db) = open() else {
        return Vec::new();
    };
    db.accounts()
        .list()
        .unwrap_or_default()
        .into_iter()
        .map(AccountDocument::from_account)
        .collect()
}

/// A durable account record for output, with every field redacted at the boundary.
#[derive(Serialize)]
pub struct AccountDocument {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl AccountDocument {
    fn from_account(account: Account) -> Self {
        Self {
            provider: redaction::redact(&account.provider),
            label: account.label.as_deref().map(redaction::redact),
            method: account.auth_method.as_deref().map(redaction::redact),
            source: account.source.as_deref().map(redaction::redact),
            created_at: account.created_at,
            updated_at: account.updated_at,
        }
    }
}

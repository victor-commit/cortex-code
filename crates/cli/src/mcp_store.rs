//! Process-resolved MCP OAuth credential-store location.
//!
//! MCP remote OAuth tokens are intentionally stored separately from provider API keys. The provider
//! auth file (`auth.json`) stores simple API keys, while MCP OAuth credentials include access,
//! refresh, expiry, token endpoint, and client id. This module places the MCP store alongside the
//! other Cortex data files and lets [`mcp::CredentialStore`] enforce the required `0600` mode.

use std::path::PathBuf;
use std::sync::Arc;

/// The MCP OAuth token store filename inside the resolved Cortex data directory.
pub(crate) const MCP_CREDENTIAL_FILE_NAME: &str = "mcp-oauth.json";

/// Resolve the MCP OAuth credential-store path from the process environment.
///
/// The path follows the data-layer precedence via [`db::DbPathInputs`]: an explicit
/// `CORTEX_DB_PATH` selects that file's directory, `CORTEX_DATA_DIR` selects that directory, and
/// otherwise the default data home is used. Returns `None` only when the data layer itself cannot be
/// resolved because neither an override nor HOME/XDG data home is present.
#[must_use]
pub(crate) fn path_from_process() -> Option<PathBuf> {
    db::DbPathInputs::from_process()
        .resolve()
        .map(|db_path| db_path.with_file_name(MCP_CREDENTIAL_FILE_NAME))
}

/// Build a process-resolved MCP OAuth credential store, if a data location can be determined.
#[must_use]
pub(crate) fn store_from_process() -> Option<Arc<mcp::CredentialStore>> {
    path_from_process().map(|path| Arc::new(mcp::CredentialStore::at(path)))
}

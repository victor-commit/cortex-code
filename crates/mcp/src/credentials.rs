//! A `0600` on-disk store for MCP OAuth tokens, keyed by server name.
//!
//! Tokens obtained through the OAuth flow are persisted here so a later connect reuses them without
//! re-prompting, and refreshes them in place on expiry. The file is created and kept at mode `0600`
//! (owner read/write only) so the secrets are never world-readable, and the token values are never
//! logged or echoed — the store file is the single place they live.

use std::collections::BTreeMap;
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One persisted OAuth credential for a configured MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredToken {
    /// The bearer access token sent as `Authorization: Bearer <token>`.
    pub access_token: String,
    /// The refresh token used to mint a new access token when the current one expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// The token type (typically `Bearer`).
    #[serde(default = "default_token_type")]
    pub token_type: String,
    /// The absolute unix-seconds expiry, when the server reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// The token endpoint used to refresh this credential.
    pub token_endpoint: String,
    /// The OAuth client id this credential was issued to.
    pub client_id: String,
}

fn default_token_type() -> String {
    "Bearer".to_owned()
}

impl StoredToken {
    /// Whether the access token is expired (or expires within `skew_secs`), so it must be refreshed
    /// before use. A credential without an expiry never reports expired.
    #[must_use]
    pub fn is_expired(&self, skew_secs: u64) -> bool {
        match self.expires_at {
            Some(expires_at) => now_secs().saturating_add(skew_secs) >= expires_at,
            None => false,
        }
    }
}

/// The current unix time in seconds.
#[must_use]
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// A `0600` JSON token store on disk.
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    /// A store backed by the file at `path` (created on first write).
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// The backing file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load every stored credential, or an empty map when the file does not yet exist.
    ///
    /// # Errors
    /// Returns an error when the file exists but cannot be read or parsed.
    pub fn load(&self) -> io::Result<BTreeMap<String, StoredToken>> {
        match std::fs::read_to_string(&self.path) {
            Ok(body) if body.trim().is_empty() => Ok(BTreeMap::new()),
            Ok(body) => serde_json::from_str(&body)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(err) => Err(err),
        }
    }

    /// The stored credential for `server`, if present.
    ///
    /// # Errors
    /// Returns an error when the store cannot be read or parsed.
    pub fn get(&self, server: &str) -> io::Result<Option<StoredToken>> {
        Ok(self.load()?.remove(server))
    }

    /// Persist `token` under `server`, creating or rewriting the store at mode `0600`.
    ///
    /// # Errors
    /// Returns an error when the parent directory cannot be created or the file cannot be written.
    pub fn put(&self, server: &str, token: StoredToken) -> io::Result<()> {
        let mut all = self.load().unwrap_or_default();
        all.insert(server.to_owned(), token);
        self.write_all(&all)
    }

    /// Remove the credential for `server` (a no-op when absent).
    ///
    /// # Errors
    /// Returns an error when the store cannot be read or rewritten.
    pub fn remove(&self, server: &str) -> io::Result<()> {
        let mut all = self.load()?;
        if all.remove(server).is_some() {
            self.write_all(&all)?;
        }
        Ok(())
    }

    /// Write the whole map, guaranteeing the file ends up at mode `0600`.
    fn write_all(&self, all: &BTreeMap<String, StoredToken>) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_vec_pretty(all)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        {
            use io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .mode(0o600)
                .open(&self.path)?;
            file.write_all(&body)?;
            file.flush()?;
        }
        // An existing file keeps its prior mode through OpenOptions, so enforce 0600 explicitly.
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cortex-mcp-cred-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn token() -> StoredToken {
        StoredToken {
            access_token: "access-abc".to_owned(),
            refresh_token: Some("refresh-xyz".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now_secs() + 3600),
            token_endpoint: "http://127.0.0.1:3201/token".to_owned(),
            client_id: "cortex".to_owned(),
        }
    }

    #[test]
    fn put_then_get_round_trips_and_is_mode_0600() {
        let dir = scratch("rt");
        let store = CredentialStore::at(dir.join("mcp").join("credentials.json"));
        store.put("demo", token()).expect("put");

        let loaded = store.get("demo").expect("get").expect("present");
        assert_eq!(loaded.access_token, "access-abc");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh-xyz"));

        let mode = std::fs::metadata(store.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "credential file must be 0600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_store_is_an_empty_map() {
        let dir = scratch("missing");
        let store = CredentialStore::at(dir.join("credentials.json"));
        assert!(store.load().expect("load").is_empty());
        assert!(store.get("demo").expect("get").is_none());
    }

    #[test]
    fn expiry_is_detected_with_skew() {
        let mut t = token();
        t.expires_at = Some(now_secs() + 10);
        assert!(!t.is_expired(0), "not yet expired");
        assert!(t.is_expired(30), "expires within the skew window");
        t.expires_at = None;
        assert!(!t.is_expired(10_000), "no expiry never expires");
    }

    #[test]
    fn remove_drops_only_the_named_server() {
        let dir = scratch("rm");
        let store = CredentialStore::at(dir.join("credentials.json"));
        store.put("a", token()).unwrap();
        store.put("b", token()).unwrap();
        store.remove("a").unwrap();
        assert!(store.get("a").unwrap().is_none());
        assert!(store.get("b").unwrap().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

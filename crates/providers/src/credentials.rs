//! The credential/auth store: resolution across layers and a 0600-protected on-disk store.
//!
//! A provider credential is resolved with a fixed, documented precedence, highest first:
//!
//! 1. **env** — the provider's environment variable (e.g. `OPENROUTER_API_KEY`).
//! 2. **0600 credential file** — an entry in the on-disk auth store written with owner-only
//!    permissions (see [`store_credential`]).
//! 3. **config indirection** — a value supplied by the layered config, already resolved through the
//!    `settings` crate's `{env:}`/`{file:}` substitution before it reaches here.
//!
//! When no layer yields a non-empty key, resolution returns the canonical
//! [`Diagnostic::NoCredential`] (never a generic or network error), so callers can branch on a
//! missing key distinctly from a transport or upstream failure.
//!
//! The resolved key is wrapped in a [`Credential`] whose `Debug`/`Display` render only the
//! redaction placeholder, so it can never accidentally leak into a log line or error message. The
//! one place the raw key is intentionally written to disk is the 0600 auth store itself
//! ([`store_credential`]); every other sink must continue to redact.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use errors::Diagnostic;
use redaction::PLACEHOLDER;
use serde::{Deserialize, Serialize};

/// The OpenRouter credential environment variable (architecture §7).
pub const OPENROUTER_ENV: &str = "OPENROUTER_API_KEY";

/// The explicit auth-file path override.
pub const ENV_AUTH_FILE: &str = "CORTEX_AUTH_FILE";

/// The data-directory override (shared with persistence).
pub const ENV_DATA_DIR: &str = "CORTEX_DATA_DIR";

/// The auth store file name placed inside the resolved data directory.
pub const AUTH_FILE_NAME: &str = "auth.json";

/// The data subdirectory under the user's data home.
const DATA_SUBDIR: &str = "cortex";

/// The owner-read/write-only mode the credential file is created and kept at.
#[cfg(unix)]
const CREDENTIAL_FILE_MODE: u32 = 0o600;

/// Which layer a resolved [`Credential`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    /// The provider's environment variable (highest precedence).
    Env,
    /// The 0600 on-disk credential file.
    File,
    /// A value supplied through the layered config (lowest precedence).
    Config,
}

/// A resolved API credential whose raw value never appears in its `Debug`/`Display` output.
///
/// Use [`Credential::expose`] only at the single point the key is handed to the transport; never
/// log, format, or persist the exposed value anywhere but the 0600 store.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential {
    secret: String,
    source: CredentialSource,
}

impl Credential {
    fn new(secret: impl Into<String>, source: CredentialSource) -> Self {
        Self {
            secret: secret.into(),
            source,
        }
    }

    /// The raw secret. Call only when handing the key to the transport; do not log or persist it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.secret
    }

    /// Which layer this credential was resolved from.
    #[must_use]
    pub fn source(&self) -> CredentialSource {
        self.source
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("secret", &PLACEHOLDER)
            .field("source", &self.source)
            .finish()
    }
}

impl fmt::Display for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PLACEHOLDER)
    }
}

/// The candidate values for one credential resolution, captured explicitly so resolution is
/// deterministic and unit-testable rather than reading the process environment ambiently.
#[derive(Debug, Clone, Default)]
pub struct CredentialInputs {
    /// The provider's environment-variable value, when set (highest precedence).
    pub env_value: Option<String>,
    /// The path to the on-disk auth store to consult, when known.
    pub credential_file: Option<PathBuf>,
    /// A credential supplied through the layered config (already `{env:}`/`{file:}`-resolved).
    pub config_value: Option<String>,
}

impl CredentialInputs {
    /// Build resolution inputs from the current process for an arbitrary provider.
    #[must_use]
    pub fn from_process(provider_env: Option<&str>, config_value: Option<String>) -> Self {
        Self {
            env_value: provider_env.and_then(non_empty_env),
            credential_file: AuthPathInputs::from_process().resolve(),
            config_value,
        }
    }

    /// Build OpenRouter resolution inputs from the current process: the `OPENROUTER_API_KEY` env
    /// var, the default auth-file path, and the given config-supplied value.
    #[must_use]
    pub fn openrouter_from_process(config_value: Option<String>) -> Self {
        Self::from_process(Some(OPENROUTER_ENV), config_value)
    }
}

/// Resolve `provider_id`'s credential across the documented precedence layers.
///
/// # Errors
/// Returns [`Diagnostic::NoCredential`] (naming `provider_id`) when no layer yields a non-empty key.
pub fn resolve_credential(
    provider_id: &str,
    inputs: &CredentialInputs,
) -> Result<Credential, Diagnostic> {
    if let Some(secret) = non_empty(inputs.env_value.as_deref()) {
        return Ok(Credential::new(secret, CredentialSource::Env));
    }

    if let Some(path) = inputs.credential_file.as_deref() {
        if let Some(secret) = read_stored_key(path, provider_id) {
            return Ok(Credential::new(secret, CredentialSource::File));
        }
    }

    if let Some(secret) = non_empty(inputs.config_value.as_deref()) {
        return Ok(Credential::new(secret, CredentialSource::Config));
    }

    Err(Diagnostic::no_credential(provider_id))
}

/// One provider entry in the on-disk auth store.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthEntry {
    key: String,
}

/// The on-disk auth store: a map of provider id -> entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AuthFile {
    #[serde(flatten)]
    entries: BTreeMap<String, AuthEntry>,
}

/// Read the stored key for `provider_id` from the auth file at `path`.
///
/// A missing file, an unreadable/malformed file, an absent entry, or an empty/whitespace key all
/// yield `None` so resolution falls through to the next layer rather than hard-failing on a problem
/// the next layer may resolve. The only terminal credential error is [`Diagnostic::NoCredential`].
fn read_stored_key(path: &Path, provider_id: &str) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let file: AuthFile = serde_json::from_str(&contents).ok()?;
    let entry = file.entries.get(provider_id)?;
    non_empty(Some(&entry.key))
}

/// List provider ids that currently have a stored credential in `path`.
///
/// Missing/empty files yield an empty list. Invalid JSON yields [`io::ErrorKind::InvalidData`]
/// so callers can surface a clear diagnostic instead of silently ignoring corruption.
///
/// # Errors
/// Returns an [`io::Error`] when `path` cannot be read or parsed.
pub fn list_stored_providers(path: &Path) -> io::Result<Vec<String>> {
    let file = load_auth_file(path)?;
    Ok(file.entries.into_keys().collect())
}

/// The provider ids the user is currently connected to (has a stored credential for), resolved
/// from the process's auth-store path.
///
/// This is the convenience the `/models` picker uses to mark connected-vs-not-connected providers.
/// A missing/empty/unreadable store yields an empty list (never an error), so the picker degrades
/// cleanly when nothing is connected yet.
#[must_use]
pub fn connected_providers() -> Vec<String> {
    AuthPathInputs::from_process()
        .resolve()
        .and_then(|path| list_stored_providers(&path).ok())
        .unwrap_or_default()
}

/// Remove `provider_id` from the auth store at `path`.
///
/// Returns `Ok(true)` when an entry was removed, `Ok(false)` when no entry existed. If the removed
/// entry was the last one, the store file is deleted.
///
/// # Errors
/// Returns an [`io::Error`] when the store cannot be read, parsed, or rewritten.
pub fn remove_stored_credential(path: &Path, provider_id: &str) -> io::Result<bool> {
    let mut file = load_auth_file(path)?;
    if file.entries.remove(provider_id).is_none() {
        return Ok(false);
    }

    if file.entries.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(error),
        }
    } else {
        let json = serde_json::to_string_pretty(&file)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        write_private(path, &json)?;
        Ok(true)
    }
}

/// Persist `secret` as `provider_id`'s credential in the 0600 auth store at `path`.
///
/// Existing entries for other providers are preserved. Parent directories are created as needed and
/// the file is created/kept at owner-only (`0600`) permissions on Unix. This is the single
/// sanctioned place the raw key is written to disk.
///
/// # Errors
/// Returns an [`io::Error`] if the directory cannot be created or the file cannot be written.
pub fn store_credential(path: &Path, provider_id: &str, secret: &str) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<AuthFile>(&contents).unwrap_or_default(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => AuthFile::default(),
        Err(error) => return Err(error),
    };
    file.entries.insert(
        provider_id.to_owned(),
        AuthEntry {
            key: secret.to_owned(),
        },
    );

    let json = serde_json::to_string_pretty(&file)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write_private(path, &json)
}

fn load_auth_file(path: &Path) -> io::Result<AuthFile> {
    match std::fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => Ok(AuthFile::default()),
        Ok(contents) => serde_json::from_str(&contents)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(AuthFile::default()),
        Err(error) => Err(error),
    }
}

/// Write `contents` to `path`, creating the file owner-only and enforcing 0600 afterward.
fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        options.mode(CREDENTIAL_FILE_MODE);
        let mut handle = options.open(path)?;
        io::Write::write_all(&mut handle, contents.as_bytes())?;
        // Enforce 0600 even if the file pre-existed with looser permissions (open ignores `mode`
        // for an existing file).
        std::fs::set_permissions(
            path,
            std::os::unix::fs::PermissionsExt::from_mode(CREDENTIAL_FILE_MODE),
        )?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

/// Inputs that determine the auth-store path, captured explicitly for deterministic resolution.
///
/// Precedence mirrors the persistence layer so the credential file lives alongside other per-user
/// state: an explicit `CORTEX_AUTH_FILE`, then `CORTEX_DATA_DIR`, then
/// `${XDG_DATA_HOME:-$HOME/.local/share}/cortex/auth.json`.
#[derive(Debug, Clone, Default)]
pub struct AuthPathInputs {
    /// `CORTEX_AUTH_FILE`, when set and non-empty: an explicit path to the auth file.
    pub auth_file: Option<PathBuf>,
    /// `CORTEX_DATA_DIR`, when set and non-empty: the directory to place the auth file in.
    pub data_dir: Option<PathBuf>,
    /// `XDG_DATA_HOME`, when set and non-empty: the base data home for the default location.
    pub xdg_data_home: Option<PathBuf>,
    /// The user's `HOME`, used for the default `${HOME}/.local/share` data home.
    pub home: Option<PathBuf>,
}

impl AuthPathInputs {
    /// Populate the inputs from the current process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            auth_file: non_empty_env_path(ENV_AUTH_FILE),
            data_dir: non_empty_env_path(ENV_DATA_DIR),
            xdg_data_home: non_empty_env_path("XDG_DATA_HOME"),
            home: non_empty_env_path("HOME"),
        }
    }

    /// Resolve the auth-file path per the documented precedence, or `None` when no override is set
    /// and neither `XDG_DATA_HOME` nor `HOME` is available.
    #[must_use]
    pub fn resolve(&self) -> Option<PathBuf> {
        if let Some(auth_file) = self.auth_file.as_ref() {
            return Some(auth_file.clone());
        }
        if let Some(data_dir) = self.data_dir.as_ref() {
            return Some(data_dir.join(AUTH_FILE_NAME));
        }
        self.default_data_home()
            .map(|home| home.join(DATA_SUBDIR).join(AUTH_FILE_NAME))
    }

    fn default_data_home(&self) -> Option<PathBuf> {
        if let Some(xdg) = self.xdg_data_home.as_ref() {
            return Some(xdg.clone());
        }
        self.home
            .as_ref()
            .map(|home| home.join(".local").join("share"))
    }
}

/// Trim a candidate value and keep it only if non-empty after trimming.
fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Read an environment variable as a string, treating unset/empty as absent.
pub(crate) fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

/// Read an environment variable as a path, treating unset/empty as absent.
pub(crate) fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake-but-key-shaped OpenRouter secret assembled from fragments so no contiguous
    /// key-shaped literal lives in source (the commit secret scanner flags those).
    fn fake_key() -> String {
        format!("sk-{}-{}", "or-v1", "abcd1234".repeat(6))
    }

    #[test]
    fn env_layer_wins_over_file_and_config() {
        let inputs = CredentialInputs {
            env_value: Some("env-key".to_owned()),
            credential_file: Some(PathBuf::from("/does/not/matter.json")),
            config_value: Some("config-key".to_owned()),
        };
        let credential = resolve_credential("openrouter", &inputs).expect("resolves");
        assert_eq!(credential.source(), CredentialSource::Env);
        assert_eq!(credential.expose(), "env-key");
    }

    #[test]
    fn config_layer_is_lowest_precedence() {
        let inputs = CredentialInputs {
            env_value: None,
            credential_file: None,
            config_value: Some("config-key".to_owned()),
        };
        let credential = resolve_credential("openrouter", &inputs).expect("resolves");
        assert_eq!(credential.source(), CredentialSource::Config);
        assert_eq!(credential.expose(), "config-key");
    }

    #[test]
    fn blank_env_value_falls_through_to_config() {
        let inputs = CredentialInputs {
            env_value: Some("   ".to_owned()),
            credential_file: None,
            config_value: Some("config-key".to_owned()),
        };
        let credential = resolve_credential("openrouter", &inputs).expect("resolves");
        assert_eq!(credential.source(), CredentialSource::Config);
    }

    #[test]
    fn no_layer_yields_the_canonical_no_credential_diagnostic() {
        let inputs = CredentialInputs::default();
        let err = resolve_credential("openrouter", &inputs).expect_err("must fail");
        assert_eq!(err.kind(), "no-credential");
        assert_eq!(err, Diagnostic::no_credential("openrouter"));
    }

    #[test]
    fn file_layer_wins_over_config_and_round_trips_through_the_store() {
        let dir = unique_temp_dir("cred-file");
        let path = dir.join("auth.json");
        let secret = fake_key();
        store_credential(&path, "openrouter", &secret).expect("store");

        let inputs = CredentialInputs {
            env_value: None,
            credential_file: Some(path),
            config_value: Some("config-key".to_owned()),
        };
        let credential = resolve_credential("openrouter", &inputs).expect("resolves");
        assert_eq!(credential.source(), CredentialSource::File);
        assert_eq!(credential.expose(), secret);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stored_entry_for_another_provider_does_not_resolve() {
        let dir = unique_temp_dir("cred-other");
        let path = dir.join("auth.json");
        store_credential(&path, "someone-else", &fake_key()).expect("store");

        let inputs = CredentialInputs {
            env_value: None,
            credential_file: Some(path),
            config_value: None,
        };
        let err = resolve_credential("openrouter", &inputs).expect_err("no openrouter entry");
        assert_eq!(err.kind(), "no-credential");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_preserves_existing_entries_for_other_providers() {
        let dir = unique_temp_dir("cred-preserve");
        let path = dir.join("auth.json");
        store_credential(&path, "openrouter", "or-key").expect("store openrouter");
        store_credential(&path, "another", "another-key").expect("store another");

        for (provider, expected) in [("openrouter", "or-key"), ("another", "another-key")] {
            assert_eq!(read_stored_key(&path, provider).as_deref(), Some(expected));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_stored_providers_is_sorted_and_missing_file_is_empty() {
        let dir = unique_temp_dir("cred-list");
        let path = dir.join("auth.json");
        assert!(
            list_stored_providers(&path)
                .expect("missing file should list as empty")
                .is_empty()
        );

        store_credential(&path, "zeta", "z").expect("store zeta");
        store_credential(&path, "alpha", "a").expect("store alpha");
        assert_eq!(
            list_stored_providers(&path).expect("list"),
            vec!["alpha".to_owned(), "zeta".to_owned()]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_stored_credential_removes_the_entry_and_keeps_others() {
        let dir = unique_temp_dir("cred-remove");
        let path = dir.join("auth.json");
        let openrouter_key = fake_key();
        store_credential(&path, "openrouter", &openrouter_key).expect("store openrouter");
        store_credential(&path, "another", "another-key").expect("store another");

        assert!(
            remove_stored_credential(&path, "openrouter").expect("remove openrouter"),
            "openrouter must be removed"
        );
        assert_eq!(
            list_stored_providers(&path).expect("list after remove"),
            vec!["another".to_owned()]
        );
        let body = std::fs::read_to_string(&path).expect("read auth file");
        assert!(
            !body.contains(&openrouter_key),
            "removed secret must not remain on disk"
        );
        assert!(
            body.contains("another-key"),
            "other provider must be preserved"
        );

        assert!(
            !remove_stored_credential(&path, "openrouter").expect("remove absent entry"),
            "removing an already-absent provider is a clean no-op"
        );
        assert!(
            remove_stored_credential(&path, "another").expect("remove final provider"),
            "final provider removal should still report true"
        );
        assert!(
            !path.exists(),
            "removing the final provider should remove the store file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_store_falls_through_instead_of_failing() {
        let dir = unique_temp_dir("cred-bad");
        let path = dir.join("auth.json");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(&path, "{ this is not json").expect("write junk");

        let inputs = CredentialInputs {
            env_value: None,
            credential_file: Some(path),
            config_value: Some("config-key".to_owned()),
        };
        let credential = resolve_credential("openrouter", &inputs).expect("falls through");
        assert_eq!(credential.source(), CredentialSource::Config);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn stored_credential_file_is_owner_only_0600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_temp_dir("cred-perms");
        let path = dir.join("auth.json");
        store_credential(&path, "openrouter", &fake_key()).expect("store");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "credential file must be owner read/write only"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn store_re_enforces_0600_on_a_preexisting_loose_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = unique_temp_dir("cred-loose");
        let path = dir.join("auth.json");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(&path, "{}").expect("seed");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("loosen");

        store_credential(&path, "openrouter", "k").expect("store");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "store must tighten loose permissions");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn credential_debug_and_display_never_reveal_the_secret() {
        let secret = fake_key();
        let credential = Credential::new(secret.clone(), CredentialSource::Env);
        let debug = format!("{credential:?}");
        let display = format!("{credential}");
        assert!(!debug.contains(&secret), "Debug must not leak the secret");
        assert!(
            !display.contains(&secret),
            "Display must not leak the secret"
        );
        assert!(debug.contains(PLACEHOLDER));
        assert_eq!(display, PLACEHOLDER);
    }

    #[test]
    fn auth_path_precedence_matches_the_data_layer() {
        let explicit = AuthPathInputs {
            auth_file: Some(PathBuf::from("/explicit/auth.json")),
            data_dir: Some(PathBuf::from("/data")),
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
        };
        assert_eq!(
            explicit.resolve(),
            Some(PathBuf::from("/explicit/auth.json"))
        );

        let data_dir = AuthPathInputs {
            auth_file: None,
            data_dir: Some(PathBuf::from("/data")),
            ..Default::default()
        };
        assert_eq!(data_dir.resolve(), Some(PathBuf::from("/data/auth.json")));

        let xdg = AuthPathInputs {
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
            ..Default::default()
        };
        assert_eq!(xdg.resolve(), Some(PathBuf::from("/xdg/cortex/auth.json")));

        let home = AuthPathInputs {
            home: Some(PathBuf::from("/home/u")),
            ..Default::default()
        };
        assert_eq!(
            home.resolve(),
            Some(PathBuf::from("/home/u/.local/share/cortex/auth.json"))
        );

        assert_eq!(AuthPathInputs::default().resolve(), None);
    }

    fn unique_temp_dir(tag: &str) -> PathBuf {
        let unique = format!(
            "cortex-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }
}

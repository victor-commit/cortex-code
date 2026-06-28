//! Durable, per-workspace **scoped environment store**.
//!
//! The environment service lets a user manage environment-like values (e.g. variables a project's
//! shell commands need) without mutating the agent's own process environment — `std::env::set_var`
//! is `unsafe` and would leak across unrelated runs. Instead, values are kept in a per-project JSON
//! store, owner-only (`0600`) on Unix because they may hold secrets, and merged into the environment
//! of commands the agent runs (the `bash` tool / server `shell` route). Setting a key makes it
//! visible to later runs; removing it makes those runs no longer see it.
//!
//! The store is **per project**: the file lives OUTSIDE the project tree, under the per-user data
//! directory, keyed by a caller-supplied workspace key (the same key the shadow checkpoint and
//! saved-permission stores use — `checkpoints::workspace_key`), so two runs from the same directory
//! address the same file and two different projects never share values. The key is passed in
//! (rather than resolved here) so this crate stays decoupled from the checkpoint store. Location
//! precedence mirrors the database/permission resolvers:
//!
//! - `CORTEX_ENV_DIR` — an explicit base directory for environment stores (highest).
//! - `CORTEX_DATA_DIR` — `<CORTEX_DATA_DIR>/environment`.
//! - `XDG_DATA_HOME` — `<XDG_DATA_HOME>/cortex/environment`.
//! - `HOME` — `<HOME>/.local/share/cortex/environment`.
//!
//! The file is a plain JSON object (`{ "KEY": "VALUE", ... }`), so it is inspectable. A missing or
//! unreadable/corrupt store is treated as "no scoped values" (the run proceeds; it never crashes on
//! a bad cache file). Secret-looking values are scrubbed by [`redact_for_display`] when the store is
//! listed, never on the bytes used to run a command.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// The owner-only file mode for the on-disk store (the values may be secrets).
#[cfg(unix)]
const STORE_FILE_MODE: u32 = 0o600;

/// The explicit environment-store base-directory override.
const ENV_STORE_DIR: &str = "CORTEX_ENV_DIR";

/// The subdirectory (under the resolved data base) holding per-project environment stores.
const STORE_SUBDIR: &str = "environment";

/// A scoped environment map: keys to their literal values, ordered by key for stable output.
pub type ScopedEnv = BTreeMap<String, String>;

/// Inputs that determine the environment-store base directory, captured explicitly so resolution is
/// deterministic and unit-testable rather than reading the process environment ambiently (mirrors
/// the database/permission path inputs).
#[derive(Debug, Clone, Default)]
pub struct EnvStoreInputs {
    /// `CORTEX_ENV_DIR`, when set and non-empty: an explicit base directory (highest).
    pub env_dir: Option<PathBuf>,
    /// `CORTEX_DATA_DIR`, when set and non-empty: the data directory (`<dir>/environment`).
    pub data_dir: Option<PathBuf>,
    /// `XDG_DATA_HOME`, when set and non-empty: the base data home for the default location.
    pub xdg_data_home: Option<PathBuf>,
    /// The user's `HOME`, used for the default `${HOME}/.local/share` data home.
    pub home: Option<PathBuf>,
}

impl EnvStoreInputs {
    /// Populate the inputs from the current process environment.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            env_dir: non_empty_var(ENV_STORE_DIR),
            data_dir: non_empty_var("CORTEX_DATA_DIR"),
            xdg_data_home: non_empty_var("XDG_DATA_HOME"),
            home: non_empty_var("HOME"),
        }
    }

    /// The base directory under which per-workspace environment stores live, by documented
    /// precedence, or `None` when no data location can be determined.
    #[must_use]
    fn base_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = self.env_dir.as_ref() {
            return Some(dir.clone());
        }
        if let Some(dir) = self.data_dir.as_ref() {
            return Some(dir.join(STORE_SUBDIR));
        }
        if let Some(xdg) = self.xdg_data_home.as_ref() {
            return Some(xdg.join("cortex").join(STORE_SUBDIR));
        }
        self.home.as_ref().map(|home| {
            home.join(".local")
                .join("share")
                .join("cortex")
                .join(STORE_SUBDIR)
        })
    }
}

/// Resolve the environment-store file for `workspace_key` against explicit `inputs`, or `None` when
/// no data location can be determined.
#[must_use]
pub fn store_path_with(inputs: &EnvStoreInputs, workspace_key: &str) -> Option<PathBuf> {
    inputs
        .base_dir()
        .map(|base| base.join(format!("{workspace_key}.json")))
}

/// Resolve the environment-store file for `workspace_key` against the process environment.
#[must_use]
fn store_path(workspace_key: &str) -> Option<PathBuf> {
    store_path_with(&EnvStoreInputs::from_process(), workspace_key)
}

/// Read the scoped environment from a store file.
///
/// A missing file or one whose contents cannot be parsed yields an empty map (the run proceeds with
/// no scoped values rather than failing on a bad/foreign cache file).
#[must_use]
pub fn read(path: &Path) -> ScopedEnv {
    let Ok(bytes) = std::fs::read(path) else {
        return ScopedEnv::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Write the scoped environment to a store file as JSON, creating the parent directory on first use
/// and enforcing owner-only (`0600`) permissions on Unix.
///
/// # Errors
/// Returns the underlying [`io::Error`] when the parent directory cannot be created or the file
/// cannot be written.
pub fn write(path: &Path, env: &ScopedEnv) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(env)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    write_private(path, &body)
}

/// Load the scoped environment for `workspace_key` from the process-resolved store, or an empty map
/// when no store exists yet or no data location can be determined.
#[must_use]
pub fn load_for(workspace_key: &str) -> ScopedEnv {
    store_path(workspace_key)
        .map(|path| read(&path))
        .unwrap_or_default()
}

/// Persist the scoped environment for `workspace_key` to the process-resolved store. A run with no
/// resolvable data location is a clean no-op.
///
/// # Errors
/// Returns the underlying [`io::Error`] when the resolved store file cannot be written.
pub fn save_for(workspace_key: &str, env: &ScopedEnv) -> io::Result<()> {
    match store_path(workspace_key) {
        Some(path) => write(&path, env),
        None => Ok(()),
    }
}

/// Set (or overwrite) one scoped value for `workspace_key`, returning the updated map.
///
/// # Errors
/// Returns the underlying [`io::Error`] when the store cannot be written.
pub fn set_for(workspace_key: &str, key: &str, value: &str) -> io::Result<ScopedEnv> {
    let mut env = load_for(workspace_key);
    env.insert(key.to_owned(), value.to_owned());
    save_for(workspace_key, &env)?;
    Ok(env)
}

/// Remove one scoped value for `workspace_key`, returning whether a value was present.
///
/// # Errors
/// Returns the underlying [`io::Error`] when the store cannot be written.
pub fn remove_for(workspace_key: &str, key: &str) -> io::Result<bool> {
    let mut env = load_for(workspace_key);
    let removed = env.remove(key).is_some();
    if removed {
        save_for(workspace_key, &env)?;
    }
    Ok(removed)
}

/// Whether `value` looks like a secret (an API-key-shaped or otherwise sensitive value) and so must
/// be scrubbed before display. Defers to the canonical [`redaction`] heuristics so the env service
/// uses the same secret detection as every other emitting sink.
#[must_use]
pub fn value_is_secret(value: &str) -> bool {
    redaction::value_is_secret(value)
}

/// The display form of a value: the canonical redaction placeholder when it looks like a secret,
/// otherwise the value verbatim. Used by `env all`/`env list`, never on the bytes used to run a
/// command.
#[must_use]
pub fn redact_for_display(value: &str) -> String {
    redaction::redact(value)
}

/// Read an environment variable, treating an unset or empty value as absent.
fn non_empty_var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Write `bytes` to `path`, creating the file owner-only and enforcing `0600` afterward on Unix.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        options.mode(STORE_FILE_MODE);
        let mut handle = options.open(path)?;
        io::Write::write_all(&mut handle, bytes)?;
        std::fs::set_permissions(
            path,
            std::os::unix::fs::PermissionsExt::from_mode(STORE_FILE_MODE),
        )?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scratch_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cortex-env-store-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn inputs_for(base: &Path) -> EnvStoreInputs {
        EnvStoreInputs {
            env_dir: Some(base.to_path_buf()),
            ..EnvStoreInputs::default()
        }
    }

    #[test]
    fn explicit_env_dir_wins_and_is_workspace_keyed() {
        let base = scratch_dir("base");
        let inputs = EnvStoreInputs {
            env_dir: Some(base.clone()),
            data_dir: Some(PathBuf::from("/data")),
            xdg_data_home: Some(PathBuf::from("/xdg")),
            home: Some(PathBuf::from("/home/u")),
        };
        let path = store_path_with(&inputs, "projkey").expect("path");
        assert!(path.starts_with(&base), "explicit dir wins: {path:?}");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("projkey.json")
        );
    }

    #[test]
    fn data_dir_xdg_then_home_fallbacks_resolve() {
        let data = EnvStoreInputs {
            data_dir: Some(PathBuf::from("/data")),
            ..EnvStoreInputs::default()
        };
        assert!(
            store_path_with(&data, "k")
                .expect("data path")
                .starts_with(PathBuf::from("/data").join(STORE_SUBDIR))
        );
        let xdg = EnvStoreInputs {
            xdg_data_home: Some(PathBuf::from("/xdg")),
            ..EnvStoreInputs::default()
        };
        assert!(
            store_path_with(&xdg, "k")
                .expect("xdg path")
                .starts_with(PathBuf::from("/xdg").join("cortex").join(STORE_SUBDIR))
        );
        let home = EnvStoreInputs {
            home: Some(PathBuf::from("/home/u")),
            ..EnvStoreInputs::default()
        };
        assert!(
            store_path_with(&home, "k")
                .expect("home path")
                .starts_with(PathBuf::from("/home/u").join(".local").join("share"))
        );
    }

    #[test]
    fn unresolvable_without_any_location() {
        assert!(store_path_with(&EnvStoreInputs::default(), "k").is_none());
    }

    #[test]
    fn missing_store_reads_as_empty() {
        let dir = scratch_dir("missing");
        assert!(read(&dir.join("absent.json")).is_empty());
    }

    #[test]
    fn corrupt_store_reads_as_empty_not_a_crash() {
        let dir = scratch_dir("corrupt");
        let path = dir.join("bad.json");
        std::fs::write(&path, b"{ this is not valid json ][").unwrap();
        assert!(read(&path).is_empty());
    }

    #[test]
    fn set_get_remove_round_trips_through_the_store_file() {
        let base = scratch_dir("round-trip");
        let inputs = inputs_for(&base);
        let path = store_path_with(&inputs, "wk").expect("path");

        let mut env = read(&path);
        env.insert("PROJECT_REGION".to_owned(), "us-west".to_owned());
        write(&path, &env).expect("write");

        let reloaded = read(&path);
        assert_eq!(
            reloaded.get("PROJECT_REGION").map(String::as_str),
            Some("us-west")
        );

        let mut after = reloaded;
        assert!(after.remove("PROJECT_REGION").is_some());
        write(&path, &after).expect("write after remove");
        assert!(read(&path).get("PROJECT_REGION").is_none());
    }

    #[test]
    fn secret_values_redact_for_display_but_plain_values_do_not() {
        let secret = format!("sk-{}-{}", "or-v1", "a".repeat(48));
        assert!(value_is_secret(&secret), "api-key-shaped value is secret");
        assert_ne!(
            redact_for_display(&secret),
            secret,
            "a secret value must be scrubbed for display"
        );
        assert_eq!(
            redact_for_display("us-west"),
            "us-west",
            "a plain value is shown verbatim"
        );
    }

    #[test]
    #[cfg(unix)]
    fn written_store_is_owner_only_0600() {
        use std::os::unix::fs::PermissionsExt;
        let base = scratch_dir("perms");
        let path = store_path_with(&inputs_for(&base), "wk").expect("path");
        let mut env = ScopedEnv::new();
        env.insert("K".to_owned(), "V".to_owned());
        write(&path, &env).expect("write");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "env store must be owner-only");
    }
}

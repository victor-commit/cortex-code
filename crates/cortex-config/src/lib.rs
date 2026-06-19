//! Layered TOML configuration for Cortex Code.
//!
//! The effective configuration is assembled by deep-merging a fixed stack of layers, lowest
//! precedence first:
//!
//! `defaults < global < project < nested < env < CLI`
//!
//! - **defaults** — built-in values (e.g. model `google/gemini-2.5-flash`).
//! - **global** — `${XDG_CONFIG_HOME:-$HOME/.config}/cortex/config.toml`.
//! - **project** — `cortex.toml` / `.cortex/config.toml` at the project root.
//! - **nested** — the same files in subdirectories closer to the invocation CWD (these win
//!   over the project root).
//! - **env** — `CORTEX_<KEY>` variables (see [`env::env_var_name`]).
//! - **CLI** — explicit flags supplied at invocation.
//!
//! Layering is a per-key deep-merge (see [`merge`]): adding a higher layer only overrides the
//! keys it sets and never erases unrelated keys from lower layers. A present-but-empty or
//! whitespace/comment-only config file parses to an empty table and is a clean no-op.

mod discovery;
mod env;
mod merge;

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use toml::Value;
use toml::value::Table;

pub use env::env_var_name;

/// The built-in default model used when no layer overrides it.
pub const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// The resolved inputs for one configuration load.
///
/// Every environment-derived value is captured explicitly (rather than read from the process
/// ambiently) so loads are deterministic and testable. Use [`ConfigInputs::from_process`] to
/// populate them from the current process for real invocations.
#[derive(Debug, Clone)]
pub struct ConfigInputs {
    /// The user's home directory; bounds the project walk and locates the global config.
    pub home: Option<PathBuf>,
    /// `XDG_CONFIG_HOME`, when set and non-empty; overrides `$HOME/.config` for the global layer.
    pub xdg_config_home: Option<PathBuf>,
    /// The invocation directory the project/nested walk starts from.
    pub cwd: PathBuf,
    /// The environment to scan for `CORTEX_<KEY>` overrides.
    pub env: Vec<(String, String)>,
    /// Highest-precedence overrides supplied via CLI flags, keyed by config key.
    pub cli_overrides: BTreeMap<String, String>,
}

impl ConfigInputs {
    /// Build inputs from the current process environment and working directory, layering the
    /// supplied CLI overrides on top.
    ///
    /// # Errors
    /// Returns an error if the current working directory cannot be determined.
    pub fn from_process(cli_overrides: BTreeMap<String, String>) -> io::Result<Self> {
        Ok(Self {
            home: std::env::var_os("HOME").map(PathBuf::from),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty()),
            cwd: std::env::current_dir()?,
            env: std::env::vars().collect(),
            cli_overrides,
        })
    }
}

/// The effective configuration after all layers have been merged.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveConfig {
    table: Table,
}

impl EffectiveConfig {
    /// The effective model, falling back to [`DEFAULT_MODEL`] if it was somehow cleared.
    #[must_use]
    pub fn model(&self) -> &str {
        self.table
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_MODEL)
    }

    /// The raw merged value for an arbitrary top-level key, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.table.get(key)
    }

    /// The full merged config table.
    #[must_use]
    pub fn table(&self) -> &Table {
        &self.table
    }

    /// Render the effective config as a single strict JSON document (compact, one line).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.table).expect("a toml table is always JSON-serializable")
    }

    /// Render the effective config as a TOML document for human inspection.
    #[must_use]
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(&self.table).unwrap_or_else(|_| self.to_json())
    }
}

/// Load and merge every configuration layer for the given inputs.
///
/// # Errors
/// Returns [`ConfigError`] if a present config file cannot be read or contains invalid TOML.
/// Absent files are skipped, and an empty/whitespace/comment-only file is a clean no-op.
pub fn load(inputs: ConfigInputs) -> Result<EffectiveConfig, ConfigError> {
    let mut table = default_table();

    for path in discovery::config_files(
        inputs.home.as_deref(),
        inputs.xdg_config_home.as_deref(),
        &inputs.cwd,
    ) {
        if let Some(layer) = read_layer(&path)? {
            merge::merge_into(&mut table, layer);
        }
    }

    apply_string_overrides(&mut table, env::overrides_from(inputs.env));
    apply_string_overrides(&mut table, inputs.cli_overrides);

    Ok(EffectiveConfig { table })
}

/// The built-in defaults layer.
fn default_table() -> Table {
    let mut table = Table::new();
    table.insert("model".to_owned(), Value::String(DEFAULT_MODEL.to_owned()));
    table
}

/// Apply a set of top-level string overrides (env or CLI) onto the merged table.
fn apply_string_overrides(table: &mut Table, overrides: BTreeMap<String, String>) {
    for (key, value) in overrides {
        table.insert(key, Value::String(value));
    }
}

/// Read one config file into a TOML table. An absent file yields `None`; a present file is
/// parsed (an empty/whitespace/comment-only file parses to an empty table).
fn read_layer(path: &Path) -> Result<Option<Table>, ConfigError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(ConfigError::read(path, error)),
    };
    let table: Table =
        toml::from_str(&contents).map_err(|error| ConfigError::parse(path, error))?;
    Ok(Some(table))
}

/// A failure loading or parsing a configuration file.
#[derive(Debug)]
pub enum ConfigError {
    /// A present config file could not be read.
    Read { path: PathBuf, source: io::Error },
    /// A config file contained invalid TOML.
    Parse { path: PathBuf, message: String },
}

impl ConfigError {
    fn read(path: &Path, source: io::Error) -> Self {
        Self::Read {
            path: path.to_path_buf(),
            source,
        }
    }

    fn parse(path: &Path, source: toml::de::Error) -> Self {
        Self::Parse {
            path: path.to_path_buf(),
            message: source.to_string(),
        }
    }

    /// The config file that caused the failure.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Read { path, .. } | Self::Parse { path, .. } => path,
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "failed to read config file {}: {source}",
                    path.display()
                )
            }
            Self::Parse { path, message } => {
                write!(
                    formatter,
                    "invalid TOML in config file {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Parse { .. } => None,
        }
    }
}

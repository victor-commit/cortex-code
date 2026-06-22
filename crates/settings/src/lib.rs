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
//!
//! After merging, two passes run over the effective table:
//!
//! - **Substitution** (see [`subst`]): a string value that is exactly `{env:NAME}` or
//!   `{file:PATH}` is replaced by the environment variable's value, or the file's trimmed
//!   contents. An unset variable or unreadable file is a hard error with a clear diagnostic;
//!   the literal token is never silently leaked into an effective value.
//! - **Type validation** (see [`schema`]): the small set of typed keys (`model`/`agent` are
//!   strings, `max_steps` an integer) is checked, so a wrongly-typed value yields a clear diagnostic
//!   naming the offending key and its expected type rather than mis-driving the tool.
//!
//! Both passes surface failures as [`ConfigError`], which the config-reporting path renders to
//! stderr with a non-zero exit.

mod discovery;
mod env;
mod merge;
mod schema;
mod subst;

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use toml::Value;
use toml::value::Table;

pub use env::env_var_name;

/// The built-in default model used when no layer overrides it.
pub const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// The built-in default agent persona used when no layer overrides it.
pub const DEFAULT_AGENT: &str = "build";

/// The built-in default step budget for a single agent turn (architecture §4, `MAX_STEPS`).
pub const DEFAULT_MAX_STEPS: i64 = 25;

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

    /// The effective default agent, falling back to [`DEFAULT_AGENT`] if it was somehow cleared.
    #[must_use]
    pub fn agent(&self) -> &str {
        self.table
            .get("agent")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_AGENT)
    }

    /// The effective step budget, falling back to [`DEFAULT_MAX_STEPS`] if it was cleared.
    #[must_use]
    pub fn max_steps(&self) -> i64 {
        self.table
            .get("max_steps")
            .and_then(Value::as_integer)
            .unwrap_or(DEFAULT_MAX_STEPS)
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
    ///
    /// This is a config-reporting sink, so API-key-shaped secrets are scrubbed to the canonical
    /// [`redaction::PLACEHOLDER`] before serialization; non-secret values are reported verbatim.
    /// The structured accessors ([`get`](Self::get), [`table`](Self::table), [`model`](Self::model))
    /// still return the real resolved values for runtime use.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.redacted_table())
            .expect("a toml table is always JSON-serializable")
    }

    /// Render the effective config as a TOML document for human inspection, with secrets scrubbed
    /// to the canonical [`redaction::PLACEHOLDER`] (see [`to_json`](Self::to_json)).
    #[must_use]
    pub fn to_toml(&self) -> String {
        let redacted = self.redacted_table();
        toml::to_string_pretty(&redacted).unwrap_or_else(|_| {
            serde_json::to_string(&redacted).expect("a toml table is always JSON-serializable")
        })
    }

    /// A clone of the effective table with every API-key-shaped secret value scrubbed, used by the
    /// reporting sinks only (the live table retains the real values for runtime).
    fn redacted_table(&self) -> Table {
        let mut table = self.table.clone();
        redact_table(&mut table);
        table
    }
}

/// Recursively scrub secrets from every string value in `table`.
fn redact_table(table: &mut Table) {
    for (_key, value) in table.iter_mut() {
        redact_value(value);
    }
}

/// Scrub secrets from a single config value, descending into nested tables and arrays so a secret
/// referenced anywhere (e.g. `[auth] key = "..."`) is masked while non-secret siblings are kept.
fn redact_value(value: &mut Value) {
    match value {
        Value::String(text) => {
            let scrubbed = redaction::redact(text);
            if scrubbed != *text {
                *text = scrubbed;
            }
        }
        Value::Table(table) => redact_table(table),
        Value::Array(items) => items.iter_mut().for_each(redact_value),
        _ => {}
    }
}

/// Load and merge every configuration layer for the given inputs.
///
/// # Errors
/// Returns [`ConfigError`] if a present config file cannot be read or contains invalid TOML.
/// Absent files are skipped, and an empty/whitespace/comment-only file is a clean no-op.
pub fn load(inputs: ConfigInputs) -> Result<EffectiveConfig, ConfigError> {
    let ConfigInputs {
        home,
        xdg_config_home,
        cwd,
        env,
        cli_overrides,
    } = inputs;

    let mut table = default_table();

    for path in discovery::config_files(home.as_deref(), xdg_config_home.as_deref(), &cwd) {
        if let Some(layer) = read_layer(&path)? {
            merge::merge_into(&mut table, layer);
        }
    }

    // Resolve `{env:}` tokens against the full environment, so capture it before the env layer
    // (which only maps `CORTEX_*`) consumes the list.
    let environment: BTreeMap<String, String> = env.iter().cloned().collect();
    apply_string_overrides(&mut table, env::overrides_from(env));
    apply_string_overrides(&mut table, cli_overrides);

    subst::substitute(&mut table, &environment, &cwd)?;
    schema::validate(&mut table)?;

    Ok(EffectiveConfig { table })
}

/// A single raw configuration layer: the file it was read from and its parsed table.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigLayer {
    /// The config file this layer was read from.
    pub source: PathBuf,
    /// The layer's parsed TOML table (raw: no merge, env, CLI, substitution, or validation).
    pub table: Table,
}

/// Persist a single top-level `key = value` string into the user's global config file
/// (`${XDG_CONFIG_HOME:-$HOME/.config}/cortex/config.toml`), creating the file and any parent
/// directories as needed and preserving every other key already present. Returns the path written.
///
/// This is the durable seam for preferences set interactively (e.g. the TUI theme picker): the
/// next [`load`] reads the value back from the global layer. The existing file is parsed, the
/// single key is upserted, and the table is re-serialized, so unrelated keys survive untouched.
///
/// # Errors
/// Returns [`ConfigError::NoConfigHome`] when no global config location can be resolved (neither
/// `$XDG_CONFIG_HOME` nor `$HOME` set), [`ConfigError::Parse`] when the existing file holds invalid
/// TOML, or [`ConfigError::Write`] when the file cannot be (re)written.
pub fn persist_global_string(
    inputs: &ConfigInputs,
    key: &str,
    value: &str,
) -> Result<PathBuf, ConfigError> {
    let path =
        discovery::global_config_file(inputs.home.as_deref(), inputs.xdg_config_home.as_deref())
            .ok_or(ConfigError::NoConfigHome)?;

    let mut table = read_layer(&path)?.unwrap_or_default();
    table.insert(key.to_owned(), Value::String(value.to_owned()));

    let body = toml::to_string_pretty(&table)
        .map_err(|error| ConfigError::write(&path, io::Error::other(error.to_string())))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| ConfigError::write(&path, error))?;
    }
    std::fs::write(&path, body).map_err(|error| ConfigError::write(&path, error))?;
    Ok(path)
}

/// A declarative MCP server entry to persist under `[mcp.<name>]` in the global config file.
///
/// This mirrors the subset of the `mcp` crate's server config that the `cortex mcp add` CLI
/// surface lets a user set; it is expressed in primitive types here so the `settings` crate (which
/// owns the on-disk config representation) stays decoupled from the `mcp` crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpServerSpecTransport {
    /// A local stdio server launched from a command array.
    Local,
    /// A remote HTTP/SSE server reached at a URL.
    Remote,
}

/// A declarative MCP server entry to persist under `[mcp.<name>]` in the global config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerSpec {
    /// The transport kind to persist. `Local` omits `type = "local"` because it is the runtime
    /// default; `Remote` writes `type = "remote"`.
    pub transport: McpServerSpecTransport,
    /// The command to launch the local stdio server: the program followed by its arguments.
    pub command: Vec<String>,
    /// The URL of the remote HTTP/SSE server, when `transport` is remote.
    pub url: Option<String>,
    /// Extra HTTP headers set on the remote client, in declared order.
    pub headers: Vec<(String, String)>,
    /// The working directory the server child is spawned in, when set.
    pub cwd: Option<String>,
    /// Extra environment variables set on the server child, in declared order.
    pub environment: Vec<(String, String)>,
    /// The per-operation timeout in seconds, when set.
    pub timeout: Option<u64>,
    /// Whether the server is enabled. A disabled entry is recorded but never spawned.
    pub enabled: bool,
}

impl McpServerSpec {
    /// Render this spec as the `[mcp.<name>]` TOML table that is written to (and compared against
    /// what is read back from) the config file. `enabled = true` is the default and is omitted, so
    /// re-adding an identical server is detected as unchanged rather than as a conflict.
    fn to_table(&self) -> Table {
        let mut table = Table::new();
        match self.transport {
            McpServerSpecTransport::Local => {
                table.insert(
                    "command".to_owned(),
                    Value::Array(
                        self.command
                            .iter()
                            .map(|arg| Value::String(arg.clone()))
                            .collect(),
                    ),
                );
            }
            McpServerSpecTransport::Remote => {
                table.insert("type".to_owned(), Value::String("remote".to_owned()));
                if let Some(url) = &self.url {
                    table.insert("url".to_owned(), Value::String(url.clone()));
                }
                if !self.headers.is_empty() {
                    let mut headers = Table::new();
                    for (key, value) in &self.headers {
                        headers.insert(key.clone(), Value::String(value.clone()));
                    }
                    table.insert("headers".to_owned(), Value::Table(headers));
                }
            }
        }
        if let Some(cwd) = &self.cwd {
            table.insert("cwd".to_owned(), Value::String(cwd.clone()));
        }
        if !self.environment.is_empty() {
            let mut env = Table::new();
            for (key, value) in &self.environment {
                env.insert(key.clone(), Value::String(value.clone()));
            }
            table.insert("environment".to_owned(), Value::Table(env));
        }
        if let Some(timeout) = self.timeout {
            table.insert("timeout".to_owned(), Value::Integer(timeout as i64));
        }
        if !self.enabled {
            table.insert("enabled".to_owned(), Value::Boolean(false));
        }
        table
    }
}

/// The result of persisting an MCP server definition into the global config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerPersist {
    /// A new server entry was written to the config file at the returned path.
    Added(PathBuf),
    /// An identical server entry was already present; the config file was left untouched.
    Unchanged(PathBuf),
    /// A different server of the same name already exists and `overwrite` was not set; the config
    /// file was left untouched so the existing definition is not corrupted.
    Conflict(PathBuf),
}

/// Persist an MCP server definition into the user's global config file under `[mcp.<name>]`,
/// preserving every other key (and every other MCP server) already present. Returns the outcome.
///
/// Re-adding the *same* definition is idempotent ([`McpServerPersist::Unchanged`], file untouched);
/// adding a *different* definition under an existing name is refused unless `overwrite` is set
/// ([`McpServerPersist::Conflict`], file untouched), so a duplicate add never corrupts the config.
///
/// # Errors
/// Returns [`ConfigError::NoConfigHome`] when no global config location can be resolved,
/// [`ConfigError::Parse`] when the existing file holds invalid TOML (or its `mcp` key is not a
/// table), or [`ConfigError::Write`] when the file cannot be (re)written.
pub fn persist_global_mcp_server(
    inputs: &ConfigInputs,
    name: &str,
    spec: &McpServerSpec,
    overwrite: bool,
) -> Result<McpServerPersist, ConfigError> {
    let path =
        discovery::global_config_file(inputs.home.as_deref(), inputs.xdg_config_home.as_deref())
            .ok_or(ConfigError::NoConfigHome)?;

    let mut root = read_layer(&path)?.unwrap_or_default();
    let entry = spec.to_table();

    let mcp_value = root
        .entry("mcp".to_owned())
        .or_insert_with(|| Value::Table(Table::new()));
    let Value::Table(servers) = mcp_value else {
        return Err(ConfigError::Parse {
            path,
            message: "the `mcp` config key is not a table of named servers".to_owned(),
        });
    };

    match servers.get(name) {
        Some(Value::Table(existing)) if *existing == entry => {
            return Ok(McpServerPersist::Unchanged(path));
        }
        Some(_) if !overwrite => return Ok(McpServerPersist::Conflict(path)),
        _ => {}
    }
    servers.insert(name.to_owned(), Value::Table(entry));

    let body = toml::to_string_pretty(&root)
        .map_err(|error| ConfigError::write(&path, io::Error::other(error.to_string())))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| ConfigError::write(&path, error))?;
    }
    std::fs::write(&path, body).map_err(|error| ConfigError::write(&path, error))?;
    Ok(McpServerPersist::Added(path))
}

/// Read each present config file as a raw, un-merged layer, in ascending precedence order
/// (global, then project root, then nested directories closer to the CWD).
///
/// Unlike [`load`], which deep-merges every layer into one effective table, this returns the
/// layers separately and in order so callers that need order-preserving, list-shaped config can
/// combine them themselves. The permission ruleset is the motivating consumer: its rules are an
/// ordered list whose cross-layer precedence is "last matching rule wins", which requires
/// concatenating the per-layer rule lists in precedence order rather than letting a higher layer's
/// array replace a lower one (the deep-merge behavior). Absent files are skipped; an
/// empty/whitespace/comment-only file yields an empty table.
///
/// # Errors
/// Returns [`ConfigError`] if a present config file cannot be read or contains invalid TOML.
pub fn layer_tables(inputs: &ConfigInputs) -> Result<Vec<ConfigLayer>, ConfigError> {
    let mut layers = Vec::new();
    for path in discovery::config_files(
        inputs.home.as_deref(),
        inputs.xdg_config_home.as_deref(),
        &inputs.cwd,
    ) {
        if let Some(table) = read_layer(&path)? {
            layers.push(ConfigLayer {
                source: path,
                table,
            });
        }
    }
    Ok(layers)
}

/// The built-in defaults layer.
fn default_table() -> Table {
    let mut table = Table::new();
    table.insert("model".to_owned(), Value::String(DEFAULT_MODEL.to_owned()));
    table.insert("agent".to_owned(), Value::String(DEFAULT_AGENT.to_owned()));
    table.insert("max_steps".to_owned(), Value::Integer(DEFAULT_MAX_STEPS));
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

/// A failure loading, parsing, substituting into, or validating a configuration.
#[derive(Debug)]
pub enum ConfigError {
    /// A present config file could not be read.
    Read { path: PathBuf, source: io::Error },
    /// A config file contained invalid TOML.
    Parse { path: PathBuf, message: String },
    /// A config file could not be written (e.g. by [`persist_global_string`]).
    Write { path: PathBuf, source: io::Error },
    /// No global config location could be resolved (neither `$XDG_CONFIG_HOME` nor `$HOME` set).
    NoConfigHome,
    /// An `{env:NAME}` substitution referenced an environment variable that is not set.
    EnvMissing { key: String, var: String },
    /// A `{file:PATH}` substitution referenced a file that could not be read.
    FileRead {
        key: String,
        path: String,
        source: io::Error,
    },
    /// A typed config key held a value of the wrong type.
    TypeMismatch {
        key: String,
        expected: &'static str,
        found: String,
    },
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

    fn write(path: &Path, source: io::Error) -> Self {
        Self::Write {
            path: path.to_path_buf(),
            source,
        }
    }

    fn env_missing(key: &str, var: &str) -> Self {
        Self::EnvMissing {
            key: key.to_owned(),
            var: var.to_owned(),
        }
    }

    fn file_read(key: &str, path: &str, source: io::Error) -> Self {
        Self::FileRead {
            key: key.to_owned(),
            path: path.to_owned(),
            source,
        }
    }

    /// Build a type-mismatch error, describing the found value from a TOML value.
    fn type_mismatch(key: &str, expected: &'static str, found: &Value) -> Self {
        Self::type_mismatch_found(key, expected, describe(found.type_str()))
    }

    /// Build a type-mismatch error with an explicit description of what was found.
    fn type_mismatch_found(key: &str, expected: &'static str, found: impl Into<String>) -> Self {
        Self::TypeMismatch {
            key: key.to_owned(),
            expected,
            found: found.into(),
        }
    }

    /// The config file that caused a read/parse failure, if this error is file-bound.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Read { path, .. } | Self::Parse { path, .. } | Self::Write { path, .. } => {
                Some(path)
            }
            Self::EnvMissing { .. }
            | Self::FileRead { .. }
            | Self::TypeMismatch { .. }
            | Self::NoConfigHome => None,
        }
    }
}

/// Render a TOML type name (`type_str`) as an article-prefixed noun (e.g. `integer` ->
/// `an integer`) for human-readable diagnostics.
fn describe(type_str: &str) -> String {
    let article = match type_str.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    };
    format!("{article} {type_str}")
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
            Self::Write { path, source } => {
                write!(
                    formatter,
                    "failed to write config file {}: {source}",
                    path.display()
                )
            }
            Self::NoConfigHome => {
                write!(
                    formatter,
                    "cannot locate a global config directory (set HOME or XDG_CONFIG_HOME)"
                )
            }
            Self::EnvMissing { key, var } => {
                write!(
                    formatter,
                    "config key `{key}`: environment variable `{var}` is not set"
                )
            }
            Self::FileRead { key, path, source } => {
                write!(
                    formatter,
                    "config key `{key}`: cannot read substitution file `{path}`: {source}"
                )
            }
            Self::TypeMismatch {
                key,
                expected,
                found,
            } => {
                write!(
                    formatter,
                    "config key `{key}`: expected {expected}, but found {found}"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. }
            | Self::FileRead { source, .. }
            | Self::Write { source, .. } => Some(source),
            Self::Parse { .. }
            | Self::EnvMissing { .. }
            | Self::TypeMismatch { .. }
            | Self::NoConfigHome => None,
        }
    }
}

#[cfg(test)]
mod persist_mcp_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique, freshly created scratch HOME so the global config write is hermetic.
    fn scratch_home(tag: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cortex-settings-mcp-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("create scratch home");
        dir
    }

    fn inputs_with_home(home: &Path) -> ConfigInputs {
        ConfigInputs {
            home: Some(home.to_path_buf()),
            xdg_config_home: None,
            cwd: home.to_path_buf(),
            env: Vec::new(),
            cli_overrides: BTreeMap::new(),
        }
    }

    fn spec() -> McpServerSpec {
        McpServerSpec {
            transport: McpServerSpecTransport::Local,
            command: vec!["/abs/echo-server".to_owned(), "--flag".to_owned()],
            url: None,
            headers: Vec::new(),
            cwd: Some("/work".to_owned()),
            environment: vec![("FOO".to_owned(), "bar".to_owned())],
            timeout: None,
            enabled: true,
        }
    }

    #[test]
    fn add_persists_a_server_that_loads_back() {
        let home = scratch_home("add");
        let inputs = inputs_with_home(&home);

        let outcome = persist_global_mcp_server(&inputs, "demo", &spec(), false).expect("persist");
        assert!(matches!(outcome, McpServerPersist::Added(_)));

        // The written definition is read back through the normal load path under `mcp.demo`.
        let effective = load(inputs_with_home(&home)).expect("load");
        let mcp = effective.get("mcp").expect("mcp table present");
        let demo = mcp.get("demo").expect("demo entry present");
        assert_eq!(
            demo.get("command")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str()),
            Some("/abs/echo-server")
        );
        assert_eq!(demo.get("cwd").and_then(|v| v.as_str()), Some("/work"));
        assert_eq!(
            demo.get("environment")
                .and_then(|e| e.get("FOO"))
                .and_then(|v| v.as_str()),
            Some("bar")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn re_adding_an_identical_server_is_idempotent_unchanged() {
        let home = scratch_home("idem");
        let inputs = inputs_with_home(&home);
        persist_global_mcp_server(&inputs, "demo", &spec(), false).expect("first add");
        let again = persist_global_mcp_server(&inputs_with_home(&home), "demo", &spec(), false)
            .expect("second add");
        assert!(
            matches!(again, McpServerPersist::Unchanged(_)),
            "re-adding the same definition is idempotent"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn adding_a_different_server_under_an_existing_name_conflicts_without_corruption() {
        let home = scratch_home("conflict");
        persist_global_mcp_server(&inputs_with_home(&home), "demo", &spec(), false)
            .expect("first add");

        let mut different = spec();
        different.command = vec!["/abs/other-server".to_owned()];
        let conflict =
            persist_global_mcp_server(&inputs_with_home(&home), "demo", &different, false)
                .expect("conflict result");
        assert!(matches!(conflict, McpServerPersist::Conflict(_)));

        // The original definition is left intact (no corruption).
        let effective = load(inputs_with_home(&home)).expect("load");
        let command = effective
            .get("mcp")
            .and_then(|m| m.get("demo"))
            .and_then(|d| d.get("command"))
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str());
        assert_eq!(command, Some("/abs/echo-server"), "the original survives");

        // With overwrite, the new definition replaces the old.
        let overwritten =
            persist_global_mcp_server(&inputs_with_home(&home), "demo", &different, true)
                .expect("overwrite");
        assert!(matches!(overwritten, McpServerPersist::Added(_)));
        let effective = load(inputs_with_home(&home)).expect("reload");
        let command = effective
            .get("mcp")
            .and_then(|m| m.get("demo"))
            .and_then(|d| d.get("command"))
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str());
        assert_eq!(command, Some("/abs/other-server"), "force overwrites");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn add_preserves_other_existing_config_keys() {
        let home = scratch_home("preserve");
        let config_dir = home.join(".config").join("cortex");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        std::fs::write(
            config_dir.join("config.toml"),
            "model = \"anthropic/claude-sonnet-4\"\n",
        )
        .expect("seed config");

        persist_global_mcp_server(&inputs_with_home(&home), "demo", &spec(), false).expect("add");

        let effective = load(inputs_with_home(&home)).expect("load");
        assert_eq!(
            effective.get("model").and_then(|v| v.as_str()),
            Some("anthropic/claude-sonnet-4"),
            "the pre-existing model key must survive the mcp add"
        );
        assert!(effective.get("mcp").is_some(), "the mcp entry was added");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn add_can_persist_a_remote_server_with_headers() {
        let home = scratch_home("remote");
        let inputs = inputs_with_home(&home);
        let spec = McpServerSpec {
            transport: McpServerSpecTransport::Remote,
            command: Vec::new(),
            url: Some("http://127.0.0.1:3210/mcp".to_owned()),
            headers: vec![("X-Cortex".to_owned(), "yes".to_owned())],
            cwd: None,
            environment: Vec::new(),
            timeout: Some(7),
            enabled: true,
        };

        persist_global_mcp_server(&inputs, "remote", &spec, false).expect("persist");
        let effective = load(inputs_with_home(&home)).expect("load");
        let remote = effective
            .get("mcp")
            .and_then(|mcp| mcp.get("remote"))
            .expect("remote entry present");

        assert_eq!(remote.get("type").and_then(Value::as_str), Some("remote"));
        assert_eq!(
            remote.get("url").and_then(Value::as_str),
            Some("http://127.0.0.1:3210/mcp")
        );
        assert_eq!(
            remote
                .get("headers")
                .and_then(|headers| headers.get("X-Cortex"))
                .and_then(Value::as_str),
            Some("yes")
        );
        assert_eq!(remote.get("timeout").and_then(Value::as_integer), Some(7));
        let _ = std::fs::remove_dir_all(&home);
    }
}

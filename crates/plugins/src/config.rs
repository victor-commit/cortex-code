use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

/// Configured plugin entries resolved for a run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginConfig {
    entries: Vec<PluginSpec>,
}

impl PluginConfig {
    /// Parse the top-level `plugin` config value.
    ///
    /// Accepted forms are:
    ///
    /// - `plugin = ["./plugin.mjs"]`
    /// - `plugin = [["./plugin.mjs", { label = "demo" }]]`
    /// - `plugin = ["@scope/plugin-package"]`
    ///
    /// Local paths are resolved relative to the run working directory. Package specs stay as
    /// package specs until the runtime host resolves them through the external JavaScript runtime.
    pub fn from_toml_value(value: Option<&TomlValue>, cwd: &Path) -> Result<Self, String> {
        let Some(value) = value else {
            return Ok(Self::default());
        };
        let Some(items) = value.as_array() else {
            return Err("expected an array of local plugin paths or [path, options] pairs".into());
        };

        let mut entries = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            entries.push(parse_entry(item, index, cwd)?);
        }
        Ok(Self { entries })
    }

    /// Whether no plugins are configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The configured plugin entries, in deterministic config order.
    #[must_use]
    pub fn entries(&self) -> &[PluginSpec] {
        &self.entries
    }
}

/// One configured plugin module entry.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginSpec {
    raw_spec: String,
    path: PathBuf,
    directory: PathBuf,
    options: JsonValue,
    source: PluginSource,
}

impl PluginSpec {
    /// Build a local plugin spec by resolving `raw_spec` against `cwd`.
    pub fn local_file(
        raw_spec: impl Into<String>,
        options: JsonValue,
        cwd: &Path,
    ) -> Result<Self, String> {
        let raw_spec = raw_spec.into();
        let trimmed = raw_spec.trim();
        if trimmed.is_empty() {
            return Err("plugin path must not be empty".into());
        }
        if is_package_spec(trimmed) {
            return Err(format!(
                "plugin `{trimmed}` is a package spec; use PluginSpec::package for package plugins"
            ));
        }
        let path = PathBuf::from(trimmed);
        let path = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        Ok(Self {
            raw_spec: trimmed.to_owned(),
            path,
            directory: cwd.to_path_buf(),
            options,
            source: PluginSource::LocalFile,
        })
    }

    /// Build an npm-package plugin spec. The module path is resolved later by the runtime host.
    pub fn package(
        raw_spec: impl Into<String>,
        options: JsonValue,
        cwd: &Path,
    ) -> Result<Self, String> {
        let raw_spec = raw_spec.into();
        let trimmed = raw_spec.trim();
        if trimmed.is_empty() {
            return Err("plugin package spec must not be empty".into());
        }
        validate_package_spec(trimmed)?;
        Ok(Self {
            raw_spec: trimmed.to_owned(),
            path: package_placeholder_path(cwd, trimmed),
            directory: cwd.to_path_buf(),
            options,
            source: PluginSource::Package {
                spec: trimmed.to_owned(),
            },
        })
    }

    /// The spec text as it appeared in config.
    #[must_use]
    pub fn raw_spec(&self) -> &str {
        &self.raw_spec
    }

    /// The resolved module path.
    ///
    /// For local-file specs this is available immediately. For package specs this is populated by
    /// the runtime host after the package has been resolved or installed.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The run working directory supplied to the plugin input context.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The options object passed as the plugin function's second argument.
    #[must_use]
    pub fn options(&self) -> &JsonValue {
        &self.options
    }

    /// Whether this entry came from a package spec rather than a local file path.
    #[must_use]
    pub fn is_package(&self) -> bool {
        matches!(self.source, PluginSource::Package { .. })
    }

    /// The configured package spec, if this entry is package-distributed.
    #[must_use]
    pub fn package_spec(&self) -> Option<&str> {
        match &self.source {
            PluginSource::LocalFile => None,
            PluginSource::Package { spec } => Some(spec),
        }
    }

    /// Return a clone with the package spec pinned to a concrete resolved module path.
    #[must_use]
    pub fn with_resolved_path(&self, path: impl Into<PathBuf>) -> Self {
        let mut resolved = self.clone();
        resolved.path = path.into();
        resolved
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PluginSource {
    LocalFile,
    Package { spec: String },
}

fn parse_entry(item: &TomlValue, index: usize, cwd: &Path) -> Result<PluginSpec, String> {
    match item {
        TomlValue::String(spec) => {
            parse_spec(spec, JsonValue::Object(Default::default()), index, cwd)
        }
        TomlValue::Array(pair) => parse_pair(pair, index, cwd),
        other => Err(format!(
            "plugin[{index}]: expected a path string or [path, options] pair, found {}",
            other.type_str()
        )),
    }
}

fn parse_pair(pair: &[TomlValue], index: usize, cwd: &Path) -> Result<PluginSpec, String> {
    if pair.len() != 2 {
        return Err(format!(
            "plugin[{index}]: expected [path, options] with exactly two items"
        ));
    }
    let spec = pair[0]
        .as_str()
        .ok_or_else(|| format!("plugin[{index}]: pair first item must be a plugin spec string"))?;
    if !matches!(pair[1], TomlValue::Table(_)) {
        return Err(format!(
            "plugin[{index}]: pair second item must be an options table"
        ));
    }
    let options = serde_json::to_value(pair[1].clone())
        .map_err(|error| format!("plugin[{index}]: failed to convert options to JSON: {error}"))?;
    parse_spec(spec, options, index, cwd)
}

fn parse_spec(
    spec: &str,
    options: JsonValue,
    index: usize,
    cwd: &Path,
) -> Result<PluginSpec, String> {
    if is_package_spec(spec) {
        PluginSpec::package(spec, options, cwd)
    } else {
        PluginSpec::local_file(spec, options, cwd)
    }
    .map_err(|message| format!("plugin[{index}]: {message}"))
}

fn is_package_spec(spec: &str) -> bool {
    if spec.starts_with("file:") || spec.starts_with("npm:") {
        return true;
    }
    if spec.starts_with('@') {
        return true;
    }
    if spec.starts_with("./")
        || spec.starts_with("../")
        || spec.starts_with('/')
        || spec.starts_with('~')
    {
        return false;
    }
    let path = Path::new(spec);
    let has_multiple_components = path.components().count() > 1;
    let has_plugin_extension = matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("js" | "mjs" | "cjs" | "ts" | "mts" | "cts")
    );
    if has_multiple_components && has_plugin_extension {
        return false;
    }
    !matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("js" | "mjs" | "cjs" | "ts" | "mts" | "cts")
    )
}

fn validate_package_spec(spec: &str) -> Result<(), String> {
    if spec.chars().any(char::is_whitespace) {
        return Err(format!(
            "plugin package spec `{spec}` is invalid: package specs must not contain whitespace"
        ));
    }
    if spec == "npm:" || spec == "file:" {
        return Err(format!(
            "plugin package spec `{spec}` is invalid: missing package name or path"
        ));
    }
    if spec.starts_with('@') {
        let mut parts = spec.split('/');
        let scope = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if scope.len() <= 1 || name.is_empty() {
            return Err(format!(
                "plugin package spec `{spec}` is invalid: scoped packages must be @scope/name"
            ));
        }
    }
    Ok(())
}

fn package_placeholder_path(cwd: &Path, spec: &str) -> PathBuf {
    let sanitized: String = spec
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    cwd.join(".cortex")
        .join("plugins")
        .join("unresolved")
        .join(format!("{sanitized}.mjs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> PathBuf {
        PathBuf::from("/tmp/cortex-plugin-config")
    }

    #[test]
    fn parses_local_path_strings_and_pair_options() {
        let value: TomlValue = toml::from_str(
            r#"
plugin = [
  "./one.mjs",
  ["./two.mjs", { label = "L2", enabled = true }]
]
"#,
        )
        .unwrap();
        let config = PluginConfig::from_toml_value(value.get("plugin"), &cwd()).unwrap();

        assert_eq!(config.entries().len(), 2);
        assert_eq!(
            config.entries()[0].path(),
            Path::new("/tmp/cortex-plugin-config/one.mjs")
        );
        assert_eq!(
            config.entries()[1].options()["label"],
            JsonValue::String("L2".to_owned())
        );
        assert_eq!(
            config.entries()[1].options()["enabled"],
            JsonValue::Bool(true)
        );
    }

    #[test]
    fn parses_package_specs_for_runtime_resolution() {
        let value: TomlValue = toml::from_str(r#"plugin = ["some-package"]"#).unwrap();
        let config = PluginConfig::from_toml_value(value.get("plugin"), &cwd()).unwrap();

        assert_eq!(config.entries()[0].raw_spec(), "some-package");
        assert_eq!(config.entries()[0].package_spec(), Some("some-package"));
        assert!(config.entries()[0].is_package());
    }

    #[test]
    fn accepts_bare_file_names_and_nested_relative_paths_as_local_specs() {
        let value: TomlValue =
            toml::from_str(r#"plugin = ["plugin.mjs", "plugins/marker.js"]"#).unwrap();
        let config = PluginConfig::from_toml_value(value.get("plugin"), &cwd()).unwrap();

        assert_eq!(
            config.entries()[0].path(),
            Path::new("/tmp/cortex-plugin-config/plugin.mjs")
        );
        assert_eq!(
            config.entries()[1].path(),
            Path::new("/tmp/cortex-plugin-config/plugins/marker.js")
        );
    }

    #[test]
    fn absent_plugin_key_is_empty() {
        let config = PluginConfig::from_toml_value(None, &cwd()).unwrap();
        assert!(config.is_empty());
    }

    #[test]
    fn rejects_malformed_package_specs_with_indexed_error() {
        let value: TomlValue = toml::from_str(r#"plugin = ["bad package"]"#).unwrap();
        let err = PluginConfig::from_toml_value(value.get("plugin"), &cwd()).unwrap_err();
        assert!(
            err.contains("plugin[0]") && err.contains("whitespace"),
            "malformed package spec is indexed: {err}"
        );
    }
}

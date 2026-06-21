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
    ///
    /// Local paths are resolved relative to the run working directory. Package distribution and
    /// lockfiles are intentionally left to later plugin distribution work; this foundation is the
    /// local-file runtime contract.
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

/// One local plugin module entry.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginSpec {
    raw_spec: String,
    path: PathBuf,
    directory: PathBuf,
    options: JsonValue,
}

impl PluginSpec {
    /// Build a local plugin spec by resolving `raw_spec` against `cwd`.
    ///
    /// Package names are rejected for now because this feature's contract is explicitly the local
    /// file path form. Later distribution work can extend this parser without weakening the
    /// fail-closed behavior for local plugins.
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
        if looks_like_package_spec(trimmed) {
            return Err(format!(
                "plugin `{trimmed}` is not a local file path; this runtime foundation supports \
                 local plugin files such as ./plugin.mjs"
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
        })
    }

    /// The spec text as it appeared in config.
    #[must_use]
    pub fn raw_spec(&self) -> &str {
        &self.raw_spec
    }

    /// The resolved local file path.
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
}

fn parse_entry(item: &TomlValue, index: usize, cwd: &Path) -> Result<PluginSpec, String> {
    match item {
        TomlValue::String(path) => {
            PluginSpec::local_file(path, JsonValue::Object(Default::default()), cwd)
                .map_err(|message| format!("plugin[{index}]: {message}"))
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
    let path = pair[0]
        .as_str()
        .ok_or_else(|| format!("plugin[{index}]: pair first item must be a path string"))?;
    if !matches!(pair[1], TomlValue::Table(_)) {
        return Err(format!(
            "plugin[{index}]: pair second item must be an options table"
        ));
    }
    let options = serde_json::to_value(pair[1].clone())
        .map_err(|error| format!("plugin[{index}]: failed to convert options to JSON: {error}"))?;
    PluginSpec::local_file(path, options, cwd)
        .map_err(|message| format!("plugin[{index}]: {message}"))
}

fn looks_like_package_spec(spec: &str) -> bool {
    if spec.starts_with("./")
        || spec.starts_with("../")
        || spec.starts_with('/')
        || spec.starts_with('~')
    {
        return false;
    }
    let path = Path::new(spec);
    if path.components().count() > 1 {
        return false;
    }
    !matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("js" | "mjs" | "cjs" | "ts" | "mts" | "cts")
    )
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
    fn rejects_package_specs_until_distribution_support_lands() {
        let value: TomlValue = toml::from_str(r#"plugin = ["some-package"]"#).unwrap();
        let err = PluginConfig::from_toml_value(value.get("plugin"), &cwd()).unwrap_err();
        assert!(
            err.contains("local file path"),
            "package-like specs fail clearly: {err}"
        );
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
}

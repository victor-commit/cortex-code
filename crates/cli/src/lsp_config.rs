//! Parse typed LSP settings from the merged config into a runtime [`lsp::LspConfigSet`].

use std::collections::BTreeMap;

use settings::{ConfigError, ConfigInputs, EffectiveConfig};
use toml::Value;

/// Load LSP config from process-visible settings.
pub(crate) fn load_from_process() -> Result<lsp::LspConfigSet, ConfigError> {
    let inputs = ConfigInputs::from_process(std::collections::BTreeMap::new()).map_err(|err| {
        ConfigError::TypeMismatch {
            key: "cwd".to_owned(),
            expected: "a resolvable current working directory",
            found: err.to_string(),
        }
    })?;
    let effective = settings::load(inputs)?;
    from_effective(&effective)
}

/// Resolve the LSP server set from one effective config table.
pub(crate) fn from_effective(
    effective: &EffectiveConfig,
) -> Result<lsp::LspConfigSet, ConfigError> {
    let mut by_language = lsp::LspConfigSet::defaults()
        .servers()
        .iter()
        .cloned()
        .map(|server| (server.language_id().to_owned(), server))
        .collect::<BTreeMap<_, _>>();

    let Some(lsp_value) = effective.get("lsp") else {
        return Ok(lsp::LspConfigSet::from_servers(
            by_language.into_values().collect(),
        ));
    };

    match lsp_value {
        Value::Boolean(false) => return Ok(lsp::LspConfigSet::empty()),
        Value::Boolean(true) => {}
        Value::Table(table) => {
            if let Some(servers_value) = table.get("servers") {
                let Some(servers) = servers_value.as_table() else {
                    return Err(type_mismatch(
                        "lsp.servers",
                        "a table",
                        describe(servers_value),
                    ));
                };
                for (language_id, entry_value) in servers {
                    let key = format!("lsp.servers.{language_id}");
                    let Some(entry) = entry_value.as_table() else {
                        return Err(type_mismatch(&key, "a table", describe(entry_value)));
                    };
                    let disabled = bool_field(entry, "disabled", &key)?;
                    if disabled.unwrap_or(false) {
                        by_language.remove(language_id);
                        continue;
                    }

                    let existing = by_language.get(language_id).cloned();
                    let program = string_field(entry, "program", &key)?
                        .or_else(|| existing.as_ref().map(|server| server.program().to_owned()));
                    let program = match program {
                        Some(program) if !program.trim().is_empty() => program,
                        _ => {
                            return Err(type_mismatch(
                                format!("{key}.program"),
                                "a non-empty string",
                                "missing value",
                            ));
                        }
                    };

                    let args = string_array_field(entry, "args", &key)?
                        .or_else(|| existing.as_ref().map(|server| server.args().to_vec()))
                        .unwrap_or_default();
                    let extensions = string_array_field(entry, "extensions", &key)?
                        .or_else(|| existing.as_ref().map(|server| server.extensions().to_vec()))
                        .unwrap_or_default();
                    if extensions.is_empty() {
                        return Err(type_mismatch(
                            format!("{key}.extensions"),
                            "a non-empty array of strings",
                            "missing value",
                        ));
                    }
                    let env = string_table_field(entry, "env", &key)?
                        .or_else(|| existing.as_ref().map(|server| server.env().clone()))
                        .unwrap_or_default();

                    let merged = existing
                        .unwrap_or_else(|| lsp::LspServerConfig::new(language_id, &program))
                        .with_program(program)
                        .with_exact_args(args)
                        .with_exact_extensions(extensions)
                        .with_envs(env);
                    by_language.insert(language_id.to_owned(), merged);
                }
            }
        }
        other => return Err(type_mismatch("lsp", "a boolean or table", describe(other))),
    }

    Ok(lsp::LspConfigSet::from_servers(
        by_language.into_values().collect(),
    ))
}

fn describe(value: &Value) -> String {
    value.type_str().to_owned()
}

fn type_mismatch(
    key: impl Into<String>,
    expected: &'static str,
    found: impl Into<String>,
) -> ConfigError {
    ConfigError::TypeMismatch {
        key: key.into(),
        expected,
        found: found.into(),
    }
}

fn bool_field(
    table: &toml::value::Table,
    key: &str,
    root: &str,
) -> Result<Option<bool>, ConfigError> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| type_mismatch(format!("{root}.{key}"), "a boolean", describe(value)))
}

fn string_field(
    table: &toml::value::Table,
    key: &str,
    root: &str,
) -> Result<Option<String>, ConfigError> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|text| Some(text.to_owned()))
        .ok_or_else(|| type_mismatch(format!("{root}.{key}"), "a string", describe(value)))
}

fn string_array_field(
    table: &toml::value::Table,
    key: &str,
    root: &str,
) -> Result<Option<Vec<String>>, ConfigError> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(items) = value.as_array() else {
        return Err(type_mismatch(
            format!("{root}.{key}"),
            "an array of strings",
            describe(value),
        ));
    };
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(text) = item.as_str() else {
            return Err(type_mismatch(
                format!("{root}.{key}[{index}]"),
                "a string",
                describe(item),
            ));
        };
        out.push(text.to_owned());
    }
    Ok(Some(out))
}

fn string_table_field(
    table: &toml::value::Table,
    key: &str,
    root: &str,
) -> Result<Option<BTreeMap<String, String>>, ConfigError> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let Some(entries) = value.as_table() else {
        return Err(type_mismatch(
            format!("{root}.{key}"),
            "a table of strings",
            describe(value),
        ));
    };
    let mut out = BTreeMap::new();
    for (env_key, env_value) in entries {
        let Some(text) = env_value.as_str() else {
            return Err(type_mismatch(
                format!("{root}.{key}.{env_key}"),
                "a string",
                describe(env_value),
            ));
        };
        out.insert(env_key.to_owned(), text.to_owned());
    }
    Ok(Some(out))
}

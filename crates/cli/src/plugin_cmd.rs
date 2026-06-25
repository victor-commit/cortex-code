//! `cortex plugin`: project plugin installation helpers.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use errors::Exit;
use serde::Serialize;
use serde_json::json;
use settings::ConfigInputs;
use toml::Value as TomlValue;

use crate::cli::{OutputFormat, PluginArgs, PluginCommand, PluginInstallArgs, PluginSurfaceType};

pub fn run(args: PluginArgs) -> Exit {
    match args.command {
        PluginCommand::Install(args) => install(args),
    }
}

fn install(args: PluginInstallArgs) -> Exit {
    let spec = args.spec.trim();
    if spec.is_empty() {
        eprintln!("cortex plugin install: plugin spec must not be empty");
        return Exit::Usage;
    }

    let cwd = match ConfigInputs::from_process(BTreeMap::new()) {
        Ok(inputs) => inputs.cwd,
        Err(err) => {
            eprintln!("cortex plugin install: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };

    if let Err(message) = plugins::PluginSpec::package(spec, json!({}), &cwd) {
        eprintln!("cortex plugin install: {message}");
        return Exit::Usage;
    }

    let probe = match install_probe_config(&cwd, spec, args.plugin_type) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("cortex plugin install: {message}");
            return Exit::Usage;
        }
    };
    let surface = install_surface(args.plugin_type);
    if let Err(err) = plugins::RuntimeHost::new().load_config_for_surface_with_config(
        &probe,
        surface,
        serde_json::Value::Object(Default::default()),
    ) {
        eprintln!("cortex plugin install: {err}");
        return Exit::Failure;
    }

    let update = match update_project_plugin_config(&cwd, spec, args.plugin_type) {
        Ok(update) => update,
        Err(message) => {
            eprintln!("cortex plugin install: {message}");
            return Exit::Failure;
        }
    };

    let document = PluginInstallDocument {
        spec: spec.to_owned(),
        plugin_type: args.plugin_type.map(plugin_type_name).map(str::to_owned),
        surface: surface.to_owned(),
        config_path: update.config_path.display().to_string(),
        changed: update.changed,
        message: if update.changed {
            "plugin installed and project config updated"
        } else {
            "plugin already installed; lock/config remain deterministic"
        },
    };

    match args.format.format {
        OutputFormat::Text => emit_text(&format!(
            "{}: `{}` ({})\n",
            document.message, document.spec, document.surface
        )),
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&document),
    }
}

fn install_probe_config(
    cwd: &Path,
    spec: &str,
    plugin_type: Option<PluginSurfaceType>,
) -> Result<plugins::PluginConfig, String> {
    let item = plugin_entry(spec, plugin_type);
    let array = TomlValue::Array(vec![item]);
    plugins::PluginConfig::from_toml_value(Some(&array), cwd)
}

fn plugin_entry(spec: &str, plugin_type: Option<PluginSurfaceType>) -> TomlValue {
    if let Some(plugin_type) = plugin_type {
        let mut options = toml::map::Map::new();
        options.insert(
            "type".to_owned(),
            TomlValue::String(plugin_type_name(plugin_type).to_owned()),
        );
        TomlValue::Array(vec![
            TomlValue::String(spec.to_owned()),
            TomlValue::Table(options),
        ])
    } else {
        TomlValue::String(spec.to_owned())
    }
}

#[derive(Debug)]
struct ConfigUpdate {
    config_path: PathBuf,
    changed: bool,
}

fn update_project_plugin_config(
    cwd: &Path,
    spec: &str,
    plugin_type: Option<PluginSurfaceType>,
) -> Result<ConfigUpdate, String> {
    let config_path = cwd.join("cortex.toml");
    let mut root = read_or_default_config(&config_path)?;
    let plugin_value = root
        .entry("plugin".to_owned())
        .or_insert_with(|| TomlValue::Array(Vec::new()));
    let array = plugin_value
        .as_array_mut()
        .ok_or_else(|| "project `cortex.toml` key `plugin` must be an array".to_owned())?;

    let desired = plugin_entry(spec, plugin_type);
    let changed = if array
        .iter()
        .any(|existing| plugin_entries_equal(existing, spec, plugin_type))
    {
        false
    } else {
        array.push(desired);
        true
    };

    if changed {
        let mut text = toml::to_string_pretty(&root)
            .map_err(|err| format!("failed to serialize project config: {err}"))?;
        text.push('\n');
        std::fs::write(&config_path, text).map_err(|err| {
            format!(
                "failed to write project config at {}: {err}",
                config_path.display()
            )
        })?;
    }

    Ok(ConfigUpdate {
        config_path,
        changed,
    })
}

fn read_or_default_config(path: &Path) -> Result<toml::map::Map<String, TomlValue>, String> {
    if !path.exists() {
        return Ok(toml::map::Map::new());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    let value: TomlValue = toml::from_str(&text)
        .map_err(|err| format!("invalid TOML in {}: {err}", path.display()))?;
    value
        .as_table()
        .cloned()
        .ok_or_else(|| format!("project config {} must be a TOML table", path.display()))
}

fn plugin_entries_equal(
    value: &TomlValue,
    spec: &str,
    plugin_type: Option<PluginSurfaceType>,
) -> bool {
    let target_type = plugin_type.map(plugin_type_name);
    match value {
        TomlValue::String(existing) => existing == spec && target_type.is_none(),
        TomlValue::Array(pair) if pair.len() == 2 => {
            let Some(existing_spec) = pair[0].as_str() else {
                return false;
            };
            if existing_spec != spec {
                return false;
            }
            let existing_type = pair[1]
                .as_table()
                .and_then(|table| table.get("type"))
                .and_then(TomlValue::as_str);
            existing_type == target_type
        }
        _ => false,
    }
}

fn install_surface(plugin_type: Option<PluginSurfaceType>) -> &'static str {
    match plugin_type {
        Some(PluginSurfaceType::Tui) => "tui",
        Some(PluginSurfaceType::Run) | None => "run",
    }
}

fn plugin_type_name(plugin_type: PluginSurfaceType) -> &'static str {
    match plugin_type {
        PluginSurfaceType::Run => "run",
        PluginSurfaceType::Tui => "tui",
    }
}

#[derive(Serialize)]
struct PluginInstallDocument {
    spec: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_type: Option<String>,
    surface: String,
    config_path: String,
    changed: bool,
    message: &'static str,
}

fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex plugin install: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

//! `cortex plugin`: project plugin installation helpers.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use errors::Exit;
use serde::Serialize;
use serde_json::json;
use settings::ConfigInputs;
use toml::Value as TomlValue;

use crate::cli::{
    OutputFormat, PluginArgs, PluginCommand, PluginInstallArgs, PluginListArgs, PluginRemoveArgs,
    PluginSurfaceType, PluginUpdateArgs,
};

pub fn run(args: PluginArgs) -> Exit {
    match args.command {
        PluginCommand::Install(args) => install(args),
        PluginCommand::List(args) => list(args),
        PluginCommand::Update(args) => update(args),
        PluginCommand::Remove(args) => remove(args),
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

fn list(args: PluginListArgs) -> Exit {
    let cwd = match project_cwd("list") {
        Ok(cwd) => cwd,
        Err(exit) => return exit,
    };

    let entries = match configured_plugins(&cwd) {
        Ok(entries) => entries,
        Err(message) => {
            eprintln!("cortex plugin list: {message}");
            return Exit::Config;
        }
    };
    let locked = match plugins::locked_plugins(&cwd) {
        Ok(locked) => locked,
        Err(message) => {
            eprintln!("cortex plugin list: {message}");
            return Exit::Failure;
        }
    };

    let plugins_doc = entries
        .iter()
        .map(|entry| PluginListEntry {
            locked: entry
                .package_spec
                .as_deref()
                .and_then(|spec| locked.iter().find(|lock| lock.spec == spec))
                .map(LockedDoc::from),
            spec: entry.spec.clone(),
            plugin_type: entry.plugin_type.clone(),
            source: entry.source,
        })
        .collect::<Vec<_>>();

    let document = PluginListDocument {
        config_path: cwd.join("cortex.toml").display().to_string(),
        plugins: plugins_doc,
    };

    match args.format.format {
        OutputFormat::Text => {
            let mut body = String::new();
            if document.plugins.is_empty() {
                body.push_str("no plugins configured in project cortex.toml\n");
            } else {
                for entry in &document.plugins {
                    let pin = match &entry.locked {
                        Some(lock) => format!("\t{}@{}", lock.package_name, lock.version),
                        None => String::new(),
                    };
                    body.push_str(&format!("{}\t({}){pin}\n", entry.spec, entry.source));
                }
            }
            emit_text(&body)
        }
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&document),
    }
}

fn update(args: PluginUpdateArgs) -> Exit {
    let spec = args.spec.trim();
    if spec.is_empty() {
        eprintln!("cortex plugin update: plugin spec must not be empty");
        return Exit::Usage;
    }

    let cwd = match project_cwd("update") {
        Ok(cwd) => cwd,
        Err(exit) => return exit,
    };

    let entries = match configured_plugins(&cwd) {
        Ok(entries) => entries,
        Err(message) => {
            eprintln!("cortex plugin update: {message}");
            return Exit::Config;
        }
    };
    let Some(entry) = entries.iter().find(|entry| entry.spec == spec) else {
        eprintln!(
            "cortex plugin update: plugin `{spec}` is not installed in project config; run `cortex plugin install {spec}` first"
        );
        return Exit::Usage;
    };
    let Some(package_spec) = entry.package_spec.clone() else {
        eprintln!(
            "cortex plugin update: plugin `{spec}` is a local-file plugin loaded directly from disk; there is nothing to re-resolve"
        );
        return Exit::Usage;
    };
    let plugin_type = args.plugin_type.or_else(|| entry.surface_type());

    // Drop the existing pin so the next resolve re-pins against the current package.
    if let Err(message) = plugins::remove_locked_plugin(&cwd, &package_spec) {
        eprintln!("cortex plugin update: {message}");
        return Exit::Failure;
    }

    let probe = match install_probe_config(&cwd, spec, plugin_type) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("cortex plugin update: {message}");
            return Exit::Usage;
        }
    };
    let surface = install_surface(plugin_type);
    if let Err(err) = plugins::RuntimeHost::new().load_config_for_surface_with_config(
        &probe,
        surface,
        serde_json::Value::Object(Default::default()),
    ) {
        eprintln!("cortex plugin update: {err}");
        return Exit::Failure;
    }

    let locked = plugins::locked_plugins(&cwd)
        .ok()
        .and_then(|locked| locked.into_iter().find(|lock| lock.spec == package_spec));

    let document = PluginUpdateDocument {
        spec: spec.to_owned(),
        surface: surface.to_owned(),
        version: locked.as_ref().map(|lock| lock.version.clone()),
        message: "plugin re-resolved and lockfile pin refreshed",
        locked: locked.as_ref().map(LockedDoc::from),
    };

    match args.format.format {
        OutputFormat::Text => {
            let version = document
                .version
                .as_deref()
                .map_or_else(String::new, |version| format!(" -> {version}"));
            emit_text(&format!(
                "{}: `{}`{version}\n",
                document.message, document.spec
            ))
        }
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&document),
    }
}

fn remove(args: PluginRemoveArgs) -> Exit {
    let spec = args.spec.trim();
    if spec.is_empty() {
        eprintln!("cortex plugin remove: plugin spec must not be empty");
        return Exit::Usage;
    }

    let cwd = match project_cwd("remove") {
        Ok(cwd) => cwd,
        Err(exit) => return exit,
    };

    let config_path = cwd.join("cortex.toml");
    let config_changed = match remove_from_project_config(&config_path, spec) {
        Ok(changed) => changed,
        Err(message) => {
            eprintln!("cortex plugin remove: {message}");
            return Exit::Failure;
        }
    };
    let lock_removed = match plugins::remove_locked_plugin(&cwd, spec) {
        Ok(removed) => removed,
        Err(message) => {
            eprintln!("cortex plugin remove: {message}");
            return Exit::Failure;
        }
    };

    if !config_changed && !lock_removed {
        eprintln!(
            "cortex plugin remove: plugin `{spec}` was not found in project config or the lockfile"
        );
        return Exit::Usage;
    }

    let document = PluginRemoveDocument {
        spec: spec.to_owned(),
        config_path: config_path.display().to_string(),
        config_changed,
        lock_removed,
        message: "plugin removed from project config",
    };

    match args.format.format {
        OutputFormat::Text => emit_text(&format!(
            "{}: `{}` (config_changed={}, lock_removed={})\n",
            document.message, document.spec, document.config_changed, document.lock_removed
        )),
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&document),
    }
}

/// Resolve the project working directory for a plugin subcommand, mapping failures to an exit.
fn project_cwd(command: &str) -> Result<PathBuf, Exit> {
    match ConfigInputs::from_process(BTreeMap::new()) {
        Ok(inputs) => Ok(inputs.cwd),
        Err(err) => {
            eprintln!("cortex plugin {command}: cannot determine the working directory: {err}");
            Err(Exit::Failure)
        }
    }
}

/// One configured plugin parsed from project `cortex.toml`.
struct ConfiguredPlugin {
    spec: String,
    plugin_type: Option<String>,
    source: &'static str,
    package_spec: Option<String>,
}

impl ConfiguredPlugin {
    /// The declared surface type parsed back into the CLI enum, when it is one we recognize.
    fn surface_type(&self) -> Option<PluginSurfaceType> {
        match self.plugin_type.as_deref() {
            Some("tui") => Some(PluginSurfaceType::Tui),
            Some("run" | "headless") => Some(PluginSurfaceType::Run),
            _ => None,
        }
    }
}

/// Parse the project's configured plugins via the canonical plugin-config parser, preserving the
/// raw spec, declared type, and whether each entry is package-distributed (and thus lockable).
fn configured_plugins(cwd: &Path) -> Result<Vec<ConfiguredPlugin>, String> {
    let config_path = cwd.join("cortex.toml");
    let root = read_or_default_config(&config_path)?;
    let plugin_value = root.get("plugin").cloned();
    let config = plugins::PluginConfig::from_toml_value(plugin_value.as_ref(), cwd)?;
    Ok(config
        .entries()
        .iter()
        .map(|entry| ConfiguredPlugin {
            spec: entry.raw_spec().to_owned(),
            plugin_type: entry.declared_type().map(str::to_owned),
            source: if entry.is_package() {
                "package"
            } else {
                "local"
            },
            package_spec: entry.package_spec().map(str::to_owned),
        })
        .collect())
}

/// Remove every project-config plugin entry whose spec equals `spec` (any declared type).
///
/// Returns whether the project config changed. The config file is only rewritten when an entry was
/// actually removed, keeping unrelated formatting and the lockfile deterministic.
fn remove_from_project_config(config_path: &Path, spec: &str) -> Result<bool, String> {
    if !config_path.exists() {
        return Ok(false);
    }
    let mut root = read_or_default_config(config_path)?;
    let Some(plugin_value) = root.get_mut("plugin") else {
        return Ok(false);
    };
    let Some(array) = plugin_value.as_array_mut() else {
        return Err("project `cortex.toml` key `plugin` must be an array".to_owned());
    };
    let before = array.len();
    array.retain(|entry| entry_spec(entry).is_none_or(|existing| existing != spec));
    let changed = array.len() != before;
    if array.is_empty() {
        root.remove("plugin");
    }
    if changed {
        let mut text = toml::to_string_pretty(&root)
            .map_err(|err| format!("failed to serialize project config: {err}"))?;
        text.push('\n');
        std::fs::write(config_path, text).map_err(|err| {
            format!(
                "failed to write project config at {}: {err}",
                config_path.display()
            )
        })?;
    }
    Ok(changed)
}

/// Extract the spec string from a `plugin` array entry (a bare string or a `[spec, options]` pair).
fn entry_spec(value: &TomlValue) -> Option<&str> {
    match value {
        TomlValue::String(spec) => Some(spec.as_str()),
        TomlValue::Array(pair) if !pair.is_empty() => pair[0].as_str(),
        _ => None,
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

#[derive(Serialize)]
struct LockedDoc {
    package_name: String,
    version: String,
    integrity: String,
}

impl From<&plugins::LockedPlugin> for LockedDoc {
    fn from(lock: &plugins::LockedPlugin) -> Self {
        Self {
            package_name: lock.package_name.clone(),
            version: lock.version.clone(),
            integrity: lock.integrity.clone(),
        }
    }
}

#[derive(Serialize)]
struct PluginListEntry {
    spec: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    plugin_type: Option<String>,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    locked: Option<LockedDoc>,
}

#[derive(Serialize)]
struct PluginListDocument {
    config_path: String,
    plugins: Vec<PluginListEntry>,
}

#[derive(Serialize)]
struct PluginUpdateDocument {
    spec: String,
    surface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    locked: Option<LockedDoc>,
}

#[derive(Serialize)]
struct PluginRemoveDocument {
    spec: String,
    config_path: String,
    config_changed: bool,
    lock_removed: bool,
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

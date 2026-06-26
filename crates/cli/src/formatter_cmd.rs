//! `cortex formatter` command surface.

use std::io::{self, Write};
use std::path::PathBuf;

use errors::Exit;
use serde::Serialize;
use settings::{ConfigError, ConfigInputs};

use crate::cli::{FormatterArgs, FormatterCommand, FormatterStatusArgs, OutputFormat};

/// Dispatch `cortex formatter` subcommands.
pub fn run(args: FormatterArgs) -> Exit {
    match args.command {
        FormatterCommand::Status(args) => status(args),
    }
}

#[derive(Serialize)]
struct StatusRow {
    name: String,
    formatter: String,
    state: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<Vec<String>>,
    #[serde(default)]
    env_keys: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    marker_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    marker_found: Option<bool>,
}

fn status(args: FormatterStatusArgs) -> Exit {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let config = match load_formatter_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("cortex formatter status: {err}");
            return Exit::Config;
        }
    };
    let rows = format::FormatterRunner::with_config(&cwd, config)
        .status()
        .into_iter()
        .map(|row| StatusRow {
            name: row.name,
            formatter: row.formatter.id().to_owned(),
            state: row.state.label().to_owned(),
            source: row.source.label().to_owned(),
            command: row.command,
            env_keys: row.env_keys,
            extensions: row.extensions,
            marker_required: row.marker_required,
            marker_found: row.marker_found,
        })
        .collect::<Vec<_>>();

    emit_status(args.format.format, &rows)
}

fn load_formatter_config() -> Result<format::FormatterConfig, ConfigError> {
    let inputs = ConfigInputs::from_process(std::collections::BTreeMap::new()).map_err(|err| {
        ConfigError::TypeMismatch {
            key: "cwd".to_owned(),
            expected: "a resolvable current working directory",
            found: err.to_string(),
        }
    })?;
    let effective = settings::load(inputs)?;
    format::FormatterConfig::from_table(effective.table()).map_err(|message| {
        ConfigError::TypeMismatch {
            key: "formatter".to_owned(),
            expected: "a formatter config table",
            found: message,
        }
    })
}

fn emit_status(format: OutputFormat, rows: &[StatusRow]) -> Exit {
    match format {
        OutputFormat::Json | OutputFormat::Jsonl => match serde_json::to_string(rows) {
            Ok(mut body) => {
                body.push('\n');
                let _ = io::stdout().write_all(body.as_bytes());
                Exit::Success
            }
            Err(err) => {
                eprintln!("cortex formatter status: failed to serialize output: {err}");
                Exit::Failure
            }
        },
        OutputFormat::Text => {
            for row in rows {
                println!("{}\t{}\tsource={}", row.name, row.state, row.source);
                if let Some(command) = &row.command {
                    println!("  command: {}", command.join(" "));
                }
                if !row.env_keys.is_empty() {
                    println!("  env: {}", row.env_keys.join(", "));
                }
                if !row.extensions.is_empty() {
                    println!("  extensions: {}", row.extensions.join(", "));
                }
                if row.marker_required {
                    let marker = if row.marker_found == Some(true) {
                        "present"
                    } else {
                        "missing"
                    };
                    println!("  marker: {marker}");
                }
            }
            Exit::Success
        }
    }
}

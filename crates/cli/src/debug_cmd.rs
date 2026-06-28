//! `cortex debug`: read-only diagnostics for the database, config, LSP, audit log, and runtime.
//!
//! Every subcommand is read-only and redaction-safe by default. `debug db` opens the database
//! read-only so it cannot change its checksum; `debug config` reuses the redacted effective-config
//! dump; `debug lsp` reuses the language-server status report; `debug audit` prints durable audit
//! records with secrets scrubbed; `debug info` reports runtime status (version, resolved paths,
//! build features, and the `CORTEX_NO_NET` kill-switch).

use std::fs;
use std::io::{self, Write};

use db::{Database, DbPathInputs};
use errors::Exit;
use serde::Serialize;

use crate::cli::{
    ConfigArgs, DebugArgs, DebugAuditArgs, DebugCommand, DebugDbArgs, DebugInfoArgs, FormatArgs,
    LspArgs, LspCommand, LspStatusArgs, OutputFormat,
};

pub fn run(args: DebugArgs) -> Exit {
    match args.command {
        DebugCommand::Db(db_args) => run_db(db_args),
        DebugCommand::Config(config_args) => crate::config::run(ConfigArgs {
            format: config_args.format.format,
            model: None,
        }),
        DebugCommand::Lsp(lsp_args) => crate::lsp_cmd::run(LspArgs {
            command: LspCommand::Status(LspStatusArgs {
                format: FormatArgs {
                    format: lsp_args.format.format,
                },
            }),
        }),
        DebugCommand::Audit(audit_args) => run_audit(audit_args),
        DebugCommand::Info(info_args) => run_info(info_args),
    }
}

// ---------------------------------------------------------------------------
// debug db
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct DbReport {
    #[serde(rename = "type")]
    kind: &'static str,
    path: String,
    schema_version: i64,
    latest_schema_version: i64,
    counts: Vec<TableCount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    integrity: Option<String>,
}

#[derive(Serialize)]
struct TableCount {
    table: String,
    rows: i64,
}

fn run_db(args: DebugDbArgs) -> Exit {
    let Some(path) = DbPathInputs::from_process().resolve() else {
        eprintln!(
            "cortex debug db: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Exit::Failure;
    };
    if !path.exists() {
        eprintln!(
            "cortex debug db: no database at {} (run a command that persists state first)",
            path.display()
        );
        return Exit::Failure;
    }

    // Read-only open: no migrations, no journal-mode switch, no writes — the database checksum is
    // left untouched.
    let db = match Database::open_read_only(&path) {
        Ok(db) => db,
        Err(err) => {
            eprintln!("cortex debug db: {err}");
            return Exit::Failure;
        }
    };

    let schema_version = match db.schema_version() {
        Ok(version) => version,
        Err(err) => {
            eprintln!("cortex debug db: {err}");
            return Exit::Failure;
        }
    };
    let counts = match db.table_counts() {
        Ok(counts) => counts
            .into_iter()
            .map(|(table, rows)| TableCount { table, rows })
            .collect(),
        Err(err) => {
            eprintln!("cortex debug db: {err}");
            return Exit::Failure;
        }
    };
    let integrity = if args.integrity {
        match db.integrity_check() {
            Ok(result) => Some(result),
            Err(err) => {
                eprintln!("cortex debug db: {err}");
                return Exit::Failure;
            }
        }
    } else {
        None
    };

    let report = DbReport {
        kind: "debug-db",
        path: path.display().to_string(),
        schema_version,
        latest_schema_version: db::latest_version(),
        counts,
        integrity,
    };

    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json("debug db", &report),
        OutputFormat::Text => {
            let mut out = String::new();
            out.push_str(&format!("path: {}\n", report.path));
            out.push_str(&format!(
                "schema_version: {} (latest {})\n",
                report.schema_version, report.latest_schema_version
            ));
            for count in &report.counts {
                out.push_str(&format!("{}: {}\n", count.table, count.rows));
            }
            if let Some(integrity) = &report.integrity {
                out.push_str(&format!("integrity: {integrity}\n"));
            }
            emit_text(&out)
        }
    }
}

// ---------------------------------------------------------------------------
// debug audit
// ---------------------------------------------------------------------------

fn run_audit(args: DebugAuditArgs) -> Exit {
    let path = std::env::var_os(audit::AUDIT_LOG_ENV)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .join(".cortex")
                .join("audit.jsonl")
        });

    let records: Vec<String> = match fs::read_to_string(&path) {
        Ok(contents) => contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(redaction::redact)
            .collect(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(err) => {
            eprintln!("cortex debug audit: cannot read {}: {err}", path.display());
            return Exit::Failure;
        }
    };

    let start = if args.limit == 0 {
        0
    } else {
        records.len().saturating_sub(args.limit)
    };
    let window = &records[start..];

    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => {
            let values: Vec<serde_json::Value> = window
                .iter()
                .map(|line| {
                    serde_json::from_str(line)
                        .unwrap_or_else(|_| serde_json::Value::String(line.clone()))
                })
                .collect();
            emit_json("debug audit", &values)
        }
        OutputFormat::Text => {
            let mut out = String::new();
            for line in window {
                out.push_str(line);
                out.push('\n');
            }
            emit_text(&out)
        }
    }
}

// ---------------------------------------------------------------------------
// debug info
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct InfoReport {
    #[serde(rename = "type")]
    kind: &'static str,
    version: &'static str,
    cwd: String,
    db_path: Option<String>,
    config_dir: Option<String>,
    network_feature: bool,
    server_feature: bool,
    no_net: bool,
    pure_plugins: bool,
}

fn run_info(args: DebugInfoArgs) -> Exit {
    let cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let db_path = DbPathInputs::from_process()
        .resolve()
        .map(|path| path.display().to_string());
    let config_dir = cortex_config_dir().map(|path| path.display().to_string());

    let report = InfoReport {
        kind: "debug-info",
        version: env!("CARGO_PKG_VERSION"),
        cwd,
        db_path,
        config_dir,
        network_feature: crate::feature_gate::network_enabled(),
        server_feature: crate::feature_gate::server_enabled(),
        no_net: redaction::guard_outbound("probe").is_err(),
        pure_plugins: crate::plugin_bridge::env_pure_enabled(),
    };

    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json("debug info", &report),
        OutputFormat::Text => {
            let mut out = String::new();
            out.push_str(&format!("version: {}\n", report.version));
            out.push_str(&format!("cwd: {}\n", report.cwd));
            if let Some(db_path) = &report.db_path {
                out.push_str(&format!("db_path: {db_path}\n"));
            }
            if let Some(config_dir) = &report.config_dir {
                out.push_str(&format!("config_dir: {config_dir}\n"));
            }
            out.push_str(&format!("network_feature: {}\n", report.network_feature));
            out.push_str(&format!("server_feature: {}\n", report.server_feature));
            out.push_str(&format!("no_net: {}\n", report.no_net));
            out.push_str(&format!("pure_plugins: {}\n", report.pure_plugins));
            emit_text(&out)
        }
    }
}

/// The Cortex config directory: `${XDG_CONFIG_HOME:-$HOME/.config}/cortex`.
fn cortex_config_dir() -> Option<std::path::PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(std::path::PathBuf::from(xdg).join("cortex"));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| {
            std::path::PathBuf::from(home)
                .join(".config")
                .join("cortex")
        })
}

// ---------------------------------------------------------------------------
// shared output helpers
// ---------------------------------------------------------------------------

fn emit_json<T: Serialize>(command: &str, value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex {command}: failed to serialize output: {err}");
            Exit::Failure
        }
    }
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

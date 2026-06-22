//! Minimal durable audit records needed by plugin execution-control hooks.
//!
//! The full M5 audit sink coverage feature owns broad action coverage. This module provides the
//! concrete denial record required when a plugin vetoes a tool before execution, using a stable
//! JSONL line so validators can inspect the denial without depending on in-memory events.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

#[derive(Serialize)]
struct PluginToolDenial<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    outcome: &'a str,
    session: &'a str,
    call_id: &'a str,
    tool: &'a str,
    plugin: &'a str,
    hook: &'a str,
    reason: &'a str,
    arguments: String,
    timestamp_ms: u128,
}

/// Record a plugin `tool.execute.before` veto as a denial in the durable audit sink.
pub(crate) fn record_plugin_tool_denial(
    session: &str,
    call_id: &str,
    tool: &str,
    plugin: &str,
    reason: &str,
    arguments: &str,
) {
    let Some(path) = audit_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let record = PluginToolDenial {
        kind: "plugin.tool.denial",
        outcome: "denied",
        session,
        call_id,
        tool,
        plugin,
        hook: "tool.execute.before",
        reason,
        arguments: redaction::redact(arguments),
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    };
    let Ok(line) = serde_json::to_string(&record) else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

fn audit_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CORTEX_AUDIT_LOG")
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    let cwd = std::env::current_dir().ok()?;
    Some(cwd.join(".cortex").join("audit.jsonl"))
}

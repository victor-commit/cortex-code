//! Durable JSONL audit sink for real action records.

use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Number, Value};

/// Environment variable overriding the audit JSONL path.
pub const AUDIT_LOG_ENV: &str = "CORTEX_AUDIT_LOG";

/// Shared audit sink handle used by surfaces and tools.
pub type SharedAuditSink = Arc<AuditSink>;

/// A durable, append-only JSONL audit sink scoped to one session.
#[derive(Debug)]
pub struct AuditSink {
    path: PathBuf,
    session: String,
    sequence: AtomicU64,
    write_lock: Mutex<()>,
}

impl AuditSink {
    /// Create an audit sink at `path` for `session`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, session: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            session: session.into(),
            sequence: AtomicU64::new(0),
            write_lock: Mutex::new(()),
        }
    }

    /// Resolve the sink path from `CORTEX_AUDIT_LOG`, or default to `<cwd>/.cortex/audit.jsonl`.
    #[must_use]
    pub fn from_env_or_cwd(session: impl Into<String>, cwd: &Path) -> Self {
        let path = std::env::var_os(AUDIT_LOG_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.join(".cortex").join("audit.jsonl"));
        Self::new(path, session)
    }

    /// The JSONL path this sink appends to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The session id carried on every record written by this sink.
    #[must_use]
    pub fn session(&self) -> &str {
        &self.session
    }

    /// Record a permission decision with the configured action and final outcome.
    pub fn record_permission_decision(
        &self,
        tool: &str,
        pattern: &str,
        request: &str,
        decision: &str,
        outcome: &str,
    ) {
        let mut fields = Map::new();
        insert_str(&mut fields, "tool", tool);
        insert_str(&mut fields, "pattern", pattern);
        insert_str(&mut fields, "request", request);
        insert_str(&mut fields, "decision", decision);
        insert_str(&mut fields, "outcome", outcome);
        self.write("permission_decision", fields);
    }

    /// Record a shell command execution.
    pub fn record_shell_execution(
        &self,
        command: &str,
        cwd: &Path,
        exit_status: Option<i32>,
        outcome: &str,
    ) {
        let mut fields = Map::new();
        insert_str(&mut fields, "tool", "bash");
        insert_str(&mut fields, "command", command);
        insert_str(&mut fields, "cwd", cwd.to_string_lossy());
        insert_optional_i64(&mut fields, "exit_status", exit_status.map(i64::from));
        insert_str(&mut fields, "outcome", outcome);
        self.write("shell_execution", fields);
    }

    /// Record a successful mutating file operation.
    pub fn record_file_mutation(&self, path: &Path, operation: &str, detail: Option<&str>) {
        let mut fields = Map::new();
        insert_str(&mut fields, "path", path.to_string_lossy());
        insert_str(&mut fields, "operation", operation);
        insert_str(&mut fields, "op", operation);
        if let Some(detail) = detail {
            insert_str(&mut fields, "detail", detail);
        }
        self.write("file_mutation", fields);
    }

    /// Record an outbound network call made by a provider or network tool.
    #[allow(clippy::too_many_arguments)]
    pub fn record_network_call(
        &self,
        actor: &str,
        provider: Option<&str>,
        model: Option<&str>,
        method: &str,
        host: &str,
        endpoint: &str,
        status: Option<u16>,
        outcome: &str,
    ) {
        let mut fields = Map::new();
        insert_str(&mut fields, "actor", actor);
        insert_str(&mut fields, "tool", actor);
        if let Some(provider) = provider {
            insert_str(&mut fields, "provider", provider);
        }
        if let Some(model) = model {
            insert_str(&mut fields, "model", model);
        }
        insert_str(&mut fields, "method", method);
        insert_str(&mut fields, "host", host);
        insert_str(&mut fields, "endpoint", endpoint);
        insert_optional_i64(&mut fields, "status", status.map(i64::from));
        insert_str(&mut fields, "outcome", outcome);
        self.write("network_call", fields);
    }

    /// Record a real MCP `tools/call` invocation.
    pub fn record_mcp_tool_call(
        &self,
        server: &str,
        tool: &str,
        qualified_tool: &str,
        arguments: &str,
        outcome: &str,
    ) {
        let mut fields = Map::new();
        insert_str(&mut fields, "server", server);
        insert_str(&mut fields, "tool", tool);
        insert_str(&mut fields, "qualified_tool", qualified_tool);
        insert_str(
            &mut fields,
            "arguments_summary",
            argument_summary(arguments),
        );
        insert_str(&mut fields, "outcome", outcome);
        self.write("mcp_tool_call", fields);
    }

    /// Record a configured plugin load observed by the external runtime.
    pub fn record_plugin_load(&self, plugin: &str, hooks: &[String], outcome: &str) {
        let mut fields = Map::new();
        insert_str(&mut fields, "plugin", plugin);
        fields.insert(
            "hooks".to_owned(),
            Value::Array(
                hooks
                    .iter()
                    .map(|hook| Value::String(hook.clone()))
                    .collect(),
            ),
        );
        insert_str(&mut fields, "outcome", outcome);
        self.write("plugin_load", fields);
    }

    /// Record a behavior-affecting plugin hook invocation.
    pub fn record_plugin_hook(
        &self,
        plugin: &str,
        hook: &str,
        action: &str,
        outcome: &str,
        detail: Option<&str>,
    ) {
        let mut fields = Map::new();
        insert_str(&mut fields, "plugin", plugin);
        insert_str(&mut fields, "hook", hook);
        insert_str(&mut fields, "action", action);
        insert_str(&mut fields, "outcome", outcome);
        if let Some(detail) = detail {
            insert_str(&mut fields, "detail", detail);
            insert_str(&mut fields, "reason", detail);
        }
        self.write("plugin_hook", fields);
    }

    /// Record a plugin tool execution through the real JS runtime.
    pub fn record_plugin_tool_execution(
        &self,
        plugin: &str,
        tool: &str,
        arguments: &str,
        outcome: &str,
    ) {
        let mut fields = Map::new();
        insert_str(&mut fields, "plugin", plugin);
        insert_str(&mut fields, "hook", "tool.execute");
        insert_str(&mut fields, "tool", tool);
        insert_str(
            &mut fields,
            "arguments_summary",
            argument_summary(arguments),
        );
        insert_str(&mut fields, "outcome", outcome);
        self.write("plugin_tool_execution", fields);
    }

    /// Record the compatibility plugin veto denial shape used by earlier M5 tests.
    pub fn record_plugin_tool_denial(
        &self,
        call_id: &str,
        tool: &str,
        plugin: &str,
        reason: &str,
        arguments: &str,
    ) {
        let mut fields = Map::new();
        insert_str(&mut fields, "outcome", "denied");
        insert_str(&mut fields, "call_id", call_id);
        insert_str(&mut fields, "tool", tool);
        insert_str(&mut fields, "plugin", plugin);
        insert_str(&mut fields, "hook", "tool.execute.before");
        insert_str(&mut fields, "reason", reason);
        insert_str(&mut fields, "arguments", arguments);
        self.write("plugin.tool.denial", fields);
    }

    fn write(&self, kind: &str, mut fields: Map<String, Value>) {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut record = Map::new();
        insert_str(&mut record, "type", kind);
        insert_str(&mut record, "session", &self.session);
        record.insert(
            "sequence".to_owned(),
            Value::Number(Number::from(
                self.sequence.fetch_add(1, Ordering::SeqCst) + 1,
            )),
        );
        record.insert(
            "timestamp_ms".to_owned(),
            Value::Number(Number::from(timestamp_ms())),
        );
        for (key, value) in std::mem::take(&mut fields) {
            record.insert(key, redact_value(value));
        }

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(line) = serde_json::to_string(&Value::Object(record)) else {
            return;
        };
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Read a JSONL audit sink into parsed values.
///
/// Empty lines are ignored. A malformed line returns the JSON parser error.
pub fn read_jsonl(path: &Path) -> io::Result<Vec<Value>> {
    let file = std::fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str(&line).map_err(io::Error::other)?;
        records.push(value);
    }
    Ok(records)
}

/// Extract the host name from an HTTP(S) endpoint string.
#[must_use]
pub fn host_from_endpoint(endpoint: &str) -> Option<String> {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

/// Produce a bounded, redacted argument summary for audit records.
#[must_use]
pub fn argument_summary(arguments: &str) -> String {
    let rendered = match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(map)) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            format!(
                "object keys: {}; json: {}",
                keys.join(","),
                Value::Object(map)
            )
        }
        Ok(value) => value.to_string(),
        Err(_) => arguments.to_owned(),
    };
    let redacted = redaction::redact(&rendered);
    const LIMIT: usize = 512;
    if redacted.chars().count() > LIMIT {
        let truncated = redacted.chars().take(LIMIT).collect::<String>();
        format!("{truncated}...[truncated]")
    } else {
        redacted
    }
}

fn insert_str(map: &mut Map<String, Value>, key: &str, value: impl AsRef<str>) {
    map.insert(key.to_owned(), Value::String(value.as_ref().to_owned()));
}

fn insert_optional_i64(map: &mut Map<String, Value>, key: &str, value: Option<i64>) {
    match value {
        Some(value) => {
            map.insert(key.to_owned(), Value::Number(Number::from(value)));
        }
        None => {
            map.insert(key.to_owned(), Value::Null);
        }
    }
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::String(value) => Value::String(redaction::redact(&value)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, redact_value(value)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cortex-audit-{tag}-{}-{}.jsonl",
            std::process::id(),
            timestamp_ms()
        ))
    }

    #[test]
    fn writes_parseable_jsonl_with_session_and_sequence() {
        let path = temp_log("basic");
        let sink = AuditSink::new(&path, "ses_audit");

        sink.record_permission_decision("bash", "echo *", "echo ok", "allow", "allowed");
        sink.record_shell_execution("echo ok", Path::new("/tmp/project"), Some(0), "success");

        let records = read_jsonl(&path).expect("read records");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["type"], "permission_decision");
        assert_eq!(records[0]["session"], "ses_audit");
        assert_eq!(records[0]["sequence"], 1);
        assert_eq!(records[1]["type"], "shell_execution");
        assert_eq!(records[1]["sequence"], 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn redacts_secret_shaped_strings_in_any_field() {
        let path = temp_log("redact");
        let sink = AuditSink::new(&path, "ses_redact");
        let secret = format!("sk-{}-{}", "or-v1", "abcdef01".repeat(8));

        sink.record_network_call(
            "llm",
            Some("openrouter"),
            Some(&secret),
            "POST",
            "openrouter.ai",
            &format!("https://openrouter.ai/{secret}"),
            None,
            "success",
        );

        let raw = std::fs::read_to_string(&path).expect("read raw");
        assert!(!raw.contains(&secret));
        assert!(raw.contains(redaction::PLACEHOLDER));
        let _ = std::fs::remove_file(path);
    }
}

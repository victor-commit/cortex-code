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
    turn_id: String,
    parent_session: Option<String>,
    sequence: Arc<AtomicU64>,
    write_lock: Arc<Mutex<()>>,
    extra_secrets: Arc<Mutex<Vec<String>>>,
}

impl AuditSink {
    /// Create an audit sink at `path` for `session`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, session: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            session: session.into(),
            turn_id: new_turn_id(),
            parent_session: None,
            sequence: Arc::new(AtomicU64::new(0)),
            write_lock: Arc::new(Mutex::new(())),
            extra_secrets: Arc::new(Mutex::new(secret_values_from_process_env())),
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

    /// The turn id carried on every record written by this sink.
    #[must_use]
    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    /// Build a child sink for a subagent session using the same path and monotonic sequence as
    /// the parent, while attributing records to the child session and linking back to the parent.
    #[must_use]
    pub fn child_session(&self, session: impl Into<String>) -> Self {
        Self {
            path: self.path.clone(),
            session: session.into(),
            turn_id: new_turn_id(),
            parent_session: Some(self.session.clone()),
            sequence: self.sequence.clone(),
            write_lock: self.write_lock.clone(),
            extra_secrets: self.extra_secrets.clone(),
        }
    }

    /// Register a secret literal discovered outside the process environment, such as a plugin
    /// `shell.env` value. Future audit records replace this exact value wherever it appears.
    pub fn register_secret(&self, value: impl AsRef<str>) {
        let Some(secret) = normalize_secret_literal(value.as_ref()) else {
            return;
        };
        if let Ok(mut secrets) = self.extra_secrets.lock()
            && !secrets.iter().any(|existing| existing == &secret)
        {
            secrets.push(secret);
        }
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
        insert_str(&mut fields, "outcome", "success");
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

    /// Record an LLM provider request without persisting the raw request body or authorization.
    #[allow(clippy::too_many_arguments)]
    pub fn record_provider_request(
        &self,
        provider: &str,
        model: &str,
        method: &str,
        host: &str,
        endpoint: &str,
        status: Option<u16>,
        duration_ms: u128,
        outcome: &str,
        extra_headers: &[(String, String)],
    ) {
        let mut fields = Map::new();
        insert_str(&mut fields, "actor", "llm");
        insert_str(&mut fields, "tool", "llm");
        insert_str(&mut fields, "kind", "provider_request");
        insert_str(&mut fields, "provider", provider);
        insert_str(&mut fields, "model", model);
        insert_str(&mut fields, "method", method);
        insert_str(&mut fields, "host", host);
        insert_str(&mut fields, "endpoint", endpoint);
        insert_optional_i64(&mut fields, "status", status.map(i64::from));
        fields.insert(
            "duration_ms".to_owned(),
            Value::Number(Number::from(u64::try_from(duration_ms).unwrap_or(u64::MAX))),
        );
        insert_str(&mut fields, "outcome", outcome);
        insert_str(&mut fields, "auth", redaction::PLACEHOLDER);
        insert_str(&mut fields, "body", redaction::PLACEHOLDER);

        let mut headers = Map::new();
        insert_str(&mut headers, "authorization", redaction::PLACEHOLDER);
        for (name, value) in extra_headers {
            headers.insert(name.clone(), Value::String(value.clone()));
        }
        fields.insert("headers".to_owned(), Value::Object(headers));

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

    /// Record a plugin `shell.env` result while preserving env field presence and redacting values
    /// whose names or shapes are secret-listed.
    pub fn record_plugin_shell_env(&self, plugin: &str, env: &[(String, String)], outcome: &str) {
        for (name, value) in env {
            if redaction::is_secret_env_var(name) || redaction::value_is_secret(value) {
                self.register_secret(value);
            }
        }

        let mut fields = Map::new();
        insert_str(&mut fields, "plugin", plugin);
        insert_str(&mut fields, "hook", "shell.env");
        insert_str(&mut fields, "action", "inject_env");
        insert_str(&mut fields, "outcome", outcome);
        insert_str(&mut fields, "detail", "environment variables provided");
        insert_str(&mut fields, "reason", "environment variables provided");

        let mut env_keys = env
            .iter()
            .map(|(name, _)| Value::String(name.clone()))
            .collect::<Vec<_>>();
        env_keys.sort_by(|left, right| {
            left.as_str()
                .unwrap_or_default()
                .cmp(right.as_str().unwrap_or_default())
        });
        fields.insert("env_keys".to_owned(), Value::Array(env_keys));

        let env_object = env
            .iter()
            .map(|(name, value)| (name.clone(), Value::String(value.clone())))
            .collect::<Map<_, _>>();
        fields.insert("env".to_owned(), Value::Object(env_object));

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
        record.insert("schema_version".to_owned(), Value::Number(Number::from(1)));
        insert_str(&mut record, "type", kind);
        insert_str(&mut record, "session", &self.session);
        insert_str(&mut record, "session_id", &self.session);
        insert_str(&mut record, "turn_id", &self.turn_id);
        if let Some(parent_session) = &self.parent_session {
            insert_str(&mut record, "parent_session", parent_session);
            insert_str(&mut record, "parent_session_id", parent_session);
        }
        record.insert(
            "sequence".to_owned(),
            Value::Number(Number::from(
                self.sequence.fetch_add(1, Ordering::SeqCst) + 1,
            )),
        );
        let now = timestamp_ms();
        record.insert("timestamp_ms".to_owned(), Value::Number(Number::from(now)));
        record.insert("timestamp".to_owned(), Value::Number(Number::from(now)));
        for (key, value) in std::mem::take(&mut fields) {
            record.insert(key, value);
        }
        record
            .entry("outcome".to_owned())
            .or_insert_with(|| Value::String("success".to_owned()));
        let record = redact_object(record, &self.secret_literals());

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

    fn secret_literals(&self) -> Vec<String> {
        let mut secrets = self
            .extra_secrets
            .lock()
            .map_or_else(|_| Vec::new(), |secrets| secrets.clone());
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        secrets.dedup();
        secrets
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
            let redacted = redact_value(Value::Object(map), &[]);
            format!("object keys: {}; json: {}", keys.join(","), redacted)
        }
        Ok(value) => redact_value(value, &[]).to_string(),
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

fn new_turn_id() -> String {
    static NEXT_TURN: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_TURN.fetch_add(1, Ordering::SeqCst);
    format!(
        "turn_{}_{}_{}",
        std::process::id(),
        timestamp_ms(),
        sequence
    )
}

fn secret_values_from_process_env() -> Vec<String> {
    let mut secrets = std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.to_string_lossy();
            let value = value.to_string_lossy();
            (redaction::is_secret_env_var(&name) || redaction::value_is_secret(&value))
                .then(|| normalize_secret_literal(&value))
                .flatten()
        })
        .collect::<Vec<_>>();
    secrets.sort();
    secrets.dedup();
    secrets
}

fn normalize_secret_literal(value: &str) -> Option<String> {
    const MIN_SECRET_LITERAL_LEN: usize = 4;
    let value = value.trim();
    if value.is_empty()
        || value == redaction::PLACEHOLDER
        || value.chars().count() < MIN_SECRET_LITERAL_LEN
    {
        return None;
    }
    Some(value.to_owned())
}

fn redact_object(map: Map<String, Value>, secrets: &[String]) -> Map<String, Value> {
    map.into_iter()
        .map(|(key, value)| {
            let redacted = redact_value_for_key(&key, value, secrets);
            (key, redacted)
        })
        .collect()
}

fn redact_value_for_key(key: &str, value: Value, secrets: &[String]) -> Value {
    if is_secret_field_name(key) {
        return Value::String(redaction::PLACEHOLDER.to_owned());
    }
    redact_value(value, secrets)
}

fn redact_value(value: Value, secrets: &[String]) -> Value {
    match value {
        Value::String(value) => Value::String(redact_text(&value, secrets)),
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| redact_value(value, secrets))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let redacted = redact_value_for_key(&key, value, secrets);
                    (key, redacted)
                })
                .collect(),
        ),
        other => other,
    }
}

fn redact_text(value: &str, secrets: &[String]) -> String {
    let trimmed = value.trim_start();
    let candidate = if trimmed.starts_with('{') || trimmed.starts_with('[') {
        serde_json::from_str::<Value>(value)
            .ok()
            .map(|parsed| redact_value(parsed, secrets).to_string())
    } else {
        None
    };
    let mut redacted = redaction::redact(candidate.as_deref().unwrap_or(value));
    for secret in secrets {
        if !secret.is_empty() && redacted.contains(secret) {
            redacted = redacted.replace(secret, redaction::PLACEHOLDER);
        }
    }
    redacted
}

fn is_secret_field_name(name: &str) -> bool {
    let normalized = name.replace(['-', '.'], "_").to_ascii_uppercase();
    matches!(
        normalized.as_str(),
        "AUTH" | "AUTHORIZATION" | "PROXY_AUTHORIZATION"
    ) || normalized.ends_with("_AUTH")
        || redaction::is_secret_env_var(&normalized)
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
        assert_eq!(records[0]["session_id"], "ses_audit");
        assert!(
            records[0]["turn_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
        assert!(records[0]["timestamp_ms"].as_u64().is_some());
        assert_eq!(records[0]["timestamp"], records[0]["timestamp_ms"]);
        assert_eq!(records[0]["sequence"], 1);
        assert_eq!(records[1]["type"], "shell_execution");
        assert_eq!(records[1]["sequence"], 2);
        assert_eq!(records[1]["turn_id"], records[0]["turn_id"]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn child_sink_shares_ordering_and_records_parent_linkage() {
        let path = temp_log("child");
        let sink = AuditSink::new(&path, "ses_parent");
        sink.record_permission_decision("task", "general", "general", "allow", "allowed");
        let child = sink.child_session("ses_child");
        child.record_shell_execution("echo child", Path::new("/tmp/project"), Some(0), "success");
        sink.record_shell_execution("echo parent", Path::new("/tmp/project"), Some(0), "success");

        let records = read_jsonl(&path).expect("read records");
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["sequence"], 1);
        assert_eq!(records[1]["sequence"], 2);
        assert_eq!(records[2]["sequence"], 3);
        assert_eq!(records[1]["session"], "ses_child");
        assert_eq!(records[1]["parent_session"], "ses_parent");
        assert_eq!(records[1]["parent_session_id"], "ses_parent");
        assert_ne!(records[1]["turn_id"], records[0]["turn_id"]);
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

    #[test]
    fn registered_secret_literals_are_redacted_from_later_records() {
        let path = temp_log("registered");
        let sink = AuditSink::new(&path, "ses_registered");
        let secret = "topsecret123";
        sink.register_secret(secret);

        sink.record_shell_execution(
            &format!("printf '%s\\n' {secret}"),
            Path::new("/tmp/project"),
            Some(0),
            "success",
        );

        let raw = std::fs::read_to_string(&path).expect("read raw");
        assert!(!raw.contains(secret), "{raw}");
        assert!(raw.contains(redaction::PLACEHOLDER), "{raw}");
        let records = read_jsonl(&path).expect("records");
        assert!(
            records[0]["command"]
                .as_str()
                .is_some_and(|command| command.contains(redaction::PLACEHOLDER)),
            "{records:#?}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn shell_env_records_presence_while_redacting_secret_named_values() {
        let path = temp_log("shell-env");
        let sink = AuditSink::new(&path, "ses_env");
        let secret = "topsecret123";

        sink.record_plugin_shell_env(
            "./plugin.mjs",
            &[
                ("SECRET".to_owned(), secret.to_owned()),
                ("VISIBLE_FLAG".to_owned(), "present".to_owned()),
            ],
            "success",
        );

        let raw = std::fs::read_to_string(&path).expect("read raw");
        assert!(!raw.contains(secret), "{raw}");
        let records = read_jsonl(&path).expect("records");
        let record = &records[0];
        assert_eq!(record["type"], "plugin_hook");
        assert_eq!(record["hook"], "shell.env");
        assert_eq!(record["env"]["SECRET"], redaction::PLACEHOLDER);
        assert_eq!(record["env"]["VISIBLE_FLAG"], "present");
        assert!(
            record["env_keys"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|key| key == "SECRET")),
            "{record:#?}"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn provider_request_records_metadata_and_redacted_auth_body_headers() {
        let path = temp_log("provider");
        let sink = AuditSink::new(&path, "ses_provider");
        let secret = "topsecret123";
        sink.register_secret(secret);

        sink.record_provider_request(
            "openrouter",
            "google/gemini-2.5-flash",
            "POST",
            "openrouter.ai",
            "https://openrouter.ai/api/v1/chat/completions",
            Some(200),
            42,
            "success",
            &[
                ("X-Trace".to_owned(), "abc".to_owned()),
                ("X-Secret-Token".to_owned(), secret.to_owned()),
            ],
        );

        let raw = std::fs::read_to_string(&path).expect("read raw");
        assert!(!raw.contains(secret), "{raw}");
        let records = read_jsonl(&path).expect("records");
        let record = &records[0];
        assert_eq!(record["type"], "network_call");
        assert_eq!(record["kind"], "provider_request");
        assert_eq!(record["model"], "google/gemini-2.5-flash");
        assert_eq!(record["host"], "openrouter.ai");
        assert_eq!(record["duration_ms"], 42);
        assert_eq!(record["auth"], redaction::PLACEHOLDER);
        assert_eq!(record["body"], redaction::PLACEHOLDER);
        assert_eq!(record["headers"]["authorization"], redaction::PLACEHOLDER);
        assert_eq!(record["headers"]["X-Trace"], "abc");
        assert_eq!(record["headers"]["X-Secret-Token"], redaction::PLACEHOLDER);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn argument_json_strings_redact_secret_named_fields() {
        let path = temp_log("arguments");
        let sink = AuditSink::new(&path, "ses_arguments");
        let secret = "topsecret123";

        sink.record_plugin_tool_denial(
            "call_a",
            "bash",
            "./plugin.mjs",
            "denied",
            &serde_json::json!({ "SECRET": secret, "visible": "ok" }).to_string(),
        );

        let raw = std::fs::read_to_string(&path).expect("read raw");
        assert!(!raw.contains(secret), "{raw}");
        let records = read_jsonl(&path).expect("records");
        assert!(
            records[0]["arguments"]
                .as_str()
                .is_some_and(|arguments| arguments.contains(redaction::PLACEHOLDER)),
            "{records:#?}"
        );
        let _ = std::fs::remove_file(path);
    }
}

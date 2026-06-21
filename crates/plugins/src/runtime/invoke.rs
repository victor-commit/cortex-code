use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::config::{PluginConfig, PluginSpec};
use crate::runtime::detection::{JsRuntime, RuntimeSelection, detect_runtime_in_path};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const RUNTIME_REQUIREMENT: &str = "Bun, Node, or Deno";
const RUNTIME_ORDER: &str = "bun, node, deno";

/// Host for loading configured local plugins through a real external JavaScript runtime.
#[derive(Clone, Debug)]
pub struct RuntimeHost {
    timeout: Duration,
    path_override: Option<OsString>,
}

impl Default for RuntimeHost {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            path_override: None,
        }
    }
}

impl RuntimeHost {
    /// Create a host with the default bounded plugin timeout.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the plugin child timeout. Values below one millisecond are promoted to one
    /// millisecond so a configured timeout always remains bounded.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.max(Duration::from_millis(1));
        self
    }

    /// Override `PATH` for runtime detection. This is used by tests and by callers that need a
    /// scrubbed runtime environment; normal runs use the process `PATH`.
    #[must_use]
    pub fn with_path(mut self, path: impl Into<OsString>) -> Self {
        self.path_override = Some(path.into());
        self
    }

    /// Load every configured plugin in order and return only their observable reports.
    ///
    /// A configured plugin fails closed: missing runtime, missing file, a runtime spawn failure, a
    /// timeout, or a plugin load throw all return [`PluginRuntimeError`] and stop further loading.
    pub fn load_config(
        &self,
        config: &PluginConfig,
    ) -> Result<Vec<PluginLoadReport>, PluginRuntimeError> {
        Ok(self
            .load_plugins(config)?
            .into_iter()
            .map(|loaded| loaded.report)
            .collect())
    }

    /// Load every configured plugin in order, preserving the original spec next to the report so
    /// callers can invoke later hooks or execute plugin-contributed tools.
    pub fn load_plugins(
        &self,
        config: &PluginConfig,
    ) -> Result<Vec<LoadedPlugin>, PluginRuntimeError> {
        if config.is_empty() {
            return Ok(Vec::new());
        }

        let runtime = self.detect_runtime().ok_or(PluginRuntimeError::NoRuntime)?;
        config
            .entries()
            .iter()
            .map(|plugin| self.load_plugin(plugin, &runtime))
            .collect()
    }

    /// Invoke a loaded plugin's `event` hook with a real event payload.
    pub fn invoke_event(
        &self,
        plugin: &PluginSpec,
        event: JsonValue,
    ) -> Result<PluginHookReport, PluginRuntimeError> {
        let runtime = self.detect_runtime().ok_or(PluginRuntimeError::NoRuntime)?;
        let (result, output) =
            self.invoke_plugin(plugin, &runtime, BridgeOperation::Event { event })?;
        if result.ok {
            Ok(PluginHookReport {
                plugin_path: plugin.path().to_path_buf(),
                runtime: runtime.runtime,
                runtime_pid: result.pid,
                invoked_hooks: result.invoked_hooks,
                stdout: redaction::redact(&String::from_utf8_lossy(&output.stdout)),
                stderr: redaction::redact(&String::from_utf8_lossy(&output.stderr)),
            })
        } else {
            Err(error_from_bridge_result(
                plugin.path(),
                &runtime,
                result,
                "event",
            ))
        }
    }

    /// Execute one plugin-contributed tool by re-loading the real plugin module in the detected JS
    /// runtime and calling that tool's real JavaScript body.
    pub fn execute_tool(
        &self,
        plugin: &PluginSpec,
        name: &str,
        arguments: JsonValue,
    ) -> Result<PluginToolExecution, PluginRuntimeError> {
        let runtime = self.detect_runtime().ok_or(PluginRuntimeError::NoRuntime)?;
        let (result, _output) = self.invoke_plugin(
            plugin,
            &runtime,
            BridgeOperation::ToolExecute {
                name: name.to_owned(),
                arguments,
            },
        )?;
        if result.ok {
            let raw = result.output.unwrap_or(JsonValue::Null);
            Ok(PluginToolExecution {
                content: output_content(&raw),
                raw,
            })
        } else {
            Err(error_from_bridge_result(
                plugin.path(),
                &runtime,
                result,
                "tool",
            ))
        }
    }

    /// Complete a turn through a plugin-declared provider/model. The provider's JavaScript
    /// `complete`/`generate` function produces the real assistant text for this run.
    pub fn complete_with_provider(
        &self,
        plugin: &PluginSpec,
        model: &str,
        messages: JsonValue,
    ) -> Result<String, PluginRuntimeError> {
        let runtime = self.detect_runtime().ok_or(PluginRuntimeError::NoRuntime)?;
        let (result, _output) = self.invoke_plugin(
            plugin,
            &runtime,
            BridgeOperation::ProviderComplete {
                model: model.to_owned(),
                messages,
            },
        )?;
        if result.ok {
            let raw = result.output.unwrap_or(JsonValue::Null);
            Ok(output_content(&raw))
        } else {
            Err(error_from_bridge_result(
                plugin.path(),
                &runtime,
                result,
                "provider",
            ))
        }
    }

    fn detect_runtime(&self) -> Option<RuntimeSelection> {
        if let Some(path) = self.path_override.as_deref() {
            detect_runtime_in_path(Some(path))
        } else {
            let path = std::env::var_os("PATH");
            detect_runtime_in_path(path.as_deref())
        }
    }

    fn load_plugin(
        &self,
        plugin: &PluginSpec,
        runtime: &RuntimeSelection,
    ) -> Result<LoadedPlugin, PluginRuntimeError> {
        let (result, output) = self.invoke_plugin(
            plugin,
            runtime,
            BridgeOperation::Load {
                config: JsonValue::Object(Default::default()),
            },
        )?;

        if result.ok {
            let report = PluginLoadReport {
                plugin_path: plugin.path().to_path_buf(),
                runtime: runtime.runtime,
                runtime_executable: runtime.executable.clone(),
                runtime_pid: result.pid,
                runtime_name: result.runtime.name,
                runtime_version: result.runtime.version,
                hooks: result.hooks,
                invoked_hooks: result.invoked_hooks,
                config_rules: result.registrations.config_rules,
                tools: result.registrations.tools,
                auth_methods: result.registrations.auth_methods,
                providers: result.registrations.providers,
                stdout: redaction::redact(&String::from_utf8_lossy(&output.stdout)),
                stderr: redaction::redact(&String::from_utf8_lossy(&output.stderr)),
            };
            Ok(LoadedPlugin {
                spec: plugin.clone(),
                report,
            })
        } else {
            Err(error_from_bridge_result(
                plugin.path(),
                runtime,
                result,
                "load",
            ))
        }
    }

    fn invoke_plugin(
        &self,
        plugin: &PluginSpec,
        runtime: &RuntimeSelection,
        operation: BridgeOperation,
    ) -> Result<(BridgeResult, ChildOutput), PluginRuntimeError> {
        if !plugin.path().is_file() {
            return Err(PluginRuntimeError::PluginFileMissing {
                path: plugin.path().to_path_buf(),
            });
        }

        let temp = InvocationTemp::create()?;
        let bridge_path = temp.path.join("plugin_bridge.mjs");
        let input_path = temp.path.join("input.json");
        let result_path = temp.path.join("result.json");
        std::fs::write(&bridge_path, BRIDGE_SCRIPT).map_err(|source| PluginRuntimeError::Io {
            action: "write plugin bridge",
            path: bridge_path.clone(),
            source,
        })?;
        std::fs::write(
            &input_path,
            serde_json::to_vec(&BridgeInput {
                plugin_path: plugin.path().to_string_lossy().into_owned(),
                directory: plugin.directory().to_string_lossy().into_owned(),
                options: plugin.options().clone(),
                operation,
            })?,
        )
        .map_err(|source| PluginRuntimeError::Io {
            action: "write plugin input",
            path: input_path.clone(),
            source,
        })?;

        let output = run_child(
            runtime,
            plugin.path(),
            &bridge_path,
            &input_path,
            &result_path,
            self.timeout,
        )?;
        let result = read_bridge_result(plugin.path(), runtime, &result_path, &output)?;
        let _ = std::fs::remove_dir_all(&temp.path);
        Ok((result, output))
    }
}

/// A configured plugin together with the facts observed when it loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedPlugin {
    /// The local plugin spec that can be re-invoked for later hooks/tool calls.
    pub spec: PluginSpec,
    /// Observable facts and registrations from loading the plugin.
    pub report: PluginLoadReport,
}

/// Observable facts from a successful plugin load.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginLoadReport {
    /// The local plugin file that was executed.
    pub plugin_path: PathBuf,
    /// The runtime chosen by the precedence list.
    pub runtime: JsRuntime,
    /// The exact runtime executable path spawned.
    pub runtime_executable: PathBuf,
    /// The direct runtime process PID, as observed inside JavaScript.
    pub runtime_pid: Option<u32>,
    /// Runtime name reported by JavaScript (`bun`, `node`, or `deno`).
    pub runtime_name: String,
    /// Runtime version reported by JavaScript.
    pub runtime_version: String,
    /// Hook names returned by the plugin function.
    pub hooks: Vec<String>,
    /// Hook names the bridge invoked for this load.
    pub invoked_hooks: Vec<String>,
    /// Permission rules contributed by the `config` hook.
    pub config_rules: Vec<PluginConfigRule>,
    /// Tools contributed by the `tool` hook.
    pub tools: Vec<PluginToolRegistration>,
    /// Authentication methods contributed by the `auth` hook.
    pub auth_methods: Vec<PluginAuthRegistration>,
    /// Model providers contributed by the `provider` hook.
    pub providers: Vec<PluginProviderRegistration>,
    /// Redacted runtime stdout.
    pub stdout: String,
    /// Redacted runtime stderr.
    pub stderr: String,
}

/// Observable facts from a later hook invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginHookReport {
    /// Plugin file whose hook was invoked.
    pub plugin_path: PathBuf,
    /// Runtime used for this invocation.
    pub runtime: JsRuntime,
    /// Direct runtime PID observed inside JavaScript.
    pub runtime_pid: Option<u32>,
    /// Hook names actually invoked by the bridge.
    pub invoked_hooks: Vec<String>,
    /// Redacted runtime stdout.
    pub stdout: String,
    /// Redacted runtime stderr.
    pub stderr: String,
}

/// A permission rule returned by a plugin `config` hook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginConfigRule {
    /// Tool name or `*`.
    pub tool: String,
    /// Glob matched against the tool's permission subject.
    pub pattern: String,
    /// `allow`, `ask`, or `deny`.
    pub action: String,
}

/// A model-callable tool returned by a plugin `tool` hook.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginToolRegistration {
    /// Tool name used in the LLM tool catalogue.
    pub name: String,
    /// Model-facing description.
    #[serde(default)]
    pub description: String,
    /// JSON Schema parameter object.
    #[serde(default = "default_parameters")]
    pub parameters: JsonValue,
}

/// A plugin-contributed auth method returned by an `auth` hook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginAuthRegistration {
    /// Stable auth method/provider id.
    pub id: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Environment variables this method can read.
    #[serde(default)]
    pub env: Vec<String>,
}

/// A plugin-contributed model provider returned by a `provider` hook.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginProviderRegistration {
    /// Stable provider id.
    pub id: String,
    /// Human-readable provider name.
    #[serde(default)]
    pub name: String,
    /// Models served by this provider.
    #[serde(default)]
    pub models: Vec<PluginModelRegistration>,
}

/// A plugin-contributed model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginModelRegistration {
    /// Fully-qualified model id (for example `plugin/example-model`).
    pub id: String,
    /// Human-readable model name.
    #[serde(default)]
    pub name: String,
    /// Whether this model supports tool calls.
    #[serde(default)]
    pub tool_call: bool,
    /// Whether this model emits reasoning.
    #[serde(default)]
    pub reasoning: bool,
    /// Context-window size, if known.
    #[serde(default)]
    pub context: u64,
    /// Output-token limit, if known.
    #[serde(default)]
    pub output: u64,
}

/// Output of a plugin tool body.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginToolExecution {
    /// Text injected as the model-visible tool result.
    pub content: String,
    /// Raw JSON returned by the JavaScript bridge.
    pub raw: JsonValue,
}

/// Fail-closed plugin runtime errors.
#[derive(Debug, Error)]
pub enum PluginRuntimeError {
    /// A plugin is configured, but no supported runtime is on `PATH`.
    #[error(
        "no JavaScript runtime ({RUNTIME_REQUIREMENT}) found on PATH for configured plugins; \
         install Bun, Node, or Deno (searched in order: {RUNTIME_ORDER})"
    )]
    NoRuntime,
    /// A configured local plugin file does not exist.
    #[error("configured plugin file {} does not exist", path.display())]
    PluginFileMissing {
        /// The missing plugin path.
        path: PathBuf,
    },
    /// A filesystem operation for the bridge failed.
    #[error("failed to {action} at {}: {source}", path.display())]
    Io {
        /// What was being done.
        action: &'static str,
        /// The affected path.
        path: PathBuf,
        /// The IO error.
        source: std::io::Error,
    },
    /// JSON serialization/deserialization failed.
    #[error("plugin bridge JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// The runtime child could not be spawned.
    #[error("failed to start {runtime} for plugin host using {}: {source}", executable.display())]
    Spawn {
        /// Selected runtime.
        runtime: JsRuntime,
        /// Exact executable path.
        executable: PathBuf,
        /// Spawn error.
        source: std::io::Error,
    },
    /// The runtime child exceeded its timeout and was killed.
    #[error(
        "plugin {} timed out after {timeout_ms}ms in {runtime}; runtime process pid {pid} was terminated",
        path.display()
    )]
    Timeout {
        /// Plugin path.
        path: PathBuf,
        /// Selected runtime.
        runtime: JsRuntime,
        /// Runtime process PID.
        pid: u32,
        /// Timeout in milliseconds.
        timeout_ms: u128,
    },
    /// The bridge exited non-zero before reporting a structured plugin error.
    #[error(
        "plugin {} failed while running in {runtime} (status {status}); stderr: {stderr}",
        path.display()
    )]
    RuntimeFailed {
        /// Plugin path.
        path: PathBuf,
        /// Selected runtime.
        runtime: JsRuntime,
        /// Exit status.
        status: String,
        /// Redacted stderr.
        stderr: String,
    },
    /// The plugin module threw while importing or while its exported function initialized.
    #[error("plugin {} failed during load/initialization in {runtime}: {message}", path.display())]
    PluginThrew {
        /// Plugin path.
        path: PathBuf,
        /// Selected runtime.
        runtime: JsRuntime,
        /// Runtime error message.
        message: String,
        /// Redacted stack, if the runtime reported one.
        stack: Option<String>,
    },
    /// A plugin hook threw after the module loaded successfully.
    #[error("plugin {} hook `{hook}` failed in {runtime}: {message}", path.display())]
    HookThrew {
        /// Plugin path.
        path: PathBuf,
        /// Selected runtime.
        runtime: JsRuntime,
        /// Hook name.
        hook: String,
        /// Runtime error message.
        message: String,
        /// Redacted stack, if the runtime reported one.
        stack: Option<String>,
    },
}

#[derive(Serialize)]
struct BridgeInput {
    plugin_path: String,
    directory: String,
    options: JsonValue,
    operation: BridgeOperation,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeOperation {
    Load { config: JsonValue },
    Event { event: JsonValue },
    ToolExecute { name: String, arguments: JsonValue },
    ProviderComplete { model: String, messages: JsonValue },
}

#[derive(Deserialize)]
struct BridgeResult {
    ok: bool,
    runtime: BridgeRuntime,
    pid: Option<u32>,
    #[serde(default)]
    hooks: Vec<String>,
    #[serde(default)]
    invoked_hooks: Vec<String>,
    #[serde(default)]
    registrations: BridgeRegistrations,
    #[serde(default)]
    output: Option<JsonValue>,
    error: Option<BridgeError>,
}

#[derive(Deserialize)]
struct BridgeRuntime {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct BridgeError {
    #[allow(dead_code)]
    name: String,
    message: String,
    stack: Option<String>,
    hook: Option<String>,
}

#[derive(Default, Deserialize)]
struct BridgeRegistrations {
    #[serde(default)]
    config_rules: Vec<PluginConfigRule>,
    #[serde(default)]
    tools: Vec<PluginToolRegistration>,
    #[serde(default)]
    auth_methods: Vec<PluginAuthRegistration>,
    #[serde(default)]
    providers: Vec<PluginProviderRegistration>,
}

struct InvocationTemp {
    path: PathBuf,
}

impl InvocationTemp {
    fn create() -> Result<Self, PluginRuntimeError> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("cortex-plugin-host-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|source| PluginRuntimeError::Io {
            action: "create plugin bridge directory",
            path: path.clone(),
            source,
        })?;
        Ok(Self { path })
    }
}

fn run_child(
    runtime: &RuntimeSelection,
    plugin_path: &Path,
    bridge_path: &Path,
    input_path: &Path,
    result_path: &Path,
    timeout: Duration,
) -> Result<ChildOutput, PluginRuntimeError> {
    let mut command = Command::new(&runtime.executable);
    match runtime.runtime {
        JsRuntime::Bun | JsRuntime::Node => {
            command.arg(bridge_path);
        }
        JsRuntime::Deno => {
            command.args([
                "run",
                "--allow-read",
                "--allow-write",
                "--allow-run",
                "--allow-net",
                "--allow-env",
            ]);
            command.arg(bridge_path);
        }
    }
    command
        .arg(input_path)
        .arg(result_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(secret_stripped_env());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = command
        .spawn()
        .map_err(|source| PluginRuntimeError::Spawn {
            runtime: runtime.runtime,
            executable: runtime.executable.clone(),
            source,
        })?;
    let pid = child.id();
    let stdout = child.stdout.take().map(read_pipe);
    let stderr = child.stderr.take().map(read_pipe);
    let started = Instant::now();

    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|source| PluginRuntimeError::Spawn {
                runtime: runtime.runtime,
                executable: runtime.executable.clone(),
                source,
            })?
        {
            break status;
        }
        if started.elapsed() > timeout {
            kill_runtime_process_group(pid);
            let _ = child.kill();
            let _ = child.wait();
            return Err(PluginRuntimeError::Timeout {
                path: plugin_path.to_path_buf(),
                runtime: runtime.runtime,
                pid,
                timeout_ms: timeout.as_millis(),
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let stdout = join_pipe(stdout);
    let stderr = join_pipe(stderr);
    Ok(ChildOutput {
        status,
        stdout,
        stderr,
    })
}

fn read_pipe<R: Read + Send + 'static>(mut pipe: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = pipe.read_to_end(&mut buffer);
        buffer
    })
}

fn join_pipe(handle: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

fn kill_runtime_process_group(pid: u32) {
    #[cfg(unix)]
    {
        let _ = Command::new("/bin/kill")
            .arg("-KILL")
            .arg("--")
            .arg(format!("-{pid}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

struct ChildOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn read_bridge_result(
    plugin_path: &Path,
    runtime: &RuntimeSelection,
    result_path: &Path,
    output: &ChildOutput,
) -> Result<BridgeResult, PluginRuntimeError> {
    let result = match std::fs::read_to_string(result_path) {
        Ok(result) => result,
        Err(_) if !output.status.success() => {
            return Err(PluginRuntimeError::RuntimeFailed {
                path: plugin_path.to_path_buf(),
                runtime: runtime.runtime,
                status: status_text(output.status),
                stderr: redaction::redact(&String::from_utf8_lossy(&output.stderr)),
            });
        }
        Err(source) => {
            return Err(PluginRuntimeError::Io {
                action: "read plugin bridge result",
                path: result_path.to_path_buf(),
                source,
            });
        }
    };
    let parsed: BridgeResult = serde_json::from_str(&result)?;
    if !output.status.success() && parsed.ok {
        return Err(PluginRuntimeError::RuntimeFailed {
            path: plugin_path.to_path_buf(),
            runtime: runtime.runtime,
            status: status_text(output.status),
            stderr: redaction::redact(&String::from_utf8_lossy(&output.stderr)),
        });
    }
    Ok(parsed)
}

fn status_text(status: ExitStatus) -> String {
    status.code().map_or_else(
        || "terminated by signal".to_owned(),
        |code| code.to_string(),
    )
}

fn secret_stripped_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, value)| {
            if redaction::is_secret_env_var(&name.to_string_lossy()) {
                return false;
            }
            !redaction::value_is_secret(&value.to_string_lossy())
        })
        .collect()
}

fn error_from_bridge_result(
    plugin_path: &Path,
    runtime: &RuntimeSelection,
    result: BridgeResult,
    fallback_hook: &str,
) -> PluginRuntimeError {
    let error = result.error.unwrap_or_else(|| BridgeError {
        name: "Error".to_owned(),
        message: "plugin failed without an error message".to_owned(),
        stack: None,
        hook: Some(fallback_hook.to_owned()),
    });
    let message = redaction::redact(&error.message);
    let stack = error.stack.map(|stack| redaction::redact(&stack));
    match error.hook.as_deref() {
        None | Some("load") => PluginRuntimeError::PluginThrew {
            path: plugin_path.to_path_buf(),
            runtime: runtime.runtime,
            message,
            stack,
        },
        Some(hook) => PluginRuntimeError::HookThrew {
            path: plugin_path.to_path_buf(),
            runtime: runtime.runtime,
            hook: hook.to_owned(),
            message,
            stack,
        },
    }
}

fn default_parameters() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true
    })
}

fn output_content(value: &JsonValue) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    for key in ["content", "text", "result", "output"] {
        if let Some(text) = value.get(key).and_then(JsonValue::as_str) {
            return text.to_owned();
        }
    }
    serde_json::to_string(value).unwrap_or_else(|_| String::new())
}

const BRIDGE_SCRIPT: &str = r#"
import processModule from "node:process";
import { pathToFileURL } from "node:url";
import { writeFile, readFile } from "node:fs/promises";
import { execFile } from "node:child_process";

const proc = globalThis.process ?? processModule;
const inputPath = proc.argv[2];
const resultPath = proc.argv[3];

let activeHook = "load";

function runtimeInfo() {
  if (globalThis.Bun) {
    return { name: "bun", version: String(globalThis.Bun.version ?? "unknown") };
  }
  if (globalThis.Deno) {
    return { name: "deno", version: String(globalThis.Deno.version?.deno ?? "unknown") };
  }
  if (proc?.versions?.node) {
    return { name: "node", version: String(proc.versions.node) };
  }
  return { name: "javascript", version: "unknown" };
}

async function writeResult(result) {
  await writeFile(resultPath, JSON.stringify(result, null, 2), "utf8");
}

function errorToJson(error) {
  return {
    name: String(error?.name ?? "Error"),
    message: String(error?.message ?? error),
    stack: error?.stack ? String(error.stack) : undefined,
    hook: activeHook
  };
}

function shellEscape(value) {
  return String(value)
    .replaceAll("\\", "\\\\")
    .replaceAll("`", "\\`")
    .replaceAll("$", "\\$");
}

function commandFromTemplate(strings, values) {
  let command = "";
  for (let i = 0; i < strings.length; i += 1) {
    command += strings[i];
    if (i < values.length) {
      command += shellEscape(values[i]);
    }
  }
  return command;
}

function makeShell(directory) {
  const run = (command, options = {}) => new Promise((resolve) => {
    const timeout = Number(options.timeout ?? 120000);
    const child = execFile(
      "bash",
      ["-lc", String(command)],
      {
        cwd: String(options.cwd ?? directory),
        timeout,
        maxBuffer: 10 * 1024 * 1024,
        env: proc.env
      },
      (error, stdout, stderr) => {
        const exitCode = typeof error?.code === "number" ? error.code : 0;
        resolve({
          stdout: String(stdout ?? ""),
          stderr: String(stderr ?? ""),
          exitCode,
          code: exitCode,
          pid: child.pid ?? null,
          timedOut: Boolean(error?.killed)
        });
      }
    );
  });

  const tagged = (strings, ...values) => {
    if (Array.isArray(strings) && Object.prototype.hasOwnProperty.call(strings, "raw")) {
      return run(commandFromTemplate(strings, values));
    }
    return run(strings);
  };
  tagged.run = run;
  return tagged;
}

function resolvePluginFunction(module) {
  const candidate = module.default ?? module.plugin ?? module.activate;
  if (typeof candidate !== "function") {
    throw new Error("plugin module must export a function (default export, plugin, or activate)");
  }
  return candidate;
}

function createInput(raw, directory) {
  const shell = makeShell(directory);
  const registeredTools = [];
  const registeredAuth = [];
  const registeredProviders = [];
  const registerTool = (tool) => {
    registeredTools.push(tool);
    return tool;
  };
  const registerAuth = (auth) => {
    registeredAuth.push(auth);
    return auth;
  };
  const registerProvider = (provider) => {
    registeredProviders.push(provider);
    return provider;
  };
  return {
    input: {
      project: { directory, root: directory },
      directory,
      worktree: directory,
      serverUrl: "local://cortex",
      server: { kind: "local", url: "local://cortex" },
      client: { kind: "local", available: true },
      shell: shell.run,
      $: shell,
      experimental_workspace: {
        register(tool) {
          return registerTool(tool);
        },
        registerTool,
        registerAuth,
        registerProvider
      }
    },
    registeredTools,
    registeredAuth,
    registeredProviders
  };
}

function entriesFrom(value) {
  if (!value) return [];
  if (Array.isArray(value)) return value;
  if (value instanceof Map) return Array.from(value.values());
  if (typeof value === "object") {
    if (value.name || value.id || value.provider || value.model) return [value];
    if (Array.isArray(value.tools)) return value.tools;
    if (Array.isArray(value.auth)) return value.auth;
    if (Array.isArray(value.providers)) return value.providers;
    if (Array.isArray(value.models)) return [value];
    return Object.entries(value).map(([key, item]) => {
      if (item && typeof item === "object" && !Array.isArray(item)) {
        return { id: key, name: key, ...item };
      }
      return item;
    });
  }
  return [];
}

function normalizeParameters(parameters) {
  if (parameters && typeof parameters === "object") return parameters;
  return { type: "object", properties: {}, additionalProperties: true };
}

function normalizeTool(tool) {
  if (!tool || typeof tool !== "object") return null;
  const name = String(tool.name ?? tool.id ?? "").trim();
  if (!name) return null;
  return {
    name,
    description: String(tool.description ?? ""),
    parameters: normalizeParameters(tool.parameters ?? tool.schema ?? tool.input_schema ?? tool.inputSchema)
  };
}

function normalizeAuth(method) {
  if (!method || typeof method !== "object") return null;
  const id = String(method.id ?? method.provider ?? method.name ?? "").trim();
  if (!id) return null;
  return {
    id,
    name: String(method.name ?? id),
    description: method.description == null ? undefined : String(method.description),
    env: Array.isArray(method.env) ? method.env.map(String) : []
  };
}

function normalizeModel(model) {
  if (!model || typeof model !== "object") return null;
  const id = String(model.id ?? model.model ?? "").trim();
  if (!id) return null;
  return {
    id,
    name: String(model.name ?? id),
    tool_call: Boolean(model.tool_call ?? model.toolCall ?? model.tools),
    reasoning: Boolean(model.reasoning),
    context: Number(model.context ?? model.limit?.context ?? 0) || 0,
    output: Number(model.output ?? model.limit?.output ?? 0) || 0
  };
}

function normalizeProvider(provider) {
  if (!provider || typeof provider !== "object") return null;
  const id = String(provider.id ?? provider.provider ?? provider.name ?? "").trim();
  if (!id) return null;
  return {
    id,
    name: String(provider.name ?? id),
    models: entriesFrom(provider.models).map(normalizeModel).filter(Boolean)
  };
}

function normalizeConfigRules(value) {
  const source = value && typeof value === "object" && Array.isArray(value.permission)
    ? value.permission
    : Array.isArray(value)
      ? value
      : [];
  return source.map((rule) => {
    if (!rule || typeof rule !== "object") return null;
    const tool = String(rule.tool ?? "*").trim() || "*";
    const pattern = String(rule.pattern ?? rule.glob ?? "*").trim() || "*";
    const action = String(rule.action ?? rule.decision ?? "ask").trim().toLowerCase();
    if (!["allow", "ask", "deny"].includes(action)) return null;
    return { tool, pattern, action };
  }).filter(Boolean);
}

function normalizeToolResult(result) {
  if (typeof result === "string") return { content: result };
  if (result == null) return { content: "" };
  if (typeof result === "object") {
    if (typeof result.content === "string" || typeof result.text === "string") return result;
    return { content: JSON.stringify(result), raw: result };
  }
  return { content: String(result) };
}

function findTool(hooks, name, registeredTools, returnedTools) {
  const candidates = [...registeredTools, ...entriesFrom(returnedTools)];
  for (const candidate of candidates) {
    if (!candidate || typeof candidate !== "object") continue;
    const candidateName = String(candidate.name ?? candidate.id ?? "").trim();
    if (candidateName === name) return candidate;
  }
  throw new Error(`plugin tool ${name} was not registered by the tool hook`);
}

function findProviderForModel(providers, model) {
  for (const provider of providers) {
    if (!provider || typeof provider !== "object") continue;
    const models = entriesFrom(provider.models);
    if (models.some((candidate) => String(candidate?.id ?? candidate?.model ?? "") === model)) {
      return provider;
    }
  }
  throw new Error(`plugin model ${model} was not registered by the provider hook`);
}

async function loadHooks(raw, directory) {
  const context = createInput(raw, directory);
  const module = await import(pathToFileURL(raw.plugin_path).href);
  const plugin = resolvePluginFunction(module);
  const hooks = await plugin(context.input, raw.options ?? {});
  const hookEntries = hooks && typeof hooks === "object"
    ? Object.keys(hooks).filter((key) => typeof hooks[key] === "function")
    : [];
  return { hooks: hooks ?? {}, hookEntries, context };
}

try {
  const raw = JSON.parse(await readFile(inputPath, "utf8"));
  const directory = String(raw.directory);
  const operation = raw.operation ?? { type: "load" };
  activeHook = "load";
  const { hooks, hookEntries, context } = await loadHooks(raw, directory);
  const invokedHooks = [];
  const registrations = { config_rules: [], tools: [], auth_methods: [], providers: [] };
  let output = undefined;

  if (operation.type === "load") {
    if (typeof hooks.event === "function") {
      activeHook = "event";
      await hooks.event({
        kind: "lifecycle",
        type: "lifecycle",
        phase: "startup",
        surface: "run",
        directory
      });
      invokedHooks.push("event");
    }
    if (typeof hooks.config === "function") {
      activeHook = "config";
      registrations.config_rules = normalizeConfigRules(await hooks.config(operation.config ?? {}));
      invokedHooks.push("config");
    }
    if (typeof hooks.tool === "function") {
      activeHook = "tool";
      const returned = await hooks.tool({
        register: context.input.experimental_workspace.registerTool,
        registerTool: context.input.experimental_workspace.registerTool
      });
      registrations.tools = [...context.registeredTools, ...entriesFrom(returned)].map(normalizeTool).filter(Boolean);
      invokedHooks.push("tool");
    }
    if (typeof hooks.auth === "function") {
      activeHook = "auth";
      const returned = await hooks.auth({ register: context.input.experimental_workspace.registerAuth });
      registrations.auth_methods = [...context.registeredAuth, ...entriesFrom(returned)].map(normalizeAuth).filter(Boolean);
      invokedHooks.push("auth");
    }
    if (typeof hooks.provider === "function") {
      activeHook = "provider";
      const returned = await hooks.provider({ register: context.input.experimental_workspace.registerProvider });
      registrations.providers = [...context.registeredProviders, ...entriesFrom(returned)].map(normalizeProvider).filter(Boolean);
      invokedHooks.push("provider");
    }
  } else if (operation.type === "event") {
    if (typeof hooks.event === "function") {
      activeHook = "event";
      await hooks.event(operation.event ?? {});
      invokedHooks.push("event");
    }
  } else if (operation.type === "tool_execute") {
    if (typeof hooks.tool !== "function") {
      throw new Error("plugin did not define a tool hook");
    }
    activeHook = "tool";
    const returned = await hooks.tool({
      register: context.input.experimental_workspace.registerTool,
      registerTool: context.input.experimental_workspace.registerTool
    });
    const tool = findTool(hooks, String(operation.name ?? ""), context.registeredTools, returned);
    const execute = tool.execute ?? tool.run ?? tool.call;
    if (typeof execute !== "function") {
      throw new Error(`plugin tool ${operation.name} does not define execute(args)`);
    }
    output = normalizeToolResult(await execute(operation.arguments ?? {}, {
      directory,
      cwd: directory,
      tool: String(operation.name ?? "")
    }));
    invokedHooks.push("tool");
  } else if (operation.type === "provider_complete") {
    if (typeof hooks.provider !== "function") {
      throw new Error("plugin did not define a provider hook");
    }
    activeHook = "provider";
    const returned = await hooks.provider({ register: context.input.experimental_workspace.registerProvider });
    const providers = [...context.registeredProviders, ...entriesFrom(returned)];
    const provider = findProviderForModel(providers, String(operation.model ?? ""));
    const complete = provider.complete ?? provider.generate ?? provider.stream;
    if (typeof complete !== "function") {
      throw new Error(`plugin provider ${provider.id ?? provider.name ?? "unknown"} does not define complete(request)`);
    }
    output = normalizeToolResult(await complete({
      model: String(operation.model ?? ""),
      messages: operation.messages ?? [],
      directory
    }));
    invokedHooks.push("provider");
  } else {
    throw new Error(`unknown plugin bridge operation: ${operation.type}`);
  }

  await writeResult({
    ok: true,
    runtime: runtimeInfo(),
    pid: typeof proc.pid === "number" ? proc.pid : null,
    hooks: hookEntries,
    invoked_hooks: invokedHooks,
    registrations,
    output
  });
} catch (error) {
  await writeResult({
    ok: false,
    runtime: runtimeInfo(),
    pid: typeof proc.pid === "number" ? proc.pid : null,
    hooks: [],
    invoked_hooks: [],
    registrations: { config_rules: [], tools: [], auth_methods: [], providers: [] },
    error: errorToJson(error)
  });
  proc.exitCode = 1;
}
"#;

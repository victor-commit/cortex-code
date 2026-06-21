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

    /// Load every configured plugin in order. If no plugins are configured, no runtime is required.
    ///
    /// A configured plugin fails closed: missing runtime, missing file, a runtime spawn failure, a
    /// timeout, or a plugin load throw all return [`PluginRuntimeError`] and stop further loading.
    pub fn load_config(
        &self,
        config: &PluginConfig,
    ) -> Result<Vec<PluginLoadReport>, PluginRuntimeError> {
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
    ) -> Result<PluginLoadReport, PluginRuntimeError> {
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

        if result.ok {
            Ok(PluginLoadReport {
                plugin_path: plugin.path().to_path_buf(),
                runtime: runtime.runtime,
                runtime_executable: runtime.executable.clone(),
                runtime_pid: result.pid,
                runtime_name: result.runtime.name,
                runtime_version: result.runtime.version,
                hooks: result.hooks,
                invoked_hooks: result.invoked_hooks,
                stdout: redaction::redact(&String::from_utf8_lossy(&output.stdout)),
                stderr: redaction::redact(&String::from_utf8_lossy(&output.stderr)),
            })
        } else {
            let error = result.error.unwrap_or_else(|| BridgeError {
                name: "Error".to_owned(),
                message: "plugin failed without an error message".to_owned(),
                stack: None,
            });
            Err(PluginRuntimeError::PluginThrew {
                path: plugin.path().to_path_buf(),
                runtime: runtime.runtime,
                message: redaction::redact(&error.message),
                stack: error.stack.map(|stack| redaction::redact(&stack)),
            })
        }
    }
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
    /// Redacted runtime stdout.
    pub stdout: String,
    /// Redacted runtime stderr.
    pub stderr: String,
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
}

#[derive(Serialize)]
struct BridgeInput {
    plugin_path: String,
    directory: String,
    options: JsonValue,
}

#[derive(Deserialize)]
struct BridgeResult {
    ok: bool,
    runtime: BridgeRuntime,
    pid: Option<u32>,
    hooks: Vec<String>,
    #[serde(default)]
    invoked_hooks: Vec<String>,
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

const BRIDGE_SCRIPT: &str = r#"
import processModule from "node:process";
import { pathToFileURL } from "node:url";
import { writeFile } from "node:fs/promises";
import { execFile } from "node:child_process";

const proc = globalThis.process ?? processModule;
const inputPath = proc.argv[2];
const resultPath = proc.argv[3];

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
    stack: error?.stack ? String(error.stack) : undefined
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

try {
  const raw = JSON.parse(await (await import("node:fs/promises")).readFile(inputPath, "utf8"));
  const directory = String(raw.directory);
  const shell = makeShell(directory);
  const input = {
    project: { directory, root: directory },
    directory,
    worktree: directory,
    serverUrl: "local://cortex",
    server: { kind: "local", url: "local://cortex" },
    client: { kind: "local", available: true },
    shell: shell.run,
    $: shell,
    experimental_workspace: {
      register() {
        return { registered: true };
      }
    }
  };

  const module = await import(pathToFileURL(raw.plugin_path).href);
  const plugin = resolvePluginFunction(module);
  const hooks = await plugin(input, raw.options ?? {});
  const hookEntries = hooks && typeof hooks === "object"
    ? Object.keys(hooks).filter((key) => typeof hooks[key] === "function")
    : [];
  const invokedHooks = [];
  if (hooks && typeof hooks.event === "function") {
    await hooks.event({
      kind: "lifecycle",
      type: "lifecycle",
      phase: "startup",
      surface: "run",
      directory
    });
    invokedHooks.push("event");
  }

  await writeResult({
    ok: true,
    runtime: runtimeInfo(),
    pid: typeof proc.pid === "number" ? proc.pid : null,
    hooks: hookEntries,
    invoked_hooks: invokedHooks
  });
} catch (error) {
  await writeResult({
    ok: false,
    runtime: runtimeInfo(),
    pid: typeof proc.pid === "number" ? proc.pid : null,
    hooks: [],
    invoked_hooks: [],
    error: errorToJson(error)
  });
  proc.exitCode = 1;
}
"#;

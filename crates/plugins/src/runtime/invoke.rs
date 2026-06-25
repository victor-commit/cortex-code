use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::config::{PluginConfig, PluginSpec};
use crate::distribution::{
    PackageLockEntry, PackageLockError, PackageLockfile, locked_entry_is_valid,
    normalize_install_spec, package_import_spec, package_install_dir,
};
use crate::runtime::detection::{JsRuntime, RuntimeSelection, detect_runtime_in_path};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const RUNTIME_REQUIREMENT: &str = "Bun, Node, or Deno";
const RUNTIME_ORDER: &str = "bun, node, deno";

/// Host for loading configured local plugins through a real external JavaScript runtime.
#[derive(Clone)]
pub struct RuntimeHost {
    timeout: Duration,
    path_override: Option<OsString>,
    sessions: Arc<Mutex<HashMap<PathBuf, Arc<RuntimeSession>>>>,
}

impl std::fmt::Debug for RuntimeHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sessions = self.sessions.lock().map(|active| active.len()).unwrap_or(0);
        f.debug_struct("RuntimeHost")
            .field("timeout", &self.timeout)
            .field("path_override", &self.path_override)
            .field("active_sessions", &sessions)
            .finish()
    }
}

impl Drop for RuntimeHost {
    fn drop(&mut self) {
        if Arc::strong_count(&self.sessions) != 1 {
            return;
        }
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, session) in sessions.drain() {
                session.shutdown();
            }
        }
    }
}

#[derive(Debug)]
struct RuntimeSession {
    runtime: RuntimeSelection,
    plugin_path: PathBuf,
    process: Mutex<RuntimeProcess>,
}

#[derive(Debug)]
struct RuntimeProcess {
    child: Child,
    stdin: ChildStdin,
    responses: Receiver<String>,
    stderr: Arc<Mutex<Vec<u8>>>,
    temp_dir: PathBuf,
}

impl RuntimeSession {
    fn invoke(
        &self,
        operation: BridgeOperation,
        timeout: Duration,
    ) -> Result<(BridgeResult, ChildOutput), PluginRuntimeError> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| PluginRuntimeError::RuntimeFailed {
                path: self.plugin_path.clone(),
                runtime: self.runtime.runtime,
                status: "session-lock-poisoned".to_owned(),
                stderr: String::new(),
            })?;

        let request = serde_json::to_string(&BridgeRequest { operation })?;
        process
            .stdin
            .write_all(request.as_bytes())
            .map_err(|source| PluginRuntimeError::Spawn {
                runtime: self.runtime.runtime,
                executable: self.runtime.executable.clone(),
                source,
            })?;
        process
            .stdin
            .write_all(b"\n")
            .map_err(|source| PluginRuntimeError::Spawn {
                runtime: self.runtime.runtime,
                executable: self.runtime.executable.clone(),
                source,
            })?;
        process
            .stdin
            .flush()
            .map_err(|source| PluginRuntimeError::Spawn {
                runtime: self.runtime.runtime,
                executable: self.runtime.executable.clone(),
                source,
            })?;

        let line = match process.responses.recv_timeout(timeout) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                let pid = process.child.id();
                shutdown_process(&mut process);
                return Err(PluginRuntimeError::Timeout {
                    path: self.plugin_path.clone(),
                    runtime: self.runtime.runtime,
                    pid,
                    timeout_ms: timeout.as_millis(),
                });
            }
            Err(RecvTimeoutError::Disconnected) => {
                let status = process
                    .child
                    .try_wait()
                    .ok()
                    .and_then(|status| status)
                    .map(status_text)
                    .unwrap_or_else(|| "terminated".to_owned());
                let stderr = process
                    .stderr
                    .lock()
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    .unwrap_or_default();
                shutdown_process(&mut process);
                return Err(PluginRuntimeError::RuntimeFailed {
                    path: self.plugin_path.clone(),
                    runtime: self.runtime.runtime,
                    status,
                    stderr: redaction::redact(&stderr),
                });
            }
        };

        let result: BridgeResult = match serde_json::from_str(&line) {
            Ok(result) => result,
            Err(_) => {
                let stderr = process
                    .stderr
                    .lock()
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    .unwrap_or_default();
                return Err(PluginRuntimeError::RuntimeFailed {
                    path: self.plugin_path.clone(),
                    runtime: self.runtime.runtime,
                    status: "protocol-parse-error".to_owned(),
                    stderr: redaction::redact(&stderr),
                });
            }
        };

        let stderr = process
            .stderr
            .lock()
            .ok()
            .map(|bytes| bytes.clone())
            .unwrap_or_default();
        Ok((
            result,
            ChildOutput {
                status: success_status(),
                stdout: Vec::new(),
                stderr,
            },
        ))
    }

    fn shutdown(&self) {
        if let Ok(mut process) = self.process.lock() {
            shutdown_process(&mut process);
        }
    }
}

fn shutdown_process(process: &mut RuntimeProcess) {
    let pid = process.child.id();
    kill_runtime_process_group(pid);
    let _ = process.child.kill();
    let _ = process.child.wait();
    let _ = std::fs::remove_dir_all(&process.temp_dir);
}

impl Default for RuntimeHost {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            path_override: None,
            sessions: Arc::new(Mutex::new(HashMap::new())),
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
        self.load_config_for_surface(config, "run")
    }

    /// Load configured plugins for a named surface and return only their observable reports.
    ///
    /// The surface name is included in the synthetic lifecycle event delivered to plugin `event`
    /// hooks while loading. Existing callers that do not care about a specific surface use
    /// [`RuntimeHost::load_config`], which preserves the headless `run` default.
    pub fn load_config_for_surface(
        &self,
        config: &PluginConfig,
        surface: &str,
    ) -> Result<Vec<PluginLoadReport>, PluginRuntimeError> {
        self.load_config_for_surface_with_config(
            config,
            surface,
            JsonValue::Object(Default::default()),
        )
    }

    /// Load configured plugins for a named surface with a resolved effective config object.
    ///
    /// The supplied `resolved_config` is passed to each plugin `config` hook. Plugins load in
    /// deterministic config order, and any config object returned/mutated by one hook is passed to
    /// the next plugin so hooks observe cumulative layered changes.
    pub fn load_config_for_surface_with_config(
        &self,
        config: &PluginConfig,
        surface: &str,
        resolved_config: JsonValue,
    ) -> Result<Vec<PluginLoadReport>, PluginRuntimeError> {
        Ok(self
            .load_plugins_for_surface_with_config(config, surface, resolved_config)?
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
        self.load_plugins_for_surface(config, "run")
    }

    /// Load every configured plugin in order for a named surface, preserving the original spec next
    /// to the report so callers can invoke later hooks or execute plugin-contributed tools.
    pub fn load_plugins_for_surface(
        &self,
        config: &PluginConfig,
        surface: &str,
    ) -> Result<Vec<LoadedPlugin>, PluginRuntimeError> {
        self.load_plugins_for_surface_with_config(
            config,
            surface,
            JsonValue::Object(Default::default()),
        )
    }

    /// Load every configured plugin in order for a named surface, passing `resolved_config` to the
    /// plugin `config` hook and chaining hook-returned config mutations across plugins.
    pub fn load_plugins_for_surface_with_config(
        &self,
        config: &PluginConfig,
        surface: &str,
        resolved_config: JsonValue,
    ) -> Result<Vec<LoadedPlugin>, PluginRuntimeError> {
        if config.is_empty() {
            return Ok(Vec::new());
        }

        let runtime = self.detect_runtime().ok_or(PluginRuntimeError::NoRuntime)?;
        let surface = non_empty_surface(surface);
        let mut chained_config = match resolved_config {
            JsonValue::Object(_) => resolved_config,
            _ => JsonValue::Object(Default::default()),
        };
        let mut loaded_plugins = Vec::with_capacity(config.entries().len());
        for plugin in config.entries() {
            let plugin = self.resolve_plugin(plugin, &runtime)?;
            let loaded = self.load_plugin(&plugin, &runtime, &surface, chained_config.clone())?;
            if let Some(next_config) = loaded.report.config.clone()
                && next_config.is_object()
            {
                chained_config = next_config;
            }
            loaded_plugins.push(loaded);
        }
        Ok(loaded_plugins)
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

    /// Invoke a plugin's `chat.message` hook with one outgoing message.
    pub fn invoke_chat_message(
        &self,
        plugin: &PluginSpec,
        message: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::ChatMessage { message },
            "chat.message",
        )
    }

    /// Invoke a plugin's `chat.params` hook with the request parameter map.
    pub fn invoke_chat_params(
        &self,
        plugin: &PluginSpec,
        params: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::ChatParams { params },
            "chat.params",
        )
    }

    /// Invoke a plugin's `chat.headers` hook with the outbound request header map.
    pub fn invoke_chat_headers(
        &self,
        plugin: &PluginSpec,
        headers: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::ChatHeaders { headers },
            "chat.headers",
        )
    }

    /// Invoke a plugin's `tool.definition` hook with one advertised tool definition.
    pub fn invoke_tool_definition(
        &self,
        plugin: &PluginSpec,
        definition: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::ToolDefinition { definition },
            "tool.definition",
        )
    }

    /// Invoke a plugin's `permission.ask` hook with the permission request/proposed decision.
    pub fn invoke_permission_ask(
        &self,
        plugin: &PluginSpec,
        request: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::PermissionAsk { request },
            "permission.ask",
        )
    }

    /// Invoke a plugin's `command.execute.before` hook with a command invocation.
    pub fn invoke_command_before(
        &self,
        plugin: &PluginSpec,
        command: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::CommandBefore { command },
            "command.execute.before",
        )
    }

    /// Invoke a plugin's `tool.execute.before` hook with a pending tool call.
    pub fn invoke_tool_before(
        &self,
        plugin: &PluginSpec,
        call: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::ToolBefore { call },
            "tool.execute.before",
        )
    }

    /// Invoke a plugin's `tool.execute.after` hook with a completed tool result.
    pub fn invoke_tool_after(
        &self,
        plugin: &PluginSpec,
        result: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::ToolAfter { result },
            "tool.execute.after",
        )
    }

    /// Invoke a plugin's `shell.env` hook and return its environment object.
    pub fn invoke_shell_env(
        &self,
        plugin: &PluginSpec,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(plugin, BridgeOperation::ShellEnv, "shell.env")
    }

    /// Invoke a plugin's `dispose` hook during clean surface shutdown.
    pub fn invoke_dispose(
        &self,
        plugin: &PluginSpec,
    ) -> Result<PluginHookReport, PluginRuntimeError> {
        let runtime = self.detect_runtime().ok_or(PluginRuntimeError::NoRuntime)?;
        let result = self.invoke_plugin(plugin, &runtime, BridgeOperation::Dispose);
        self.remove_session(plugin.path());
        let (result, output) = result?;
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
                "dispose",
            ))
        }
    }

    /// Invoke a plugin's messages-array transform hook.
    pub fn invoke_chat_messages_transform(
        &self,
        plugin: &PluginSpec,
        messages: JsonValue,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::ChatMessagesTransform { messages },
            "experimental.chat.messages.transform",
        )
    }

    /// Invoke a plugin's system-prompt transform hook.
    pub fn invoke_chat_system_transform(
        &self,
        plugin: &PluginSpec,
        system: String,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        self.invoke_value_hook(
            plugin,
            BridgeOperation::ChatSystemTransform { system },
            "experimental.chat.system.transform",
        )
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

    fn invoke_value_hook(
        &self,
        plugin: &PluginSpec,
        operation: BridgeOperation,
        hook_name: &str,
    ) -> Result<PluginHookOutput, PluginRuntimeError> {
        let runtime = self.detect_runtime().ok_or(PluginRuntimeError::NoRuntime)?;
        let (result, output) = self.invoke_plugin(plugin, &runtime, operation)?;
        if result.ok {
            Ok(PluginHookOutput {
                plugin_path: plugin.path().to_path_buf(),
                runtime: runtime.runtime,
                runtime_pid: result.pid,
                invoked_hooks: result.invoked_hooks,
                output: result.output,
                stdout: redaction::redact(&String::from_utf8_lossy(&output.stdout)),
                stderr: redaction::redact(&String::from_utf8_lossy(&output.stderr)),
            })
        } else {
            Err(error_from_bridge_result(
                plugin.path(),
                &runtime,
                result,
                hook_name,
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

    fn resolve_plugin(
        &self,
        plugin: &PluginSpec,
        runtime: &RuntimeSelection,
    ) -> Result<PluginSpec, PluginRuntimeError> {
        let Some(spec) = plugin.package_spec() else {
            return Ok(plugin.clone());
        };
        let cwd = plugin.directory();
        let mut lockfile =
            PackageLockfile::load(cwd).map_err(|err| package_lock_error(spec, err))?;
        if let Some(entry) = lockfile.entry(cwd, spec) {
            locked_entry_is_valid(&entry).map_err(|message| {
                PluginRuntimeError::PackageResolveFailed {
                    spec: spec.to_owned(),
                    message,
                }
            })?;
            return Ok(plugin.with_resolved_path(entry.module_path));
        }
        let import_spec = package_import_spec(spec, cwd).map_err(|message| {
            PluginRuntimeError::PackageResolveFailed {
                spec: spec.to_owned(),
                message,
            }
        })?;

        if let Ok(entry) = self.resolve_package(runtime, spec, &import_spec, cwd) {
            lockfile
                .upsert(cwd, entry.clone())
                .map_err(|err| package_lock_error(spec, err))?;
            return Ok(plugin.with_resolved_path(entry.module_path));
        }

        if redaction::network_disabled() {
            return Err(PluginRuntimeError::PackageNetDisabled {
                spec: spec.to_owned(),
            });
        }

        self.install_package(runtime, spec, cwd)?;
        let install_dir = package_install_dir(cwd);
        let entry = self.resolve_package(runtime, spec, &import_spec, &install_dir)?;
        lockfile
            .upsert(cwd, entry.clone())
            .map_err(|err| package_lock_error(spec, err))?;
        Ok(plugin.with_resolved_path(entry.module_path))
    }

    fn resolve_package(
        &self,
        runtime: &RuntimeSelection,
        spec: &str,
        import_spec: &str,
        base_directory: &Path,
    ) -> Result<PackageLockEntry, PluginRuntimeError> {
        let temp = InvocationTemp::create()?;
        let bridge_path = temp.path.join("package_resolver.mjs");
        let input_path = temp.path.join("input.json");
        let result_path = temp.path.join("result.json");
        std::fs::write(&bridge_path, PACKAGE_RESOLVER_SCRIPT).map_err(|source| {
            PluginRuntimeError::Io {
                action: "write plugin package resolver",
                path: bridge_path.clone(),
                source,
            }
        })?;
        std::fs::write(
            &input_path,
            serde_json::to_vec(&PackageResolveInput {
                spec: spec.to_owned(),
                import_spec: import_spec.to_owned(),
                base_directory: base_directory.to_string_lossy().into_owned(),
            })?,
        )
        .map_err(|source| PluginRuntimeError::Io {
            action: "write plugin package resolver input",
            path: input_path.clone(),
            source,
        })?;

        let output = run_package_child(
            runtime,
            spec,
            &bridge_path,
            &input_path,
            &result_path,
            self.timeout,
        )?;
        let result = read_package_resolve_result(spec, runtime, &result_path, &output)?;
        let _ = std::fs::remove_dir_all(&temp.path);
        if result.ok {
            let module_path =
                result
                    .resolved_path
                    .ok_or_else(|| PluginRuntimeError::PackageResolveFailed {
                        spec: spec.to_owned(),
                        message: "resolver did not return a module path".to_owned(),
                    })?;
            let version =
                result
                    .version
                    .ok_or_else(|| PluginRuntimeError::PackageResolveFailed {
                        spec: spec.to_owned(),
                        message: "resolver did not return a package version".to_owned(),
                    })?;
            Ok(PackageLockEntry {
                spec: spec.to_owned(),
                import_spec: import_spec.to_owned(),
                package_name: result
                    .package_name
                    .unwrap_or_else(|| import_spec.to_owned()),
                version,
                module_path: PathBuf::from(module_path),
            })
        } else {
            Err(package_error_from_bridge_result(
                spec, runtime, result, "resolve",
            ))
        }
    }

    fn install_package(
        &self,
        runtime: &RuntimeSelection,
        spec: &str,
        cwd: &Path,
    ) -> Result<(), PluginRuntimeError> {
        let install_dir = package_install_dir(cwd);
        std::fs::create_dir_all(&install_dir).map_err(|source| PluginRuntimeError::Io {
            action: "create plugin package install directory",
            path: install_dir.clone(),
            source,
        })?;
        let package_json = install_dir.join("package.json");
        if !package_json.exists() {
            std::fs::write(
                &package_json,
                b"{\"private\":true,\"type\":\"module\",\"dependencies\":{}}\n",
            )
            .map_err(|source| PluginRuntimeError::Io {
                action: "write plugin package install package.json",
                path: package_json,
                source,
            })?;
        }

        let install_spec = normalize_install_spec(spec, cwd);
        let (program, args): (PathBuf, Vec<String>) = match runtime.runtime {
            JsRuntime::Bun => (
                runtime.executable.clone(),
                vec![
                    "add".to_owned(),
                    "--cwd".to_owned(),
                    install_dir.to_string_lossy().into_owned(),
                    install_spec,
                ],
            ),
            JsRuntime::Node => (
                PathBuf::from("npm"),
                vec![
                    "install".to_owned(),
                    "--prefix".to_owned(),
                    install_dir.to_string_lossy().into_owned(),
                    "--no-audit".to_owned(),
                    "--no-fund".to_owned(),
                    install_spec,
                ],
            ),
            JsRuntime::Deno => {
                return Err(PluginRuntimeError::PackageResolveFailed {
                    spec: spec.to_owned(),
                    message:
                        "Deno cannot install npm packages for Cortex plugins; install the package in node_modules or use Bun/Node"
                            .to_owned(),
                });
            }
        };

        let output = run_install_child(&program, &args, cwd, self.timeout).map_err(|source| {
            PluginRuntimeError::Spawn {
                runtime: runtime.runtime,
                executable: program.clone(),
                source,
            }
        })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(PluginRuntimeError::PackageInstallFailed {
                spec: spec.to_owned(),
                runtime: runtime.runtime,
                status: status_text(output.status),
                stderr: redaction::redact(&String::from_utf8_lossy(&output.stderr)),
            })
        }
    }

    fn load_plugin(
        &self,
        plugin: &PluginSpec,
        runtime: &RuntimeSelection,
        surface: &str,
        resolved_config: JsonValue,
    ) -> Result<LoadedPlugin, PluginRuntimeError> {
        let (result, output) = self.invoke_plugin(
            plugin,
            runtime,
            BridgeOperation::Load {
                config: resolved_config,
                surface: surface.to_owned(),
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
                config: result.registrations.config,
                tools: result.registrations.tools,
                auth_methods: result.registrations.auth_methods,
                providers: result.registrations.providers,
                workspace_entries: result.registrations.workspace_entries,
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

        let session = self.session_for(plugin, runtime)?;
        match session.invoke(operation, self.timeout) {
            Ok(result) => Ok(result),
            Err(err) => {
                self.remove_session(plugin.path());
                Err(err)
            }
        }
    }

    fn session_for(
        &self,
        plugin: &PluginSpec,
        runtime: &RuntimeSelection,
    ) -> Result<Arc<RuntimeSession>, PluginRuntimeError> {
        if let Ok(sessions) = self.sessions.lock()
            && let Some(existing) = sessions.get(plugin.path())
        {
            return Ok(existing.clone());
        }

        let created = Arc::new(self.start_session(plugin, runtime)?);
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PluginRuntimeError::RuntimeFailed {
                path: plugin.path().to_path_buf(),
                runtime: runtime.runtime,
                status: "session-lock-poisoned".to_owned(),
                stderr: String::new(),
            })?;
        Ok(sessions
            .entry(plugin.path().to_path_buf())
            .or_insert_with(|| created.clone())
            .clone())
    }

    fn remove_session(&self, plugin_path: &Path) {
        if let Ok(mut sessions) = self.sessions.lock()
            && let Some(session) = sessions.remove(plugin_path)
        {
            session.shutdown();
        }
    }

    fn start_session(
        &self,
        plugin: &PluginSpec,
        runtime: &RuntimeSelection,
    ) -> Result<RuntimeSession, PluginRuntimeError> {
        let temp = InvocationTemp::create()?;
        let bridge_path = temp.path.join("plugin_bridge.mjs");
        let init_path = temp.path.join("init.json");
        std::fs::write(&bridge_path, BRIDGE_SCRIPT).map_err(|source| PluginRuntimeError::Io {
            action: "write plugin bridge",
            path: bridge_path.clone(),
            source,
        })?;

        let context = PluginRuntimeContext::resolve(plugin.directory());
        std::fs::write(
            &init_path,
            serde_json::to_vec(&BridgeInput {
                plugin_path: plugin.path().to_string_lossy().into_owned(),
                directory: plugin.directory().to_string_lossy().into_owned(),
                root: context.root.to_string_lossy().into_owned(),
                worktree: context.worktree,
                server_url: context.server_url,
                options: plugin.options().clone(),
            })?,
        )
        .map_err(|source| PluginRuntimeError::Io {
            action: "write plugin init input",
            path: init_path.clone(),
            source,
        })?;

        let mut command = Command::new(&runtime.executable);
        match runtime.runtime {
            JsRuntime::Bun | JsRuntime::Node => {
                command.arg(&bridge_path);
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
                command.arg(&bridge_path);
            }
        }
        command
            .arg(&init_path)
            .stdin(Stdio::piped())
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
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginRuntimeError::Spawn {
                runtime: runtime.runtime,
                executable: runtime.executable.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "plugin runtime stdin unavailable",
                ),
            })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginRuntimeError::Spawn {
                runtime: runtime.runtime,
                executable: runtime.executable.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "plugin runtime stdout unavailable",
                ),
            })?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| PluginRuntimeError::Spawn {
                runtime: runtime.runtime,
                executable: runtime.executable.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "plugin runtime stderr unavailable",
                ),
            })?;

        let (tx, rx): (Sender<String>, Receiver<String>) = mpsc::channel();
        std::thread::spawn(move || runtime_stdout_loop(stdout, tx));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        {
            let stderr_target = stderr.clone();
            std::thread::spawn(move || runtime_stderr_loop(stderr_pipe, stderr_target));
        }

        Ok(RuntimeSession {
            runtime: runtime.clone(),
            plugin_path: plugin.path().to_path_buf(),
            process: Mutex::new(RuntimeProcess {
                child,
                stdin,
                responses: rx,
                stderr,
                temp_dir: temp.path,
            }),
        })
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

/// Lightweight event-hook host for surfaces that only need lifecycle/event plugin callbacks.
///
/// This intentionally does not expose model/tool/provider registration. It gives non-headless
/// surfaces a narrow, reusable bridge for configured plugin `event` hooks without duplicating the
/// external-runtime contract.
#[derive(Clone, Debug)]
pub struct PluginEventHooks {
    loaded: Vec<LoadedPlugin>,
    host: RuntimeHost,
    surface: Arc<str>,
    disposed: Arc<AtomicBool>,
    audit: Option<Arc<audit::AuditSink>>,
}

impl PluginEventHooks {
    /// A disabled event-hook state used when pure mode suppresses plugins.
    #[must_use]
    pub fn disabled(surface: &str) -> Self {
        Self {
            loaded: Vec::new(),
            host: RuntimeHost::new(),
            surface: Arc::from(non_empty_surface(surface)),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: None,
        }
    }

    /// Load configured plugins for `surface` through the real external runtime.
    pub fn load(config: &PluginConfig, surface: &str) -> Result<Self, PluginRuntimeError> {
        Self::load_with_config(config, surface, JsonValue::Object(Default::default()))
    }

    /// Load configured plugins for `surface` through the real external runtime with a resolved
    /// effective config object passed into plugin `config` hooks.
    pub fn load_with_config(
        config: &PluginConfig,
        surface: &str,
        resolved_config: JsonValue,
    ) -> Result<Self, PluginRuntimeError> {
        let host = RuntimeHost::new();
        let surface = non_empty_surface(surface);
        let loaded =
            host.load_plugins_for_surface_with_config(config, &surface, resolved_config)?;
        Ok(Self {
            loaded,
            host,
            surface: Arc::from(surface),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: None,
        })
    }

    /// Attach the durable audit sink so plugin lifecycle/hook activity on this surface is recorded
    /// into the same shared sink as headless runs, with the sink's surface/source attribution.
    ///
    /// Attaching records one `plugin_load` entry per configured plugin, mirroring the headless
    /// bridge's `audit_loaded_plugins`, so a configured plugin is attributable even before any
    /// event fires. Subsequent [`PluginEventHooks::invoke_event`] calls record a typed
    /// `plugin_hook` entry for each real event-hook invocation.
    #[must_use]
    pub fn with_audit(mut self, audit: Arc<audit::AuditSink>) -> Self {
        for loaded in &self.loaded {
            audit.record_plugin_load(loaded.spec.raw_spec(), &loaded.report.hooks, "success");
        }
        self.audit = Some(audit);
        self
    }

    /// Load configured plugins unless pure mode disabled all plugin behavior.
    pub fn load_or_disabled(
        config: &PluginConfig,
        pure: bool,
        surface: &str,
    ) -> Result<Self, PluginRuntimeError> {
        Self::load_or_disabled_with_config(
            config,
            pure,
            surface,
            JsonValue::Object(Default::default()),
        )
    }

    /// Load configured plugins unless pure mode disabled all plugin behavior, passing a resolved
    /// effective config object into plugin `config` hooks.
    pub fn load_or_disabled_with_config(
        config: &PluginConfig,
        pure: bool,
        surface: &str,
        resolved_config: JsonValue,
    ) -> Result<Self, PluginRuntimeError> {
        if pure {
            Ok(Self::disabled(surface))
        } else {
            Self::load_with_config(config, surface, resolved_config)
        }
    }

    /// Whether this state contains no loaded plugins.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }

    /// Workspace adapters contributed by loaded plugins through
    /// `experimental_workspace.register(type, adapter)`.
    #[must_use]
    pub fn workspace_entries(&self) -> Vec<PluginWorkspaceRegistration> {
        self.loaded
            .iter()
            .flat_map(|loaded| loaded.report.workspace_entries.iter().cloned())
            .collect()
    }

    /// Invoke every loaded plugin's `event` hook with a serializable event payload.
    ///
    /// Hook failures are reported on stderr and isolated from the caller's turn. The event payload
    /// receives a `surface` field when it did not already carry one, and a `type` alias derived from
    /// `kind`, matching the existing CLI plugin event bridge.
    pub fn invoke_event<T: Serialize>(&self, event: &T) {
        if self.loaded.is_empty() {
            return;
        }
        let value = match event_payload_for_surface(event, &self.surface) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("cortex plugin event: failed to serialize event: {err}");
                return;
            }
        };
        let event_kind = value
            .get("type")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        for loaded in &self.loaded {
            if !loaded.report.hooks.iter().any(|hook| hook == "event") {
                continue;
            }
            match self.host.invoke_event(&loaded.spec, value.clone()) {
                Ok(_) => self.record_hook("event", "success", event_kind.as_deref(), loaded),
                Err(err) => {
                    eprintln!("cortex plugin event: {err}");
                    self.record_hook("event", "error", Some(&err.to_string()), loaded);
                }
            }
        }
    }

    /// Invoke every loaded plugin's `dispose` hook once.
    pub fn dispose(&self) {
        if self.disposed.swap(true, Ordering::SeqCst) {
            return;
        }
        for loaded in &self.loaded {
            if !loaded.report.hooks.iter().any(|hook| hook == "dispose") {
                continue;
            }
            match self.host.invoke_dispose(&loaded.spec) {
                Ok(_) => self.record_hook("dispose", "success", None, loaded),
                Err(err) => {
                    eprintln!("cortex plugin dispose: {err}");
                    self.record_hook("dispose", "error", Some(&err.to_string()), loaded);
                }
            }
        }
    }

    /// Record a typed `plugin_hook` audit entry for one real hook invocation when an audit sink is
    /// attached. The sink supplies the surface/source attribution.
    fn record_hook(&self, hook: &str, outcome: &str, detail: Option<&str>, loaded: &LoadedPlugin) {
        if let Some(audit) = &self.audit {
            audit.record_plugin_hook(loaded.spec.raw_spec(), hook, hook, outcome, detail);
        }
    }
}

impl Default for PluginEventHooks {
    fn default() -> Self {
        Self::disabled("run")
    }
}

impl Drop for PluginEventHooks {
    fn drop(&mut self) {
        self.dispose();
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
    /// Permission rules contributed by the `config` hook.
    pub config_rules: Vec<PluginConfigRule>,
    /// The effective config object observed/returned by the plugin `config` hook chain.
    pub config: Option<JsonValue>,
    /// Tools contributed by the `tool` hook.
    pub tools: Vec<PluginToolRegistration>,
    /// Authentication methods contributed by the `auth` hook.
    pub auth_methods: Vec<PluginAuthRegistration>,
    /// Model providers contributed by the `provider` hook.
    pub providers: Vec<PluginProviderRegistration>,
    /// Workspace adapters contributed by `experimental_workspace.register`.
    pub workspace_entries: Vec<PluginWorkspaceRegistration>,
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

/// Output from a later hook invocation that transforms a JSON value.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginHookOutput {
    /// Plugin file whose hook was invoked.
    pub plugin_path: PathBuf,
    /// Runtime used for this invocation.
    pub runtime: JsRuntime,
    /// Direct runtime PID observed inside JavaScript.
    pub runtime_pid: Option<u32>,
    /// Hook names actually invoked by the bridge.
    pub invoked_hooks: Vec<String>,
    /// Hook return value after bridge normalization.
    pub output: Option<JsonValue>,
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

/// A plugin-contributed workspace adapter registered through `experimental_workspace.register`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginWorkspaceRegistration {
    /// Stable workspace adapter type id.
    #[serde(rename = "type")]
    pub workspace_type: String,
    /// Human-readable adapter name.
    #[serde(default)]
    pub name: String,
    /// Optional description shown in workspace discovery surfaces.
    #[serde(default)]
    pub description: String,
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
    /// A package plugin could not be resolved by the external runtime resolver.
    #[error("failed to resolve plugin package `{spec}`: {message}")]
    PackageResolveFailed {
        /// Configured package spec.
        spec: String,
        /// Resolver diagnostic.
        message: String,
    },
    /// Package resolution would need the network while the kill-switch is engaged.
    #[error(
        "failed to resolve plugin package `{spec}`: CORTEX_NO_NET=1 prevents fetching package plugins; install it locally or keep the plugin lockfile"
    )]
    PackageNetDisabled {
        /// Configured package spec.
        spec: String,
    },
    /// A package manager install failed.
    #[error(
        "failed to install plugin package `{spec}` with {runtime} (status {status}); stderr: {stderr}"
    )]
    PackageInstallFailed {
        /// Configured package spec.
        spec: String,
        /// Selected runtime.
        runtime: JsRuntime,
        /// Exit status.
        status: String,
        /// Redacted stderr.
        stderr: String,
    },
    /// The plugin lockfile could not be read or written.
    #[error("failed to use plugin lockfile for `{spec}`: {message}")]
    PackageLock {
        /// Configured package spec.
        spec: String,
        /// Lockfile diagnostic.
        message: String,
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
    root: String,
    worktree: String,
    server_url: Option<String>,
    options: JsonValue,
}

#[derive(Serialize)]
struct BridgeRequest {
    operation: BridgeOperation,
}

#[derive(Serialize)]
struct PackageResolveInput {
    spec: String,
    import_spec: String,
    base_directory: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeOperation {
    Load { config: JsonValue, surface: String },
    Event { event: JsonValue },
    ChatMessage { message: JsonValue },
    ChatParams { params: JsonValue },
    ChatHeaders { headers: JsonValue },
    ToolDefinition { definition: JsonValue },
    PermissionAsk { request: JsonValue },
    CommandBefore { command: JsonValue },
    ToolBefore { call: JsonValue },
    ToolAfter { result: JsonValue },
    ShellEnv,
    Dispose,
    ChatMessagesTransform { messages: JsonValue },
    ChatSystemTransform { system: String },
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
struct PackageResolveResult {
    ok: bool,
    resolved_path: Option<String>,
    package_name: Option<String>,
    version: Option<String>,
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
    config: Option<JsonValue>,
    #[serde(default)]
    tools: Vec<PluginToolRegistration>,
    #[serde(default)]
    auth_methods: Vec<PluginAuthRegistration>,
    #[serde(default)]
    providers: Vec<PluginProviderRegistration>,
    #[serde(default)]
    workspace_entries: Vec<PluginWorkspaceRegistration>,
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

fn run_package_child(
    runtime: &RuntimeSelection,
    spec: &str,
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
            return Err(PluginRuntimeError::PackageResolveFailed {
                spec: spec.to_owned(),
                message: format!(
                    "runtime resolver timed out after {}ms; runtime process pid {pid} was terminated",
                    timeout.as_millis()
                ),
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

fn run_install_child(
    program: &Path,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<ChildOutput, std::io::Error> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
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

    let mut child = command.spawn()?;
    let pid = child.id();
    let stdout = child.stdout.take().map(read_pipe);
    let stderr = child.stderr.take().map(read_pipe);
    let started = Instant::now();

    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            kill_runtime_process_group(pid);
            let _ = child.kill();
            let _ = child.wait();
            return Ok(ChildOutput {
                status: timeout_status(),
                stdout: join_pipe(stdout),
                stderr: join_pipe(stderr),
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    Ok(ChildOutput {
        status,
        stdout: join_pipe(stdout),
        stderr: join_pipe(stderr),
    })
}

#[cfg(unix)]
fn timeout_status() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(124 << 8)
}

#[cfg(not(unix))]
fn timeout_status() -> ExitStatus {
    Command::new("false")
        .status()
        .unwrap_or_else(|_| panic!("failed to synthesize timeout exit status"))
}

#[cfg(unix)]
fn success_status() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

#[cfg(not(unix))]
fn success_status() -> ExitStatus {
    Command::new("true")
        .status()
        .unwrap_or_else(|_| panic!("failed to synthesize success exit status"))
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

fn runtime_stdout_loop<R: Read>(stdout: R, tx: Sender<String>) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if tx.send(trimmed.to_owned()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn runtime_stderr_loop<R: Read>(mut stderr_pipe: R, target: Arc<Mutex<Vec<u8>>>) {
    let mut buffer = Vec::new();
    let _ = stderr_pipe.read_to_end(&mut buffer);
    if let Ok(mut stderr) = target.lock() {
        stderr.extend(buffer);
    }
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

fn read_package_resolve_result(
    spec: &str,
    runtime: &RuntimeSelection,
    result_path: &Path,
    output: &ChildOutput,
) -> Result<PackageResolveResult, PluginRuntimeError> {
    let result = match std::fs::read_to_string(result_path) {
        Ok(result) => result,
        Err(_) if !output.status.success() => {
            return Err(PluginRuntimeError::PackageResolveFailed {
                spec: spec.to_owned(),
                message: format!(
                    "runtime resolver failed in {} (status {}); stderr: {}",
                    runtime.runtime,
                    status_text(output.status),
                    redaction::redact(&String::from_utf8_lossy(&output.stderr))
                ),
            });
        }
        Err(source) => {
            return Err(PluginRuntimeError::Io {
                action: "read plugin package resolver result",
                path: result_path.to_path_buf(),
                source,
            });
        }
    };
    let parsed: PackageResolveResult = serde_json::from_str(&result)?;
    if !output.status.success() && parsed.ok {
        return Err(PluginRuntimeError::PackageResolveFailed {
            spec: spec.to_owned(),
            message: format!(
                "runtime resolver exited non-zero in {} (status {}); stderr: {}",
                runtime.runtime,
                status_text(output.status),
                redaction::redact(&String::from_utf8_lossy(&output.stderr))
            ),
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

fn package_error_from_bridge_result(
    spec: &str,
    runtime: &RuntimeSelection,
    result: PackageResolveResult,
    fallback_hook: &str,
) -> PluginRuntimeError {
    let error = result.error.unwrap_or_else(|| BridgeError {
        name: "Error".to_owned(),
        message: "package resolver failed without an error message".to_owned(),
        stack: None,
        hook: Some(fallback_hook.to_owned()),
    });
    let message = redaction::redact(&error.message);
    PluginRuntimeError::PackageResolveFailed {
        spec: spec.to_owned(),
        message: format!(
            "{} resolver error in {}: {message}",
            runtime.runtime,
            runtime.executable.display()
        ),
    }
}

fn package_lock_error(spec: &str, err: PackageLockError) -> PluginRuntimeError {
    PluginRuntimeError::PackageLock {
        spec: spec.to_owned(),
        message: err.to_string(),
    }
}

fn default_parameters() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": true
    })
}

#[derive(Clone, Debug)]
struct PluginRuntimeContext {
    root: PathBuf,
    worktree: String,
    server_url: Option<String>,
}

impl PluginRuntimeContext {
    fn resolve(directory: &Path) -> Self {
        let root = git_output(directory, &["rev-parse", "--show-toplevel"])
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.to_path_buf());
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        let worktree = git_output(directory, &["rev-parse", "--absolute-git-dir"])
            .map(|path| {
                std::fs::canonicalize(Path::new(&path))
                    .unwrap_or_else(|_| PathBuf::from(path))
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| root.to_string_lossy().into_owned());

        let server_url = std::env::var("CORTEX_SERVER_URL")
            .ok()
            .and_then(|raw| normalize_server_url(&raw))
            .or_else(discover_loopback_server_url);

        Self {
            root,
            worktree,
            server_url,
        }
    }
}

fn git_output(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn normalize_server_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let authority = trimmed.strip_prefix("http://")?.split('/').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    let port = port.parse::<u16>().ok()?;
    if !(3200..=3299).contains(&port) {
        return None;
    }
    if host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") {
        Some(format!("http://127.0.0.1:{port}"))
    } else {
        None
    }
}

fn discover_loopback_server_url() -> Option<String> {
    for port in 3200..=3299 {
        if loopback_healthcheck(port) {
            return Some(format!("http://127.0.0.1:{port}"));
        }
    }
    None
}

fn loopback_healthcheck(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(10)) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(30)));
    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut buffer = [0_u8; 256];
    let Ok(read) = stream.read(&mut buffer) else {
        return false;
    };
    if read == 0 {
        return false;
    }
    let response = String::from_utf8_lossy(&buffer[..read]);
    response.contains(" 200 ") && response.contains("\"status\":\"ok\"")
}

fn non_empty_surface(surface: &str) -> String {
    let surface = surface.trim();
    if surface.is_empty() {
        "run".to_owned()
    } else {
        surface.to_owned()
    }
}

fn event_payload_for_surface<T: Serialize>(
    event: &T,
    surface: &str,
) -> Result<JsonValue, serde_json::Error> {
    let surface = non_empty_surface(surface);
    match serde_json::to_value(event)? {
        JsonValue::Object(mut map) => {
            map.entry("surface".to_owned())
                .or_insert_with(|| JsonValue::String(surface));
            if !map.contains_key("type")
                && let Some(kind) = map.get("kind").and_then(JsonValue::as_str)
            {
                map.insert("type".to_owned(), JsonValue::String(kind.to_owned()));
            }
            Ok(JsonValue::Object(map))
        }
        other => {
            let mut map = serde_json::Map::new();
            map.insert("kind".to_owned(), JsonValue::String("event".to_owned()));
            map.insert("type".to_owned(), JsonValue::String("event".to_owned()));
            map.insert("surface".to_owned(), JsonValue::String(surface));
            map.insert("payload".to_owned(), other);
            Ok(JsonValue::Object(map))
        }
    }
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

const PACKAGE_RESOLVER_SCRIPT: &str = r#"
import processModule from "node:process";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { readFile, writeFile } from "node:fs/promises";

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
    stack: error?.stack ? String(error.stack) : undefined,
    hook: "resolve"
  };
}

async function nearestPackageJson(resolvedPath) {
  let current = dirname(resolvedPath);
  for (;;) {
    const candidate = join(current, "package.json");
    try {
      const text = await readFile(candidate, "utf8");
      const parsed = JSON.parse(text);
      if (parsed && typeof parsed === "object") {
        return { path: candidate, packageJson: parsed };
      }
    } catch (_error) {
      // Keep walking toward the filesystem root.
    }
    const parent = dirname(current);
    if (parent === current) break;
    current = parent;
  }
  throw new Error(`could not find package.json for resolved module ${resolvedPath}`);
}

try {
  const raw = JSON.parse(await readFile(inputPath, "utf8"));
  const baseDirectory = String(raw.base_directory);
  const importSpec = String(raw.import_spec);
  const require = createRequire(pathToFileURL(join(baseDirectory, "package.json")).href);
  const resolvedPath = require.resolve(importSpec);
  const { packageJson } = await nearestPackageJson(resolvedPath);
  const version = String(packageJson.version ?? "");
  if (!version) {
    throw new Error(`package ${importSpec} resolved to ${resolvedPath} but package.json has no version`);
  }
  await writeResult({
    ok: true,
    runtime: runtimeInfo(),
    pid: typeof proc.pid === "number" ? proc.pid : null,
    resolved_path: resolvedPath,
    package_name: String(packageJson.name ?? importSpec),
    version
  });
} catch (error) {
  await writeResult({
    ok: false,
    runtime: runtimeInfo(),
    pid: typeof proc.pid === "number" ? proc.pid : null,
    error: errorToJson(error)
  });
  proc.exitCode = 1;
}
"#;

const BRIDGE_SCRIPT: &str = r#"
import processModule from "node:process";
import { pathToFileURL } from "node:url";
import { readFile } from "node:fs/promises";
import { execFile } from "node:child_process";
import { createInterface } from "node:readline";

const proc = globalThis.process ?? processModule;
const inputPath = proc.argv[2];

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

function stringifyValue(value) {
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch (_error) {
    return String(value);
  }
}

function writeStderr(...values) {
  try {
    proc.stderr.write(`${values.map(stringifyValue).join(" ")}\n`);
  } catch (_error) {
    // ignore logging failures
  }
}

if (globalThis.console) {
  globalThis.console.log = (...values) => writeStderr(...values);
  globalThis.console.info = (...values) => writeStderr(...values);
  globalThis.console.warn = (...values) => writeStderr(...values);
  globalThis.console.error = (...values) => writeStderr(...values);
}

async function writeResultLine(result) {
  proc.stdout.write(`${JSON.stringify(result)}\n`);
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
  const normalizeEnv = (baseEnv, extraEnv) => {
    if (!extraEnv || typeof extraEnv !== "object" || Array.isArray(extraEnv)) {
      return baseEnv;
    }
    const merged = { ...baseEnv };
    for (const [name, value] of Object.entries(extraEnv)) {
      if (value === undefined || value === null) continue;
      merged[String(name)] = String(value);
    }
    return merged;
  };

  const toShellOutput = (stdoutRaw, stderrRaw, exitCode, pid, timedOut) => {
    const stdout = String(stdoutRaw ?? "");
    const stderr = String(stderrRaw ?? "");
    const bytes = Buffer.from(stdout);
    return {
      stdout,
      stderr,
      exitCode,
      code: exitCode,
      ok: exitCode === 0,
      pid: pid ?? null,
      timedOut: Boolean(timedOut),
      text() {
        return stdout;
      },
      json() {
        if (!stdout.trim()) return null;
        return JSON.parse(stdout);
      },
      arrayBuffer() {
        return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
      },
      bytes() {
        return new Uint8Array(bytes);
      },
      blob() {
        return new Blob([bytes]);
      }
    };
  };

  const decoratePromise = (promise) => {
    const shellPromise = promise;
    shellPromise.cwd = () => shellPromise;
    shellPromise.env = () => shellPromise;
    shellPromise.quiet = () => shellPromise;
    shellPromise.nothrow = () => shellPromise;
    shellPromise.throws = () => shellPromise;
    shellPromise.lines = async function* () {
      const text = await shellPromise.text();
      for (const line of text.split(/\r?\n/u)) {
        if (line.length > 0) {
          yield line;
        }
      }
    };
    shellPromise.text = async () => (await shellPromise).text();
    shellPromise.json = async () => (await shellPromise).json();
    shellPromise.arrayBuffer = async () => (await shellPromise).arrayBuffer();
    shellPromise.blob = async () => (await shellPromise).blob();
    return shellPromise;
  };

  const createShell = (defaults) => {
    const run = (command, options = {}) =>
      decoratePromise(
        new Promise((resolve) => {
          const timeout = Number(options.timeout ?? defaults.timeout);
          const cwd = String(options.cwd ?? defaults.cwd ?? directory);
          const env = normalizeEnv(normalizeEnv(proc.env, defaults.env), options.env);
          const child = execFile(
            "bash",
            ["-lc", String(command)],
            {
              cwd,
              timeout,
              maxBuffer: 10 * 1024 * 1024,
              env
            },
            (error, stdout, stderr) => {
              const exitCode = typeof error?.code === "number" ? error.code : 0;
              resolve(
                toShellOutput(
                  stdout,
                  stderr,
                  exitCode,
                  child.pid ?? null,
                  Boolean(error?.killed)
                )
              );
            }
          );
        })
      );

    const tagged = (strings, ...values) => {
      if (Array.isArray(strings) && Object.prototype.hasOwnProperty.call(strings, "raw")) {
        return run(commandFromTemplate(strings, values));
      }
      return run(strings);
    };
    tagged.run = run;
    tagged.escape = shellEscape;
    tagged.braces = (pattern) => [String(pattern)];
    tagged.cwd = (cwd) =>
      createShell({
        ...defaults,
        cwd: String(cwd ?? defaults.cwd ?? directory)
      });
    tagged.env = (extraEnv) =>
      createShell({
        ...defaults,
        env: normalizeEnv(defaults.env ?? {}, extraEnv)
      });
    tagged.nothrow = () => tagged;
    tagged.throws = () => tagged;
    return tagged;
  };

  return createShell({
    cwd: directory,
    env: {},
    timeout: 120000
  });
}

function resolvePluginFunction(module) {
  const candidate = module.default ?? module.plugin ?? module.activate;
  if (typeof candidate !== "function") {
    throw new Error("plugin module must export a function (default export, plugin, or activate)");
  }
  return candidate;
}

function normalizeServerUrl(value) {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function buildClient(serverUrl) {
  const available = Boolean(serverUrl);
  async function request(path, init = {}) {
    if (!available) {
      throw new Error("plugin client unavailable: no local server URL was resolved");
    }
    const target = new URL(String(path), `${serverUrl}/`).toString();
    const response = await fetch(target, init);
    const bodyText = await response.text();
    let body;
    try {
      body = bodyText.length > 0 ? JSON.parse(bodyText) : null;
    } catch (_error) {
      body = bodyText;
    }
    if (!response.ok) {
      throw new Error(`plugin client request failed (${response.status} ${response.statusText})`);
    }
    return body;
  }
  const session = {
    list: async () => request("/session"),
    create: async (payload = {}) => request("/session", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload ?? {})
    })
  };
  return {
    kind: available ? "server" : "none",
    available,
    serverUrl,
    request,
    session,
    sessions: session
  };
}

function createInput(raw, directory) {
  const shell = makeShell(directory);
  const root = String(raw.root ?? directory);
  const worktree = String(raw.worktree ?? root);
  const serverUrl = normalizeServerUrl(raw.server_url);
  const client = buildClient(serverUrl);
  const registeredTools = [];
  const registeredAuth = [];
  const registeredProviders = [];
  const registeredWorkspaces = [];
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
  const registerWorkspace = (type, adapter) => {
    const workspaceType = String(type ?? "").trim();
    if (!workspaceType || !adapter || typeof adapter !== "object") {
      return adapter;
    }
    registeredWorkspaces.push({
      type: workspaceType,
      name: String(adapter.name ?? workspaceType),
      description: String(adapter.description ?? "")
    });
    return adapter;
  };
  return {
    input: {
      project: { directory, root, worktree },
      directory,
      root,
      worktree,
      serverUrl,
      server: { kind: serverUrl ? "loopback" : "none", url: serverUrl },
      client,
      shell: shell.run,
      $: shell,
      experimental_workspace: {
        register(type, adapter) {
          return registerWorkspace(type, adapter);
        },
        registerTool,
        registerAuth,
        registerProvider,
        registerWorkspace
      }
    },
    registeredTools,
    registeredAuth,
    registeredProviders,
    registeredWorkspaces
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

function normalizeWorkspace(entry) {
  if (!entry || typeof entry !== "object") return null;
  const type = String(entry.type ?? entry.id ?? "").trim();
  if (!type) return null;
  return {
    type,
    name: String(entry.name ?? type),
    description: String(entry.description ?? "")
  };
}

function cloneObject(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {};
  }
  return JSON.parse(JSON.stringify(value));
}

function normalizeConfigResult(returned, mutated) {
  const base = cloneObject(mutated);
  if (returned === undefined) {
    return base;
  }
  if (Array.isArray(returned)) {
    base.permission = returned;
    return base;
  }
  if (returned && typeof returned === "object") {
    return { ...base, ...returned };
  }
  return base;
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

function getHook(hooks, path) {
  if (!hooks || typeof hooks !== "object") return undefined;
  if (typeof hooks[path] === "function") return hooks[path];
  const segments = String(path).split(".");
  let current = hooks;
  for (let i = 0; i < segments.length; i += 1) {
    if (!current || typeof current !== "object") return undefined;
    const remaining = segments.slice(i).join(".");
    if (typeof current[remaining] === "function") return current[remaining];
    current = current[segments[i]];
  }
  return typeof current === "function" ? current : undefined;
}

function listHookEntries(value, prefix = "") {
  if (!value || typeof value !== "object") return [];
  const entries = [];
  for (const [key, item] of Object.entries(value)) {
    const name = prefix ? `${prefix}.${key}` : key;
    if (typeof item === "function") {
      entries.push(name);
    } else if (item && typeof item === "object" && !Array.isArray(item)) {
      entries.push(...listHookEntries(item, name));
    }
  }
  return entries;
}

async function callHook(hooks, path, args, invokedHooks) {
  const hook = getHook(hooks, path);
  if (typeof hook !== "function") return undefined;
  activeHook = path;
  const output = await hook(...args);
  invokedHooks.push(path);
  return output;
}

function normalizeObjectResult(returned, mutated) {
  const source = returned === undefined || returned === null ? mutated : returned;
  if (source && typeof source === "object" && !Array.isArray(source)) return source;
  return {};
}

function normalizeArrayResult(returned, mutated) {
  const source = returned === undefined || returned === null ? mutated : returned;
  return Array.isArray(source) ? source : mutated;
}

function normalizeMessageResult(returned, mutated) {
  const source = returned === undefined || returned === null ? mutated : returned;
  if (typeof source === "string") return { ...mutated, content: source };
  if (source && typeof source === "object" && !Array.isArray(source)) {
    return { ...mutated, ...source };
  }
  return mutated;
}

function normalizeDefinitionResult(returned, mutated) {
  const source = returned === undefined || returned === null ? mutated : returned;
  if (source && typeof source === "object" && !Array.isArray(source)) {
    return { ...mutated, ...source };
  }
  return mutated;
}

function normalizeObjectMutationResult(returned, mutated) {
  if (returned === undefined || returned === null) return mutated;
  if (typeof returned === "string") return { ...mutated, decision: returned };
  if (returned && typeof returned === "object" && !Array.isArray(returned)) {
    return { ...mutated, ...returned };
  }
  return mutated;
}

function normalizeEnvResult(returned) {
  if (!returned || typeof returned !== "object" || Array.isArray(returned)) return {};
  const env = returned.env && typeof returned.env === "object" && !Array.isArray(returned.env)
    ? returned.env
    : returned;
  const out = {};
  for (const [key, value] of Object.entries(env)) {
    if (value === undefined || value === null) continue;
    out[String(key)] = String(value);
  }
  return out;
}

function normalizeSystemResult(returned, mutated) {
  if (typeof returned === "string") return returned;
  if (returned && typeof returned === "object" && typeof returned.content === "string") {
    return returned.content;
  }
  return mutated;
}

async function loadHooks(raw, directory) {
  const context = createInput(raw, directory);
  const module = await import(pathToFileURL(raw.plugin_path).href);
  const plugin = resolvePluginFunction(module);
  const hooks = await plugin(context.input, raw.options ?? {});
  const hookEntries = listHookEntries(hooks);
  return { hooks: hooks ?? {}, hookEntries, context };
}

async function runOperation(operation, hooks, context, directory) {
  const invokedHooks = [];
  const registrations = {
    config_rules: [],
    config: undefined,
    tools: [],
    auth_methods: [],
    providers: [],
    workspace_entries: []
  };
  let output = undefined;

  if (operation.type === "load") {
    const eventHook = getHook(hooks, "event");
    if (typeof eventHook === "function") {
      activeHook = "event";
      const surface = String(operation.surface ?? "run").trim() || "run";
      await eventHook({
        kind: "lifecycle",
        type: "lifecycle",
        phase: "startup",
        surface,
        directory
      });
      invokedHooks.push("event");
    }
    const configHook = getHook(hooks, "config");
    if (typeof configHook === "function") {
      activeHook = "config";
      const inputConfig = cloneObject(operation.config);
      const returned = await configHook(inputConfig);
      const normalized = normalizeConfigResult(returned, inputConfig);
      registrations.config = normalized;
      registrations.config_rules = normalizeConfigRules(normalized);
      invokedHooks.push("config");
    }
    const toolHook = hooks?.tool;
    if (typeof toolHook === "function") {
      activeHook = "tool";
      const returned = await toolHook({
        register: context.input.experimental_workspace.registerTool,
        registerTool: context.input.experimental_workspace.registerTool
      });
      registrations.tools = [...context.registeredTools, ...entriesFrom(returned)].map(normalizeTool).filter(Boolean);
      invokedHooks.push("tool");
    } else if (toolHook && typeof toolHook === "object") {
      registrations.tools = entriesFrom(toolHook).map(normalizeTool).filter(Boolean);
      invokedHooks.push("tool");
    }
    const authHook = getHook(hooks, "auth");
    if (typeof authHook === "function") {
      activeHook = "auth";
      const returned = await authHook({ register: context.input.experimental_workspace.registerAuth });
      registrations.auth_methods = [...context.registeredAuth, ...entriesFrom(returned)].map(normalizeAuth).filter(Boolean);
      invokedHooks.push("auth");
    }
    const providerHook = getHook(hooks, "provider");
    if (typeof providerHook === "function") {
      activeHook = "provider";
      const returned = await providerHook({ register: context.input.experimental_workspace.registerProvider });
      registrations.providers = [...context.registeredProviders, ...entriesFrom(returned)].map(normalizeProvider).filter(Boolean);
      invokedHooks.push("provider");
    }
    registrations.workspace_entries = context.registeredWorkspaces.map(normalizeWorkspace).filter(Boolean);
  } else if (operation.type === "event") {
    const eventHook = getHook(hooks, "event");
    if (typeof eventHook === "function") {
      activeHook = "event";
      await eventHook(operation.event ?? {});
      invokedHooks.push("event");
    }
  } else if (operation.type === "chat_message") {
    const message = operation.message && typeof operation.message === "object"
      ? { ...operation.message }
      : { role: "user", content: String(operation.message ?? "") };
    const returned = await callHook(hooks, "chat.message", [message, {
      directory,
      hook: "chat.message"
    }], invokedHooks);
    output = normalizeMessageResult(returned, message);
  } else if (operation.type === "chat_params") {
    const params = operation.params && typeof operation.params === "object" && !Array.isArray(operation.params)
      ? { ...operation.params }
      : {};
    const returned = await callHook(hooks, "chat.params", [params, {
      directory,
      hook: "chat.params"
    }], invokedHooks);
    output = normalizeObjectResult(returned, params);
  } else if (operation.type === "chat_headers") {
    const headers = operation.headers && typeof operation.headers === "object" && !Array.isArray(operation.headers)
      ? { ...operation.headers }
      : {};
    const returned = await callHook(hooks, "chat.headers", [headers, {
      directory,
      hook: "chat.headers"
    }], invokedHooks);
    output = normalizeObjectResult(returned, headers);
  } else if (operation.type === "tool_definition") {
    const definition = operation.definition && typeof operation.definition === "object" && !Array.isArray(operation.definition)
      ? { ...operation.definition }
      : {};
    const returned = await callHook(hooks, "tool.definition", [definition, {
      directory,
      hook: "tool.definition"
    }], invokedHooks);
    output = normalizeDefinitionResult(returned, definition);
  } else if (operation.type === "permission_ask") {
    const request = operation.request && typeof operation.request === "object" && !Array.isArray(operation.request)
      ? { ...operation.request }
      : {};
    const returned = await callHook(hooks, "permission.ask", [request, {
      directory,
      hook: "permission.ask"
    }], invokedHooks);
    output = normalizeObjectMutationResult(returned, request);
  } else if (operation.type === "command_before") {
    const command = operation.command && typeof operation.command === "object" && !Array.isArray(operation.command)
      ? { ...operation.command }
      : {};
    const returned = await callHook(hooks, "command.execute.before", [command, {
      directory,
      hook: "command.execute.before"
    }], invokedHooks);
    output = normalizeObjectMutationResult(returned, command);
  } else if (operation.type === "tool_before") {
    const call = operation.call && typeof operation.call === "object" && !Array.isArray(operation.call)
      ? { ...operation.call }
      : {};
    const returned = await callHook(hooks, "tool.execute.before", [call, {
      directory,
      hook: "tool.execute.before"
    }], invokedHooks);
    output = normalizeObjectMutationResult(returned, call);
  } else if (operation.type === "tool_after") {
    const result = operation.result && typeof operation.result === "object" && !Array.isArray(operation.result)
      ? { ...operation.result }
      : {};
    const returned = await callHook(hooks, "tool.execute.after", [result, {
      directory,
      hook: "tool.execute.after"
    }], invokedHooks);
    output = normalizeObjectMutationResult(returned, result);
  } else if (operation.type === "shell_env") {
    const returned = await callHook(hooks, "shell.env", [{
      directory,
      cwd: directory,
      env: {}
    }, {
      directory,
      hook: "shell.env"
    }], invokedHooks);
    output = normalizeEnvResult(returned);
  } else if (operation.type === "dispose") {
    await callHook(hooks, "dispose", [{
      directory,
      cwd: directory
    }, {
      directory,
      hook: "dispose"
    }], invokedHooks);
  } else if (operation.type === "chat_messages_transform") {
    const messages = Array.isArray(operation.messages)
      ? operation.messages.map((message) => message && typeof message === "object" ? { ...message } : message)
      : [];
    let returned = await callHook(hooks, "experimental.chat.messages.transform", [messages, {
      directory,
      hook: "experimental.chat.messages.transform"
    }], invokedHooks);
    if (returned === undefined && getHook(hooks, "chat.messages")) {
      returned = await callHook(hooks, "chat.messages", [messages, {
        directory,
        hook: "chat.messages"
      }], invokedHooks);
    }
    output = normalizeArrayResult(returned, messages);
  } else if (operation.type === "chat_system_transform") {
    const system = String(operation.system ?? "");
    let returned = await callHook(hooks, "experimental.chat.system.transform", [system, {
      directory,
      hook: "experimental.chat.system.transform"
    }], invokedHooks);
    if (returned === undefined && getHook(hooks, "chat.system")) {
      returned = await callHook(hooks, "chat.system", [system, {
        directory,
        hook: "chat.system"
      }], invokedHooks);
    }
    output = normalizeSystemResult(returned, system);
  } else if (operation.type === "tool_execute") {
    const toolHook = hooks?.tool;
    if (!(typeof toolHook === "function" || (toolHook && typeof toolHook === "object"))) {
      throw new Error("plugin did not define a tool hook");
    }
    let returned = undefined;
    if (typeof toolHook === "function") {
      activeHook = "tool";
      returned = await toolHook({
        register: context.input.experimental_workspace.registerTool,
        registerTool: context.input.experimental_workspace.registerTool
      });
    } else {
      returned = toolHook;
    }
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
    const providerHook = getHook(hooks, "provider");
    if (typeof providerHook !== "function") {
      throw new Error("plugin did not define a provider hook");
    }
    activeHook = "provider";
    const returned = await providerHook({ register: context.input.experimental_workspace.registerProvider });
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

  return { invokedHooks, registrations, output };
}

async function handleRequest(request, hooks, hookEntries, context, directory) {
  try {
    const operation = request?.operation ?? request ?? { type: "load" };
    const { invokedHooks, registrations, output } = await runOperation(
      operation,
      hooks,
      context,
      directory
    );
    await writeResultLine({
      ok: true,
      runtime: runtimeInfo(),
      pid: typeof proc.pid === "number" ? proc.pid : null,
      hooks: hookEntries,
      invoked_hooks: invokedHooks,
      registrations,
      output
    });
  } catch (error) {
    await writeResultLine({
      ok: false,
      runtime: runtimeInfo(),
      pid: typeof proc.pid === "number" ? proc.pid : null,
      hooks: [],
      invoked_hooks: [],
      registrations: {
        config_rules: [],
        config: undefined,
        tools: [],
        auth_methods: [],
        providers: [],
        workspace_entries: []
      },
      error: errorToJson(error)
    });
  }
}

try {
  const raw = JSON.parse(await readFile(inputPath, "utf8"));
  const directory = String(raw.directory);
  activeHook = "load";
  const { hooks, hookEntries, context } = await loadHooks(raw, directory);
  const lines = createInterface({ input: proc.stdin, crlfDelay: Infinity, terminal: false });
  for await (const line of lines) {
    const trimmed = String(line ?? "").trim();
    if (!trimmed) continue;
    let request;
    try {
      request = JSON.parse(trimmed);
    } catch (error) {
      await writeResultLine({
        ok: false,
        runtime: runtimeInfo(),
        pid: typeof proc.pid === "number" ? proc.pid : null,
        hooks: [],
        invoked_hooks: [],
        registrations: {
          config_rules: [],
          config: undefined,
          tools: [],
          auth_methods: [],
          providers: [],
          workspace_entries: []
        },
        error: errorToJson(error)
      });
      continue;
    }
    await handleRequest(request, hooks, hookEntries, context, directory);
  }
} catch (error) {
  await writeResultLine({
    ok: false,
    runtime: runtimeInfo(),
    pid: typeof proc.pid === "number" ? proc.pid : null,
    hooks: [],
    invoked_hooks: [],
    registrations: {
      config_rules: [],
      config: undefined,
      tools: [],
      auth_methods: [],
      providers: [],
      workspace_entries: []
    },
    error: errorToJson(error)
  });
  proc.exitCode = 1;
}
"#;

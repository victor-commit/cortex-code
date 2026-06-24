//! CLI-owned integration of plugin registrations with run/auth/model surfaces.

use std::ffi::OsString;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use engine::ToolDispatcher;
use events::{Event, Subscriber};
use futures_util::future::BoxFuture;
use llm::{
    FinishReason, LlmError, LlmEvent, LlmRequest, LlmStream, Message, Provider, Role, ToolCall,
    ToolDef,
};
use permissions::{Action, Decision, DecisionOverride, PermissionRequest, Rule};
use plugins::{
    LoadedPlugin, PluginConfigRule, PluginModelRegistration, PluginProviderRegistration,
    PluginRuntimeError, PluginSpec, PluginToolRegistration, RuntimeHost,
};
use serde::Serialize;
use serde_json::Value as JsonValue;
use settings::{ConfigError, ConfigInputs};
use tools::{Tool, ToolContext, ToolOutput};

const PURE_ENV: &str = "CORTEX_PURE";

/// Loaded plugin runtime state for one surface invocation.
#[derive(Clone, Debug)]
pub(crate) struct PluginState {
    loaded: Vec<LoadedPlugin>,
    host: RuntimeHost,
    surface: Arc<str>,
    disposed: Arc<AtomicBool>,
    audit: Arc<Mutex<Option<Arc<audit::AuditSink>>>>,
}

impl Default for PluginState {
    fn default() -> Self {
        Self {
            loaded: Vec::new(),
            host: RuntimeHost::new(),
            surface: Arc::from("run"),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: Arc::new(Mutex::new(None)),
        }
    }
}

impl PluginState {
    /// A disabled plugin state used by `--pure` and `CORTEX_PURE=1`.
    #[must_use]
    pub(crate) fn disabled() -> Self {
        Self::disabled_for_surface("run")
    }

    /// A disabled plugin state for a named surface.
    #[must_use]
    pub(crate) fn disabled_for_surface(surface: &str) -> Self {
        Self {
            loaded: Vec::new(),
            host: RuntimeHost::new(),
            surface: Arc::from(non_empty_surface(surface)),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: Arc::new(Mutex::new(None)),
        }
    }

    /// Load configured plugins using the real external JS runtime.
    pub(crate) fn load(config: &plugins::PluginConfig) -> Result<Self, PluginRuntimeError> {
        Self::load_for_surface(config, "run")
    }

    /// Load configured plugins for a named surface using the real external JS runtime.
    pub(crate) fn load_for_surface(
        config: &plugins::PluginConfig,
        surface: &str,
    ) -> Result<Self, PluginRuntimeError> {
        let host = RuntimeHost::new();
        let surface = non_empty_surface(surface);
        let loaded = host.load_plugins_for_surface(config, &surface)?;
        Ok(Self {
            loaded,
            host,
            surface: Arc::from(surface),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: Arc::new(Mutex::new(None)),
        })
    }

    /// Load configured plugins unless pure mode disabled all plugin behavior.
    pub(crate) fn load_or_disabled(
        config: &plugins::PluginConfig,
        pure: bool,
    ) -> Result<Self, PluginRuntimeError> {
        Self::load_or_disabled_for_surface(config, pure, "run")
    }

    /// Load configured plugins for a named surface unless pure mode disabled all plugin behavior.
    pub(crate) fn load_or_disabled_for_surface(
        config: &plugins::PluginConfig,
        pure: bool,
        surface: &str,
    ) -> Result<Self, PluginRuntimeError> {
        if pure {
            Ok(Self::disabled_for_surface(surface))
        } else {
            Self::load_for_surface(config, surface)
        }
    }

    /// Attach the durable audit sink after the run session id has been resolved.
    pub(crate) fn attach_audit(&self, audit: Arc<audit::AuditSink>) {
        if let Ok(mut slot) = self.audit.lock() {
            *slot = Some(audit);
        }
    }

    /// Clone this plugin state while replacing the attached audit sink.
    ///
    /// The returned state reuses the same loaded plugins/runtime host and shares dispose
    /// coordination, but it owns an independent audit slot so nested runs (for example subagents)
    /// can attribute plugin-initiated records to a different session.
    #[must_use]
    pub(crate) fn clone_with_audit(&self, audit: Option<Arc<audit::AuditSink>>) -> Self {
        Self {
            loaded: self.loaded.clone(),
            host: self.host.clone(),
            surface: self.surface.clone(),
            disposed: self.disposed.clone(),
            audit: Arc::new(Mutex::new(audit)),
        }
    }

    fn audit_sink(&self) -> Option<Arc<audit::AuditSink>> {
        self.audit.lock().ok().and_then(|slot| slot.clone())
    }

    /// Record the already-completed plugin load reports in the attached audit sink.
    pub(crate) fn audit_loaded_plugins(&self) {
        let Some(audit) = self.audit_sink() else {
            return;
        };
        for loaded in &self.loaded {
            audit.record_plugin_load(loaded.spec.raw_spec(), &loaded.report.hooks, "success");
        }
    }

    /// Resolve the current process config and load configured plugins for a non-run surface.
    pub(crate) fn from_process_config(pure: bool) -> Result<Self, ConfigOrRuntimeError> {
        if pure {
            return Ok(Self::disabled());
        }
        let inputs =
            ConfigInputs::from_process(Default::default()).map_err(ConfigOrRuntimeError::Io)?;
        let cwd = inputs.cwd.clone();
        let effective = settings::load(inputs).map_err(ConfigOrRuntimeError::Config)?;
        let config = plugins::PluginConfig::from_toml_value(effective.get("plugin"), &cwd)
            .map_err(|message| {
                ConfigOrRuntimeError::Config(ConfigError::TypeMismatch {
                    key: "plugin".to_owned(),
                    expected: "an array of plugin specs or [spec, options] pairs",
                    found: message,
                })
            })?;
        Self::load(&config).map_err(ConfigOrRuntimeError::Runtime)
    }

    /// Permission rules contributed by every loaded plugin's `config` hook.
    #[must_use]
    pub(crate) fn permission_rules(&self) -> Vec<Rule> {
        self.loaded
            .iter()
            .flat_map(|loaded| {
                loaded
                    .report
                    .config_rules
                    .iter()
                    .filter_map(plugin_rule_to_permission_rule)
            })
            .collect()
    }

    /// Model-callable tools contributed by every loaded plugin's `tool` hook.
    #[must_use]
    pub(crate) fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.loaded
            .iter()
            .flat_map(|loaded| {
                loaded.report.tools.iter().cloned().map(|registration| {
                    Box::new(PluginTool {
                        host: self.host.clone(),
                        spec: loaded.spec.clone(),
                        registration,
                    }) as Box<dyn Tool>
                })
            })
            .collect()
    }

    /// Providers/models contributed by plugin `provider` hooks.
    #[must_use]
    pub(crate) fn providers(&self) -> Vec<ProviderDoc> {
        self.loaded
            .iter()
            .flat_map(|loaded| loaded.report.providers.iter())
            .map(ProviderDoc::from_plugin)
            .collect()
    }

    /// A permission decision override hook, if any loaded plugin registered `permission.ask`.
    #[must_use]
    pub(crate) fn permission_override(&self) -> Option<Arc<dyn DecisionOverride>> {
        self.has_hook("permission.ask").then(|| {
            Arc::new(PluginPermissionOverride {
                state: self.clone(),
            }) as Arc<dyn DecisionOverride>
        })
    }

    /// Environment variables contributed by `shell.env` hooks for bash children.
    #[must_use]
    pub(crate) fn shell_env(&self) -> Vec<(OsString, OsString)> {
        let mut env = Vec::new();
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "shell.env") {
                continue;
            }
            match self.host.invoke_shell_env(&loaded.spec) {
                Ok(output) => {
                    if let Some(JsonValue::Object(map)) = output.output {
                        let entries = map
                            .into_iter()
                            .filter_map(|(name, value)| {
                                let value = match value {
                                    JsonValue::Null => return None,
                                    JsonValue::String(value) => value,
                                    other => other.to_string(),
                                };
                                Some((name, value))
                            })
                            .collect::<Vec<_>>();
                        if let Some(audit) = self.audit_sink() {
                            audit.record_plugin_shell_env(
                                loaded.spec.raw_spec(),
                                &entries,
                                "success",
                            );
                        }
                        env.extend(
                            entries
                                .into_iter()
                                .map(|(name, value)| (OsString::from(name), OsString::from(value))),
                        );
                    }
                }
                Err(err) => eprintln!("cortex plugin shell.env: {err}"),
            }
        }
        env
    }

    /// Find the plugin provider that declared `model`.
    #[must_use]
    pub(crate) fn provider_for_model(&self, model: &str) -> Option<PluginModelProvider> {
        self.loaded.iter().find_map(|loaded| {
            loaded.report.providers.iter().find_map(|provider| {
                provider.models.iter().find_map(|registered| {
                    (registered.id == model).then(|| PluginModelProvider {
                        host: self.host.clone(),
                        spec: loaded.spec.clone(),
                        provider: provider.clone(),
                        model: registered.clone(),
                    })
                })
            })
        })
    }

    /// Wrap the selected provider so request-shaping hooks run immediately before each outbound
    /// provider request. Plugins compose in the order they were configured.
    #[must_use]
    pub(crate) fn wrap_provider(&self, inner: Arc<dyn Provider>) -> Arc<dyn Provider> {
        if self.has_request_shaping_hooks() {
            Arc::new(PluginShapingProvider {
                inner,
                state: self.clone(),
            })
        } else {
            inner
        }
    }

    /// Wrap a dispatcher so `tool.definition` hooks can reshape the advertised schema snapshot.
    #[must_use]
    pub(crate) fn tool_definition_dispatcher<'a>(
        &'a self,
        inner: &'a dyn ToolDispatcher,
        session_id: impl Into<String>,
    ) -> PluginToolDefinitionDispatcher<'a> {
        PluginToolDefinitionDispatcher {
            inner,
            state: self,
            _session_id: session_id.into(),
        }
    }

    /// Apply `command.execute.before` hooks to a command invocation.
    pub(crate) fn command_before(&self, mut command: CommandInvocation) -> CommandBeforeOutcome {
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "command.execute.before") {
                continue;
            }
            let input = command.to_json();
            match self.host.invoke_command_before(&loaded.spec, input) {
                Ok(output) => {
                    let Some(value) = output.output else {
                        continue;
                    };
                    if let Some(reason) = veto_reason(&value) {
                        return CommandBeforeOutcome::Veto {
                            plugin: loaded.spec.raw_spec().to_owned(),
                            reason,
                        };
                    }
                    command = command_from_output(command, value);
                }
                Err(err) => eprintln!("cortex plugin command.execute.before: {err}"),
            }
        }
        CommandBeforeOutcome::Run(command)
    }

    fn permission_override_decision(
        &self,
        request: &PermissionRequest,
        proposed: Action,
    ) -> Option<Decision> {
        let mut current = proposed;
        let mut overridden = None;
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "permission.ask") {
                continue;
            }
            let input = serde_json::json!({
                "tool": request.tool,
                "subject": request.subject,
                "request": request.subject,
                "action": current.as_str(),
                "decision": current.as_str(),
            });
            match self.host.invoke_permission_ask(&loaded.spec, input) {
                Ok(output) => {
                    if let Some(value) = output.output
                        && let Some(decision) = decision_from_value(&value)
                    {
                        if let Some(audit) = self.audit_sink() {
                            audit.record_plugin_hook(
                                loaded.spec.raw_spec(),
                                "permission.ask",
                                "override",
                                decision.action().as_str(),
                                Some(&request.subject),
                            );
                        }
                        current = decision.action();
                        overridden = Some(decision);
                    }
                }
                Err(err) => eprintln!("cortex plugin permission.ask: {err}"),
            }
        }
        overridden
    }

    fn apply_tool_before_hooks(&self, call: &ToolCall) -> (ToolCall, Option<ToolVeto>) {
        let mut current = call.clone();
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "tool.execute.before") {
                continue;
            }
            let input = tool_call_to_json(&current);
            match self.host.invoke_tool_before(&loaded.spec, input) {
                Ok(output) => {
                    let Some(value) = output.output else {
                        continue;
                    };
                    if let Some(reason) = veto_reason(&value) {
                        if let Some(audit) = self.audit_sink() {
                            audit.record_plugin_hook(
                                loaded.spec.raw_spec(),
                                "tool.execute.before",
                                "veto",
                                "denied",
                                Some(&reason),
                            );
                        }
                        return (
                            current,
                            Some(ToolVeto {
                                plugin: loaded.spec.raw_spec().to_owned(),
                                reason,
                            }),
                        );
                    }
                    current = tool_call_from_output(current, value);
                }
                Err(err) => eprintln!("cortex plugin tool.execute.before: {err}"),
            }
        }
        (current, None)
    }

    fn apply_tool_after_hooks(
        &self,
        call: &ToolCall,
        mut result: engine::ToolDispatchResult,
    ) -> engine::ToolDispatchResult {
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "tool.execute.after") {
                continue;
            }
            let input = tool_result_to_json(call, &result);
            match self.host.invoke_tool_after(&loaded.spec, input) {
                Ok(output) => {
                    if let Some(value) = output.output {
                        if let Some(audit) = self.audit_sink() {
                            audit.record_plugin_hook(
                                loaded.spec.raw_spec(),
                                "tool.execute.after",
                                "transform",
                                "success",
                                Some(&call.name),
                            );
                        }
                        result = tool_result_from_output(result, value);
                    }
                }
                Err(err) => eprintln!("cortex plugin tool.execute.after: {err}"),
            }
        }
        result
    }

    /// Invoke every loaded plugin's `dispose` hook once.
    pub(crate) fn dispose(&self) {
        if self.disposed.swap(true, Ordering::SeqCst) {
            return;
        }
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "dispose") {
                continue;
            }
            if let Err(err) = self.host.invoke_dispose(&loaded.spec) {
                eprintln!("cortex plugin dispose: {err}");
            }
        }
    }

    fn has_request_shaping_hooks(&self) -> bool {
        self.has_hook("chat.message")
            || self.has_hook("chat.params")
            || self.has_hook("chat.headers")
            || self.has_any_hook(&["experimental.chat.messages.transform", "chat.messages"])
            || self.has_any_hook(&["experimental.chat.system.transform", "chat.system"])
    }

    fn has_hook(&self, hook: &str) -> bool {
        self.loaded
            .iter()
            .any(|loaded| plugin_has_hook(loaded, hook))
    }

    fn has_any_hook(&self, hooks: &[&str]) -> bool {
        hooks.iter().any(|hook| self.has_hook(hook))
    }

    fn shape_request(&self, mut request: LlmRequest) -> Result<LlmRequest, PluginRuntimeError> {
        self.shape_chat_messages(&mut request.messages)?;
        self.shape_chat_params(&mut request)?;
        self.shape_chat_headers(&mut request)?;
        Ok(request)
    }

    fn shape_chat_messages(&self, messages: &mut Vec<Message>) -> Result<(), PluginRuntimeError> {
        // Per-message hooks run before whole-array/system transforms. Within each hook family,
        // plugins run in configuration order, so repeated runs observe the same composition order.
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "chat.message") {
                continue;
            }
            for message in messages.iter_mut() {
                let input = serde_json::to_value(&*message)?;
                let Some(output) = self.host.invoke_chat_message(&loaded.spec, input)?.output
                else {
                    continue;
                };
                if let Some(audit) = self.audit_sink() {
                    audit.record_plugin_hook(
                        loaded.spec.raw_spec(),
                        "chat.message",
                        "shape_message",
                        "success",
                        Some(&format!("{:?}", message.role)),
                    );
                }
                *message = serde_json::from_value(output)?;
            }
        }

        for loaded in &self.loaded {
            if !plugin_has_any_hook(
                loaded,
                &["experimental.chat.messages.transform", "chat.messages"],
            ) {
                continue;
            }
            let hook_name = first_plugin_hook(
                loaded,
                &["experimental.chat.messages.transform", "chat.messages"],
            );
            let input = serde_json::to_value(&*messages)?;
            let Some(output) = self
                .host
                .invoke_chat_messages_transform(&loaded.spec, input)?
                .output
            else {
                continue;
            };
            if let Some(audit) = self.audit_sink() {
                audit.record_plugin_hook(
                    loaded.spec.raw_spec(),
                    hook_name,
                    "transform_messages",
                    "success",
                    Some(&format!("{} messages", messages.len())),
                );
            }
            *messages = serde_json::from_value(output)?;
        }

        for loaded in &self.loaded {
            if !plugin_has_any_hook(
                loaded,
                &["experimental.chat.system.transform", "chat.system"],
            ) {
                continue;
            }
            let hook_name = first_plugin_hook(
                loaded,
                &["experimental.chat.system.transform", "chat.system"],
            );
            let system = messages
                .iter()
                .find(|message| message.role == Role::System)
                .map(|message| message.content.clone())
                .unwrap_or_default();
            let output = self
                .host
                .invoke_chat_system_transform(&loaded.spec, system.clone())?
                .output
                .unwrap_or(serde_json::Value::String(system));
            if let Some(audit) = self.audit_sink() {
                audit.record_plugin_hook(
                    loaded.spec.raw_spec(),
                    hook_name,
                    "transform_system",
                    "success",
                    None,
                );
            }
            let transformed = system_output_to_string(output);
            if let Some(message) = messages
                .iter_mut()
                .find(|message| message.role == Role::System)
            {
                message.content = transformed;
            } else if !transformed.is_empty() {
                messages.insert(0, Message::system(transformed));
            }
        }

        Ok(())
    }

    fn shape_chat_params(&self, request: &mut LlmRequest) -> Result<(), PluginRuntimeError> {
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "chat.params") {
                continue;
            }
            let Some(output) = self
                .host
                .invoke_chat_params(&loaded.spec, params_to_json(request))?
                .output
            else {
                continue;
            };
            if let Some(audit) = self.audit_sink() {
                audit.record_plugin_hook(
                    loaded.spec.raw_spec(),
                    "chat.params",
                    "shape_params",
                    "success",
                    None,
                );
            }
            request.extra_params = json_object(output);
        }
        Ok(())
    }

    fn shape_chat_headers(&self, request: &mut LlmRequest) -> Result<(), PluginRuntimeError> {
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "chat.headers") {
                continue;
            }
            let Some(output) = self
                .host
                .invoke_chat_headers(&loaded.spec, headers_to_json(&request.extra_headers))?
                .output
            else {
                continue;
            };
            if let Some(audit) = self.audit_sink() {
                audit.record_plugin_hook(
                    loaded.spec.raw_spec(),
                    "chat.headers",
                    "shape_headers",
                    "success",
                    None,
                );
            }
            request.extra_headers = json_headers(output);
        }
        Ok(())
    }

    fn shape_tool_definitions_result(
        &self,
        mut definitions: Vec<ToolDef>,
    ) -> Result<Vec<ToolDef>, PluginRuntimeError> {
        for loaded in &self.loaded {
            if !plugin_has_hook(loaded, "tool.definition") {
                continue;
            }
            for definition in &mut definitions {
                let input = serde_json::to_value(&*definition)?;
                let Some(output) = self
                    .host
                    .invoke_tool_definition(&loaded.spec, input)?
                    .output
                else {
                    continue;
                };
                if let Some(audit) = self.audit_sink() {
                    audit.record_plugin_hook(
                        loaded.spec.raw_spec(),
                        "tool.definition",
                        "shape_definition",
                        "success",
                        Some(&definition.name),
                    );
                }
                *definition = serde_json::from_value(output)?;
            }
        }
        Ok(definitions)
    }

    fn shape_tool_definitions(&self, definitions: Vec<ToolDef>) -> Vec<ToolDef> {
        match self.shape_tool_definitions_result(definitions.clone()) {
            Ok(shaped) => shaped,
            Err(err) => {
                eprintln!("cortex plugin tool.definition: {err}");
                definitions
            }
        }
    }

    /// Build an event-bus subscriber that dispatches real bus events to plugin `event` hooks.
    #[must_use]
    pub(crate) fn event_subscriber(&self) -> Option<Arc<dyn Subscriber>> {
        self.loaded
            .iter()
            .any(|loaded| loaded.report.hooks.iter().any(|hook| hook == "event"))
            .then(|| {
                Arc::new(PluginEventSubscriber {
                    host: self.host.clone(),
                    loaded: self.loaded.clone(),
                    surface: self.surface.clone(),
                }) as Arc<dyn Subscriber>
            })
    }
}

/// Whether plugin pure mode is enabled through its environment equivalent.
#[must_use]
pub(crate) fn env_pure_enabled() -> bool {
    std::env::var_os(PURE_ENV).is_some_and(|value| value.to_string_lossy() == "1")
}

impl Drop for PluginState {
    fn drop(&mut self) {
        self.dispose();
    }
}

/// Loading plugins for an auxiliary surface can fail because config failed or because the real
/// JavaScript runtime/plugin failed.
#[derive(Debug)]
pub(crate) enum ConfigOrRuntimeError {
    Io(io::Error),
    Config(ConfigError),
    Runtime(PluginRuntimeError),
}

impl std::fmt::Display for ConfigOrRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "cannot determine the working directory: {err}"),
            Self::Config(err) => write!(f, "{err}"),
            Self::Runtime(err) => write!(f, "{err}"),
        }
    }
}

fn plugin_rule_to_permission_rule(rule: &PluginConfigRule) -> Option<Rule> {
    let action = Action::parse(&rule.action)?;
    Some(Rule::new(rule.tool.clone(), rule.pattern.clone(), action))
}

/// One command/slash-command invocation passed through `command.execute.before`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandInvocation {
    pub(crate) name: String,
    pub(crate) args: Vec<String>,
}

impl CommandInvocation {
    #[must_use]
    pub(crate) fn new(name: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }

    fn to_json(&self) -> JsonValue {
        serde_json::json!({
            "name": self.name,
            "command": self.name,
            "args": self.args,
            "arguments": self.args,
        })
    }
}

/// Result of the command-before hook chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommandBeforeOutcome {
    Run(CommandInvocation),
    Veto { plugin: String, reason: String },
}

/// One provider as emitted by `cortex models`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ProviderDoc {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) source: &'static str,
    pub(crate) models: Vec<ModelDoc>,
}

impl ProviderDoc {
    fn from_plugin(provider: &PluginProviderRegistration) -> Self {
        Self {
            id: provider.id.clone(),
            name: non_empty(&provider.name).unwrap_or_else(|| provider.id.clone()),
            source: "plugin",
            models: provider
                .models
                .iter()
                .map(|model| ModelDoc::from_plugin(&provider.id, model))
                .collect(),
        }
    }
}

/// Per-token pricing for a model, in USD per million tokens (the models.dev convention).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub(crate) struct ModelCostDoc {
    pub(crate) input: f64,
    pub(crate) output: f64,
}

/// One model as emitted by `cortex models`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ModelDoc {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) provider: String,
    pub(crate) default: bool,
    pub(crate) attachment: bool,
    pub(crate) tool_call: bool,
    pub(crate) reasoning: bool,
    pub(crate) cost: ModelCostDoc,
    pub(crate) context_length: u64,
    pub(crate) max_output_tokens: u64,
    pub(crate) source: &'static str,
}

impl ModelDoc {
    fn from_plugin(provider_id: &str, model: &PluginModelRegistration) -> Self {
        Self {
            id: model.id.clone(),
            name: non_empty(&model.name).unwrap_or_else(|| model.id.clone()),
            provider: provider_id.to_owned(),
            default: false,
            attachment: false,
            tool_call: model.tool_call,
            reasoning: model.reasoning,
            cost: ModelCostDoc {
                input: 0.0,
                output: 0.0,
            },
            context_length: model.context,
            max_output_tokens: model.output,
            source: "plugin",
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn plugin_has_hook(loaded: &LoadedPlugin, hook: &str) -> bool {
    loaded
        .report
        .hooks
        .iter()
        .any(|candidate| candidate == hook)
}

fn plugin_has_any_hook(loaded: &LoadedPlugin, hooks: &[&str]) -> bool {
    hooks.iter().any(|hook| plugin_has_hook(loaded, hook))
}

fn first_plugin_hook<'a>(loaded: &LoadedPlugin, hooks: &'a [&'a str]) -> &'a str {
    hooks
        .iter()
        .copied()
        .find(|hook| plugin_has_hook(loaded, hook))
        .unwrap_or_else(|| hooks.first().copied().unwrap_or(""))
}

fn params_to_json(request: &LlmRequest) -> JsonValue {
    let mut params = serde_json::Map::new();
    if let Some(max_tokens) = request.max_tokens {
        params.insert("max_tokens".to_owned(), JsonValue::from(max_tokens));
    }
    if let Some(temperature) = request.temperature {
        if let Some(number) = serde_json::Number::from_f64(f64::from(temperature)) {
            params.insert("temperature".to_owned(), JsonValue::Number(number));
        }
    }
    params.extend(request.extra_params.clone());
    JsonValue::Object(params)
}

fn headers_to_json(headers: &[(String, String)]) -> JsonValue {
    JsonValue::Object(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), JsonValue::String(value.clone())))
            .collect(),
    )
}

fn json_object(value: JsonValue) -> serde_json::Map<String, JsonValue> {
    match value {
        JsonValue::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

fn json_headers(value: JsonValue) -> Vec<(String, String)> {
    let JsonValue::Object(map) = value else {
        return Vec::new();
    };
    map.into_iter()
        .filter_map(|(name, value)| {
            let value = match value {
                JsonValue::Null => return None,
                JsonValue::String(value) => value,
                other => other.to_string(),
            };
            Some((name, value))
        })
        .collect()
}

fn decision_from_value(value: &JsonValue) -> Option<Decision> {
    let raw = value
        .as_str()
        .or_else(|| value.get("decision").and_then(JsonValue::as_str))
        .or_else(|| value.get("action").and_then(JsonValue::as_str))
        .or_else(|| value.get("permission").and_then(JsonValue::as_str))?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "allow" | "allowed" => Some(Decision::Allow),
        "deny" | "denied" | "block" | "blocked" => Some(Decision::Deny),
        _ => None,
    }
}

fn veto_reason(value: &JsonValue) -> Option<String> {
    let object = value.as_object()?;
    let veto = object
        .get("veto")
        .and_then(JsonValue::as_bool)
        .or_else(|| object.get("deny").and_then(JsonValue::as_bool))
        .or_else(|| object.get("denied").and_then(JsonValue::as_bool))
        .unwrap_or(false);
    let decision_denies = decision_from_value(value) == Some(Decision::Deny);
    (veto || decision_denies).then(|| {
        object
            .get("reason")
            .or_else(|| object.get("message"))
            .and_then(JsonValue::as_str)
            .unwrap_or("denied by plugin")
            .to_owned()
    })
}

fn string_array(value: &JsonValue) -> Option<Vec<String>> {
    value
        .as_array()
        .map(|items| items.iter().map(json_to_string).collect())
}

fn json_to_string(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn command_from_output(mut command: CommandInvocation, value: JsonValue) -> CommandInvocation {
    let Some(object) = value.as_object() else {
        return command;
    };
    if let Some(name) = object
        .get("name")
        .or_else(|| object.get("command"))
        .and_then(JsonValue::as_str)
    {
        command.name = name.to_owned();
    }
    if let Some(args) = object
        .get("args")
        .or_else(|| object.get("arguments"))
        .and_then(string_array)
    {
        command.args = args;
    }
    command
}

fn tool_call_to_json(call: &ToolCall) -> JsonValue {
    let arguments = serde_json::from_str::<JsonValue>(&call.arguments)
        .unwrap_or_else(|_| JsonValue::String(call.arguments.clone()));
    serde_json::json!({
        "id": call.id,
        "name": call.name,
        "tool": call.name,
        "arguments": arguments,
    })
}

fn tool_call_from_output(mut call: ToolCall, value: JsonValue) -> ToolCall {
    let Some(object) = value.as_object() else {
        return call;
    };
    if let Some(name) = object
        .get("name")
        .or_else(|| object.get("tool"))
        .and_then(JsonValue::as_str)
    {
        call.name = name.to_owned();
    }
    if let Some(arguments) = object.get("arguments").or_else(|| object.get("args")) {
        call.arguments = match arguments {
            JsonValue::String(raw) => raw.clone(),
            other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_owned()),
        };
    }
    call
}

fn tool_result_to_json(call: &ToolCall, result: &engine::ToolDispatchResult) -> JsonValue {
    serde_json::json!({
        "id": call.id,
        "name": call.name,
        "tool": call.name,
        "arguments": serde_json::from_str::<JsonValue>(&call.arguments).unwrap_or(JsonValue::Null),
        "content": result.content,
        "parts": result.parts,
        "is_error": result.is_error,
    })
}

fn tool_result_from_output(
    mut result: engine::ToolDispatchResult,
    value: JsonValue,
) -> engine::ToolDispatchResult {
    match value {
        JsonValue::String(content) => {
            result.content = content.clone();
            result.parts = vec![protocol::ToolResultPart::text(content)];
            result
        }
        JsonValue::Object(mut object) => {
            if let Some(is_error) = object.get("is_error").and_then(JsonValue::as_bool) {
                result.is_error = is_error;
            } else if let Some(is_error) = object.get("error").and_then(JsonValue::as_bool) {
                result.is_error = is_error;
            }
            if let Some(parts) = object.remove("parts")
                && let Ok(parsed) = serde_json::from_value(parts)
            {
                result.parts = parsed;
            }
            if let Some(content) = object
                .get("content")
                .or_else(|| object.get("text"))
                .and_then(JsonValue::as_str)
            {
                result.content = content.to_owned();
                if result.parts.is_empty()
                    || result
                        .parts
                        .iter()
                        .all(|part| matches!(part, protocol::ToolResultPart::Text { .. }))
                {
                    result.parts = vec![protocol::ToolResultPart::text(content.to_owned())];
                }
            }
            result
        }
        _ => result,
    }
}

fn system_output_to_string(value: JsonValue) -> String {
    match value {
        JsonValue::String(text) => text,
        JsonValue::Object(map) => map
            .get("content")
            .and_then(JsonValue::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| JsonValue::Object(map).to_string()),
        other => other.to_string(),
    }
}

struct PluginShapingProvider {
    inner: Arc<dyn Provider>,
    state: PluginState,
}

impl Provider for PluginShapingProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let request = self
                .state
                .shape_request(request)
                .map_err(|err| LlmError::upstream(err.to_string()))?;
            self.inner.stream(request).await
        })
    }
}

pub(crate) struct PluginToolDefinitionDispatcher<'a> {
    inner: &'a dyn ToolDispatcher,
    state: &'a PluginState,
    _session_id: String,
}

impl ToolDispatcher for PluginToolDefinitionDispatcher<'_> {
    fn definitions(&self) -> Vec<ToolDef> {
        self.state.shape_tool_definitions(self.inner.definitions())
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, engine::ToolDispatchResult> {
        Box::pin(async move {
            let (call, veto) = self.state.apply_tool_before_hooks(call);
            if let Some(veto) = veto {
                if let Some(audit) = self.state.audit_sink() {
                    audit.record_plugin_tool_denial(
                        &call.id,
                        &call.name,
                        &veto.plugin,
                        &veto.reason,
                        &call.arguments,
                    );
                }
                let message = format!(
                    "{}: plugin {} vetoed tool.execute.before: {}",
                    call.name, veto.plugin, veto.reason
                );
                return engine::ToolDispatchResult {
                    content: message.clone(),
                    parts: vec![protocol::ToolResultPart::text(message)],
                    is_error: true,
                };
            }

            let result = self.inner.dispatch(&call).await;
            self.state.apply_tool_after_hooks(&call, result)
        })
    }
}

struct PluginPermissionOverride {
    state: PluginState,
}

impl DecisionOverride for PluginPermissionOverride {
    fn override_decision(&self, request: &PermissionRequest, proposed: Action) -> Option<Decision> {
        self.state.permission_override_decision(request, proposed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolVeto {
    plugin: String,
    reason: String,
}

struct PluginTool {
    host: RuntimeHost,
    spec: PluginSpec,
    registration: PluginToolRegistration,
}

impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.registration.name
    }

    fn description(&self) -> &str {
        &self.registration.description
    }

    fn parameters(&self) -> JsonValue {
        self.registration.parameters.clone()
    }

    fn execute<'a>(
        &'a self,
        arguments: JsonValue,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let subject = serde_json::to_string(&arguments).unwrap_or_default();
            if !ctx.permits(self.name(), &subject) {
                return ToolOutput::error(format!("{}: permission denied", self.name()));
            }
            match self
                .host
                .execute_tool(&self.spec, self.name(), arguments.clone())
            {
                Ok(output) => {
                    if let Some(audit) = ctx.audit_sink() {
                        audit.record_plugin_tool_execution(
                            self.spec.raw_spec(),
                            self.name(),
                            &subject,
                            "success",
                        );
                    }
                    ToolOutput::text(output.content)
                }
                Err(err) => {
                    if let Some(audit) = ctx.audit_sink() {
                        audit.record_plugin_tool_execution(
                            self.spec.raw_spec(),
                            self.name(),
                            &subject,
                            "error",
                        );
                    }
                    ToolOutput::error(err.to_string())
                }
            }
        })
    }
}

/// A plugin-declared provider selected by `--model`.
#[derive(Clone, Debug)]
pub(crate) struct PluginModelProvider {
    host: RuntimeHost,
    spec: PluginSpec,
    provider: PluginProviderRegistration,
    model: PluginModelRegistration,
}

impl PluginModelProvider {
    #[must_use]
    pub(crate) fn provider_id(&self) -> &str {
        &self.provider.id
    }

    #[must_use]
    pub(crate) fn model_id(&self) -> &str {
        &self.model.id
    }
}

impl Provider for PluginModelProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let messages = serde_json::to_value(&request.messages)
                .map_err(|err| LlmError::decode(err.to_string()))?;
            let text = self
                .host
                .complete_with_provider(&self.spec, &request.model, messages)
                .map_err(|err| LlmError::upstream(err.to_string()))?;
            let stream = futures_util::stream::iter([
                LlmEvent::TextDelta { text },
                LlmEvent::Done {
                    reason: FinishReason::Stop,
                },
            ]);
            Ok(Box::pin(stream) as LlmStream)
        })
    }
}

struct PluginEventSubscriber {
    host: RuntimeHost,
    loaded: Vec<LoadedPlugin>,
    surface: Arc<str>,
}

impl Subscriber for PluginEventSubscriber {
    fn on_event(&self, event: &Event) {
        let mut value = match serde_json::to_value(event) {
            Ok(JsonValue::Object(map)) => map,
            Ok(other) => {
                let mut map = serde_json::Map::new();
                map.insert("payload".to_owned(), other);
                map
            }
            Err(err) => {
                eprintln!("cortex plugin event: failed to serialize event: {err}");
                return;
            }
        };
        value
            .entry("surface".to_owned())
            .or_insert_with(|| JsonValue::String(self.surface.to_string()));
        value
            .entry("type".to_owned())
            .or_insert_with(|| JsonValue::String(event.kind().to_owned()));

        for loaded in &self.loaded {
            if !loaded.report.hooks.iter().any(|hook| hook == "event") {
                continue;
            }
            if let Err(err) = self
                .host
                .invoke_event(&loaded.spec, JsonValue::Object(value.clone()))
            {
                eprintln!("cortex plugin event: {err}");
            }
        }
    }
}

fn non_empty_surface(surface: &str) -> String {
    let surface = surface.trim();
    if surface.is_empty() {
        "run".to_owned()
    } else {
        surface.to_owned()
    }
}

/// Convert plugin tool registrations into LLM definitions for tests/surfaces that need a snapshot
/// without building a full registry.
#[cfg(test)]
pub(crate) fn tool_defs_from_state(state: &PluginState) -> Vec<ToolDef> {
    state
        .loaded
        .iter()
        .flat_map(|loaded| loaded.report.tools.iter())
        .map(|tool| ToolDef {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use permissions::{PermissionEngine, Ruleset};
    use plugins::{PluginConfig, RuntimeHost, detect_runtime};
    use serde_json::json;
    use toml::Value as TomlValue;
    use tools::TodoStore;

    use super::*;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cortex-cli-plugin-bridge-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn config_from(project: &Path, body: &str) -> PluginConfig {
        let value: TomlValue = toml::from_str(body).expect("valid toml");
        PluginConfig::from_toml_value(value.get("plugin"), project).expect("plugin config")
    }

    fn skip_without_runtime() -> bool {
        if detect_runtime().is_none() {
            eprintln!("skipping plugin bridge proof: no Bun, Node, or Deno on PATH");
            true
        } else {
            false
        }
    }

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "call_1".to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    struct NoopTodos;

    impl TodoStore for NoopTodos {
        fn list(&self, _session_id: &str) -> Result<Vec<protocol::TodoItem>, String> {
            Ok(Vec::new())
        }

        fn replace(&self, _session_id: &str, _items: &[protocol::TodoItem]) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn plugin_tools_are_gated_before_the_js_body_runs() {
        if skip_without_runtime() {
            return;
        }

        let project = scratch("gating");
        let marker = project.join("marker.txt");
        std::fs::write(
            project.join("plugin.mjs"),
            r#"
import { writeFile } from "node:fs/promises";

export default async function plugin(input, options) {
  return {
    async tool() {
      return {
        name: "cortex_write_marker",
        description: "Write a marker file.",
        parameters: {
          type: "object",
          properties: { text: { type: "string" } },
          required: ["text"],
          additionalProperties: false
        },
        async execute(args) {
          await writeFile(options.marker, args.text);
          return `WROTE:${args.text}`;
        }
      };
    }
  };
}
"#,
        )
        .expect("write plugin");
        let config = config_from(
            &project,
            &format!(
                "plugin = [[\"./plugin.mjs\", {{ marker = \"{}\" }}]]",
                marker.display()
            ),
        );
        let loaded = RuntimeHost::new()
            .with_timeout(Duration::from_secs(15))
            .load_plugins(&config)
            .expect("load plugin");
        let state = PluginState {
            loaded,
            host: RuntimeHost::new().with_timeout(Duration::from_secs(15)),
            surface: Arc::from("run"),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: Arc::new(Mutex::new(None)),
        };
        assert_eq!(tool_defs_from_state(&state)[0].name, "cortex_write_marker");

        let mut tools = state.tools();
        let tool = tools.pop().expect("plugin tool");
        let deny = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "cortex_write_marker",
            "*",
            Action::Deny,
        )]));
        let denied_ctx = ToolContext::new(&project).with_permissions(Arc::new(deny));
        let denied = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(tool.execute(json!({ "text": "DENIED" }), &denied_ctx));
        assert!(denied.is_error);
        assert!(
            !marker.exists(),
            "denied plugin tool must not execute its JS body"
        );

        let allow = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "cortex_write_marker",
            "*",
            Action::Allow,
        )]));
        let allowed_ctx = ToolContext::new(&project).with_permissions(Arc::new(allow));
        let allowed = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(tool.execute(json!({ "text": "ALLOWED" }), &allowed_ctx));
        assert!(!allowed.is_error, "{}", allowed.model_content());
        assert_eq!(allowed.model_content(), "WROTE:ALLOWED");
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "ALLOWED");
    }

    #[test]
    fn permission_ask_hook_overrides_allowed_and_denied_actions() {
        if skip_without_runtime() {
            return;
        }

        let project = scratch("permission-ask");
        std::fs::write(
            project.join("plugin.mjs"),
            r#"
export default async function plugin() {
  return {
    "permission.ask": async (request) => {
      if (request.subject.includes("force-deny")) return "deny";
      if (request.subject.includes("force-allow")) return { decision: "allow" };
      return request.decision;
    }
  };
}
"#,
        )
        .expect("write plugin");
        let config = config_from(&project, "plugin = [\"./plugin.mjs\"]\n");
        let loaded = RuntimeHost::new()
            .with_timeout(Duration::from_secs(15))
            .load_plugins(&config)
            .expect("load plugin");
        let state = PluginState {
            loaded,
            host: RuntimeHost::new().with_timeout(Duration::from_secs(15)),
            surface: Arc::from("run"),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: Arc::new(Mutex::new(None)),
        };
        let override_hook = state.permission_override().expect("permission hook");

        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("bash", "echo *", Action::Allow),
            Rule::new("write", "force-allow.txt", Action::Deny),
        ]))
        .with_override(override_hook);

        assert_eq!(
            engine.evaluate(&permissions::PermissionRequest::new(
                "bash",
                "echo force-deny"
            )),
            permissions::Outcome::Denied,
            "plugin-forced deny blocks an otherwise allowed command"
        );
        assert_eq!(
            engine.evaluate(&permissions::PermissionRequest::new(
                "write",
                "force-allow.txt"
            )),
            permissions::Outcome::Allowed,
            "plugin-forced allow permits an otherwise denied write"
        );
    }

    #[test]
    fn tool_before_after_and_shell_env_hooks_have_real_effects() {
        if skip_without_runtime() {
            return;
        }

        let project = scratch("tool-hooks");
        std::fs::write(
            project.join("plugin.mjs"),
            r#"
export default async function plugin() {
  return {
    "tool.execute.before": async (call) => {
      if (call.name === "write" && call.arguments?.path === "a.txt") {
        return { ...call, arguments: { ...call.arguments, path: "b.txt" } };
      }
      return call;
    },
    "tool.execute.after": async (result) => {
      if (result.name === "read") {
        return { ...result, content: `${result.content}\n[via-plugin]` };
      }
      return result;
    },
    "shell.env": async () => ({ CORTEX_PLUGIN_ENV: "on" })
  };
}
"#,
        )
        .expect("write plugin");
        let config = config_from(&project, "plugin = [\"./plugin.mjs\"]\n");
        let loaded = RuntimeHost::new()
            .with_timeout(Duration::from_secs(15))
            .load_plugins(&config)
            .expect("load plugin");
        let state = PluginState {
            loaded,
            host: RuntimeHost::new().with_timeout(Duration::from_secs(15)),
            surface: Arc::from("run"),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: Arc::new(Mutex::new(None)),
        };

        let dispatcher = crate::tooling::RegistryDispatcher::for_model(
            project.clone(),
            "google/gemini-2.5-flash",
            "ses_hooks".to_owned(),
            Arc::new(NoopTodos),
        )
        .with_shell_env(state.shell_env());
        let dispatcher = state.tool_definition_dispatcher(&dispatcher, "ses_hooks");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let write = runtime
            .block_on(dispatcher.dispatch(&call("write", r#"{"path":"a.txt","content":"hello"}"#)));
        assert!(!write.is_error, "{}", write.content);
        assert!(
            !project.join("a.txt").exists(),
            "the original write target must stay absent"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("b.txt")).unwrap(),
            "hello"
        );

        let read = runtime.block_on(dispatcher.dispatch(&call("read", r#"{"path":"b.txt"}"#)));
        assert!(!read.is_error, "{}", read.content);
        assert!(
            read.content.ends_with("[via-plugin]"),
            "tool.execute.after transforms the model-visible result: {}",
            read.content
        );

        let env = runtime.block_on(
            dispatcher.dispatch(&call("bash", r#"{"command":"printenv CORTEX_PLUGIN_ENV"}"#)),
        );
        assert!(!env.is_error, "{}", env.content);
        assert!(
            env.content.contains("on"),
            "shell.env injection should be visible to bash: {}",
            env.content
        );
    }

    #[test]
    fn throwing_runtime_hook_is_isolated_and_later_hooks_still_fire() {
        if skip_without_runtime() {
            return;
        }

        let project = scratch("hook-throw");
        std::fs::write(
            project.join("thrower.mjs"),
            r#"
export default async function plugin() {
  return {
    "tool.execute.before": async () => {
      throw new Error("hook-boom");
    }
  };
}
"#,
        )
        .expect("write throwing plugin");
        std::fs::write(
            project.join("rewriter.mjs"),
            r#"
export default async function plugin() {
  return {
    "tool.execute.before": async (call) => {
      if (call.name === "write") {
        return { ...call, arguments: { ...call.arguments, path: "after-throw.txt" } };
      }
      return call;
    }
  };
}
"#,
        )
        .expect("write rewriter plugin");
        let config = config_from(
            &project,
            "plugin = [\"./thrower.mjs\", \"./rewriter.mjs\"]\n",
        );
        let loaded = RuntimeHost::new()
            .with_timeout(Duration::from_secs(15))
            .load_plugins(&config)
            .expect("load plugins");
        let state = PluginState {
            loaded,
            host: RuntimeHost::new().with_timeout(Duration::from_secs(15)),
            surface: Arc::from("run"),
            disposed: Arc::new(AtomicBool::new(false)),
            audit: Arc::new(Mutex::new(None)),
        };
        let dispatcher = crate::tooling::RegistryDispatcher::for_model(
            project.clone(),
            "google/gemini-2.5-flash",
            "ses_throw".to_owned(),
            Arc::new(NoopTodos),
        );
        let dispatcher = state.tool_definition_dispatcher(&dispatcher, "ses_throw");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let result = runtime.block_on(
            dispatcher.dispatch(&call("write", r#"{"path":"original.txt","content":"ok"}"#)),
        );

        assert!(
            !result.is_error,
            "a throwing hook is isolated so the turn/tool survives: {}",
            result.content
        );
        assert!(
            !project.join("original.txt").exists(),
            "later hook transformed away from original path"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("after-throw.txt")).unwrap(),
            "ok"
        );
    }
}

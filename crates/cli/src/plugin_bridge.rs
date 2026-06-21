//! CLI-owned integration of plugin registrations with run/auth/model surfaces.

use std::io;
use std::sync::Arc;

use engine::ToolDispatcher;
use events::{Event, Subscriber};
use futures_util::future::BoxFuture;
use llm::{
    FinishReason, LlmError, LlmEvent, LlmRequest, LlmStream, Message, Provider, Role, ToolCall,
    ToolDef,
};
use permissions::{Action, Rule};
use plugins::{
    LoadedPlugin, PluginAuthRegistration, PluginConfigRule, PluginModelRegistration,
    PluginProviderRegistration, PluginRuntimeError, PluginSpec, PluginToolRegistration,
    RuntimeHost,
};
use serde::Serialize;
use serde_json::Value as JsonValue;
use settings::{ConfigError, ConfigInputs};
use tools::{Tool, ToolContext, ToolOutput};

/// Loaded plugin runtime state for one surface invocation.
#[derive(Clone, Debug, Default)]
pub(crate) struct PluginState {
    loaded: Vec<LoadedPlugin>,
    host: RuntimeHost,
}

impl PluginState {
    /// Load configured plugins using the real external JS runtime.
    pub(crate) fn load(config: &plugins::PluginConfig) -> Result<Self, PluginRuntimeError> {
        let host = RuntimeHost::new();
        let loaded = host.load_plugins(config)?;
        Ok(Self { loaded, host })
    }

    /// Resolve the current process config and load configured plugins for a non-run surface.
    pub(crate) fn from_process_config() -> Result<Self, ConfigOrRuntimeError> {
        let inputs =
            ConfigInputs::from_process(Default::default()).map_err(ConfigOrRuntimeError::Io)?;
        let cwd = inputs.cwd.clone();
        let effective = settings::load(inputs).map_err(ConfigOrRuntimeError::Config)?;
        let config = plugins::PluginConfig::from_toml_value(effective.get("plugin"), &cwd)
            .map_err(|message| {
                ConfigOrRuntimeError::Config(ConfigError::TypeMismatch {
                    key: "plugin".to_owned(),
                    expected: "an array of local plugin paths or [path, options] pairs",
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

    /// Auth methods contributed by plugin `auth` hooks.
    #[must_use]
    pub(crate) fn auth_methods(&self) -> Vec<AuthMethodDoc> {
        self.loaded
            .iter()
            .flat_map(|loaded| loaded.report.auth_methods.iter())
            .map(AuthMethodDoc::from_plugin)
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
    ) -> PluginToolDefinitionDispatcher<'a> {
        PluginToolDefinitionDispatcher { inner, state: self }
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
            let input = serde_json::to_value(&*messages)?;
            let Some(output) = self
                .host
                .invoke_chat_messages_transform(&loaded.spec, input)?
                .output
            else {
                continue;
            };
            *messages = serde_json::from_value(output)?;
        }

        for loaded in &self.loaded {
            if !plugin_has_any_hook(
                loaded,
                &["experimental.chat.system.transform", "chat.system"],
            ) {
                continue;
            }
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
                }) as Arc<dyn Subscriber>
            })
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

/// One plugin-auth method as emitted by `cortex auth list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct AuthMethodDoc {
    pub(crate) id: String,
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) env: Vec<String>,
    pub(crate) source: &'static str,
}

impl AuthMethodDoc {
    pub(crate) fn openrouter() -> Self {
        Self {
            id: providers::OPENROUTER_ID.to_owned(),
            name: "OpenRouter".to_owned(),
            description: Some("API key via OPENROUTER_API_KEY or credential store".to_owned()),
            env: vec![providers::OPENROUTER_ENV.to_owned()],
            source: "builtin",
        }
    }

    fn from_plugin(method: &PluginAuthRegistration) -> Self {
        Self {
            id: method.id.clone(),
            name: non_empty(&method.name).unwrap_or_else(|| method.id.clone()),
            description: method.description.clone(),
            env: method.env.clone(),
            source: "plugin",
        }
    }
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
            models: provider.models.iter().map(ModelDoc::from_plugin).collect(),
        }
    }
}

/// One model as emitted by `cortex models`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ModelDoc {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) tool_call: bool,
    pub(crate) reasoning: bool,
    pub(crate) source: &'static str,
}

impl ModelDoc {
    fn from_plugin(model: &PluginModelRegistration) -> Self {
        Self {
            id: model.id.clone(),
            name: non_empty(&model.name).unwrap_or_else(|| model.id.clone()),
            tool_call: model.tool_call,
            reasoning: model.reasoning,
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
}

impl ToolDispatcher for PluginToolDefinitionDispatcher<'_> {
    fn definitions(&self) -> Vec<ToolDef> {
        self.state.shape_tool_definitions(self.inner.definitions())
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, engine::ToolDispatchResult> {
        self.inner.dispatch(call)
    }
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
                Ok(output) => ToolOutput::text(output.content),
                Err(err) => ToolOutput::error(err.to_string()),
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
}

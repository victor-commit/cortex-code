//! CLI-owned integration of plugin registrations with run/auth/model surfaces.

use std::io;
use std::sync::Arc;

use events::{Event, Subscriber};
use futures_util::future::BoxFuture;
#[cfg(test)]
use llm::ToolDef;
use llm::{FinishReason, LlmError, LlmEvent, LlmRequest, LlmStream, Provider};
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

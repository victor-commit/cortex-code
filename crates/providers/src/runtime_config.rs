//! Typed provider/runtime configuration parsed from the effective settings table.
//!
//! This module ports provider-configuration wiring into real shipped paths:
//! - global provider allow/deny lists (`[provider].enabled` / `[provider].disabled`);
//! - global and per-provider model overrides (`model_overrides`);
//! - provider endpoint/auth/options (`base_url`, `auth`, `options`);
//! - optional configured provider model metadata (`models`) used by `cortex models`.
//!
//! The parser is intentionally strict for known keys and types while preserving forward
//! compatibility for unknown provider-specific keys.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use toml::Value;
use toml::value::Table;

use crate::catalogue::{
    Catalogue, DEFAULT_MODEL, DEFAULT_PROVIDER, ModelCost, ModelCostTier, ModelInfo, ModelLimit,
    ProviderInfo, ResolvedModel,
};
use crate::credentials::{CredentialInputs, OPENROUTER_ENV};
use crate::openrouter::OPENROUTER_ID;
use crate::provider_profiles;

const ROOT_PROVIDER_KEY: &str = "provider";
const ROOT_ENABLED_KEY: &str = "enabled";
const ROOT_DISABLED_KEY: &str = "disabled";
const ROOT_MODEL_OVERRIDES_KEY: &str = "model_overrides";
const ROOT_OPENROUTER_BASE_URL_KEY: &str = "openrouter_base_url";
const ROOT_API_KEY_KEY: &str = "api_key";

const PROVIDER_ENABLED_KEY: &str = "enabled";
const PROVIDER_BASE_URL_KEY: &str = "base_url";
const PROVIDER_API_KEY_KEY: &str = "api_key";
const PROVIDER_API_KEY_ENV_KEY: &str = "api_key_env";
const PROVIDER_AUTH_KEY: &str = "auth";
const PROVIDER_OPTIONS_KEY: &str = "options";
const PROVIDER_PROTOCOL_KEY: &str = "protocol";
const PROVIDER_MODEL_OVERRIDES_KEY: &str = "model_overrides";
const PROVIDER_MODELS_KEY: &str = "models";

const OPTIONS_HEADERS_KEY: &str = "headers";
const OPTIONS_PARAMS_KEY: &str = "params";

const MODEL_ID_KEY: &str = "id";
const MODEL_NAME_KEY: &str = "name";
const MODEL_TOOL_CALL_KEY: &str = "tool_call";
const MODEL_REASONING_KEY: &str = "reasoning";
const MODEL_ATTACHMENT_KEY: &str = "attachment";
const MODEL_CONTEXT_LENGTH_KEY: &str = "context_length";
const MODEL_MAX_OUTPUT_LIMIT_KEY: &str = "max_output_tokens";
const MODEL_CONTEXT_ALIAS_KEY: &str = "context";
const MODEL_OUTPUT_ALIAS_KEY: &str = "output";
const MODEL_COST_KEY: &str = "cost";
const MODEL_COST_INPUT_KEY: &str = "input";
const MODEL_COST_OUTPUT_KEY: &str = "output";
const MODEL_COST_INPUT_ALIAS_KEY: &str = "cost_input";
const MODEL_COST_OUTPUT_ALIAS_KEY: &str = "cost_output";
const MODEL_COST_CACHE_WRITE_KEY: &str = "cache_write";
const MODEL_COST_CACHE_READ_KEY: &str = "cache_read";
const MODEL_COST_REASONING_KEY: &str = "reasoning";
const MODEL_COST_CACHE_WRITE_ALIAS_KEY: &str = "cost_cache_write";
const MODEL_COST_CACHE_READ_ALIAS_KEY: &str = "cost_cache_read";
const MODEL_COST_REASONING_ALIAS_KEY: &str = "cost_reasoning";
const MODEL_COST_OVER_THRESHOLD_KEY: &str = "over_input_threshold";
const MODEL_COST_THRESHOLD_KEY: &str = "threshold_input_tokens";

/// One typed provider-auth section.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProviderAuthConfig {
    /// Optional config-supplied API key (lowest precedence; env/file can still override).
    pub api_key: Option<String>,
    /// Optional environment-variable name used to source the API key.
    pub api_key_env: Option<String>,
}

/// One typed provider-options section.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderOptionsConfig {
    /// Extra HTTP headers merged into outbound requests for this provider.
    pub headers: BTreeMap<String, String>,
    /// Provider-specific OpenAI-compatible request parameters merged into request JSON.
    pub params: JsonMap<String, JsonValue>,
}

/// The wire protocol used for a configured provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderProtocol {
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
    Gemini,
}

impl ProviderProtocol {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderProtocol::OpenAiChat => "openai-chat",
            ProviderProtocol::OpenAiResponses => "openai-responses",
            ProviderProtocol::AnthropicMessages => "anthropic-messages",
            ProviderProtocol::Gemini => "gemini",
        }
    }
}

/// One typed provider entry (`[provider.<id>]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderRuntimeConfig {
    pub id: String,
    pub enabled: bool,
    pub base_url: Option<String>,
    pub protocol: Option<ProviderProtocol>,
    pub auth: ProviderAuthConfig,
    pub model_overrides: BTreeMap<String, String>,
    pub models: BTreeMap<String, ModelInfo>,
    pub options: ProviderOptionsConfig,
}

/// The typed provider-config set resolved from the effective config table.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderConfigSet {
    /// Optional provider allow-list from `[provider].enabled`.
    pub enabled: Option<BTreeSet<String>>,
    /// Provider deny-list from `[provider].disabled`.
    pub disabled: BTreeSet<String>,
    /// Global model overrides from `[provider].model_overrides`.
    pub model_overrides: BTreeMap<String, String>,
    /// Per-provider configuration entries from `[provider.<id>]`.
    pub providers: BTreeMap<String, ProviderRuntimeConfig>,
}

impl ProviderConfigSet {
    /// Parse the typed provider configuration from the effective settings table.
    ///
    /// This also preserves legacy OpenRouter keys:
    /// - top-level `openrouter_base_url` overrides `[provider.openrouter].base_url`;
    /// - top-level `api_key` remains the fallback beneath `[provider.openrouter].api_key`.
    pub fn from_table(table: &Table) -> Result<Self, ProviderConfigError> {
        let mut config = ProviderConfigSet::default();

        if let Some(value) = table.get(ROOT_PROVIDER_KEY) {
            let provider_table = value.as_table().ok_or_else(|| {
                ProviderConfigError::type_mismatch("$.provider", "a table", value)
            })?;

            if let Some(enabled) = provider_table.get(ROOT_ENABLED_KEY) {
                config.enabled = Some(parse_string_set("$.provider.enabled", enabled)?);
            }
            if let Some(disabled) = provider_table.get(ROOT_DISABLED_KEY) {
                config.disabled = parse_string_set("$.provider.disabled", disabled)?;
            }
            if let Some(overrides) = provider_table.get(ROOT_MODEL_OVERRIDES_KEY) {
                config.model_overrides = parse_string_map("$.provider.model_overrides", overrides)?;
            }

            for (key, value) in provider_table {
                if matches!(
                    key.as_str(),
                    ROOT_ENABLED_KEY | ROOT_DISABLED_KEY | ROOT_MODEL_OVERRIDES_KEY
                ) {
                    continue;
                }
                let provider_table = value.as_table().ok_or_else(|| {
                    ProviderConfigError::type_mismatch(
                        format!("$.provider.{key}"),
                        "a table",
                        value,
                    )
                })?;
                let provider = parse_provider_entry(key, provider_table)?;
                config.providers.insert(key.clone(), provider);
            }
        }

        apply_legacy_openrouter_aliases(table, &mut config)?;
        Ok(config)
    }

    /// Whether `provider_id` is enabled after applying allow/deny lists and provider-local
    /// `enabled` toggles.
    #[must_use]
    pub fn provider_enabled(&self, provider_id: &str) -> bool {
        if self.disabled.contains(provider_id) {
            return false;
        }
        if let Some(enabled) = &self.enabled
            && !enabled.contains(provider_id)
        {
            return false;
        }
        self.providers
            .get(provider_id)
            .map(|provider| provider.enabled)
            .unwrap_or(true)
    }

    /// The typed provider entry for `provider_id`, when configured.
    #[must_use]
    pub fn provider(&self, provider_id: &str) -> Option<&ProviderRuntimeConfig> {
        self.providers.get(provider_id)
    }

    /// The configured or inferred protocol for `provider_id`.
    #[must_use]
    pub fn provider_protocol(&self, provider_id: &str) -> ProviderProtocol {
        if let Some(protocol) = self
            .providers
            .get(provider_id)
            .and_then(|provider| provider.protocol)
        {
            return protocol;
        }
        if let Some(profile) = provider_profiles::profile(provider_id)
            && let Some(protocol) = protocol_from_profile(profile.default_protocol)
        {
            return protocol;
        }
        infer_provider_protocol(provider_id)
    }

    /// Resolve the requested model id using global/per-provider model overrides and provider
    /// enable/disable policy.
    ///
    /// Unknown but well-formed ids preserve the existing passthrough behavior: they resolve to the
    /// default provider (OpenRouter) unless the model id prefixes a configured provider.
    pub fn resolve_model(
        &self,
        base_catalogue: &Catalogue,
        requested: Option<&str>,
    ) -> Result<ResolvedModel, ProviderConfigError> {
        let merged = self.merged_catalogue(base_catalogue);
        let mut model_id = requested
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or(DEFAULT_MODEL)
            .to_owned();
        model_id = apply_override_chain(&self.model_overrides, &model_id);

        let mut resolved = merged.resolve_model(Some(&model_id));
        if resolved.provider_id == DEFAULT_PROVIDER
            && let Some((provider, _)) = model_id.split_once('/')
            && self.providers.contains_key(provider)
        {
            resolved.provider_id = provider.to_owned();
            resolved.model_id = model_id.clone();
            resolved.info = merged.model(provider, &model_id).cloned();
        }

        if let Some(provider) = self.providers.get(&resolved.provider_id) {
            let overridden = apply_override_chain(&provider.model_overrides, &resolved.model_id);
            if overridden != resolved.model_id {
                resolved.model_id = overridden;
                if let Some(info) = merged.model(&resolved.provider_id, &resolved.model_id) {
                    resolved.info = Some(info.clone());
                } else {
                    resolved.info = None;
                }
            }
        }

        if !self.provider_enabled(&resolved.provider_id) {
            return Err(ProviderConfigError::provider_disabled(
                &resolved.provider_id,
            ));
        }
        Ok(resolved)
    }

    /// Build a catalogue view with configured providers/models merged on top of `base`.
    #[must_use]
    pub fn merged_catalogue(&self, base: &Catalogue) -> Catalogue {
        let mut providers: BTreeMap<String, ProviderInfo> = base
            .providers()
            .map(|provider| (provider.id.clone(), provider.clone()))
            .collect();

        for provider in self.providers.values() {
            let entry = providers
                .entry(provider.id.clone())
                .or_insert_with(|| ProviderInfo {
                    id: provider.id.clone(),
                    name: provider.id.clone(),
                    api: None,
                    env: Vec::new(),
                    models: BTreeMap::new(),
                });
            if let Some(base_url) = &provider.base_url {
                entry.api = Some(base_url.clone());
            }
            if let Some(api_key_env) = &provider.auth.api_key_env {
                entry.env = vec![api_key_env.clone()];
            }
            entry.models.extend(
                provider
                    .models
                    .iter()
                    .map(|(id, model)| (id.clone(), model.clone())),
            );
        }

        Catalogue::from_providers(providers)
    }

    /// The merged catalogue built on top of the dynamically-resolved models.dev base.
    ///
    /// This is the single entry point the `/models` picker and `cortex models` use: it resolves
    /// the models.dev catalogue (a live refresh when online, else the on-disk cache, else the
    /// embedded snapshot — see [`crate::refresh::resolve_catalogue`]) and folds configured
    /// providers/models on top. The excluded Zen provider is omitted throughout.
    #[must_use]
    pub fn resolved_catalogue(&self) -> Catalogue {
        self.merged_catalogue(&crate::refresh::resolve_catalogue())
    }

    /// The provider base URL if configured, otherwise `None`.
    #[must_use]
    pub fn provider_base_url(&self, provider_id: &str) -> Option<String> {
        self.providers
            .get(provider_id)
            .and_then(|provider| provider.base_url.clone())
    }

    /// Build credential-resolution inputs for `provider_id`.
    ///
    /// Precedence remains env > 0600 auth file > config value.
    #[must_use]
    pub fn credential_inputs(&self, provider_id: &str) -> CredentialInputs {
        let provider = self.providers.get(provider_id);
        let env_name = provider
            .and_then(|provider| provider.auth.api_key_env.clone())
            .or_else(|| {
                provider_profiles::profile(provider_id)
                    .and_then(|profile| profile.default_api_key_env)
                    .map(ToOwned::to_owned)
            })
            .or_else(|| (provider_id == OPENROUTER_ID).then(|| OPENROUTER_ENV.to_owned()));
        let config_value = provider.and_then(|provider| provider.auth.api_key.clone());
        CredentialInputs::from_process(env_name.as_deref(), config_value)
    }

    /// Extra request headers configured for `provider_id`.
    #[must_use]
    pub fn provider_headers(&self, provider_id: &str) -> Vec<(String, String)> {
        self.providers
            .get(provider_id)
            .map(|provider| {
                provider
                    .options
                    .headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Extra JSON request params configured for `provider_id`.
    #[must_use]
    pub fn provider_params(&self, provider_id: &str) -> JsonMap<String, JsonValue> {
        self.providers
            .get(provider_id)
            .map(|provider| provider.options.params.clone())
            .unwrap_or_default()
    }

    /// Resolve the provider base URL, honoring OpenRouter defaults when unconfigured.
    pub fn resolve_provider_base_url(
        &self,
        provider_id: &str,
    ) -> Result<String, ProviderConfigError> {
        if let Some(base_url) = self.provider_base_url(provider_id) {
            return Ok(base_url);
        }
        if let Some(base_url) =
            provider_profiles::profile(provider_id).and_then(|profile| profile.default_base_url)
        {
            return Ok(base_url.to_owned());
        }
        if provider_id == OPENROUTER_ID {
            return Ok(crate::openrouter::OPENROUTER_API_BASE.to_owned());
        }
        Err(ProviderConfigError::missing_base_url(provider_id))
    }

    /// Resolve the OpenAI-compatible chat-completions endpoint URL for `provider_id`.
    pub fn resolve_chat_endpoint(&self, provider_id: &str) -> Result<String, ProviderConfigError> {
        let base_url = self.resolve_provider_base_url(provider_id)?;
        Ok(format!(
            "{}/chat/completions",
            base_url.trim_end_matches('/')
        ))
    }

    /// Resolve the protocol-specific streaming endpoint URL for `provider_id` and `model_id`.
    pub fn resolve_stream_endpoint(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<String, ProviderConfigError> {
        let base_url = self.resolve_provider_base_url(provider_id)?;
        let endpoint = match self.provider_protocol(provider_id) {
            ProviderProtocol::OpenAiChat => {
                format!("{}/chat/completions", base_url.trim_end_matches('/'))
            }
            ProviderProtocol::OpenAiResponses => {
                format!("{}/responses", base_url.trim_end_matches('/'))
            }
            ProviderProtocol::AnthropicMessages => {
                format!("{}/messages", base_url.trim_end_matches('/'))
            }
            ProviderProtocol::Gemini => {
                let model = normalize_gemini_model_id(model_id);
                let model_path = if model.starts_with("models/") {
                    model
                } else {
                    format!("models/{model}")
                };
                format!(
                    "{}/{model_path}:streamGenerateContent?alt=sse",
                    base_url.trim_end_matches('/')
                )
            }
        };
        Ok(endpoint)
    }

    /// Redacted provider-configuration report for machine-readable output.
    #[must_use]
    pub fn redacted_report(&self) -> ProviderConfigReport {
        ProviderConfigReport {
            enabled: self
                .enabled
                .as_ref()
                .map(|ids| ids.iter().cloned().collect::<Vec<_>>()),
            disabled: self.disabled.iter().cloned().collect(),
            model_overrides: self.model_overrides.clone(),
            providers: self
                .providers
                .iter()
                .map(|(id, provider)| {
                    (
                        id.clone(),
                        ProviderReport {
                            enabled: provider.enabled,
                            base_url: provider.base_url.clone(),
                            protocol: provider
                                .protocol
                                .map(|protocol| protocol.as_str().to_owned()),
                            auth: ProviderAuthReport {
                                api_key: provider
                                    .auth
                                    .api_key
                                    .as_ref()
                                    .map(|_| "[REDACTED]".to_owned()),
                                api_key_env: provider.auth.api_key_env.clone(),
                            },
                            model_overrides: provider.model_overrides.clone(),
                            model_count: provider.models.len(),
                            options: ProviderOptionsReport {
                                headers: provider
                                    .options
                                    .headers
                                    .keys()
                                    .cloned()
                                    .collect::<Vec<_>>(),
                                params: provider.options.params.clone(),
                            },
                        },
                    )
                })
                .collect(),
        }
    }
}

/// Redacted provider-configuration document for JSON outputs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderConfigReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Vec<String>>,
    pub disabled: Vec<String>,
    pub model_overrides: BTreeMap<String, String>,
    pub providers: BTreeMap<String, ProviderReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderReport {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    pub auth: ProviderAuthReport,
    pub model_overrides: BTreeMap<String, String>,
    pub model_count: usize,
    pub options: ProviderOptionsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderAuthReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderOptionsReport {
    pub headers: Vec<String>,
    pub params: JsonMap<String, JsonValue>,
}

/// Parser/validation error for provider runtime config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfigError {
    pub key: String,
    pub message: String,
}

impl ProviderConfigError {
    fn type_mismatch(key: impl Into<String>, expected: &str, found: &Value) -> Self {
        Self {
            key: key.into(),
            message: format!("expected {expected}, got {}", value_kind(found)),
        }
    }

    fn invalid(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            message: message.into(),
        }
    }

    fn provider_disabled(provider_id: &str) -> Self {
        Self::invalid(
            format!("$.provider.{provider_id}.enabled"),
            format!("provider `{provider_id}` is disabled by provider policy"),
        )
    }

    fn missing_base_url(provider_id: &str) -> Self {
        Self::invalid(
            format!("$.provider.{provider_id}.base_url"),
            format!(
                "provider `{provider_id}` is selected but has no `base_url`; configure `[provider.{provider_id}].base_url`"
            ),
        )
    }
}

impl std::fmt::Display for ProviderConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.key, self.message)
    }
}

impl std::error::Error for ProviderConfigError {}

fn apply_legacy_openrouter_aliases(
    table: &Table,
    config: &mut ProviderConfigSet,
) -> Result<(), ProviderConfigError> {
    let legacy_base_url = parse_optional_string(
        table.get(ROOT_OPENROUTER_BASE_URL_KEY),
        &format!("$.{ROOT_OPENROUTER_BASE_URL_KEY}"),
    )?;
    let legacy_api_key = parse_optional_string(
        table.get(ROOT_API_KEY_KEY),
        &format!("$.{ROOT_API_KEY_KEY}"),
    )?;

    if legacy_base_url.is_some() || legacy_api_key.is_some() {
        let provider = config
            .providers
            .entry(OPENROUTER_ID.to_owned())
            .or_insert_with(|| ProviderRuntimeConfig {
                id: OPENROUTER_ID.to_owned(),
                enabled: true,
                ..ProviderRuntimeConfig::default()
            });
        // Legacy top-level base_url wins over nested value to preserve current behavior.
        if let Some(base_url) = legacy_base_url {
            provider.base_url = Some(base_url);
        }
        // Legacy top-level api_key is fallback only; explicit nested value still wins.
        if provider.auth.api_key.is_none() {
            provider.auth.api_key = legacy_api_key;
        }
    }

    Ok(())
}

fn parse_provider_entry(
    provider_id: &str,
    table: &Table,
) -> Result<ProviderRuntimeConfig, ProviderConfigError> {
    let enabled = table
        .get(PROVIDER_ENABLED_KEY)
        .map(|value| parse_bool(value, &format!("$.provider.{provider_id}.enabled")))
        .transpose()?
        .unwrap_or(true);
    let base_url = parse_optional_string(
        table.get(PROVIDER_BASE_URL_KEY),
        &format!("$.provider.{provider_id}.base_url"),
    )?;
    let protocol = table
        .get(PROVIDER_PROTOCOL_KEY)
        .map(|value| parse_provider_protocol(value, &format!("$.provider.{provider_id}.protocol")))
        .transpose()?;
    let api_key = parse_optional_string(
        table.get(PROVIDER_API_KEY_KEY),
        &format!("$.provider.{provider_id}.api_key"),
    )?;
    let api_key_env = parse_optional_string(
        table.get(PROVIDER_API_KEY_ENV_KEY),
        &format!("$.provider.{provider_id}.api_key_env"),
    )?;

    let mut auth = ProviderAuthConfig {
        api_key,
        api_key_env,
    };
    if let Some(auth_value) = table.get(PROVIDER_AUTH_KEY) {
        let auth_table = auth_value.as_table().ok_or_else(|| {
            ProviderConfigError::type_mismatch(
                format!("$.provider.{provider_id}.auth"),
                "a table",
                auth_value,
            )
        })?;
        if let Some(value) = auth_table.get(PROVIDER_API_KEY_KEY) {
            auth.api_key = Some(parse_required_string(
                value,
                &format!("$.provider.{provider_id}.auth.api_key"),
            )?);
        }
        if let Some(value) = auth_table.get(PROVIDER_API_KEY_ENV_KEY) {
            auth.api_key_env = Some(parse_required_string(
                value,
                &format!("$.provider.{provider_id}.auth.api_key_env"),
            )?);
        }
    }

    let model_overrides = table
        .get(PROVIDER_MODEL_OVERRIDES_KEY)
        .map(|value| parse_string_map(&format!("$.provider.{provider_id}.model_overrides"), value))
        .transpose()?
        .unwrap_or_default();

    let models = table
        .get(PROVIDER_MODELS_KEY)
        .map(|value| parse_models(&format!("$.provider.{provider_id}.models"), value))
        .transpose()?
        .unwrap_or_default();

    let options = table
        .get(PROVIDER_OPTIONS_KEY)
        .map(|value| parse_provider_options(&format!("$.provider.{provider_id}.options"), value))
        .transpose()?
        .unwrap_or_default();

    Ok(ProviderRuntimeConfig {
        id: provider_id.to_owned(),
        enabled,
        base_url,
        protocol,
        auth,
        model_overrides,
        models,
        options,
    })
}

fn parse_provider_options(
    key_path: &str,
    value: &Value,
) -> Result<ProviderOptionsConfig, ProviderConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ProviderConfigError::type_mismatch(key_path, "a table", value))?;

    let headers = table
        .get(OPTIONS_HEADERS_KEY)
        .map(|headers| parse_string_map(&format!("{key_path}.headers"), headers))
        .transpose()?
        .unwrap_or_default();
    let params = table
        .get(OPTIONS_PARAMS_KEY)
        .map(|params| parse_json_map(&format!("{key_path}.params"), params))
        .transpose()?
        .unwrap_or_default();
    Ok(ProviderOptionsConfig { headers, params })
}

fn parse_models(
    key_path: &str,
    value: &Value,
) -> Result<BTreeMap<String, ModelInfo>, ProviderConfigError> {
    match value {
        Value::Array(items) => {
            let mut models = BTreeMap::new();
            for (index, item) in items.iter().enumerate() {
                let table = item.as_table().ok_or_else(|| {
                    ProviderConfigError::type_mismatch(
                        format!("{key_path}[{index}]"),
                        "a table",
                        item,
                    )
                })?;
                let id = table
                    .get(MODEL_ID_KEY)
                    .ok_or_else(|| {
                        ProviderConfigError::invalid(
                            format!("{key_path}[{index}].{MODEL_ID_KEY}"),
                            "missing required model id",
                        )
                    })
                    .and_then(|id| {
                        parse_required_string(id, &format!("{key_path}[{index}].{MODEL_ID_KEY}"))
                    })?;
                let model = parse_model_table(&format!("{key_path}[{index}]"), table, Some(&id))?;
                models.insert(id, model);
            }
            Ok(models)
        }
        Value::Table(models) => {
            let mut parsed = BTreeMap::new();
            for (model_id, model_value) in models {
                let table = model_value.as_table().ok_or_else(|| {
                    ProviderConfigError::type_mismatch(
                        format!("{key_path}.{model_id}"),
                        "a table",
                        model_value,
                    )
                })?;
                parsed.insert(
                    model_id.clone(),
                    parse_model_table(&format!("{key_path}.{model_id}"), table, Some(model_id))?,
                );
            }
            Ok(parsed)
        }
        _ => Err(ProviderConfigError::type_mismatch(
            key_path,
            "an array or table",
            value,
        )),
    }
}

fn parse_model_table(
    key_path: &str,
    table: &Table,
    fallback_model_id: Option<&str>,
) -> Result<ModelInfo, ProviderConfigError> {
    let id = table
        .get(MODEL_ID_KEY)
        .map(|value| parse_required_string(value, &format!("{key_path}.{MODEL_ID_KEY}")))
        .transpose()?
        .or_else(|| fallback_model_id.map(ToOwned::to_owned))
        .ok_or_else(|| {
            ProviderConfigError::invalid(
                format!("{key_path}.{MODEL_ID_KEY}"),
                "missing required model id",
            )
        })?;
    let name = table
        .get(MODEL_NAME_KEY)
        .map(|value| parse_required_string(value, &format!("{key_path}.{MODEL_NAME_KEY}")))
        .transpose()?
        .unwrap_or_else(|| id.clone());
    let tool_call = table
        .get(MODEL_TOOL_CALL_KEY)
        .map(|value| parse_bool(value, &format!("{key_path}.{MODEL_TOOL_CALL_KEY}")))
        .transpose()?
        .unwrap_or(false);
    let reasoning = table
        .get(MODEL_REASONING_KEY)
        .map(|value| parse_bool(value, &format!("{key_path}.{MODEL_REASONING_KEY}")))
        .transpose()?
        .unwrap_or(false);
    let attachment = table
        .get(MODEL_ATTACHMENT_KEY)
        .map(|value| parse_bool(value, &format!("{key_path}.{MODEL_ATTACHMENT_KEY}")))
        .transpose()?
        .unwrap_or(false);

    let context_length = parse_optional_u64(
        table.get(MODEL_CONTEXT_LENGTH_KEY),
        &format!("{key_path}.{MODEL_CONTEXT_LENGTH_KEY}"),
    )?
    .or(parse_optional_u64(
        table.get(MODEL_CONTEXT_ALIAS_KEY),
        &format!("{key_path}.{MODEL_CONTEXT_ALIAS_KEY}"),
    )?)
    .unwrap_or(0);

    let max_output_limit = parse_optional_u64(
        table.get(MODEL_MAX_OUTPUT_LIMIT_KEY),
        &format!("{key_path}.{MODEL_MAX_OUTPUT_LIMIT_KEY}"),
    )?
    .or(parse_optional_u64(
        table.get(MODEL_OUTPUT_ALIAS_KEY),
        &format!("{key_path}.{MODEL_OUTPUT_ALIAS_KEY}"),
    )?)
    .unwrap_or(0);

    let nested_cost = match table.get(MODEL_COST_KEY) {
        Some(cost) => Some(cost.as_table().ok_or_else(|| {
            ProviderConfigError::type_mismatch(
                format!("{key_path}.{MODEL_COST_KEY}"),
                "a table",
                cost,
            )
        })?),
        None => None,
    };
    let has_cost_metadata = table.contains_key(MODEL_COST_KEY)
        || table.contains_key(MODEL_COST_INPUT_ALIAS_KEY)
        || table.contains_key(MODEL_COST_OUTPUT_ALIAS_KEY)
        || table.contains_key(MODEL_COST_CACHE_WRITE_ALIAS_KEY)
        || table.contains_key(MODEL_COST_CACHE_READ_ALIAS_KEY)
        || table.contains_key(MODEL_COST_REASONING_ALIAS_KEY);
    let cost_input = parse_optional_f64(
        table.get(MODEL_COST_INPUT_ALIAS_KEY),
        &format!("{key_path}.{MODEL_COST_INPUT_ALIAS_KEY}"),
    )?
    .or(parse_optional_f64(
        nested_cost.and_then(|cost| cost.get(MODEL_COST_INPUT_KEY)),
        &format!("{key_path}.{MODEL_COST_KEY}.{MODEL_COST_INPUT_KEY}"),
    )?)
    .unwrap_or(0.0);
    let cost_output = parse_optional_f64(
        table.get(MODEL_COST_OUTPUT_ALIAS_KEY),
        &format!("{key_path}.{MODEL_COST_OUTPUT_ALIAS_KEY}"),
    )?
    .or(parse_optional_f64(
        nested_cost.and_then(|cost| cost.get(MODEL_COST_OUTPUT_KEY)),
        &format!("{key_path}.{MODEL_COST_KEY}.{MODEL_COST_OUTPUT_KEY}"),
    )?)
    .unwrap_or(0.0);
    let cost_cache_write = parse_optional_f64(
        table.get(MODEL_COST_CACHE_WRITE_ALIAS_KEY),
        &format!("{key_path}.{MODEL_COST_CACHE_WRITE_ALIAS_KEY}"),
    )?
    .or(parse_optional_f64(
        nested_cost.and_then(|cost| cost.get(MODEL_COST_CACHE_WRITE_KEY)),
        &format!("{key_path}.{MODEL_COST_KEY}.{MODEL_COST_CACHE_WRITE_KEY}"),
    )?);
    let cost_cache_read = parse_optional_f64(
        table.get(MODEL_COST_CACHE_READ_ALIAS_KEY),
        &format!("{key_path}.{MODEL_COST_CACHE_READ_ALIAS_KEY}"),
    )?
    .or(parse_optional_f64(
        nested_cost.and_then(|cost| cost.get(MODEL_COST_CACHE_READ_KEY)),
        &format!("{key_path}.{MODEL_COST_KEY}.{MODEL_COST_CACHE_READ_KEY}"),
    )?);
    let cost_reasoning = parse_optional_f64(
        table.get(MODEL_COST_REASONING_ALIAS_KEY),
        &format!("{key_path}.{MODEL_COST_REASONING_ALIAS_KEY}"),
    )?
    .or(parse_optional_f64(
        nested_cost.and_then(|cost| cost.get(MODEL_COST_REASONING_KEY)),
        &format!("{key_path}.{MODEL_COST_KEY}.{MODEL_COST_REASONING_KEY}"),
    )?);
    let over_input_threshold =
        match nested_cost.and_then(|cost| cost.get(MODEL_COST_OVER_THRESHOLD_KEY)) {
            Some(value) => Some(parse_cost_tier(
                value,
                &format!("{key_path}.{MODEL_COST_KEY}.{MODEL_COST_OVER_THRESHOLD_KEY}"),
            )?),
            None => None,
        };

    Ok(ModelInfo {
        id,
        name,
        tool_call,
        reasoning,
        attachment,
        cost: ModelCost {
            input: cost_input,
            output: cost_output,
            cache_write: cost_cache_write,
            cache_read: cost_cache_read,
            reasoning: cost_reasoning,
            over_input_threshold,
            ..ModelCost::default()
        }
        .with_known(has_cost_metadata),
        limit: ModelLimit {
            context: context_length,
            output: max_output_limit,
        },
        release_date: None,
    })
}

fn parse_cost_tier(value: &Value, key_path: &str) -> Result<ModelCostTier, ProviderConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ProviderConfigError::type_mismatch(key_path, "a table", value))?;
    let tier_cutoff = parse_optional_u64(
        table.get(MODEL_COST_THRESHOLD_KEY),
        &format!("{key_path}.{MODEL_COST_THRESHOLD_KEY}"),
    )?
    .unwrap_or(200_000);
    let input = parse_optional_f64(
        table.get(MODEL_COST_INPUT_KEY),
        &format!("{key_path}.{MODEL_COST_INPUT_KEY}"),
    )?
    .unwrap_or(0.0);
    let output = parse_optional_f64(
        table.get(MODEL_COST_OUTPUT_KEY),
        &format!("{key_path}.{MODEL_COST_OUTPUT_KEY}"),
    )?
    .unwrap_or(0.0);
    let cache_write = parse_optional_f64(
        table.get(MODEL_COST_CACHE_WRITE_KEY),
        &format!("{key_path}.{MODEL_COST_CACHE_WRITE_KEY}"),
    )?;
    let cache_read = parse_optional_f64(
        table.get(MODEL_COST_CACHE_READ_KEY),
        &format!("{key_path}.{MODEL_COST_CACHE_READ_KEY}"),
    )?;
    let reasoning = parse_optional_f64(
        table.get(MODEL_COST_REASONING_KEY),
        &format!("{key_path}.{MODEL_COST_REASONING_KEY}"),
    )?;
    Ok(ModelCostTier {
        threshold_input_tokens: tier_cutoff,
        input,
        output,
        cache_write,
        cache_read,
        reasoning,
    })
}

fn parse_string_set(
    key_path: &str,
    value: &Value,
) -> Result<BTreeSet<String>, ProviderConfigError> {
    let array = value.as_array().ok_or_else(|| {
        ProviderConfigError::type_mismatch(key_path, "an array of strings", value)
    })?;
    let mut set = BTreeSet::new();
    for (index, item) in array.iter().enumerate() {
        let entry = parse_required_string(item, &format!("{key_path}[{index}]"))?;
        set.insert(entry);
    }
    Ok(set)
}

fn parse_string_map(
    key_path: &str,
    value: &Value,
) -> Result<BTreeMap<String, String>, ProviderConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ProviderConfigError::type_mismatch(key_path, "a table of strings", value))?;
    let mut out = BTreeMap::new();
    for (key, value) in table {
        out.insert(
            key.clone(),
            parse_required_string(value, &format!("{key_path}.{key}"))?,
        );
    }
    Ok(out)
}

fn parse_json_map(
    key_path: &str,
    value: &Value,
) -> Result<JsonMap<String, JsonValue>, ProviderConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| ProviderConfigError::type_mismatch(key_path, "a table", value))?;
    table
        .iter()
        .map(|(key, value)| {
            toml_to_json(value)
                .map(|value| (key.clone(), value))
                .ok_or_else(|| {
                    ProviderConfigError::invalid(
                        format!("{key_path}.{key}"),
                        "value cannot be represented as JSON",
                    )
                })
        })
        .collect()
}

fn parse_optional_string(
    value: Option<&Value>,
    key_path: &str,
) -> Result<Option<String>, ProviderConfigError> {
    value
        .map(|value| parse_required_string(value, key_path))
        .transpose()
}

fn parse_required_string(value: &Value, key_path: &str) -> Result<String, ProviderConfigError> {
    let string = value
        .as_str()
        .ok_or_else(|| ProviderConfigError::type_mismatch(key_path, "a string", value))?;
    Ok(string.trim().to_owned())
}

fn parse_provider_protocol(
    value: &Value,
    key_path: &str,
) -> Result<ProviderProtocol, ProviderConfigError> {
    let raw = parse_required_string(value, key_path)?.to_ascii_lowercase();
    match raw.as_str() {
        "openai-chat" | "openai_chat" | "chat" => Ok(ProviderProtocol::OpenAiChat),
        "openai-responses" | "openai_responses" | "responses" => {
            Ok(ProviderProtocol::OpenAiResponses)
        }
        "anthropic-messages" | "anthropic_messages" | "anthropic" => {
            Ok(ProviderProtocol::AnthropicMessages)
        }
        "gemini" | "google-gemini" | "gemini-native" => Ok(ProviderProtocol::Gemini),
        _ => Err(ProviderConfigError::invalid(
            key_path.to_owned(),
            "expected one of: openai-chat, openai-responses, anthropic-messages, gemini",
        )),
    }
}

fn protocol_from_profile(profile_protocol: &str) -> Option<ProviderProtocol> {
    match profile_protocol {
        "openai-chat" => Some(ProviderProtocol::OpenAiChat),
        "openai-responses" => Some(ProviderProtocol::OpenAiResponses),
        "anthropic-messages" => Some(ProviderProtocol::AnthropicMessages),
        "gemini" => Some(ProviderProtocol::Gemini),
        _ => None,
    }
}

fn infer_provider_protocol(provider_id: &str) -> ProviderProtocol {
    match provider_id {
        "openai-responses" | "openai_responses" => ProviderProtocol::OpenAiResponses,
        "anthropic" | "bedrock" | "amazon-bedrock" => ProviderProtocol::AnthropicMessages,
        "google" | "gemini" | "vertex" | "google-vertex" => ProviderProtocol::Gemini,
        _ => ProviderProtocol::OpenAiChat,
    }
}

fn normalize_gemini_model_id(model_id: &str) -> String {
    let Some((prefix, rest)) = model_id.split_once('/') else {
        return model_id.to_owned();
    };
    if prefix.eq_ignore_ascii_case("google") || prefix.eq_ignore_ascii_case("gemini") {
        rest.to_owned()
    } else {
        model_id.to_owned()
    }
}

fn parse_bool(value: &Value, key_path: &str) -> Result<bool, ProviderConfigError> {
    value
        .as_bool()
        .ok_or_else(|| ProviderConfigError::type_mismatch(key_path, "a boolean", value))
}

fn parse_optional_u64(
    value: Option<&Value>,
    key_path: &str,
) -> Result<Option<u64>, ProviderConfigError> {
    value
        .map(|value| match value {
            Value::Integer(integer) if *integer >= 0 => Ok(*integer as u64),
            Value::Integer(_) => Err(ProviderConfigError::invalid(
                key_path,
                "must be a non-negative integer",
            )),
            _ => Err(ProviderConfigError::type_mismatch(
                key_path,
                "a non-negative integer",
                value,
            )),
        })
        .transpose()
}

fn parse_optional_f64(
    value: Option<&Value>,
    key_path: &str,
) -> Result<Option<f64>, ProviderConfigError> {
    value
        .map(|value| match value {
            Value::Float(number) => Ok(*number),
            Value::Integer(number) => Ok(*number as f64),
            _ => Err(ProviderConfigError::type_mismatch(
                key_path, "a number", value,
            )),
        })
        .transpose()
}

fn toml_to_json(value: &Value) -> Option<JsonValue> {
    serde_json::to_value(value).ok()
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::String(_) => "string",
        Value::Integer(_) => "integer",
        Value::Float(_) => "float",
        Value::Boolean(_) => "boolean",
        Value::Datetime(_) => "datetime",
        Value::Array(_) => "array",
        Value::Table(_) => "table",
    }
}

fn apply_override_chain(overrides: &BTreeMap<String, String>, model_id: &str) -> String {
    let mut current = model_id.to_owned();
    let mut visited = BTreeSet::new();
    for _ in 0..16 {
        let Some(next) = overrides.get(&current) else {
            break;
        };
        if !visited.insert(current.clone()) || visited.contains(next) {
            break;
        }
        current = next.clone();
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_set(toml_src: &str) -> ProviderConfigSet {
        let table: Table = toml::from_str(toml_src).expect("valid toml fixture");
        ProviderConfigSet::from_table(&table).expect("provider config parses")
    }

    #[test]
    fn nested_model_cost_accepts_integer_values() {
        let config = parse_set(
            r#"
[provider.local]
base_url = "http://127.0.0.1:18765"
models = [{ id = "local/demo", cost = { input = 2, output = 3 } }]
"#,
        );
        let model = config
            .providers
            .get("local")
            .and_then(|provider| provider.models.get("local/demo"))
            .expect("configured model");
        assert_eq!(model.cost.input, 2.0);
        assert_eq!(model.cost.output, 3.0);
    }

    #[test]
    fn nested_model_cost_type_errors_are_reported() {
        let table: Table = toml::from_str(
            r#"
[provider.local]
base_url = "http://127.0.0.1:18765"
models = [{ id = "local/demo", cost = { input = "cheap", output = 3 } }]
"#,
        )
        .expect("valid toml fixture");
        let err = ProviderConfigSet::from_table(&table).expect_err("invalid cost type must fail");
        assert!(
            err.key.contains("cost.input"),
            "error key should point at nested cost input, got: {}",
            err.key
        );
        assert!(
            err.message.contains("expected a number"),
            "error should mention numeric expectation, got: {}",
            err.message
        );
    }

    #[test]
    fn top_level_cost_aliases_override_nested_cost_values() {
        let config = parse_set(
            r#"
[provider.local]
base_url = "http://127.0.0.1:18765"
models = [{ id = "local/demo", cost_input = 7, cost_output = 11, cost = { input = 2, output = 3 } }]
"#,
        );
        let model = config
            .providers
            .get("local")
            .and_then(|provider| provider.models.get("local/demo"))
            .expect("configured model");
        assert_eq!(model.cost.input, 7.0);
        assert_eq!(model.cost.output, 11.0);
    }

    #[test]
    fn nested_model_cost_parses_cache_reasoning_and_tier_fields() {
        let config = parse_set(
            r#"
[provider.local]
base_url = "http://127.0.0.1:18765"
models = [{ id = "local/demo", cost = { input = 2, output = 3, cache_write = 1.1, cache_read = 0.7, reasoning = 4.2, over_input_threshold = { threshold_input_tokens = 123456, input = 5, output = 8, cache_write = 2, cache_read = 1, reasoning = 9 } } }]
"#,
        );
        let model = config
            .providers
            .get("local")
            .and_then(|provider| provider.models.get("local/demo"))
            .expect("configured model");
        assert_eq!(model.cost.cache_write, Some(1.1));
        assert_eq!(model.cost.cache_read, Some(0.7));
        assert_eq!(model.cost.reasoning, Some(4.2));
        let tier = model.cost.over_input_threshold.expect("tier");
        assert_eq!(tier.threshold_input_tokens, 123_456);
        assert_eq!(tier.input, 5.0);
        assert_eq!(tier.output, 8.0);
        assert_eq!(tier.cache_write, Some(2.0));
        assert_eq!(tier.cache_read, Some(1.0));
        assert_eq!(tier.reasoning, Some(9.0));
    }

    #[test]
    fn built_in_profiles_seed_default_protocol_and_local_base_url() {
        let config = ProviderConfigSet::default();
        assert_eq!(config.provider_protocol("vertex"), ProviderProtocol::Gemini);
        assert_eq!(
            config.provider_protocol("bedrock"),
            ProviderProtocol::AnthropicMessages
        );
        assert_eq!(
            config.resolve_provider_base_url("ollama").as_deref(),
            Ok("http://127.0.0.1:11434/v1")
        );
    }

    #[test]
    fn merged_catalogue_excludes_zen_from_configured_providers() {
        // Even a config that names the excluded Zen provider must not surface it in the merged
        // catalogue.
        let zen = crate::EXCLUDED_ZEN_PROVIDER_IDS[0].to_owned();
        let toml_src = format!(
            r#"
[provider.{zen}]
base_url = "https://{zen}.ai/zen/v1"
models = [{{ id = "{zen}/grok-code", name = "code model" }}]
"#
        );
        let config = parse_set(&toml_src);
        let merged = config.merged_catalogue(Catalogue::embedded());
        assert!(
            merged.provider(&zen).is_none(),
            "the excluded Zen provider is stripped from the merged catalogue"
        );
        assert!(merged.find_model(&format!("{zen}/grok-code")).is_none());
        // The embedded openrouter base is still present.
        assert!(merged.provider(OPENROUTER_ID).is_some());
    }
}

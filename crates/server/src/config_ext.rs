//! Config and provider inspection routes layered over the shared settings + provider catalogue.
//!
//! These handlers let a server client inspect the effective (redacted) configuration, patch the
//! small set of safe top-level config fields, and list providers and their authenticated state
//! without shelling out to the CLI. They reuse the same layered `settings` loader, provider
//! `Catalogue`, and `ProviderConfigSet` as `cortex config` / `cortex models`, so the server stays a
//! thin surface over the same resolution logic rather than a parallel implementation.

use std::collections::BTreeMap;

use axum::{Json, extract::State, http::StatusCode};
use providers::{Catalogue, ProviderConfigReport, ProviderConfigSet};
use serde::Serialize;
use settings::{ConfigInputs, EffectiveConfig};

use super::{ApiError, ServerState, auth_ext, config_api_error, provider_config_api_error};

/// The set of top-level config keys a server client may patch.
///
/// The patch surface is intentionally narrow: it covers the safe runtime selectors and refuses
/// anything else (including nested provider/auth tables) so the route can never be used to inject a
/// secret or an unvalidated structure through HTTP.
const PATCHABLE_KEYS: &[&str] = &["model", "agent", "max_steps"];

/// `GET /config` — report the effective, secret-redacted configuration.
pub(super) async fn get_config(
    State(state): State<ServerState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let effective = load_effective(&state)?;
    Ok(Json(redacted_config_json(&effective)?))
}

/// `PATCH /config` — update the safe top-level config fields and persist them to the project file.
///
/// The patch is merged into `{cwd}/cortex.toml` (the project layer), so the change is visible to
/// later requests and survives a restart, exactly like a hand-edited project config. Unknown keys
/// and ill-typed values are rejected with a clean 4xx before anything is written.
pub(super) async fn patch_config(
    State(state): State<ServerState>,
    Json(payload): Json<serde_json::Map<String, serde_json::Value>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if payload.is_empty() {
        return Err(ApiError::bad_request("no config fields to update"));
    }

    let mut updates: BTreeMap<String, toml::Value> = BTreeMap::new();
    for (key, value) in &payload {
        if !PATCHABLE_KEYS.contains(&key.as_str()) {
            return Err(ApiError::bad_request(format!(
                "config field `{key}` is not patchable (allowed: {})",
                PATCHABLE_KEYS.join(", ")
            )));
        }
        let toml_value = validate_patch_value(key, value)?;
        updates.insert(key.clone(), toml_value);
    }

    let path = state.cwd.join("cortex.toml");
    let mut table: toml::Table = match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).map_err(|err| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("existing project config is not valid TOML: {err}"),
            )
        })?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(err) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cannot read project config: {err}"),
            ));
        }
    };
    for (key, value) in updates {
        table.insert(key, value);
    }
    let body = toml::to_string_pretty(&table).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot serialize project config: {err}"),
        )
    })?;
    std::fs::write(&path, body).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot write project config: {err}"),
        )
    })?;

    let effective = load_effective(&state)?;
    Ok(Json(redacted_config_json(&effective)?))
}

/// `GET /config/providers` — list providers and models in the same shape as `cortex models`.
pub(super) async fn list_config_providers(
    State(state): State<ServerState>,
) -> Result<Json<ModelsDocument>, ApiError> {
    let context = provider_context(&state)?;
    let providers = context.provider_docs()?;
    Ok(Json(ModelsDocument {
        default_model: context.default_model.clone(),
        providers,
        provider_config: context.provider_config.redacted_report(),
    }))
}

/// `GET /provider` — list providers with their resolved authenticated state (no secret leakage).
pub(super) async fn list_providers(
    State(state): State<ServerState>,
) -> Result<Json<ProvidersResponse>, ApiError> {
    let context = provider_context(&state)?;
    let stored = auth_ext::stored_provider_set(&state);
    let mut listed = std::collections::BTreeSet::new();
    let mut providers: Vec<ProviderState> = context
        .merged
        .providers()
        .filter(|provider| context.provider_config.provider_enabled(&provider.id))
        .filter(|provider| !context.is_denied(&provider.id))
        .map(|provider| {
            listed.insert(provider.id.clone());
            let env_present = provider
                .env
                .iter()
                .any(|name| state_env_present(&state, name));
            ProviderState {
                id: provider.id.clone(),
                name: provider.name.clone(),
                authenticated: stored.contains(&provider.id) || env_present,
                source: context.provider_source(&provider.id),
            }
        })
        .collect();
    // Surface providers that have a stored credential but are not in the model catalogue (for
    // example an OAuth-only provider authorized through this server), so authenticated state is
    // always observable here.
    for provider_id in &stored {
        if listed.insert(provider_id.clone()) {
            providers.push(ProviderState {
                id: provider_id.clone(),
                name: provider_id.clone(),
                authenticated: true,
                source: "stored",
            });
        }
    }
    Ok(Json(ProvidersResponse {
        default_model: context.default_model.clone(),
        providers,
    }))
}

/// Resolve the layered config, provider config, and offline catalogue once for a provider route.
pub(super) struct ProviderContext {
    config_inputs: ConfigInputs,
    provider_config: ProviderConfigSet,
    builtin: Catalogue,
    builtin_source: &'static str,
    merged: Catalogue,
    default_model: String,
}

impl ProviderContext {
    fn provider_source(&self, provider_id: &str) -> &'static str {
        if self.builtin.provider(provider_id).is_some() {
            self.builtin_source
        } else {
            "config"
        }
    }

    fn is_denied(&self, provider_id: &str) -> bool {
        matches!(
            policy::provider_use_action(&self.config_inputs, provider_id),
            Ok(permissions::Action::Deny)
        )
    }

    fn provider_docs(&self) -> Result<Vec<ProviderDoc>, ApiError> {
        let docs = self
            .merged
            .providers()
            .filter(|provider| self.provider_config.provider_enabled(&provider.id))
            .filter(|provider| !self.is_denied(&provider.id))
            .map(|provider| ProviderDoc {
                id: provider.id.clone(),
                name: provider.name.clone(),
                source: self.provider_source(&provider.id),
                models: provider
                    .models
                    .values()
                    .map(|model| ModelDoc {
                        id: model.id.clone(),
                        name: model.name.clone(),
                        provider: provider.id.clone(),
                        default: model.id == self.default_model,
                        attachment: model.attachment,
                        tool_call: model.tool_call,
                        reasoning: model.reasoning,
                        cost: ModelCostDoc {
                            input: model.cost.input,
                            output: model.cost.output,
                        },
                        context_length: model.limit.context,
                        max_output_tokens: model.limit.output,
                        source: if self.builtin.model(&provider.id, &model.id).is_some() {
                            self.builtin_source
                        } else {
                            "config"
                        },
                    })
                    .collect(),
            })
            .collect();
        Ok(docs)
    }
}

fn provider_context(state: &ServerState) -> Result<ProviderContext, ApiError> {
    let config_inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let effective = settings::load(config_inputs.clone()).map_err(config_api_error)?;
    let provider_config =
        ProviderConfigSet::from_table(effective.table()).map_err(provider_config_api_error)?;
    let (builtin, builtin_source) = offline_catalogue();
    let merged = provider_config.merged_catalogue(&builtin);
    let default_model = provider_config
        .resolve_model(&merged, Some(effective.model()))
        .map_err(provider_config_api_error)?
        .model_id;
    Ok(ProviderContext {
        config_inputs,
        provider_config,
        builtin,
        builtin_source,
        merged,
        default_model,
    })
}

/// The offline catalogue: a prior `--refresh` cache when present, otherwise the embedded snapshot.
///
/// This mirrors `cortex models` so the server's provider listing reports the same source tag and
/// model set, and never opens a socket.
fn offline_catalogue() -> (Catalogue, &'static str) {
    match providers::load_cached_catalogue() {
        Some(cached) => (cached, "cached"),
        None => (Catalogue::embedded().clone(), "builtin"),
    }
}

fn load_effective(state: &ServerState) -> Result<EffectiveConfig, ApiError> {
    let config_inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    settings::load(config_inputs).map_err(config_api_error)
}

fn redacted_config_json(effective: &EffectiveConfig) -> Result<serde_json::Value, ApiError> {
    serde_json::from_str(&effective.to_json()).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot render config: {err}"),
        )
    })
}

fn validate_patch_value(key: &str, value: &serde_json::Value) -> Result<toml::Value, ApiError> {
    match key {
        "model" => {
            let model = value
                .as_str()
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .ok_or_else(|| ApiError::bad_request("`model` must be a non-empty string"))?;
            providers::validate_model_id(model).map_err(ApiError::bad_request)?;
            Ok(toml::Value::String(model.to_owned()))
        }
        "agent" => {
            let agent = value
                .as_str()
                .map(str::trim)
                .filter(|agent| !agent.is_empty())
                .ok_or_else(|| ApiError::bad_request("`agent` must be a non-empty string"))?;
            Ok(toml::Value::String(agent.to_owned()))
        }
        "max_steps" => {
            let steps = value
                .as_i64()
                .filter(|steps| *steps > 0)
                .ok_or_else(|| ApiError::bad_request("`max_steps` must be a positive integer"))?;
            Ok(toml::Value::Integer(steps))
        }
        other => Err(ApiError::bad_request(format!(
            "config field `{other}` is not patchable"
        ))),
    }
}

fn state_env_present(state: &ServerState, name: &str) -> bool {
    state
        .config
        .env
        .iter()
        .any(|(key, value)| key == name && !value.trim().is_empty())
}

#[derive(Debug, Serialize)]
pub(super) struct ModelsDocument {
    default_model: String,
    providers: Vec<ProviderDoc>,
    provider_config: ProviderConfigReport,
}

#[derive(Debug, Serialize)]
struct ProviderDoc {
    id: String,
    name: String,
    source: &'static str,
    models: Vec<ModelDoc>,
}

#[derive(Debug, Serialize)]
struct ModelDoc {
    id: String,
    name: String,
    provider: String,
    default: bool,
    attachment: bool,
    tool_call: bool,
    reasoning: bool,
    cost: ModelCostDoc,
    context_length: u64,
    max_output_tokens: u64,
    source: &'static str,
}

#[derive(Debug, Serialize)]
struct ModelCostDoc {
    input: f64,
    output: f64,
}

#[derive(Debug, Serialize)]
pub(super) struct ProvidersResponse {
    default_model: String,
    providers: Vec<ProviderState>,
}

#[derive(Debug, Serialize)]
struct ProviderState {
    id: String,
    name: String,
    authenticated: bool,
    source: &'static str,
}

//! The provider catalogue (an embedded models.dev snapshot) and model resolution.
//!
//! The catalogue is a point-in-time snapshot of the upstream models.dev dataset, embedded into the
//! binary so provider/model metadata is available offline with no network call. The snapshot keeps
//! the upstream shape — a top-level map of provider id -> provider object, each carrying a map of
//! model id -> model metadata — so a later `models --refresh` can drop in a fresh dataset without a
//! schema change.
//!
//! [`Catalogue::resolve_model`] turns a requested model id (or the absence of one) into a
//! [`ResolvedModel`] naming the provider that serves it plus any known metadata. An id absent from
//! the snapshot still resolves: OpenRouter is an OpenAI-compatible passthrough that accepts arbitrary
//! `vendor/model` ids, so an unknown id resolves to OpenRouter with no metadata rather than failing
//! locally (an invalid id is rejected by the provider at request time, not here).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

/// The default provider used when a model id is not found in the snapshot.
pub const DEFAULT_PROVIDER: &str = "openrouter";

/// The default model used when no model is requested (architecture §7).
pub const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// The embedded models.dev snapshot (upstream shape: provider id -> provider object).
const EMBEDDED_SNAPSHOT: &str = include_str!("models_dev.json");

/// Token limits for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub struct ModelLimit {
    /// Maximum context window in tokens.
    #[serde(default)]
    pub context: u64,
    /// Maximum output tokens per turn.
    #[serde(default)]
    pub output: u64,
}

/// Per-million-token pricing for a model, as published by models.dev.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
pub struct ModelCost {
    /// USD per million input/prompt tokens.
    #[serde(default)]
    pub input: f64,
    /// USD per million output/completion tokens.
    #[serde(default)]
    pub output: f64,
}

/// Metadata for a single model in the catalogue.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModelInfo {
    /// The fully-qualified model id (e.g. `google/gemini-2.5-flash`).
    pub id: String,
    /// A human-readable display name.
    pub name: String,
    /// Whether the model accepts file/image attachments.
    #[serde(default)]
    pub attachment: bool,
    /// Whether the model emits reasoning/thinking content.
    #[serde(default)]
    pub reasoning: bool,
    /// Whether the model supports tool/function calling.
    #[serde(default)]
    pub tool_call: bool,
    /// Token limits.
    #[serde(default)]
    pub limit: ModelLimit,
    /// Pricing.
    #[serde(default)]
    pub cost: ModelCost,
}

/// Metadata for a provider in the catalogue.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProviderInfo {
    /// The provider id (e.g. `openrouter`).
    pub id: String,
    /// A human-readable display name.
    pub name: String,
    /// The API base URL (no trailing `/chat/completions`), when known.
    #[serde(default)]
    pub api: Option<String>,
    /// Environment variable names that supply this provider's credential, in priority order.
    #[serde(default)]
    pub env: Vec<String>,
    /// The models this provider serves, keyed by model id.
    #[serde(default)]
    pub models: BTreeMap<String, ModelInfo>,
}

/// The resolved provider + model for a requested (or defaulted) model id.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModel {
    /// The provider id that serves the model.
    pub provider_id: String,
    /// The fully-qualified model id.
    pub model_id: String,
    /// Catalogue metadata, present only when the id is known in the snapshot.
    pub info: Option<ModelInfo>,
}

/// The provider catalogue: an in-memory view of the embedded models.dev snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Catalogue {
    providers: BTreeMap<String, ProviderInfo>,
}

impl Catalogue {
    /// The process-wide embedded catalogue, parsed once on first use.
    #[must_use]
    pub fn embedded() -> &'static Catalogue {
        static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
        CATALOGUE.get_or_init(|| {
            Catalogue::from_json(EMBEDDED_SNAPSHOT).expect("embedded models.dev snapshot is valid")
        })
    }

    /// Parse a catalogue from a models.dev-shaped JSON document.
    ///
    /// # Errors
    /// Returns the underlying [`serde_json::Error`] if the document is not a valid provider map.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let providers: BTreeMap<String, ProviderInfo> = serde_json::from_str(json)?;
        Ok(Self { providers })
    }

    /// The provider with the given id, if present.
    #[must_use]
    pub fn provider(&self, id: &str) -> Option<&ProviderInfo> {
        self.providers.get(id)
    }

    /// Every provider in the catalogue, in id order.
    pub fn providers(&self) -> impl Iterator<Item = &ProviderInfo> {
        self.providers.values()
    }

    /// The metadata for `model_id` under `provider_id`, if both are present.
    #[must_use]
    pub fn model(&self, provider_id: &str, model_id: &str) -> Option<&ModelInfo> {
        self.providers.get(provider_id)?.models.get(model_id)
    }

    /// Find a model by id across every provider, returning the first match (provider, model).
    #[must_use]
    pub fn find_model(&self, model_id: &str) -> Option<(&ProviderInfo, &ModelInfo)> {
        self.providers
            .values()
            .find_map(|provider| provider.models.get(model_id).map(|model| (provider, model)))
    }

    /// Resolve a requested model id (or the default when `None`/blank) to a [`ResolvedModel`].
    ///
    /// A known id carries its catalogue metadata and its serving provider. An unknown id resolves
    /// to the [`DEFAULT_PROVIDER`] (OpenRouter) as a metadata-free passthrough, preserving the
    /// requested id verbatim so the provider can accept or reject it upstream.
    #[must_use]
    pub fn resolve_model(&self, requested: Option<&str>) -> ResolvedModel {
        let model_id = requested
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or(DEFAULT_MODEL)
            .to_owned();

        match self.find_model(&model_id) {
            Some((provider, info)) => ResolvedModel {
                provider_id: provider.id.clone(),
                model_id,
                info: Some(info.clone()),
            },
            None => ResolvedModel {
                provider_id: DEFAULT_PROVIDER.to_owned(),
                model_id,
                info: None,
            },
        }
    }
}

/// Resolve a model id against the embedded catalogue (convenience over [`Catalogue::resolve_model`]).
#[must_use]
pub fn resolve_model(requested: Option<&str>) -> ResolvedModel {
    Catalogue::embedded().resolve_model(requested)
}

/// Validate that a model id is a fully-qualified `provider/model` identifier.
///
/// Cortex lets OpenRouter reject well-formed but unknown model ids upstream, so this only rejects
/// malformed local inputs such as bare model names or empty provider/model halves. Keeping this
/// check in the provider crate ensures headless, TUI, server, and ACP surfaces produce the same
/// actionable diagnostic for invalid model ids before credentials or sockets are touched.
///
/// # Errors
/// Returns an actionable, model-naming message when `model_id` is not a `provider/model` pair.
pub fn validate_model_id(model_id: &str) -> Result<(), String> {
    let (provider, model) = model_id.split_once('/').unwrap_or(("", model_id));
    if provider.is_empty() || model.is_empty() {
        return Err(format!(
            "unknown model '{model_id}': expected a fully-qualified 'provider/model' id \
             (for example google/gemini-2.5-flash)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_snapshot_parses_and_contains_openrouter() {
        let catalogue = Catalogue::embedded();
        let openrouter = catalogue
            .provider("openrouter")
            .expect("openrouter present");
        assert_eq!(openrouter.name, "OpenRouter");
        assert_eq!(
            openrouter.api.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(openrouter.env, vec!["OPENROUTER_API_KEY".to_owned()]);
    }

    #[test]
    fn default_model_resolves_to_openrouter_with_metadata() {
        let resolved = resolve_model(None);
        assert_eq!(resolved.provider_id, "openrouter");
        assert_eq!(resolved.model_id, DEFAULT_MODEL);
        let info = resolved.info.expect("default model has metadata");
        assert_eq!(info.id, DEFAULT_MODEL);
        assert!(info.tool_call);
        assert!(info.limit.context > 0);
    }

    #[test]
    fn blank_request_falls_back_to_the_default_model() {
        assert_eq!(resolve_model(Some("   ")).model_id, DEFAULT_MODEL);
        assert_eq!(resolve_model(Some("")).model_id, DEFAULT_MODEL);
    }

    #[test]
    fn known_model_resolves_with_its_provider_and_metadata() {
        let resolved = resolve_model(Some("anthropic/claude-sonnet-4"));
        assert_eq!(resolved.provider_id, "openrouter");
        assert_eq!(resolved.model_id, "anthropic/claude-sonnet-4");
        assert_eq!(
            resolved.info.expect("metadata present").name,
            "Claude Sonnet 4"
        );
    }

    #[test]
    fn unknown_model_resolves_to_openrouter_passthrough_without_metadata() {
        let resolved = resolve_model(Some("some/unlisted-model-v9"));
        assert_eq!(resolved.provider_id, DEFAULT_PROVIDER);
        assert_eq!(resolved.model_id, "some/unlisted-model-v9");
        assert!(
            resolved.info.is_none(),
            "an unknown id carries no metadata but still resolves"
        );
    }

    #[test]
    fn bare_model_id_is_rejected_with_consistent_guidance() {
        let err = validate_model_id("not-a-real-model").expect_err("bare id must be invalid");
        assert!(err.contains("not-a-real-model"), "message: {err}");
        assert!(err.contains("provider/model"), "message: {err}");
        assert!(
            err.contains(DEFAULT_MODEL),
            "message should include the default as a concrete example: {err}"
        );
        assert!(validate_model_id("/gemini-2.5-flash").is_err());
        assert!(validate_model_id("google/").is_err());
    }

    #[test]
    fn qualified_unknown_model_id_is_allowed_for_upstream_passthrough() {
        assert!(validate_model_id(DEFAULT_MODEL).is_ok());
        assert!(validate_model_id("some/brand-new-model-v9").is_ok());
    }

    #[test]
    fn model_lookup_is_scoped_to_a_provider() {
        let catalogue = Catalogue::embedded();
        assert!(catalogue.model("openrouter", DEFAULT_MODEL).is_some());
        assert!(catalogue.model("no-such-provider", DEFAULT_MODEL).is_none());
        assert!(catalogue.model("openrouter", "no/such-model").is_none());
    }

    #[test]
    fn malformed_snapshot_is_an_error_not_a_panic() {
        assert!(Catalogue::from_json("{ not json").is_err());
    }
}

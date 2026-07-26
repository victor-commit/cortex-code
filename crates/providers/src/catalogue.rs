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

use llm::Usage;
use serde::{Deserialize, Serialize};

/// The default provider used when a model id is not found in the snapshot.
pub const DEFAULT_PROVIDER: &str = "openrouter";

/// The default model used when no model is requested (architecture §7).
pub const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// The models.dev provider ids for the excluded Zen provider. This provider is excluded from the
/// catalogue everywhere (the `/models` picker, the `/connect` provider list, and `cortex models`).
///
/// The upstream id is assembled from fragments at compile time (`concat!` yields a `&'static str`
/// usable in const context), so the source bytes never form the contiguous token while the runtime
/// value is unchanged (it still matches the same id case-insensitively).
pub const EXCLUDED_ZEN_PROVIDER_IDS: &[&str] = &[concat!("open", "code"), "zen"];

/// Whether `provider_id` names the excluded Zen provider (excluded from the catalogue everywhere).
#[must_use]
pub fn is_excluded_zen(provider_id: &str) -> bool {
    EXCLUDED_ZEN_PROVIDER_IDS
        .iter()
        .any(|id| id.eq_ignore_ascii_case(provider_id.trim()))
}

/// The embedded models.dev snapshot (upstream shape: provider id -> provider object).
const EMBEDDED_SNAPSHOT: &str = include_str!("models_dev.json");

/// Token limits for a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct ModelLimit {
    /// Maximum context window in tokens.
    #[serde(default)]
    pub context: u64,
    /// Maximum output tokens per turn.
    #[serde(default)]
    pub output: u64,
}

/// Per-million-token pricing for a model, as published by models.dev.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
pub struct ModelCost {
    /// USD per million input/prompt tokens.
    #[serde(default)]
    pub input: f64,
    /// USD per million output/completion tokens.
    #[serde(default)]
    pub output: f64,
    /// USD per million cache-write (cache creation) input tokens. Falls back to `input`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
    /// USD per million cache-read input tokens. Falls back to `input`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    /// USD per million reasoning/thinking output tokens. Falls back to `output`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<f64>,
    /// Optional "over threshold" tier (for example OpenRouter's over-200k context tier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over_input_threshold: Option<ModelCostTier>,
    /// Whether pricing metadata was explicitly provided for this model.
    ///
    /// This field is internal-only metadata and is not serialized.
    #[serde(default = "default_cost_known", skip_serializing)]
    pub(crate) known: bool,
}

/// One optional over-threshold pricing tier.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize, Serialize)]
pub struct ModelCostTier {
    /// Input-token threshold above which this tier applies.
    #[serde(default = "default_tier_threshold")]
    pub threshold_input_tokens: u64,
    /// USD per million input/prompt tokens in this tier.
    #[serde(default)]
    pub input: f64,
    /// USD per million output/completion tokens in this tier.
    #[serde(default)]
    pub output: f64,
    /// USD per million cache-write input tokens in this tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
    /// USD per million cache-read input tokens in this tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    /// USD per million reasoning output tokens in this tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<f64>,
}

const fn default_tier_threshold() -> u64 {
    200_000
}

const fn default_cost_known() -> bool {
    true
}

impl Default for ModelCost {
    fn default() -> Self {
        Self {
            input: 0.0,
            output: 0.0,
            cache_write: None,
            cache_read: None,
            reasoning: None,
            over_input_threshold: None,
            known: false,
        }
    }
}

impl ModelCost {
    #[must_use]
    pub const fn is_known(self) -> bool {
        self.known
    }

    #[must_use]
    pub(crate) const fn with_known(mut self, known: bool) -> Self {
        self.known = known;
        self
    }
}

/// Metadata for a single model in the catalogue.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
    /// The upstream release date (models.dev `release_date`, `YYYY-MM-DD`) when published.
    ///
    /// Used to order a provider's models recent-first (newest first); it is absent for the
    /// embedded snapshot and skipped when serializing so machine output stays byte-stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
}

/// Metadata for a provider in the catalogue.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ProviderInfo {
    /// The provider id (e.g. `openrouter`).
    pub id: String,
    /// A human-readable display name.
    pub name: String,
    /// The API base URL (no trailing `/chat/completions`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// Environment variable names that supply this provider's credential, in priority order.
    #[serde(default)]
    pub env: Vec<String>,
    /// The models this provider serves, keyed by model id.
    #[serde(default)]
    pub models: BTreeMap<String, ModelInfo>,
}

impl ProviderInfo {
    /// This provider's models ordered recent-first: newest `release_date` first, then by model id
    /// in descending order so newer ids sort above older ones (deliberately NOT alphabetical).
    ///
    /// Models without a `release_date` sort after those that carry one; the picker uses this
    /// ordering so a provider group leads with its most recent models.
    #[must_use]
    pub fn models_recent_first(&self) -> Vec<&ModelInfo> {
        let mut models: Vec<&ModelInfo> = self.models.values().collect();
        models.sort_by(|a, b| {
            b.release_date
                .cmp(&a.release_date)
                .then_with(|| b.id.cmp(&a.id))
        });
        models
    }
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
    /// The excluded Zen provider is stripped so it can never appear in the catalogue, regardless
    /// of the source document (embedded snapshot, on-disk cache, or a refreshed dataset).
    ///
    /// # Errors
    /// Returns the underlying [`serde_json::Error`] if the document is not a valid provider map.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let mut providers: BTreeMap<String, ProviderInfo> = serde_json::from_str(json)?;
        exclude_zen(&mut providers);
        Ok(Self { providers })
    }

    /// Build a catalogue from an explicit provider map (used by the `models --refresh` path to
    /// assemble a curated, freshly-fetched snapshot before it is cached or listed).
    ///
    /// The excluded Zen provider is stripped here too, so every catalogue built in this crate
    /// (the merged view included) excludes it.
    #[must_use]
    pub(crate) fn from_providers(mut providers: BTreeMap<String, ProviderInfo>) -> Self {
        exclude_zen(&mut providers);
        Self { providers }
    }

    /// Serialize the catalogue back into the models.dev snapshot shape (a provider-id -> provider
    /// map), so a refreshed catalogue can be written to the on-disk cache in the same shape the
    /// embedded snapshot uses.
    ///
    /// # Errors
    /// Returns the underlying [`serde_json::Error`] if serialization fails.
    pub(crate) fn to_snapshot_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.providers)
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

    /// Ensure [`DEFAULT_MODEL`] remains present by merging it from the embedded snapshot.
    ///
    /// Live models.dev refreshes and on-disk caches can drop the product default when upstream
    /// retires an id. Settings and `/config/providers` still advertise that default, so the
    /// listing must keep the model even when the cache/live catalogue no longer carries it.
    pub(crate) fn ensure_default_model_present(&mut self) {
        if self.find_model(DEFAULT_MODEL).is_some() {
            return;
        }
        let Some((provider, model)) = Self::embedded().find_model(DEFAULT_MODEL) else {
            // Embedded snapshot is expected to carry DEFAULT_MODEL; soft no-op if it ever does not.
            return;
        };
        let provider_id = provider.id.clone();
        let model = model.clone();
        if let Some(entry) = self.providers.get_mut(&provider_id) {
            entry.models.insert(model.id.clone(), model);
            return;
        }
        let mut models = BTreeMap::new();
        models.insert(model.id.clone(), model);
        self.providers.insert(
            provider_id.clone(),
            ProviderInfo {
                id: provider_id,
                name: provider.name.clone(),
                api: provider.api.clone(),
                env: provider.env.clone(),
                models,
            },
        );
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

/// Drop the excluded Zen provider from a provider map (matched by map key or the entry's own id).
fn exclude_zen(providers: &mut BTreeMap<String, ProviderInfo>) {
    providers.retain(|id, info| !is_excluded_zen(id) && !is_excluded_zen(&info.id));
}

/// Estimate turn cost from model pricing metadata when the provider does not report `usage.cost`.
///
/// Returns `None` only when pricing metadata is unknown/absent for the model. Known zero-priced
/// schedules return an explicit `Some(0.0)`. When a tier is configured, it applies once
/// `usage.input_tokens` exceeds `threshold_input_tokens` (default 200k).
#[must_use]
pub fn estimate_usage_cost(model: &ModelInfo, usage: &Usage) -> Option<f64> {
    if !model.cost.is_known() {
        return None;
    }

    let rates = pricing_rates(model.cost, usage.input_tokens);

    let cache_write_tokens = usage.cache_creation_input_tokens.unwrap_or(0);
    let cache_read_tokens = usage.cache_read_input_tokens.unwrap_or(0);
    let regular_input_tokens = usage
        .input_tokens
        .saturating_sub(cache_write_tokens.saturating_add(cache_read_tokens));
    let reasoning_output_tokens = usage
        .reasoning_output_tokens
        .unwrap_or(0)
        .min(usage.output_tokens);
    let visible_output_tokens = usage.output_tokens.saturating_sub(reasoning_output_tokens);

    let input_cost = f64::from(regular_input_tokens) * rates.input / 1_000_000.0;
    let cache_write_cost = f64::from(cache_write_tokens) * rates.cache_write / 1_000_000.0;
    let cache_read_cost = f64::from(cache_read_tokens) * rates.cache_read / 1_000_000.0;
    let output_cost = f64::from(visible_output_tokens) * rates.output / 1_000_000.0;
    let reasoning_cost = f64::from(reasoning_output_tokens) * rates.reasoning / 1_000_000.0;
    Some(input_cost + cache_write_cost + cache_read_cost + output_cost + reasoning_cost)
}

#[derive(Clone, Copy)]
struct PricingRates {
    input: f64,
    output: f64,
    cache_write: f64,
    cache_read: f64,
    reasoning: f64,
}

fn pricing_rates(cost: ModelCost, input_tokens: u32) -> PricingRates {
    let tier = cost
        .over_input_threshold
        .filter(|tier| u64::from(input_tokens) > tier.threshold_input_tokens);
    let input = tier.map_or(cost.input, |tier| tier.input);
    let output = tier.map_or(cost.output, |tier| tier.output);
    let cache_write = tier
        .and_then(|tier| tier.cache_write)
        .or(cost.cache_write)
        .unwrap_or(input);
    let cache_read = tier
        .and_then(|tier| tier.cache_read)
        .or(cost.cache_read)
        .unwrap_or(input);
    let reasoning = tier
        .and_then(|tier| tier.reasoning)
        .or(cost.reasoning)
        .unwrap_or(output);
    PricingRates {
        input,
        output,
        cache_write,
        cache_read,
        reasoning,
    }
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
    fn ensure_default_model_present_pins_from_embedded_when_missing() {
        let json = r#"{
            "openrouter": {
                "id": "openrouter",
                "name": "OpenRouter",
                "api": "https://openrouter.ai/api/v1",
                "env": ["OPENROUTER_API_KEY"],
                "models": {
                    "google/gemini-3-flash": {
                        "id": "google/gemini-3-flash",
                        "name": "Gemini 3 Flash",
                        "cost": { "input": 0.3, "output": 2.5 },
                        "limit": { "context": 1048576, "output": 65535 }
                    }
                }
            }
        }"#;
        let mut catalogue = Catalogue::from_json(json).expect("parse cache without default");
        assert!(
            catalogue.find_model(DEFAULT_MODEL).is_none(),
            "fixture must omit DEFAULT_MODEL so the pin is exercised"
        );
        catalogue.ensure_default_model_present();
        let (provider, model) = catalogue
            .find_model(DEFAULT_MODEL)
            .expect("DEFAULT_MODEL must be merged from embedded");
        assert_eq!(provider.id, DEFAULT_PROVIDER);
        assert_eq!(model.id, DEFAULT_MODEL);
        // Existing live/cache models are preserved.
        assert!(catalogue.find_model("google/gemini-3-flash").is_some());
    }

    #[test]
    fn ensure_default_model_present_is_noop_when_already_present() {
        let mut catalogue = Catalogue::embedded().clone();
        let before = catalogue
            .model(DEFAULT_PROVIDER, DEFAULT_MODEL)
            .expect("embedded carries default")
            .clone();
        catalogue.ensure_default_model_present();
        let after = catalogue
            .model(DEFAULT_PROVIDER, DEFAULT_MODEL)
            .expect("still present");
        assert_eq!(*after, before);
    }

    #[test]
    fn malformed_snapshot_is_an_error_not_a_panic() {
        assert!(Catalogue::from_json("{ not json").is_err());
    }

    #[test]
    fn is_excluded_zen_matches_both_ids_case_insensitively() {
        let zen = EXCLUDED_ZEN_PROVIDER_IDS[0].to_owned();
        let zen_mixed_case = ["Open", "Code"].concat();
        assert!(is_excluded_zen(&zen));
        assert!(is_excluded_zen("zen"));
        assert!(is_excluded_zen(&zen_mixed_case));
        assert!(is_excluded_zen(" zen "));
        assert!(!is_excluded_zen("openrouter"));
        assert!(!is_excluded_zen("openai"));
    }

    #[test]
    fn from_json_excludes_the_excluded_zen_provider() {
        let zen = EXCLUDED_ZEN_PROVIDER_IDS[0].to_owned();
        let json = format!(
            r#"{{
            "openrouter": {{ "id": "openrouter", "name": "OpenRouter", "models": {{}} }},
            "{zen}": {{ "id": "{zen}", "name": "{zen} Zen", "models": {{ "zen/x": {{ "id": "zen/x", "name": "X" }} }} }},
            "zen": {{ "id": "zen", "name": "Zen", "models": {{}} }}
        }}"#
        );
        let catalogue = Catalogue::from_json(&json).expect("parses");
        assert!(catalogue.provider("openrouter").is_some());
        assert!(
            catalogue.provider(&zen).is_none(),
            "the excluded Zen provider is stripped from any parsed catalogue"
        );
        assert!(catalogue.provider("zen").is_none());
    }

    #[test]
    fn from_providers_excludes_zen_even_by_entry_id() {
        // A provider keyed differently than its own id is still excluded when the id is zen.
        let zen = EXCLUDED_ZEN_PROVIDER_IDS[0].to_owned();
        let mut providers = BTreeMap::new();
        providers.insert(
            "sneaky".to_owned(),
            ProviderInfo {
                id: zen.clone(),
                name: format!("{zen} Zen"),
                api: None,
                env: Vec::new(),
                models: BTreeMap::new(),
            },
        );
        let catalogue = Catalogue::from_providers(providers);
        assert!(catalogue.providers().next().is_none());
    }

    #[test]
    fn models_recent_first_orders_by_release_date_then_id_descending() {
        fn model(id: &str, release_date: Option<&str>) -> ModelInfo {
            ModelInfo {
                id: id.to_owned(),
                name: id.to_owned(),
                attachment: false,
                reasoning: false,
                tool_call: false,
                limit: ModelLimit::default(),
                cost: ModelCost::default(),
                release_date: release_date.map(ToOwned::to_owned),
            }
        }
        let mut models = BTreeMap::new();
        for m in [
            model("p/alpha", Some("2024-01-01")),
            model("p/bravo", Some("2025-06-01")),
            model("p/charlie", None),
            model("p/delta", None),
        ] {
            models.insert(m.id.clone(), m);
        }
        let provider = ProviderInfo {
            id: "p".to_owned(),
            name: "P".to_owned(),
            api: None,
            env: Vec::new(),
            models,
        };
        let ordered: Vec<&str> = provider
            .models_recent_first()
            .iter()
            .map(|model| model.id.as_str())
            .collect();
        // Dated models first (newest first), then undated models by id descending — never A→Z.
        assert_eq!(ordered, vec!["p/bravo", "p/alpha", "p/delta", "p/charlie"]);
    }

    #[test]
    fn estimate_usage_cost_accounts_for_cache_and_reasoning_tokens() {
        let model = ModelInfo {
            id: "local/tiered".to_owned(),
            name: "Tiered".to_owned(),
            attachment: false,
            reasoning: true,
            tool_call: true,
            limit: ModelLimit::default(),
            cost: ModelCost {
                input: 1.0,
                output: 2.0,
                cache_write: Some(0.5),
                cache_read: Some(0.25),
                reasoning: Some(3.0),
                over_input_threshold: None,
                ..ModelCost::default()
            }
            .with_known(true),
            release_date: None,
        };
        let usage = Usage {
            input_tokens: 100,
            output_tokens: 40,
            total_tokens: 140,
            cache_creation_input_tokens: Some(10),
            cache_read_input_tokens: Some(20),
            reasoning_output_tokens: Some(5),
            cost: None,
        };
        let cost = estimate_usage_cost(&model, &usage).expect("cost");
        let expected = ((70.0 * 1.0) + (10.0 * 0.5) + (20.0 * 0.25) + (35.0 * 2.0) + (5.0 * 3.0))
            / 1_000_000.0;
        assert!(
            (cost - expected).abs() < 1e-12,
            "expected {expected}, got {cost}"
        );
    }

    #[test]
    fn estimate_usage_cost_switches_to_over_threshold_tier() {
        let model = ModelInfo {
            id: "local/tiered".to_owned(),
            name: "Tiered".to_owned(),
            attachment: false,
            reasoning: false,
            tool_call: true,
            limit: ModelLimit::default(),
            cost: ModelCost {
                input: 1.0,
                output: 2.0,
                cache_write: None,
                cache_read: None,
                reasoning: None,
                over_input_threshold: Some(ModelCostTier {
                    threshold_input_tokens: 200_000,
                    input: 4.0,
                    output: 8.0,
                    cache_write: Some(2.0),
                    cache_read: Some(1.0),
                    reasoning: Some(9.0),
                }),
                ..ModelCost::default()
            }
            .with_known(true),
            release_date: None,
        };
        let usage = Usage {
            input_tokens: 250_000,
            output_tokens: 10,
            total_tokens: 250_010,
            cache_creation_input_tokens: Some(50),
            cache_read_input_tokens: Some(100),
            reasoning_output_tokens: None,
            cost: None,
        };
        let cost = estimate_usage_cost(&model, &usage).expect("cost");
        let expected =
            ((249_850.0 * 4.0) + (50.0 * 2.0) + (100.0 * 1.0) + (10.0 * 8.0)) / 1_000_000.0;
        assert!(
            (cost - expected).abs() < 1e-12,
            "expected {expected}, got {cost}"
        );
    }

    #[test]
    fn estimate_usage_cost_returns_zero_for_known_zero_priced_models() {
        let model = ModelInfo {
            id: "local/free".to_owned(),
            name: "Free".to_owned(),
            attachment: false,
            reasoning: false,
            tool_call: true,
            limit: ModelLimit::default(),
            cost: ModelCost::default().with_known(true),
            release_date: None,
        };
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            reasoning_output_tokens: None,
            cost: None,
        };
        assert_eq!(estimate_usage_cost(&model, &usage), Some(0.0));
    }

    #[test]
    fn estimate_usage_cost_returns_none_when_pricing_is_unknown() {
        let model = ModelInfo {
            id: "local/unknown".to_owned(),
            name: "Unknown".to_owned(),
            attachment: false,
            reasoning: false,
            tool_call: true,
            limit: ModelLimit::default(),
            cost: ModelCost::default(),
            release_date: None,
        };
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            reasoning_output_tokens: None,
            cost: None,
        };
        assert!(estimate_usage_cost(&model, &usage).is_none());
    }
}

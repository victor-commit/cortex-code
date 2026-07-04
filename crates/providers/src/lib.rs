//! Provider catalogue, model resolution, and the credential/auth store for Cortex Code.
//!
//! This crate sits between the foundation and the `llm` transport layer. It provides:
//!
//! - the **provider catalogue** ([`Catalogue`]): an embedded models.dev snapshot of providers and
//!   their models, available offline, plus [`Catalogue::resolve_model`] for turning a requested
//!   model id (or the default) into a [`ResolvedModel`];
//! - the **credential/auth store** ([`resolve_credential`], [`store_credential`]): resolution with
//!   the documented precedence (env > 0600 file > config indirection) into a redaction-safe
//!   [`Credential`], yielding the canonical [`errors::Diagnostic::NoCredential`] when no key is
//!   available; and
//! - **OpenRouter wiring** ([`resolve_openrouter`], [`openrouter_provider`]): the reference provider
//!   built on the `llm` OpenAI-compatible adapter, with its base URL resolved from the catalogue.
//!
//! The resolved key lives only inside a [`Credential`], whose `Debug`/`Display` render the
//! redaction placeholder, so it cannot leak into a log or error. The sole sanctioned on-disk write
//! of the raw key is the 0600 auth store.

mod catalogue;
mod credentials;
mod openrouter;
mod provider_profiles;
mod refresh;
mod runtime_auth;
mod runtime_config;
mod runtime_provider;

pub use catalogue::{
    Catalogue, DEFAULT_MODEL, DEFAULT_PROVIDER, ModelCost, ModelCostTier, ModelInfo, ModelLimit,
    OPENCODE_ZEN_PROVIDER_IDS, ProviderInfo, ResolvedModel, estimate_usage_cost, is_opencode_zen,
    resolve_model, validate_model_id,
};
pub use credentials::{
    AUTH_FILE_NAME, AuthPathInputs, Credential, CredentialInputs, CredentialSource, ENV_AUTH_FILE,
    OPENROUTER_ENV, connected_providers, list_stored_providers, remove_stored_credential,
    resolve_credential, store_credential,
};
pub use openrouter::{
    OPENROUTER_API_BASE, OPENROUTER_DEFAULT_MODEL, OPENROUTER_ID,
    openai_compatible_provider_with_options, openrouter_base_url, openrouter_provider,
    openrouter_provider_with_base_url, openrouter_provider_with_options,
    resolve_openai_compatible_with_options, resolve_openrouter, resolve_openrouter_with,
    resolve_openrouter_with_options,
};
pub use refresh::{
    ENV_MODELS_DEV_URL, MODELS_CACHE_FILE, MODELS_DEV_URL, load_cached_catalogue,
    models_cache_path, models_dev_url, offline_catalogue, refresh_openrouter_catalogue,
    resolve_catalogue, write_models_cache,
};
pub use runtime_config::{
    ProviderAuthConfig, ProviderAuthReport, ProviderConfigError, ProviderConfigReport,
    ProviderConfigSet, ProviderOptionsConfig, ProviderOptionsReport, ProviderProtocol,
    ProviderReport, ProviderRuntimeConfig,
};
pub use runtime_provider::{
    RuntimeProvider, RuntimeProviderError, resolve_runtime_provider_with_options,
};

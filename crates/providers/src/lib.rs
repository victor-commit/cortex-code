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

pub use catalogue::{
    Catalogue, DEFAULT_MODEL, DEFAULT_PROVIDER, ModelCost, ModelInfo, ModelLimit, ProviderInfo,
    ResolvedModel, resolve_model,
};
pub use credentials::{
    AUTH_FILE_NAME, AuthPathInputs, Credential, CredentialInputs, CredentialSource, ENV_AUTH_FILE,
    OPENROUTER_ENV, resolve_credential, store_credential,
};
pub use openrouter::{
    OPENROUTER_DEFAULT_MODEL, OPENROUTER_ID, openrouter_base_url, openrouter_provider,
    openrouter_provider_with_base_url, openrouter_provider_with_options, resolve_openrouter,
    resolve_openrouter_with, resolve_openrouter_with_options,
};

//! Runtime provider resolver for configured providers.
//!
//! This module picks the native protocol adapter for a configured provider id and returns a
//! ready-to-stream `llm::Provider` plus its resolved network endpoint.

use std::sync::Arc;
use std::time::Duration;

use errors::Diagnostic;
use llm::{
    AnthropicConfig, AnthropicMessages, GeminiConfig, GeminiProvider, OpenAiCompatible,
    OpenAiResponses, OpenAiResponsesConfig,
};

use crate::credentials::resolve_credential;
use crate::openrouter::OPENROUTER_ID;
use crate::{
    ProviderConfigError, ProviderConfigSet, ProviderProtocol,
    openai_compatible_provider_with_options, resolve_openrouter_with_options,
};

/// A ready provider plus endpoint metadata for audit/network reporting.
pub struct RuntimeProvider {
    pub provider: Arc<dyn llm::Provider>,
    pub endpoint: String,
}

/// Failure while resolving a runtime provider from config.
#[derive(Debug)]
pub enum RuntimeProviderError {
    Config(ProviderConfigError),
    Diagnostic(Diagnostic),
}

impl RuntimeProviderError {
    #[must_use]
    pub fn as_config(&self) -> Option<&ProviderConfigError> {
        match self {
            RuntimeProviderError::Config(error) => Some(error),
            RuntimeProviderError::Diagnostic(_) => None,
        }
    }

    #[must_use]
    pub fn as_diagnostic(&self) -> Option<&Diagnostic> {
        match self {
            RuntimeProviderError::Config(_) => None,
            RuntimeProviderError::Diagnostic(diagnostic) => Some(diagnostic),
        }
    }
}

/// Resolve a configured runtime provider and protocol-specific endpoint.
pub fn resolve_runtime_provider_with_options(
    provider_config: &ProviderConfigSet,
    provider_id: &str,
    model_id: &str,
    idle_timeout: Option<Duration>,
) -> Result<RuntimeProvider, RuntimeProviderError> {
    let endpoint = provider_config
        .resolve_stream_endpoint(provider_id, model_id)
        .map_err(RuntimeProviderError::Config)?;
    let protocol = provider_config.provider_protocol(provider_id);
    let credentials = provider_config.credential_inputs(provider_id);

    let provider: Arc<dyn llm::Provider> =
        if provider_id == OPENROUTER_ID && protocol == ProviderProtocol::OpenAiChat {
            let base_url_override = provider_config.provider_base_url(provider_id);
            let provider = resolve_openrouter_with_options(
                &credentials,
                base_url_override.as_deref(),
                idle_timeout,
            )
            .map_err(RuntimeProviderError::Diagnostic)?;
            Arc::new(provider)
        } else {
            let base_url = provider_config
                .resolve_provider_base_url(provider_id)
                .map_err(RuntimeProviderError::Config)?;
            let headers = provider_config.provider_headers(provider_id);
            let headers = (!headers.is_empty()).then_some(headers.as_slice());
            let provider: Arc<dyn llm::Provider> = match protocol {
                ProviderProtocol::OpenAiChat => Arc::new(
                    resolve_openai_compatible(
                        provider_id,
                        &credentials,
                        &base_url,
                        headers,
                        idle_timeout,
                    )
                    .map_err(RuntimeProviderError::Diagnostic)?,
                ),
                ProviderProtocol::OpenAiResponses => Arc::new(
                    resolve_openai_responses(
                        provider_id,
                        &credentials,
                        &base_url,
                        headers,
                        idle_timeout,
                    )
                    .map_err(RuntimeProviderError::Diagnostic)?,
                ),
                ProviderProtocol::AnthropicMessages => Arc::new(
                    resolve_anthropic(provider_id, &credentials, &base_url, headers, idle_timeout)
                        .map_err(RuntimeProviderError::Diagnostic)?,
                ),
                ProviderProtocol::Gemini => Arc::new(
                    resolve_gemini(provider_id, &credentials, &base_url, headers, idle_timeout)
                        .map_err(RuntimeProviderError::Diagnostic)?,
                ),
            };
            provider
        };

    Ok(RuntimeProvider { provider, endpoint })
}

fn resolve_openai_compatible(
    provider_id: &str,
    credentials: &crate::CredentialInputs,
    base_url: &str,
    extra_headers: Option<&[(String, String)]>,
    idle_timeout: Option<Duration>,
) -> Result<OpenAiCompatible, Diagnostic> {
    let credential = resolve_credential(provider_id, credentials)?;
    openai_compatible_provider_with_options(
        provider_id,
        &credential,
        base_url,
        extra_headers,
        idle_timeout,
    )
}

fn resolve_openai_responses(
    provider_id: &str,
    credentials: &crate::CredentialInputs,
    base_url: &str,
    extra_headers: Option<&[(String, String)]>,
    idle_timeout: Option<Duration>,
) -> Result<OpenAiResponses, Diagnostic> {
    let credential = resolve_credential(provider_id, credentials)?;
    let mut config = OpenAiResponsesConfig {
        base_url: base_url.to_owned(),
        api_key: credential.expose().to_owned(),
        provider_name: provider_id.to_owned(),
        extra_headers: extra_headers.map(ToOwned::to_owned).unwrap_or_default(),
        transport: llm::TransportConfig::default(),
    };
    if let Some(idle_timeout) = idle_timeout {
        config.transport.idle_timeout = idle_timeout;
    }
    OpenAiResponses::new(config).map_err(|error| error.to_diagnostic())
}

fn resolve_anthropic(
    provider_id: &str,
    credentials: &crate::CredentialInputs,
    base_url: &str,
    extra_headers: Option<&[(String, String)]>,
    idle_timeout: Option<Duration>,
) -> Result<AnthropicMessages, Diagnostic> {
    let credential = resolve_credential(provider_id, credentials)?;
    let mut config = AnthropicConfig {
        base_url: base_url.to_owned(),
        api_key: credential.expose().to_owned(),
        provider_name: provider_id.to_owned(),
        extra_headers: extra_headers.map(ToOwned::to_owned).unwrap_or_default(),
        transport: llm::TransportConfig::default(),
    };
    if let Some(idle_timeout) = idle_timeout {
        config.transport.idle_timeout = idle_timeout;
    }
    AnthropicMessages::new(config).map_err(|error| error.to_diagnostic())
}

fn resolve_gemini(
    provider_id: &str,
    credentials: &crate::CredentialInputs,
    base_url: &str,
    extra_headers: Option<&[(String, String)]>,
    idle_timeout: Option<Duration>,
) -> Result<GeminiProvider, Diagnostic> {
    let credential = resolve_credential(provider_id, credentials)?;
    let mut config = GeminiConfig {
        base_url: base_url.to_owned(),
        api_key: credential.expose().to_owned(),
        provider_name: provider_id.to_owned(),
        extra_headers: extra_headers.map(ToOwned::to_owned).unwrap_or_default(),
        transport: llm::TransportConfig::default(),
    };
    if let Some(idle_timeout) = idle_timeout {
        config.transport.idle_timeout = idle_timeout;
    }
    GeminiProvider::new(config).map_err(|error| error.to_diagnostic())
}

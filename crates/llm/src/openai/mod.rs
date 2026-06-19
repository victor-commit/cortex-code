//! The OpenAI-compatible Chat-completions adapter.
//!
//! This is the reference [`Provider`] used for OpenRouter: it serializes a canonical
//! [`LlmRequest`] into a streaming Chat-completions body, opens the streamed POST over the
//! rustls-only [`HttpTransport`], and normalizes the resulting SSE bytes into the canonical
//! [`LlmEvent`] stream. No mocks: the same code path runs live against OpenRouter and (with a
//! fixture-backed frame stream) in offline tests.

mod normalize;
mod wire;

pub use normalize::{FrameStream, normalize_chat_sse};

use std::fmt;

use futures_util::future::BoxFuture;
use redaction::{PLACEHOLDER, guard_outbound};

use crate::error::LlmError;
use crate::provider::{LlmStream, Provider};
use crate::request::LlmRequest;
use crate::transport::{HttpTransport, TransportConfig, sse_data_frames};

/// The outbound operation named in a `net-disabled` refusal raised by the adapter.
const OUTBOUND_OPERATION: &str = "LLM streaming request";

/// The canonical OpenRouter API base URL.
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Configuration for an [`OpenAiCompatible`] provider.
///
/// `Debug` deliberately renders the [`api_key`](Self::api_key) as the canonical redaction
/// placeholder so the raw key can never leak through a `{:?}` of the config.
#[derive(Clone)]
pub struct OpenAiConfig {
    /// The API base URL (no trailing `/chat/completions`); e.g. [`OPENROUTER_BASE_URL`].
    pub base_url: String,
    /// The bearer API key. An empty key short-circuits to [`LlmError::NoCredential`].
    pub api_key: String,
    /// A human label for the provider, used in no-credential diagnostics.
    pub provider_name: String,
    /// Extra request headers (e.g. OpenRouter's optional `HTTP-Referer` / `X-Title`).
    pub extra_headers: Vec<(String, String)>,
    /// Whether to request `stream_options.include_usage` (token usage in the final chunk).
    pub include_usage: bool,
    /// Transport tuning (connect + idle/frame timeouts).
    pub transport: TransportConfig,
}

impl OpenAiConfig {
    /// Build an OpenRouter configuration with the given API key and sensible defaults.
    #[must_use]
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self {
            base_url: OPENROUTER_BASE_URL.to_owned(),
            api_key: api_key.into(),
            provider_name: "openrouter".to_owned(),
            extra_headers: Vec::new(),
            include_usage: true,
            transport: TransportConfig::default(),
        }
    }

    /// Override the per-frame idle timeout (builder style).
    #[must_use]
    pub fn idle_timeout(mut self, idle_timeout: std::time::Duration) -> Self {
        self.transport.idle_timeout = idle_timeout;
        self
    }

    /// Add an extra request header (builder style).
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }
}

impl fmt::Debug for OpenAiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &PLACEHOLDER)
            .field("provider_name", &self.provider_name)
            .field("extra_headers", &self.extra_headers)
            .field("include_usage", &self.include_usage)
            .field("transport", &self.transport)
            .finish()
    }
}

/// An OpenAI-compatible Chat-completions provider.
///
/// `Debug` deliberately renders the bearer key as the canonical redaction placeholder so the raw
/// key can never leak through a `{:?}` of the provider.
#[derive(Clone)]
pub struct OpenAiCompatible {
    transport: HttpTransport,
    base_url: String,
    api_key: String,
    provider_name: String,
    extra_headers: Vec<(String, String)>,
    include_usage: bool,
}

impl OpenAiCompatible {
    /// Build a provider from configuration, constructing the rustls-only transport.
    pub fn new(config: OpenAiConfig) -> Result<Self, LlmError> {
        let transport = HttpTransport::new(config.transport)?;
        Ok(Self {
            transport,
            base_url: config.base_url,
            api_key: config.api_key,
            provider_name: config.provider_name,
            extra_headers: config.extra_headers,
            include_usage: config.include_usage,
        })
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

impl fmt::Debug for OpenAiCompatible {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiCompatible")
            .field("transport", &self.transport)
            .field("base_url", &self.base_url)
            .field("api_key", &PLACEHOLDER)
            .field("provider_name", &self.provider_name)
            .field("extra_headers", &self.extra_headers)
            .field("include_usage", &self.include_usage)
            .finish()
    }
}

impl Provider for OpenAiCompatible {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            // Fail-closed: the kill-switch is consulted before the credential check and before any
            // request is built, so it always wins over a present (or absent) credential and no
            // socket is ever created when engaged.
            guard_outbound(OUTBOUND_OPERATION).map_err(LlmError::from_diagnostic)?;

            if self.api_key.is_empty() {
                return Err(LlmError::no_credential(&self.provider_name));
            }

            let body = wire::build_chat_request(&request, self.include_usage)?;
            let url = self.chat_url();
            let idle_timeout = self.transport.config().idle_timeout;

            let bytes = self
                .transport
                .open_post_sse(&url, &self.api_key, &self.extra_headers, body)
                .await?;

            let frames: FrameStream = Box::pin(sse_data_frames(bytes, idle_timeout));
            let events = normalize_chat_sse(frames);
            Ok(Box::pin(events) as LlmStream)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_key_short_circuits_with_no_credential_before_any_socket() {
        // No CORTEX_NO_NET in this process, so the guard passes and the empty-key check fires.
        let provider = OpenAiCompatible::new(OpenAiConfig::openrouter("")).expect("build");
        let result = provider
            .stream(LlmRequest::new("google/gemini-2.5-flash").user("hi"))
            .await;
        let Err(err) = result else {
            panic!("expected a no-credential error");
        };
        assert_eq!(err.kind(), "no-credential");
    }

    #[test]
    fn chat_url_appends_the_completions_path() {
        let provider = OpenAiCompatible::new(OpenAiConfig::openrouter("k")).expect("build");
        assert_eq!(
            provider.chat_url(),
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }

    #[test]
    fn debug_redacts_the_api_key_in_config_and_provider() {
        // Assembled from fragments so no contiguous key-shaped literal lives in source.
        let secret = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(6));
        let config = OpenAiConfig::openrouter(secret.clone());
        let config_debug = format!("{config:?}");
        assert!(
            !config_debug.contains(&secret),
            "config Debug must not leak the key: {config_debug}"
        );
        assert!(config_debug.contains(PLACEHOLDER));

        let provider =
            OpenAiCompatible::new(OpenAiConfig::openrouter(secret.clone())).expect("build");
        let provider_debug = format!("{provider:?}");
        assert!(
            !provider_debug.contains(&secret),
            "provider Debug must not leak the key: {provider_debug}"
        );
        assert!(provider_debug.contains(PLACEHOLDER));
    }
}

//! Provider wrapping for subagent child turns.
//!
//! A child turn records its provider requests under its own child session (via [`AuditedProvider`])
//! and, on surfaces that support plugin request-shaping, runs them through the same plugin
//! middleware as the parent (via a surface-supplied [`ChildProviderWrapper`]).

use std::sync::Arc;
use std::time::Instant;

use futures_util::future::BoxFuture;
use llm::{LlmError, LlmRequest, LlmStream, Provider};

/// Provider-audit metadata copied from the parent run so child turns can emit their own
/// `provider_request` records with child-session attribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderAuditConfig {
    pub provider: String,
    pub endpoint: String,
}

impl ProviderAuditConfig {
    #[must_use]
    pub fn new(provider: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            endpoint: endpoint.into(),
        }
    }
}

/// Surface-supplied hook that applies any additional provider wrapping (e.g. plugin request-shaping
/// middleware) around a child turn's provider, attributing plugin-initiated records to the child
/// session's audit sink.
///
/// The headless CLI implements this over its plugin runtime; the TUI (which does not run plugin
/// request-shaping middleware) supplies no wrapper, so the child provider is used unwrapped —
/// matching the TUI's own parent-turn behavior.
pub trait ChildProviderWrapper: Send + Sync {
    /// Wrap `base` for a child turn whose plugin-initiated records should be attributed to
    /// `child_audit`.
    fn wrap(
        &self,
        base: Arc<dyn Provider>,
        child_audit: &Arc<audit::AuditSink>,
    ) -> Arc<dyn Provider>;
}

/// A [`Provider`] decorator that records one `provider_request` audit entry per outbound request,
/// attributed to the audit sink it was constructed with (a child session's sink for subagents).
pub struct AuditedProvider {
    inner: Arc<dyn Provider>,
    audit: Arc<audit::AuditSink>,
    provider: String,
    endpoint: String,
    host: String,
}

impl AuditedProvider {
    #[must_use]
    pub fn new(
        inner: Arc<dyn Provider>,
        audit: Arc<audit::AuditSink>,
        provider: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        let endpoint = endpoint.into();
        let host = audit::host_from_endpoint(&endpoint).unwrap_or_default();
        Self {
            inner,
            audit,
            provider: provider.into(),
            endpoint,
            host,
        }
    }
}

impl Provider for AuditedProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let model = request.model.clone();
            let extra_headers = request.extra_headers.clone();
            let started = Instant::now();
            let result = self.inner.stream(request).await;
            let outcome = if result.is_ok() { "success" } else { "error" };
            self.audit.record_provider_request(
                &self.provider,
                &model,
                "POST",
                &self.host,
                &self.endpoint,
                None,
                started.elapsed().as_millis(),
                outcome,
                &extra_headers,
            );
            result
        })
    }
}

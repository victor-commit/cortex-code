//! The [`Provider`] trait: the single seam the runner calls to stream a turn.
//!
//! A provider takes an [`LlmRequest`] and returns an ordered [`LlmStream`] of normalized
//! [`LlmEvent`]s. Setup failures that occur before any event can flow (the network kill-switch,
//! an absent credential, a failed connect, or a non-success upstream status) are reported as an
//! `Err(LlmError)` from [`Provider::stream`]; failures that surface mid-turn are normalized into
//! an [`LlmEvent::Error`] within the stream, which still terminates with a single
//! [`LlmEvent::Done`].

use std::pin::Pin;
use std::sync::Arc;

use futures_util::Stream;
use futures_util::future::BoxFuture;

use crate::error::LlmError;
use crate::event::LlmEvent;
use crate::request::LlmRequest;

/// An ordered, owned stream of normalized events for a single turn.
pub type LlmStream = Pin<Box<dyn Stream<Item = LlmEvent> + Send>>;

/// A model provider that streams a normalized turn.
///
/// The trait is object-safe (`dyn Provider`) so the runner can hold a boxed provider chosen at
/// runtime. [`Provider::stream`] is asynchronous because establishing the upstream connection may
/// await I/O before the first event is available.
pub trait Provider: Send + Sync {
    /// Stream one turn for `request`.
    ///
    /// Returns the ordered [`LlmStream`] on success, or a normalized [`LlmError`] if the turn could
    /// not even begin (kill-switch engaged, missing credential, transport failure, or an upstream
    /// error status).
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>>;
}

/// A request-level provider middleware hook.
///
/// Middlewares run in registration order and may transform the request before it reaches the
/// underlying provider. This is the canonical hook point for plugin and surface-specific request
/// shaping without re-implementing ad-hoc provider wrappers.
pub trait RequestMiddleware: Send + Sync {
    /// Transform one outbound request.
    ///
    /// Returning an error aborts the turn before the provider is called.
    fn transform_request(&self, request: LlmRequest) -> Result<LlmRequest, LlmError>;
}

struct MiddlewareProvider {
    inner: Arc<dyn Provider>,
    middlewares: Vec<Arc<dyn RequestMiddleware>>,
}

impl Provider for MiddlewareProvider {
    fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
        Box::pin(async move {
            let mut request = request;
            for middleware in &self.middlewares {
                request = middleware.transform_request(request)?;
            }
            self.inner.stream(request).await
        })
    }
}

/// Wrap `inner` with request middlewares.
#[must_use]
pub fn with_request_middlewares(
    inner: Arc<dyn Provider>,
    middlewares: Vec<Arc<dyn RequestMiddleware>>,
) -> Arc<dyn Provider> {
    if middlewares.is_empty() {
        return inner;
    }
    Arc::new(MiddlewareProvider { inner, middlewares })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures_util::{StreamExt, future::BoxFuture, stream};

    use super::*;
    use crate::LlmEvent;

    #[derive(Default)]
    struct EchoProvider;

    impl Provider for EchoProvider {
        fn stream<'a>(&'a self, request: LlmRequest) -> BoxFuture<'a, Result<LlmStream, LlmError>> {
            Box::pin(async move {
                let stream = stream::iter(vec![
                    LlmEvent::TextDelta {
                        text: request.model.clone(),
                    },
                    LlmEvent::Done {
                        reason: crate::FinishReason::Stop,
                    },
                ]);
                Ok(Box::pin(stream) as LlmStream)
            })
        }
    }

    struct RenameModelMiddleware {
        to: String,
    }

    impl RequestMiddleware for RenameModelMiddleware {
        fn transform_request(&self, mut request: LlmRequest) -> Result<LlmRequest, LlmError> {
            request.model = self.to.clone();
            Ok(request)
        }
    }

    #[tokio::test]
    async fn middleware_wrap_transforms_request_before_provider_call() {
        let provider = with_request_middlewares(
            Arc::new(EchoProvider),
            vec![Arc::new(RenameModelMiddleware {
                to: "local/middleware-model".to_owned(),
            })],
        );
        let mut events = provider
            .stream(LlmRequest::new("google/gemini-2.5-flash"))
            .await
            .expect("stream opens");
        let first = events.next().await.expect("event");
        match first {
            LlmEvent::TextDelta { text } => assert_eq!(text, "local/middleware-model"),
            other => panic!("unexpected event: {other:?}"),
        }
    }
}

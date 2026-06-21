//! The rustls-only HTTP transport and SSE frame extraction.
//!
//! [`HttpTransport`] is a thin wrapper over a `reqwest` client built with **rustls only** (no
//! openssl/native-tls). Its one job is to open a streaming POST and hand back a raw byte stream;
//! it never interprets the protocol. Before it touches the network it consults the `CORTEX_NO_NET`
//! outbound guard, so the kill-switch fails closed before any socket is created.
//!
//! [`sse_data_frames`] turns a raw byte stream into the SSE `data:` payloads using
//! `eventsource-stream`, wrapping each frame read in a per-frame **idle timeout** so a stalled
//! upstream aborts the turn with a transport error instead of hanging forever.

use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt, TryStreamExt};
use redaction::guard_outbound;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

use crate::error::LlmError;

/// A raw, owned stream of response body chunks (or a normalized transport error).
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, LlmError>> + Send>>;

/// The outbound operation named in a `net-disabled` refusal raised by the transport.
const OUTBOUND_OPERATION: &str = "LLM streaming request";

/// Tuning knobs for the transport.
#[derive(Debug, Clone, Copy)]
pub struct TransportConfig {
    /// Maximum time to establish the TCP/TLS connection before failing.
    pub connect_timeout: Duration,
    /// Maximum time to wait for each successive SSE frame before treating the stream as stalled.
    /// This is the per-frame idle/frame timeout; it bounds total turn time only between frames, so
    /// long but live streams are never cut off.
    pub idle_timeout: Duration,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(120),
        }
    }
}

/// A rustls-only HTTP transport for streamed provider requests.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: reqwest::Client,
    config: TransportConfig,
}

impl HttpTransport {
    /// Build a transport with the given configuration.
    ///
    /// The client is pinned to rustls (`use_rustls_tls`) so the binary never links openssl or
    /// native-tls regardless of ambient features.
    pub fn new(config: TransportConfig) -> Result<Self, LlmError> {
        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(config.connect_timeout)
            .user_agent(concat!("cortex-code/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| LlmError::transport(format!("failed to build HTTP client: {err}")))?;
        Ok(Self { client, config })
    }

    /// The active configuration (notably the idle/frame timeout).
    #[must_use]
    pub fn config(&self) -> TransportConfig {
        self.config
    }

    /// Open a streaming `POST` and return the raw response byte stream.
    ///
    /// The outbound guard is consulted FIRST, before the request is built or sent, so under
    /// `CORTEX_NO_NET=1` this returns [`LlmError::NetDisabled`] without opening any socket. A
    /// non-success HTTP status is drained and normalized into an upstream error (see
    /// [`classify_upstream`]); a connection-level failure becomes a transport error.
    pub async fn open_post_sse(
        &self,
        url: &str,
        bearer_token: &str,
        extra_headers: &[(String, String)],
        body: String,
    ) -> Result<ByteStream, LlmError> {
        guard_outbound(OUTBOUND_OPERATION).map_err(LlmError::from_diagnostic)?;

        let headers = request_headers(bearer_token, extra_headers)?;
        let response = self
            .client
            .post(url)
            .headers(headers)
            .body(body)
            .send()
            .await
            .map_err(|err| LlmError::transport(err.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(classify_upstream(status, &detail));
        }

        let bytes = response
            .bytes_stream()
            .map_err(|err| LlmError::transport(err.to_string()));
        Ok(Box::pin(bytes))
    }
}

fn request_headers(
    bearer_token: &str,
    extra_headers: &[(String, String)],
) -> Result<HeaderMap, LlmError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(
        reqwest::header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {bearer_token}"))
            .map_err(|err| LlmError::transport(format!("invalid authorization header: {err}")))?,
    );
    for (name, value) in extra_headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|err| LlmError::transport(format!("invalid request header name: {err}")))?;
        let value = HeaderValue::from_str(value).map_err(|err| {
            LlmError::transport(format!("invalid request header value for {name}: {err}"))
        })?;
        headers.insert(name, value);
    }
    Ok(headers)
}

/// Normalize a non-success HTTP status (with its drained body) into an [`LlmError`].
///
/// Authentication rejections, unknown models, rate-limits, and server errors all originate at the
/// provider once the connection succeeded, so they are [`LlmError::Upstream`] (distinct from a
/// transport failure). The detail is scrubbed of secret-shaped substrings by the constructor.
#[must_use]
pub fn classify_upstream(status: StatusCode, detail: &str) -> LlmError {
    let trimmed = detail.trim();
    let summary = if trimmed.is_empty() {
        format!("provider returned HTTP {status}")
    } else {
        format!("provider returned HTTP {status}: {trimmed}")
    };
    LlmError::upstream(summary)
}

/// Extract the SSE `data:` payloads from a raw byte stream, with a per-frame idle timeout.
///
/// Each yielded item is either the `data` field of one server-sent event or a transport error. If
/// no frame arrives within `idle_timeout` the stream yields a single [`LlmError::Transport`] (an
/// idle-timeout) and ends, so a stalled upstream can never hang the caller. SSE comments and
/// keep-alive lines are filtered by `eventsource-stream` and never surface as data frames.
pub fn sse_data_frames(
    bytes: ByteStream,
    idle_timeout: Duration,
) -> impl Stream<Item = Result<String, LlmError>> + Send {
    let frames = Box::pin(bytes.eventsource());
    futures_util::stream::unfold(Some((frames, idle_timeout)), move |state| async move {
        let (mut frames, idle) = state?;
        match tokio::time::timeout(idle, frames.next()).await {
            Err(_elapsed) => {
                let err = LlmError::transport("idle timeout: no SSE frame received in time");
                Some((Err(err), None))
            }
            Ok(None) => None,
            Ok(Some(Ok(event))) => Some((Ok(event.data), Some((frames, idle)))),
            Ok(Some(Err(err))) => {
                let err = LlmError::transport(format!("SSE stream error: {err}"));
                Some((Err(err), None))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bytes::Bytes;
    use futures_util::StreamExt;

    use super::*;

    fn byte_stream(chunks: Vec<&'static str>) -> ByteStream {
        let iter = chunks
            .into_iter()
            .map(|chunk| Ok::<Bytes, LlmError>(Bytes::from(chunk)));
        Box::pin(futures_util::stream::iter(iter))
    }

    #[tokio::test]
    async fn extracts_data_payloads_and_skips_comments() {
        let stream = byte_stream(vec![
            ": keep-alive comment\n\n",
            "data: hello\n\n",
            "data: world\n\n",
        ]);
        let frames: Vec<_> = sse_data_frames(stream, Duration::from_secs(5))
            .collect()
            .await;
        let datas: Vec<String> = frames.into_iter().map(|r| r.expect("frame")).collect();
        assert_eq!(datas, vec!["hello".to_owned(), "world".to_owned()]);
    }

    #[tokio::test]
    async fn reassembles_frames_split_across_byte_chunks() {
        // The JSON payload is deliberately split mid-frame to prove buffering across chunks.
        let stream = byte_stream(vec!["data: hel", "lo wo", "rld\n\n"]);
        let datas: Vec<String> = sse_data_frames(stream, Duration::from_secs(5))
            .map(|r| r.expect("frame"))
            .collect()
            .await;
        assert_eq!(datas, vec!["hello world".to_owned()]);
    }

    #[tokio::test]
    async fn idle_timeout_aborts_a_stalled_stream_with_a_transport_error() {
        // A stream that never produces a frame: the idle timeout must fire and end the stream.
        let stalled: ByteStream = Box::pin(futures_util::stream::pending());
        let mut frames = Box::pin(sse_data_frames(stalled, Duration::from_millis(20)));
        let first = frames.next().await.expect("an item");
        let err = first.expect_err("idle timeout is an error");
        assert_eq!(err.kind(), "transport-unavailable");
        assert!(err.to_string().contains("idle timeout"));
        assert!(
            frames.next().await.is_none(),
            "stream ends after the timeout"
        );
    }

    #[test]
    fn upstream_classification_includes_status_and_detail() {
        let err = classify_upstream(StatusCode::UNAUTHORIZED, "{\"error\":\"bad key\"}");
        assert_eq!(err.kind(), "upstream");
        let rendered = err.to_string();
        assert!(rendered.contains("401"));
        assert!(rendered.contains("bad key"));
    }

    #[test]
    fn building_a_transport_succeeds_with_rustls() {
        let transport = HttpTransport::new(TransportConfig::default()).expect("rustls client");
        assert_eq!(transport.config().idle_timeout, Duration::from_secs(120));
    }

    #[test]
    fn extra_headers_override_prior_header_values() {
        let headers = request_headers(
            "k",
            &[
                ("X-Cortex-Plugin".to_owned(), "old".to_owned()),
                ("x-cortex-plugin".to_owned(), "new".to_owned()),
            ],
        )
        .expect("headers");
        assert_eq!(
            headers
                .get("x-cortex-plugin")
                .and_then(|value| value.to_str().ok()),
            Some("new")
        );
    }
}

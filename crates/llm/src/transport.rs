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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use eventsource_stream::Eventsource;
use futures_util::{Stream, StreamExt, TryStreamExt};
use redaction::guard_outbound;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RETRY_AFTER};

use crate::error::LlmError;

/// A raw, owned stream of response body chunks (or a normalized transport error).
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, LlmError>> + Send>>;

/// The outbound operation named in a `net-disabled` refusal raised by the transport.
const OUTBOUND_OPERATION: &str = "LLM streaming request";
const RETRY_AFTER_MS_HEADER: &str = "retry-after-ms";
const MAX_RETRIES: u32 = 2;
const BASE_DELAY_MS: u64 = 500;
const MAX_DELAY_MS: u64 = 10_000;
const JITTER_PERCENT: u64 = 20;

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
    /// retryable non-success HTTP status (`429`, `529`, `5xx`) is retried with jittered
    /// exponential backoff; `retry-after-ms` / `retry-after` headers override the jittered delay
    /// (safely capped). Remaining non-success statuses are drained and normalized into an upstream
    /// error (see [`classify_upstream`]); a connection-level failure becomes a transport error.
    pub async fn open_post_sse(
        &self,
        url: &str,
        bearer_token: &str,
        extra_headers: &[(String, String)],
        body: String,
    ) -> Result<ByteStream, LlmError> {
        guard_outbound(OUTBOUND_OPERATION).map_err(LlmError::from_diagnostic)?;

        let headers = request_headers(bearer_token, extra_headers)?;
        let mut retries_performed = 0_u32;
        loop {
            let response = self
                .client
                .post(url)
                .headers(headers.clone())
                .body(body.clone())
                .send()
                .await
                .map_err(|err| LlmError::transport(err.to_string()))?;

            let status = response.status();
            if !status.is_success() {
                let response_headers = response.headers().clone();
                let detail = response.text().await.unwrap_or_default();
                if let Some(delay) = next_retry_delay(status, &response_headers, retries_performed)
                {
                    retries_performed = retries_performed.saturating_add(1);
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(classify_upstream(status, &detail));
            }

            let bytes = response
                .bytes_stream()
                .map_err(|err| LlmError::transport(err.to_string()));
            return Ok(Box::pin(bytes));
        }
    }
}

fn next_retry_delay(
    status: StatusCode,
    headers: &HeaderMap,
    retries_performed: u32,
) -> Option<Duration> {
    if retries_performed >= MAX_RETRIES || !is_retryable_status(status) {
        return None;
    }
    if let Some(delay) = parse_retry_after(headers) {
        return Some(delay.min(Duration::from_millis(MAX_DELAY_MS)));
    }
    Some(jittered_exponential_backoff(retries_performed))
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.as_u16() == 529 || status.is_server_error()
}

fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    if let Some(value) = headers.get(RETRY_AFTER_MS_HEADER)
        && let Some(delay) = parse_retry_after_ms(value)
    {
        return Some(delay);
    }
    if let Some(value) = headers.get(RETRY_AFTER) {
        return parse_retry_after_seconds_or_date(value);
    }
    None
}

fn parse_retry_after_ms(value: &HeaderValue) -> Option<Duration> {
    value
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_millis)
}

fn parse_retry_after_seconds_or_date(value: &HeaderValue) -> Option<Duration> {
    let raw = value.to_str().ok()?.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let when = httpdate::parse_http_date(raw).ok()?;
    match when.duration_since(SystemTime::now()) {
        Ok(duration) => Some(duration),
        Err(_in_past) => Some(Duration::ZERO),
    }
}

fn jittered_exponential_backoff(retries_performed: u32) -> Duration {
    let exponent = retries_performed.min(20);
    let base = BASE_DELAY_MS
        .saturating_mul(1_u64 << exponent)
        .min(MAX_DELAY_MS);
    let lower = base.saturating_mul(100 - JITTER_PERCENT) / 100;
    let upper = base.saturating_mul(100 + JITTER_PERCENT) / 100;
    let capped_upper = upper.min(MAX_DELAY_MS);
    let span = capped_upper.saturating_sub(lower);
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let jitter = if span == 0 { 0 } else { seed % (span + 1) };
    Duration::from_millis(lower.saturating_add(jitter).min(MAX_DELAY_MS))
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
    use std::time::{Duration, SystemTime};

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
    fn retries_cover_429_529_and_5xx_statuses() {
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(
            StatusCode::from_u16(529).expect("valid status")
        ));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
    }

    #[test]
    fn retry_after_ms_takes_precedence_over_retry_after() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static(RETRY_AFTER_MS_HEADER),
            HeaderValue::from_static("75"),
        );
        headers.insert(RETRY_AFTER, HeaderValue::from_static("5"));
        let delay = parse_retry_after(&headers).expect("retry-after delay");
        assert_eq!(delay, Duration::from_millis(75));
    }

    #[test]
    fn retry_after_parses_seconds_and_http_date() {
        let mut second_headers = HeaderMap::new();
        second_headers.insert(RETRY_AFTER, HeaderValue::from_static("2"));
        assert_eq!(
            parse_retry_after(&second_headers),
            Some(Duration::from_secs(2))
        );

        let future_date = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(2));
        let mut date_headers = HeaderMap::new();
        date_headers.insert(
            RETRY_AFTER,
            HeaderValue::from_str(&future_date).expect("http-date header"),
        );
        let date_delay = parse_retry_after(&date_headers).expect("parsed http-date delay");
        assert!(
            date_delay > Duration::from_millis(500),
            "expected a positive delay for future http-date, got {date_delay:?}"
        );
    }

    #[test]
    fn retry_delay_none_for_non_retryable_or_exhausted_retries() {
        let headers = HeaderMap::new();
        assert!(
            next_retry_delay(StatusCode::BAD_REQUEST, &headers, 0).is_none(),
            "400 is not retryable"
        );
        assert!(
            next_retry_delay(StatusCode::SERVICE_UNAVAILABLE, &headers, MAX_RETRIES).is_none(),
            "retry budget exhausted"
        );
    }

    #[test]
    fn jittered_backoff_stays_in_expected_bounds() {
        let first = jittered_exponential_backoff(0);
        assert!(
            (Duration::from_millis(400)..=Duration::from_millis(600)).contains(&first),
            "first retry delay should be around 500ms ±20%, got {first:?}"
        );

        let second = jittered_exponential_backoff(1);
        assert!(
            (Duration::from_millis(800)..=Duration::from_millis(1_200)).contains(&second),
            "second retry delay should be around 1000ms ±20%, got {second:?}"
        );
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

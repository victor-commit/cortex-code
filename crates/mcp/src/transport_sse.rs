//! A client transport for the legacy MCP "HTTP + SSE" transport (protocol 2024-11-05).
//!
//! Newer servers speak Streamable HTTP (a single endpoint that answers POSTs with JSON or an SSE
//! body); older servers instead expose a long-lived `GET` SSE channel that first emits an
//! `endpoint` event naming a separate URL to `POST` client messages to. `rmcp` 1.7 ships only the
//! Streamable HTTP client, so this module implements the legacy transport directly as an
//! [`rmcp::transport::Transport`]: it opens the SSE stream, learns the message endpoint, forwards
//! every server `message` event into the receive channel, and POSTs each outgoing client message to
//! the learned endpoint. The cortex remote client tries Streamable HTTP first and falls back to
//! this transport, so an SSE-only server still connects (VAL-MCP-007).

use std::time::Duration;

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use http::HeaderMap;
use rmcp::model::{ClientJsonRpcMessage, ServerJsonRpcMessage};
use rmcp::service::RoleClient;
use rmcp::transport::Transport;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// A failure opening or operating the legacy SSE transport.
#[derive(Debug, thiserror::Error)]
pub enum SseTransportError {
    /// The HTTP request itself failed (connect/transport error).
    #[error("SSE transport HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// The server rejected the SSE handshake (non-success status, e.g. a server that does not
    /// support SSE at all).
    #[error("SSE handshake rejected: HTTP {0}")]
    Status(u16),
    /// The server's SSE response was not an `text/event-stream`.
    #[error("SSE handshake returned an unexpected content type: {0}")]
    ContentType(String),
    /// The server never sent the initial `endpoint` event within the timeout.
    #[error("timed out waiting for the SSE endpoint event")]
    EndpointTimeout,
    /// The SSE stream closed before the endpoint event arrived.
    #[error("the SSE stream closed before announcing an endpoint")]
    StreamClosed,
    /// The endpoint URL announced by the server could not be resolved.
    #[error("invalid SSE endpoint URL `{0}`")]
    InvalidEndpoint(String),
    /// Outgoing message serialization failed.
    #[error("failed to encode an outgoing MCP message: {0}")]
    Encode(String),
}

/// A connected legacy SSE transport.
pub(crate) struct SseClientTransport {
    http: reqwest::Client,
    post_url: String,
    headers: HeaderMap,
    rx: mpsc::Receiver<ServerJsonRpcMessage>,
    reader: Option<JoinHandle<()>>,
}

impl SseClientTransport {
    /// Open the SSE stream at `sse_url`, learn the message endpoint, and start forwarding messages.
    ///
    /// `headers` are sent on both the SSE `GET` and every message `POST` (custom config headers
    /// plus any `Authorization` token). The endpoint event must arrive within `timeout`.
    pub(crate) async fn connect(
        sse_url: &str,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<Self, SseTransportError> {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .build()?;

        let response = http
            .get(sse_url)
            .headers(headers.clone())
            .header(http::header::ACCEPT, "text/event-stream")
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(SseTransportError::Status(status.as_u16()));
        }
        let content_type = response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if !content_type.starts_with("text/event-stream") {
            return Err(SseTransportError::ContentType(content_type));
        }

        let base = url::Url::parse(sse_url)
            .map_err(|_| SseTransportError::InvalidEndpoint(sse_url.to_owned()))?;

        let (endpoint_tx, endpoint_rx) = oneshot::channel::<Result<String, SseTransportError>>();
        let (msg_tx, msg_rx) = mpsc::channel::<ServerJsonRpcMessage>(64);

        let reader = tokio::spawn(async move {
            let mut events = response.bytes_stream().eventsource();
            let mut endpoint_tx = Some(endpoint_tx);
            while let Some(event) = events.next().await {
                let Ok(event) = event else { break };
                match event.event.as_str() {
                    "endpoint" => {
                        if let Some(tx) = endpoint_tx.take() {
                            let resolved = base
                                .join(event.data.trim())
                                .map(|url| url.to_string())
                                .map_err(|_| {
                                    SseTransportError::InvalidEndpoint(event.data.clone())
                                });
                            let _ = tx.send(resolved);
                        }
                    }
                    // The default SSE event type is "message"; legacy servers use it for JSON-RPC.
                    "message" | "" => {
                        if let Ok(message) =
                            serde_json::from_str::<ServerJsonRpcMessage>(event.data.trim())
                        {
                            if msg_tx.send(message).await.is_err() {
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        });

        let post_url = match tokio::time::timeout(timeout, endpoint_rx).await {
            Ok(Ok(result)) => result?,
            Ok(Err(_)) => return Err(SseTransportError::StreamClosed),
            Err(_) => {
                reader.abort();
                return Err(SseTransportError::EndpointTimeout);
            }
        };

        Ok(Self {
            http,
            post_url,
            headers,
            rx: msg_rx,
            reader: Some(reader),
        })
    }
}

impl Transport<RoleClient> for SseClientTransport {
    type Error = SseTransportError;

    fn send(
        &mut self,
        item: ClientJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let http = self.http.clone();
        let url = self.post_url.clone();
        let headers = self.headers.clone();
        async move {
            let body = serde_json::to_vec(&item)
                .map_err(|err| SseTransportError::Encode(err.to_string()))?;
            let response = http
                .post(&url)
                .headers(headers)
                .header(http::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await?;
            let status = response.status();
            if !status.is_success() {
                return Err(SseTransportError::Status(status.as_u16()));
            }
            Ok(())
        }
    }

    fn receive(
        &mut self,
    ) -> impl std::future::Future<Output = Option<ServerJsonRpcMessage>> + Send {
        self.rx.recv()
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
        self.rx.close();
        std::future::ready(Ok(()))
    }
}

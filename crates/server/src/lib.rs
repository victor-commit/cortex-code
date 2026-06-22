//! Axum HTTP surface for `cortex serve`.
//!
//! This crate owns the local loopback listener and lightweight HTTP routes. Later M6 features can
//! add session, message, SSE, and OpenAPI handlers here while reusing the same lifecycle shell.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::{Json, Router, routing::get};
use events::{Event, EventBus, LifecyclePhase, Surface};
use serde::Serialize;
use thiserror::Error;
use tokio::net::TcpListener;

/// First allowed port for `cortex serve` and server test fixtures.
pub const MIN_PORT: u16 = 3200;
/// Last allowed port for `cortex serve` and server test fixtures.
pub const MAX_PORT: u16 = 3299;
/// Canonical loopback bind address required by the server validation contract.
pub const LOOPBACK_HOST: &str = "127.0.0.1";

/// Small machine-readable health response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HealthResponse {
    /// Affirmative status marker for clients and validators.
    pub status: &'static str,
}

/// Errors produced while preparing or running the local server.
#[derive(Debug, Error)]
pub enum ServeError {
    /// The requested hostname is not the canonical loopback host.
    #[error("hostname {hostname:?} is not allowed; cortex serve binds only to 127.0.0.1")]
    NonLoopbackHost {
        /// Hostname supplied by the CLI.
        hostname: String,
    },
    /// The requested port is outside the mission-reserved server range.
    #[error("port {port} is outside the reserved cortex serve range 3200-3299")]
    PortOutOfRange {
        /// Port supplied by the CLI.
        port: u16,
    },
    /// The requested loopback port already has a listener.
    #[error("port {port} is already in use on 127.0.0.1; choose a free port in 3200-3299")]
    PortBusy {
        /// Busy port.
        port: u16,
        /// Underlying OS bind error.
        #[source]
        source: std::io::Error,
    },
    /// Binding failed for a reason other than an already-listening port.
    #[error("failed to bind 127.0.0.1:{port}: {source}")]
    Bind {
        /// Requested port.
        port: u16,
        /// Underlying OS bind error.
        #[source]
        source: std::io::Error,
    },
    /// Inspecting the listener failed after a successful bind.
    #[error("failed to inspect local listener address: {source}")]
    LocalAddr {
        /// Underlying OS error.
        #[source]
        source: std::io::Error,
    },
    /// The axum serving loop returned an IO error.
    #[error("server failed while listening on {addr}: {source}")]
    Serve {
        /// Listener address.
        addr: SocketAddr,
        /// Underlying server IO error.
        #[source]
        source: std::io::Error,
    },
}

impl ServeError {
    /// Whether this failure came from invalid CLI input rather than runtime conditions.
    #[must_use]
    pub const fn is_usage_error(&self) -> bool {
        matches!(
            self,
            ServeError::NonLoopbackHost { .. } | ServeError::PortOutOfRange { .. }
        )
    }
}

/// Construct the axum application for the currently implemented server routes.
pub fn app() -> Router {
    Router::new().route("/health", get(health))
}

/// Resolve and validate the requested bind address.
///
/// The lifecycle contract is intentionally strict: even if another loopback address is available,
/// the public server surface binds only to `127.0.0.1` inside the reserved 3200-3299 range.
pub fn bind_addr(hostname: &str, port: u16) -> Result<SocketAddr, ServeError> {
    if !(MIN_PORT..=MAX_PORT).contains(&port) {
        return Err(ServeError::PortOutOfRange { port });
    }

    match hostname.trim() {
        LOOPBACK_HOST | "localhost" => Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)),
        other => Err(ServeError::NonLoopbackHost {
            hostname: other.to_owned(),
        }),
    }
}

/// Bind the loopback listener for `cortex serve`.
pub async fn bind_listener(hostname: &str, port: u16) -> Result<TcpListener, ServeError> {
    let addr = bind_addr(hostname, port)?;
    TcpListener::bind(addr)
        .await
        .map_err(|source| match source.kind() {
            std::io::ErrorKind::AddrInUse => ServeError::PortBusy { port, source },
            _ => ServeError::Bind { port, source },
        })
}

/// Serve the app on an already-bound listener until the provided shutdown future resolves.
pub async fn serve_with_shutdown<F>(
    listener: TcpListener,
    app: Router,
    shutdown: F,
) -> Result<(), ServeError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let addr = listener
        .local_addr()
        .map_err(|source| ServeError::LocalAddr { source })?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|source| ServeError::Serve { addr, source })
}

/// Serve the app and shut down cleanly on SIGINT or SIGTERM.
pub async fn serve_with_signals(
    listener: TcpListener,
    bus: Arc<EventBus>,
    version: impl Into<String>,
) -> Result<(), ServeError> {
    let version = version.into();
    serve_with_shutdown(listener, app(), shutdown_signal(bus, version)).await
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn shutdown_signal(bus: Arc<EventBus>, version: String) {
    wait_for_shutdown_signal().await;
    bus.publish(&Event::lifecycle(
        LifecyclePhase::Shutdown,
        Surface::Serve,
        version,
    ));
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let terminate = async {
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

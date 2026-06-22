//! Axum HTTP surface for `cortex serve`.
//!
//! This crate owns the local loopback listener and lightweight HTTP routes. Session CRUD is backed
//! by the shared SQLite repositories; later M6 features can add message, SSE, and OpenAPI handlers
//! here while reusing the same lifecycle shell.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use db::{Database, DbError, Session};
use events::{Event, EventBus, LifecyclePhase, Surface};
use serde::{Deserialize, Serialize};
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

/// Shared immutable state for HTTP handlers.
///
/// The server stores a database path, not an in-memory session map. Each route opens the shared
/// SQLite database and uses the `db` crate repositories, so sessions are durable across process
/// restarts and visible to other surfaces that use the same path.
#[derive(Clone, Debug)]
pub struct ServerState {
    db_path: PathBuf,
}

impl ServerState {
    /// Build server state for an explicit SQLite database file.
    #[must_use]
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }

    /// Resolve the process database path and verify the database can be opened.
    ///
    /// This is the startup path used by `cortex serve`, so a broken database location fails before
    /// the listener starts advertising health.
    pub fn from_process() -> Result<Self, ServeError> {
        let db_path = db::DbPathInputs::from_process()
            .resolve()
            .ok_or(ServeError::DatabasePath)?;
        Database::open(&db_path).map_err(|source| ServeError::Database { source })?;
        Ok(Self::new(db_path))
    }

    /// The database path shared by every route.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
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
    /// The process environment does not provide enough information to resolve a DB path.
    #[error(
        "cannot resolve database path; set CORTEX_DB_PATH or HOME before starting cortex serve"
    )]
    DatabasePath,
    /// Opening the shared session database failed during server startup.
    #[error("cannot open session database: {source}")]
    Database {
        /// Underlying DB error.
        #[source]
        source: DbError,
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
pub fn app(db_path: impl Into<PathBuf>) -> Router {
    app_with_state(ServerState::new(db_path))
}

/// Construct the axum application for an explicit database file.
pub fn app_with_db_path(db_path: impl Into<PathBuf>) -> Router {
    app(db_path)
}

/// Construct the axum application from pre-resolved server state.
pub fn app_with_state(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/session", get(list_sessions).post(create_session))
        .route("/session/{id}", get(get_session).delete(delete_session))
        .with_state(state)
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
    state: ServerState,
) -> Result<(), ServeError> {
    let version = version.into();
    serve_with_shutdown(
        listener,
        app_with_state(state),
        shutdown_signal(bus, version),
    )
    .await
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSessionRequest {
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn not_found(id: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("no such session: {id}"))
    }

    fn from_db(source: DbError) -> Self {
        let status = match &source {
            DbError::NotFound { .. } => StatusCode::NOT_FOUND,
            DbError::Busy { .. } => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, source.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response()
    }
}

async fn create_session(
    State(state): State<ServerState>,
    Json(payload): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    let db = open_database(&state)?;
    let session = db
        .sessions()
        .create(payload.title.as_deref())
        .map_err(ApiError::from_db)?;
    Ok((StatusCode::CREATED, Json(session)))
}

async fn list_sessions(State(state): State<ServerState>) -> Result<Json<Vec<Session>>, ApiError> {
    let db = open_database(&state)?;
    let sessions = db.sessions().list().map_err(ApiError::from_db)?;
    Ok(Json(sessions))
}

async fn get_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Session>, ApiError> {
    let db = open_database(&state)?;
    let session = db.sessions().require(&id).map_err(ApiError::from_db)?;
    Ok(Json(session))
}

async fn delete_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let db = open_database(&state)?;
    if db.sessions().delete(&id).map_err(ApiError::from_db)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(&id))
    }
}

fn open_database(state: &ServerState) -> Result<Database, ApiError> {
    Database::open(state.db_path()).map_err(ApiError::from_db)
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

//! Axum HTTP surface for `cortex serve`.
//!
//! This crate owns the local loopback listener and lightweight HTTP routes. Session CRUD and prompt
//! turns are backed by the shared SQLite repositories and the core durable runner so the server
//! remains a thin surface over the engine and store.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        Html, IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, post},
};
use db::{Database, DbError, Message, Session, UsageTotals};
use engine::{
    CompactionPolicy, EnvironmentBlock, HistoryMessage, ModelFamily, RunError, RunOutcome,
    RunnerEvent, SessionRunner, SystemContext, ToolDispatchResult, ToolDispatcher, Turn,
    project_history,
};
use events::{Event, EventBus, LifecyclePhase, Surface};
use futures_util::{future::BoxFuture, stream};
use llm::{Provider, ToolCall, ToolDef, Usage};
use permissions::{PermissionEngine, RuleError};
use protocol::TodoItem;
use providers::ResolvedModel;
use redaction::guard_outbound;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{net::TcpListener, sync::broadcast, sync::watch};
use tools::{ToolContext, ToolRegistry};

mod openapi;

/// First allowed port for `cortex serve` and server test fixtures.
pub const MIN_PORT: u16 = 3200;
/// Last allowed port for `cortex serve` and server test fixtures.
pub const MAX_PORT: u16 = 3299;
/// Canonical loopback bind address required by the server validation contract.
pub const LOOPBACK_HOST: &str = "127.0.0.1";
/// The outbound operation named when the network kill-switch refuses a server prompt turn.
const OUTBOUND_OPERATION: &str = "LLM turn";
static REMOTE_COMMAND_COUNTER: AtomicU64 = AtomicU64::new(0);
static LOCAL_SHARE_COUNTER: AtomicU64 = AtomicU64::new(0);

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
#[derive(Clone)]
pub struct ServerState {
    db_path: PathBuf,
    provider: ProviderSource,
    active_turns: ActiveTurns,
    event_bus: Arc<EventBus>,
    prompt_buffers: PromptBuffers,
    local_shares: LocalShares,
}

impl ServerState {
    /// Build server state for an explicit SQLite database file.
    #[must_use]
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            provider: ProviderSource::OpenRouter,
            active_turns: ActiveTurns::default(),
            event_bus: Arc::new(EventBus::new()),
            prompt_buffers: PromptBuffers::default(),
            local_shares: LocalShares::default(),
        }
    }

    /// Build server state with a preconstructed provider, used by in-process route tests.
    #[must_use]
    pub fn with_provider(db_path: impl Into<PathBuf>, provider: Arc<dyn Provider>) -> Self {
        Self {
            db_path: db_path.into(),
            provider: ProviderSource::Fixed {
                model_id: providers::OPENROUTER_DEFAULT_MODEL.to_owned(),
                provider,
            },
            active_turns: ActiveTurns::default(),
            event_bus: Arc::new(EventBus::new()),
            prompt_buffers: PromptBuffers::default(),
            local_shares: LocalShares::default(),
        }
    }

    /// Attach the event bus that backs `/event` and turn lifecycle publication.
    #[must_use]
    pub fn with_event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        self.event_bus = event_bus;
        self
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

impl std::fmt::Debug for ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerState")
            .field("db_path", &self.db_path)
            .field("event_subscribers", &self.event_bus.subscriber_count())
            .field("prompt_buffers", &self.prompt_buffers.len())
            .field("local_shares", &self.local_shares.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
enum ProviderSource {
    OpenRouter,
    Fixed {
        model_id: String,
        provider: Arc<dyn Provider>,
    },
}

#[derive(Clone, Default)]
struct ActiveTurns {
    inner: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

#[derive(Clone, Default)]
struct PromptBuffers {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

#[derive(Clone, Default)]
struct LocalShares {
    inner: Arc<Mutex<HashMap<String, String>>>,
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

/// Construct the axum application with a preconstructed provider.
///
/// This keeps route tests deterministic while the process `cortex serve` path still resolves the
/// real OpenRouter provider at request time.
pub fn app_with_provider(db_path: impl Into<PathBuf>, provider: Arc<dyn Provider>) -> Router {
    app_with_state(ServerState::with_provider(db_path, provider))
}

/// Construct the axum application from pre-resolved server state.
pub fn app_with_state(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/openapi.json", get(openapi::openapi_json))
        .route("/event", get(event_stream))
        .route("/session", get(list_sessions).post(create_session))
        .route("/session/{id}", get(get_session).delete(delete_session))
        .route(
            "/session/{id}/message",
            get(list_messages).post(post_message),
        )
        .route("/session/{id}/prompt", get(get_prompt_draft))
        .route("/session/{id}/prompt/append", post(append_prompt))
        .route("/session/{id}/prompt/submit", post(submit_prompt))
        .route("/session/{id}/prompt/clear", post(clear_prompt))
        .route("/session/{id}/command", post(execute_command))
        .route("/session/{id}/abort", post(abort_session))
        .route("/session/{id}/share", post(create_share))
        .route("/session/{id}/unshare", post(unshare_session))
        .route("/share/{share_id}", get(get_share))
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
        app_with_state(state.with_event_bus(bus.clone())),
        shutdown_signal(bus, version),
    )
    .await
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn event_stream(
    State(state): State<ServerState>,
) -> Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>> {
    let receiver = state.event_bus.subscribe_stream();
    let stream = stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event).unwrap_or_else(|err| {
                        serde_json::json!({
                            "kind": "event.serialization_error",
                            "message": err.to_string(),
                        })
                        .to_string()
                    });
                    let frame = SseEvent::default().event(event.kind()).data(data);
                    return Some((Ok(frame), receiver));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSessionRequest {
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageRequest {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    parts: Vec<MessagePart>,
}

impl MessageRequest {
    fn into_prompt(self) -> Option<String> {
        let part_text = self
            .parts
            .into_iter()
            .filter(|part| part.kind.as_deref().unwrap_or("text") == "text")
            .filter_map(|part| part.text)
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        non_empty(&part_text)
            .or_else(|| self.text.and_then(|text| non_empty(&text)))
            .or_else(|| self.content.and_then(|content| non_empty(&content)))
            .or_else(|| self.message.and_then(|message| non_empty(&message)))
            .or_else(|| self.prompt.and_then(|prompt| non_empty(&prompt)))
    }
}

#[derive(Debug, Deserialize)]
struct MessagePart {
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Serialize)]
struct MessageListResponse {
    session: Session,
    messages: Vec<Message>,
    usage: UsageTotals,
}

#[derive(Debug, Serialize)]
struct PromptResponse {
    status: &'static str,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<llm::FinishReason>,
}

#[derive(Debug, Serialize)]
struct PromptDraftResponse {
    status: &'static str,
    text: String,
}

#[derive(Debug, Serialize)]
struct AbortResponse {
    status: &'static str,
    active: bool,
}

#[derive(Debug, Serialize)]
struct ShareResponse {
    status: &'static str,
    share_id: String,
    url: String,
}

#[derive(Debug, Serialize)]
struct UnshareResponse {
    status: &'static str,
    revoked: bool,
}

#[derive(Debug, Deserialize)]
struct CommandRequest {
    command: String,
}

#[derive(Debug, Serialize)]
struct CommandResponse {
    status: &'static str,
    command: String,
    output: String,
    is_error: bool,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
    kind: Option<String>,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            kind: None,
        }
    }

    fn diagnostic(status: StatusCode, diagnostic: errors::Diagnostic) -> Self {
        Self {
            status,
            kind: Some(diagnostic.kind().to_owned()),
            message: diagnostic.to_string(),
        }
    }

    fn not_found(id: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("no such session: {id}"))
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn from_db(source: DbError) -> Self {
        let status = match &source {
            DbError::NotFound { .. } => StatusCode::NOT_FOUND,
            DbError::Busy { .. } => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, source.to_string())
    }

    fn from_run_error(error: &RunError) -> Self {
        let status = match error.kind() {
            "net-disabled" => StatusCode::SERVICE_UNAVAILABLE,
            "no-credential" => StatusCode::UNAUTHORIZED,
            "transport-unavailable" | "upstream" => StatusCode::BAD_GATEWAY,
            "step-limit-exceeded" | "context-overflow" => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            kind: Some(error.kind().to_owned()),
            message: error.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                error: self.message,
                kind: self.kind,
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

async fn list_messages(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<MessageListResponse>, ApiError> {
    let db = open_database(&state)?;
    let session = db.sessions().require(&id).map_err(ApiError::from_db)?;
    let messages = db
        .messages()
        .list_for_session(&id)
        .map_err(ApiError::from_db)?;
    let usage = db.usage().session_totals(&id).map_err(ApiError::from_db)?;
    Ok(Json(MessageListResponse {
        session,
        messages,
        usage,
    }))
}

async fn post_message(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<MessageRequest>,
) -> Result<Json<PromptResponse>, ApiError> {
    let prompt = payload
        .into_prompt()
        .ok_or_else(|| ApiError::bad_request("message body must include non-empty text"))?;
    run_message_turn(state, id, prompt).await.map(Json)
}

async fn get_prompt_draft(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PromptDraftResponse>, ApiError> {
    require_session(&state, &id)?;
    let text = state.prompt_buffers.get(&id)?;
    Ok(Json(PromptDraftResponse {
        status: "draft",
        text,
    }))
}

async fn append_prompt(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<MessageRequest>,
) -> Result<Json<PromptDraftResponse>, ApiError> {
    require_session(&state, &id)?;
    let text = payload
        .into_prompt()
        .ok_or_else(|| ApiError::bad_request("prompt append body must include non-empty text"))?;
    let text = state.prompt_buffers.append(&id, &text)?;
    Ok(Json(PromptDraftResponse {
        status: "updated",
        text,
    }))
}

async fn submit_prompt(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PromptResponse>, ApiError> {
    require_session(&state, &id)?;
    let prompt = state.prompt_buffers.take(&id)?;
    let prompt = non_empty(&prompt).ok_or_else(|| {
        ApiError::bad_request("prompt buffer is empty; append text before submit")
    })?;
    run_message_turn(state, id, prompt).await.map(Json)
}

async fn clear_prompt(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PromptDraftResponse>, ApiError> {
    require_session(&state, &id)?;
    state.prompt_buffers.clear(&id)?;
    Ok(Json(PromptDraftResponse {
        status: "cleared",
        text: String::new(),
    }))
}

async fn execute_command(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<CommandRequest>,
) -> Result<Json<CommandResponse>, ApiError> {
    require_session(&state, &id)?;
    let command = non_empty(&payload.command)
        .ok_or_else(|| ApiError::bad_request("command body must include a non-empty command"))?;
    let call_id = format!(
        "remote-command-{}-{}",
        std::process::id(),
        REMOTE_COMMAND_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let arguments = serde_json::json!({ "command": &command }).to_string();
    state.event_bus.publish(&Event::tool_call_for_turn(
        id.clone(),
        call_id.clone(),
        call_id.clone(),
        "bash",
        arguments.clone(),
    ));

    let permission_engine = permission_engine_for_turn(state.event_bus.clone(), &id, &call_id)?;
    let mut context =
        ToolContext::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .with_session(id.clone())
            .with_todo_store(Arc::new(ServerTodoStore {
                db_path: state.db_path.clone(),
            }));
    if let Some(permission_engine) = permission_engine {
        context = context.with_permissions(permission_engine);
    }
    let output = ToolRegistry::builtin()
        .execute("bash", &arguments, &context)
        .await;
    context.background_jobs().shutdown().await;
    state.event_bus.publish(&Event::tool_result_for_turn(
        id,
        call_id.clone(),
        call_id,
        "bash",
        output.is_error,
    ));
    let is_error = output.is_error;
    Ok(Json(CommandResponse {
        status: if is_error { "error" } else { "completed" },
        command,
        output: output.model_content(),
        is_error,
    }))
}

async fn abort_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<AbortResponse>, ApiError> {
    let db = open_database(&state)?;
    db.sessions().require(&id).map_err(ApiError::from_db)?;
    let active = state.active_turns.abort(&id)?;
    Ok(Json(AbortResponse {
        status: "abort_requested",
        active,
    }))
}

async fn create_share(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<ShareResponse>, ApiError> {
    require_session(&state, &id)?;
    let origin = loopback_share_origin(&headers)?;
    let share_id = state.local_shares.create(&id)?;
    let url = format!("{origin}/share/{share_id}");
    Ok(Json(ShareResponse {
        status: "shared",
        share_id,
        url,
    }))
}

async fn unshare_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<UnshareResponse>, ApiError> {
    require_session(&state, &id)?;
    let revoked = state.local_shares.revoke_session(&id)?;
    Ok(Json(UnshareResponse {
        status: "unshared",
        revoked,
    }))
}

async fn get_share(
    State(state): State<ServerState>,
    AxumPath(share_id): AxumPath<String>,
) -> Result<Html<String>, ApiError> {
    let session_id = state
        .local_shares
        .session_for_share(&share_id)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "share not found"))?;
    let db = open_database(&state)?;
    let session = db
        .sessions()
        .require(&session_id)
        .map_err(ApiError::from_db)?;
    let messages = db
        .messages()
        .list_for_session(&session_id)
        .map_err(ApiError::from_db)?;
    Ok(Html(render_share_page(&session, &messages)))
}

fn open_database(state: &ServerState) -> Result<Database, ApiError> {
    Database::open(state.db_path()).map_err(ApiError::from_db)
}

fn require_session(state: &ServerState, id: &str) -> Result<Session, ApiError> {
    let db = open_database(state)?;
    db.sessions().require(id).map_err(ApiError::from_db)
}

async fn run_message_turn(
    state: ServerState,
    session_id: String,
    prompt: String,
) -> Result<PromptResponse, ApiError> {
    let db = open_database(&state)?;
    let session = db
        .sessions()
        .require(&session_id)
        .map_err(ApiError::from_db)?;

    // The kill-switch wins before credential resolution, prompt persistence, or any outbound
    // provider setup, so `CORTEX_NO_NET=1` leaves the server healthy but produces no assistant.
    if let Err(diagnostic) = guard_outbound(OUTBOUND_OPERATION) {
        return Err(ApiError::diagnostic(
            StatusCode::SERVICE_UNAVAILABLE,
            diagnostic,
        ));
    }

    let resolved = resolve_turn_provider(&state.provider, session.model.as_deref())?;
    let history = load_history(&db, &session_id)?;
    let user_message = db
        .messages()
        .create(&session_id, "user", &prompt)
        .map_err(ApiError::from_db)?;
    state
        .event_bus
        .publish(&Event::turn_started(&session_id, &user_message.id));
    let (abort_tx, abort_rx) = watch::channel(false);
    let _active = state
        .active_turns
        .insert(session_id.clone(), abort_tx)
        .map_err(|message| ApiError::new(StatusCode::CONFLICT, message))?;

    let system =
        SystemContext::for_model(&resolved.model_id, EnvironmentBlock::from_process()).render();
    let turn = Turn::new(&resolved.model_id, system, prompt)
        .with_history(history)
        .with_compaction(compaction_policy(&resolved.model));
    let permission_engine =
        permission_engine_for_turn(state.event_bus.clone(), &session_id, &user_message.id)?;
    let dispatcher = ServerToolDispatcher::for_model(
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        &resolved.model_id,
        session_id.clone(),
        state.db_path.clone(),
        permission_engine,
    );
    let outcome = run_cancellable_turn(
        resolved.provider.as_ref(),
        turn,
        &dispatcher,
        abort_rx,
        state.event_bus.clone(),
        session_id.clone(),
        user_message.id.clone(),
    )
    .await;
    dispatcher.shutdown().await;

    let Some(outcome) = outcome else {
        state.event_bus.publish(&Event::turn_done(
            &session_id,
            &user_message.id,
            "aborted",
            0,
        ));
        return Ok(PromptResponse {
            status: "aborted",
            model: resolved.model_id,
            role: None,
            content: None,
            message: None,
            usage: None,
            finish_reason: None,
        });
    };

    if let Some(error) = &outcome.error {
        return Err(ApiError::from_run_error(error));
    }

    persist_usage(&db, &session_id, &outcome);
    let assistant = db
        .messages()
        .create(&session_id, "assistant", outcome.answer())
        .map_err(ApiError::from_db)?;

    Ok(PromptResponse {
        status: "completed",
        model: resolved.model_id,
        role: Some("assistant"),
        content: Some(outcome.answer().to_owned()),
        message: Some(assistant),
        usage: outcome.usage,
        finish_reason: Some(outcome.finish_reason),
    })
}

struct ResolvedTurnProvider {
    model_id: String,
    model: ResolvedModel,
    provider: Arc<dyn Provider>,
}

fn resolve_turn_provider(
    source: &ProviderSource,
    session_model: Option<&str>,
) -> Result<ResolvedTurnProvider, ApiError> {
    match source {
        ProviderSource::OpenRouter => {
            let model = session_model
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .unwrap_or(providers::OPENROUTER_DEFAULT_MODEL);
            validate_model_id(model).map_err(ApiError::bad_request)?;
            let resolved = providers::resolve_model(Some(model));
            let inputs = providers::CredentialInputs::openrouter_from_process(None);
            let provider = providers::resolve_openrouter(&inputs).map_err(|diagnostic| {
                let status = match diagnostic.kind() {
                    "no-credential" => StatusCode::UNAUTHORIZED,
                    "net-disabled" => StatusCode::SERVICE_UNAVAILABLE,
                    _ => StatusCode::BAD_GATEWAY,
                };
                ApiError::diagnostic(status, diagnostic)
            })?;
            Ok(ResolvedTurnProvider {
                model_id: resolved.model_id.clone(),
                model: resolved,
                provider: Arc::new(provider),
            })
        }
        ProviderSource::Fixed { model_id, provider } => {
            let resolved = providers::resolve_model(Some(model_id));
            Ok(ResolvedTurnProvider {
                model_id: resolved.model_id.clone(),
                model: resolved,
                provider: provider.clone(),
            })
        }
    }
}

async fn run_cancellable_turn(
    provider: &dyn Provider,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
    mut abort_rx: watch::Receiver<bool>,
    bus: Arc<EventBus>,
    session_id: String,
    message_id: String,
) -> Option<RunOutcome> {
    let runner = SessionRunner::default();
    tokio::select! {
        outcome = runner.run_with_tools(provider, turn, dispatcher, move |event| {
            publish_runner_event(&bus, &session_id, &message_id, &event);
        }) => Some(outcome),
        () = wait_for_abort(&mut abort_rx) => None,
    }
}

fn publish_runner_event(bus: &EventBus, session_id: &str, message_id: &str, event: &RunnerEvent) {
    match event {
        RunnerEvent::TextDelta { text } => bus.publish(&Event::assistant_text_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::Reasoning { text } => bus.publish(&Event::assistant_reasoning_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::ToolCall {
            id,
            name,
            arguments,
        } => bus.publish(&Event::tool_call_for_turn(
            session_id.to_owned(),
            message_id.to_owned(),
            id.clone(),
            name.clone(),
            arguments.clone(),
        )),
        RunnerEvent::ToolResult {
            id, name, is_error, ..
        } => bus.publish(&Event::tool_result_for_turn(
            session_id.to_owned(),
            message_id.to_owned(),
            id.clone(),
            name.clone(),
            *is_error,
        )),
        RunnerEvent::Usage(usage) => bus.publish(&Event::turn_usage(
            session_id.to_owned(),
            message_id.to_owned(),
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.cost,
        )),
        RunnerEvent::Error { message, kind } => bus.publish(&Event::turn_error(
            session_id.to_owned(),
            message_id.to_owned(),
            kind.clone(),
            message.clone(),
        )),
        RunnerEvent::Done { reason, steps } => bus.publish(&Event::turn_done(
            session_id.to_owned(),
            message_id.to_owned(),
            finish_reason_name(reason),
            *steps,
        )),
        RunnerEvent::ToolSetChanged { .. } => {}
    }
}

fn finish_reason_name(reason: &llm::FinishReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{reason:?}"))
}

async fn wait_for_abort(abort_rx: &mut watch::Receiver<bool>) {
    loop {
        if *abort_rx.borrow() {
            break;
        }
        if abort_rx.changed().await.is_err() {
            break;
        }
    }
}

fn load_history(db: &Database, session_id: &str) -> Result<Vec<llm::Message>, ApiError> {
    let messages = db
        .messages()
        .list_for_session(session_id)
        .map_err(ApiError::from_db)?;
    let history = messages
        .into_iter()
        .map(|message| HistoryMessage::new(message.role, message.content))
        .collect::<Vec<_>>();
    Ok(project_history(&history))
}

fn persist_usage(db: &Database, session_id: &str, outcome: &RunOutcome) {
    let Some(usage) = outcome.usage.as_ref() else {
        return;
    };
    if let Err(err) = db.usage().record(
        session_id,
        u64::from(usage.input_tokens),
        u64::from(usage.output_tokens),
        u64::from(usage.total_tokens),
        usage.cost,
    ) {
        eprintln!("cortex serve: failed to persist usage for session {session_id}: {err}");
    }
}

fn compaction_policy(model: &ResolvedModel) -> CompactionPolicy {
    let (context, output) = match &model.info {
        Some(info) => (
            usize::try_from(info.limit.context).unwrap_or(usize::MAX),
            usize::try_from(info.limit.output).unwrap_or(0),
        ),
        None => (engine::DEFAULT_CONTEXT_LIMIT_TOKENS, 0),
    };
    CompactionPolicy::for_context_window(context, output)
}

fn validate_model_id(model_id: &str) -> Result<(), String> {
    let (provider, model) = model_id.split_once('/').unwrap_or(("", model_id));
    if provider.is_empty() || model.is_empty() {
        return Err(format!(
            "unknown model '{model_id}': expected a fully-qualified 'provider/model' id \
             (for example google/gemini-2.5-flash)"
        ));
    }
    Ok(())
}

fn permission_engine_for_turn(
    bus: Arc<EventBus>,
    session_id: &str,
    message_id: &str,
) -> Result<Option<Arc<PermissionEngine>>, ApiError> {
    let inputs = settings::ConfigInputs::from_process(BTreeMap::new()).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot load permission configuration context: {err}"),
        )
    })?;
    let mut ruleset = permissions::load_layered(&inputs).map_err(permission_config_error)?;
    let trust = policy::resolve_trust(&inputs).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot load trust policy: {err}"),
        )
    })?;
    for rule in policy::trust_ruleset(trust).rules() {
        ruleset.push(rule.clone());
    }
    if ruleset.is_empty() {
        return Ok(None);
    }
    Ok(Some(Arc::new(
        PermissionEngine::new(ruleset)
            .with_bus(bus)
            .with_event_context(session_id.to_owned(), message_id.to_owned()),
    )))
}

fn permission_config_error(err: RuleError) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("invalid permission configuration: {err}"),
    )
}

struct ServerToolDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
}

impl ServerToolDispatcher {
    fn for_model(
        cwd: PathBuf,
        model_id: &str,
        session_id: String,
        db_path: PathBuf,
        permissions: Option<Arc<PermissionEngine>>,
    ) -> Self {
        let registry = match ModelFamily::from_model_id(model_id) {
            ModelFamily::Gpt | ModelFamily::Codex => ToolRegistry::gpt(),
            ModelFamily::Anthropic | ModelFamily::Gemini | ModelFamily::Default => {
                ToolRegistry::builtin()
            }
        };
        let mut context = ToolContext::new(cwd)
            .with_session(session_id)
            .with_todo_store(Arc::new(ServerTodoStore { db_path }));
        if let Some(permissions) = permissions {
            context = context.with_permissions(permissions);
        }
        Self { registry, context }
    }

    async fn shutdown(&self) {
        self.context.background_jobs().shutdown().await;
    }
}

impl ToolDispatcher for ServerToolDispatcher {
    fn definitions(&self) -> Vec<ToolDef> {
        self.registry.definitions()
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
        Box::pin(async move {
            let output = self
                .registry
                .execute(&call.name, &call.arguments, &self.context)
                .await;
            ToolDispatchResult {
                content: output.model_content(),
                parts: output.parts,
                is_error: output.is_error,
            }
        })
    }
}

struct ServerTodoStore {
    db_path: PathBuf,
}

impl tools::TodoStore for ServerTodoStore {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        let db = Database::open(&self.db_path).map_err(|err| err.to_string())?;
        db.todos()
            .list_for_session(session_id)
            .map_err(|err| err.to_string())
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        let db = Database::open(&self.db_path).map_err(|err| err.to_string())?;
        db.todos()
            .replace_for_session(session_id, items)
            .map_err(|err| err.to_string())
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn loopback_share_origin(headers: &HeaderMap) -> Result<String, ApiError> {
    let host = headers
        .get(header::HOST)
        .ok_or_else(|| ApiError::bad_request("share requires a loopback Host header"))?
        .to_str()
        .map_err(|_| ApiError::bad_request("share Host header must be valid UTF-8"))?;
    let (hostname, port) = parse_loopback_host(host)?;
    Ok(format!("http://{hostname}:{port}"))
}

fn parse_loopback_host(host: &str) -> Result<(&'static str, u16), ApiError> {
    let trimmed = host.trim();
    let (hostname, port) = trimmed.rsplit_once(':').ok_or_else(|| {
        ApiError::bad_request("share Host header must include a loopback port in 3200-3299")
    })?;
    let port = port.parse::<u16>().map_err(|_| {
        ApiError::bad_request("share Host header must include a numeric port in 3200-3299")
    })?;
    if !(MIN_PORT..=MAX_PORT).contains(&port) {
        return Err(ApiError::bad_request(format!(
            "share Host header port {port} is outside the loopback range 3200-3299"
        )));
    }
    match hostname.trim() {
        "127.0.0.1" => Ok(("127.0.0.1", port)),
        host if host.eq_ignore_ascii_case("localhost") => Ok(("localhost", port)),
        "[::1]" => Ok(("127.0.0.1", port)),
        _ => Err(ApiError::bad_request(
            "share Host header must reference loopback (127.0.0.1 or localhost)",
        )),
    }
}

fn render_share_page(session: &Session, messages: &[Message]) -> String {
    let title = session.title.as_deref().unwrap_or(&session.id);
    let mut html = String::from(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    html.push_str("<title>");
    html.push_str(&escape_html(title));
    html.push_str("</title><style>");
    html.push_str(
        "body{font-family:system-ui,-apple-system,BlinkMacSystemFont,sans-serif;margin:2rem;\
         line-height:1.45;color:#111;background:#fff}main{max-width:860px}\
         article{border:1px solid #ddd;border-radius:8px;padding:1rem;margin:1rem 0}\
         h1{font-size:1.6rem}h2{font-size:.9rem;text-transform:uppercase;color:#555}\
         pre{white-space:pre-wrap;word-wrap:break-word;font-family:inherit}",
    );
    html.push_str("</style></head><body><main><h1>");
    html.push_str(&escape_html(title));
    html.push_str("</h1><p>Local loopback share for session <code>");
    html.push_str(&escape_html(&session.id));
    html.push_str("</code>.</p>");
    if messages.is_empty() {
        html.push_str("<p>No messages in this session.</p>");
    } else {
        for message in messages {
            html.push_str("<article><h2>");
            html.push_str(&escape_html(&message.role));
            html.push_str("</h2><pre>");
            html.push_str(&escape_html(&message.content));
            html.push_str("</pre></article>");
        }
    }
    html.push_str("</main></body></html>");
    html
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

impl ActiveTurns {
    fn insert(
        &self,
        session_id: String,
        abort: watch::Sender<bool>,
    ) -> Result<ActiveTurnGuard, String> {
        let mut active = self
            .inner
            .lock()
            .map_err(|_| "active turn registry is unavailable".to_owned())?;
        if active.contains_key(&session_id) {
            return Err(format!(
                "session {session_id} already has an in-flight turn; abort it before starting another"
            ));
        }
        active.insert(session_id.clone(), abort);
        Ok(ActiveTurnGuard {
            active: self.clone(),
            session_id,
        })
    }

    fn abort(&self, session_id: &str) -> Result<bool, ApiError> {
        let abort = self
            .inner
            .lock()
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "active turn registry is unavailable",
                )
            })?
            .get(session_id)
            .cloned();
        let Some(abort) = abort else {
            return Ok(false);
        };
        let _ = abort.send(true);
        Ok(true)
    }

    fn remove(&self, session_id: &str) {
        if let Ok(mut active) = self.inner.lock() {
            active.remove(session_id);
        }
    }
}

impl PromptBuffers {
    fn len(&self) -> usize {
        self.inner.lock().map(|buffers| buffers.len()).unwrap_or(0)
    }

    fn get(&self, session_id: &str) -> Result<String, ApiError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "prompt buffer registry is unavailable",
                )
            })?
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    fn append(&self, session_id: &str, text: &str) -> Result<String, ApiError> {
        let mut buffers = self.inner.lock().map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "prompt buffer registry is unavailable",
            )
        })?;
        let buffer = buffers.entry(session_id.to_owned()).or_default();
        buffer.push_str(text);
        Ok(buffer.clone())
    }

    fn clear(&self, session_id: &str) -> Result<(), ApiError> {
        self.inner
            .lock()
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "prompt buffer registry is unavailable",
                )
            })?
            .remove(session_id);
        Ok(())
    }

    fn take(&self, session_id: &str) -> Result<String, ApiError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "prompt buffer registry is unavailable",
                )
            })?
            .remove(session_id)
            .unwrap_or_default())
    }
}

impl LocalShares {
    fn len(&self) -> usize {
        self.inner.lock().map(|shares| shares.len()).unwrap_or(0)
    }

    fn create(&self, session_id: &str) -> Result<String, ApiError> {
        let share_id = format!(
            "shr_{}_{}",
            std::process::id(),
            LOCAL_SHARE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        self.inner
            .lock()
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "local share registry is unavailable",
                )
            })?
            .insert(share_id.clone(), session_id.to_owned());
        Ok(share_id)
    }

    fn session_for_share(&self, share_id: &str) -> Result<Option<String>, ApiError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "local share registry is unavailable",
                )
            })?
            .get(share_id)
            .cloned())
    }

    fn revoke_session(&self, session_id: &str) -> Result<bool, ApiError> {
        let mut shares = self.inner.lock().map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "local share registry is unavailable",
            )
        })?;
        let before = shares.len();
        shares.retain(|_, shared_session| shared_session != session_id);
        Ok(before != shares.len())
    }
}

struct ActiveTurnGuard {
    active: ActiveTurns,
    session_id: String,
}

impl Drop for ActiveTurnGuard {
    fn drop(&mut self) {
        self.active.remove(&self.session_id);
    }
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

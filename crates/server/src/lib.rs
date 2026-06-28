#![recursion_limit = "512"]

//! Axum HTTP surface for `cortex serve`.
//!
//! This crate owns the local loopback listener and lightweight HTTP routes. Session CRUD and prompt
//! turns are backed by the shared SQLite repositories and the core durable runner so the server
//! remains a thin surface over the engine and store.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{
        Html, IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{get, patch, post, put},
};
use db::{Database, DbError, Message, Session, UsageTotals};
use engine::{
    Agent, CompactionPolicy, EnvironmentBlock, HistoryMessage, ModelFamily, PartLifecycleTracker,
    RunError, RunOutcome, RunnerEvent, SessionRunner, SystemContext, ToolDispatchResult,
    ToolDispatcher, Turn, TurnQueue, build_assistant_message_parts, extract_message_content_parts,
    project_history,
};
use events::{
    Event, EventBus, LifecyclePhase, StreamBoundary, StreamDelta, Surface, ToolEnded, ToolStarted,
};
use futures_util::{future::BoxFuture, stream};
use llm::{Provider, ToolCall, ToolDef, Usage};
use permissions::{PermissionEngine, RuleError};
use protocol::{MessagePart as RichMessagePart, TodoItem};
use providers::ResolvedModel;
use redaction::guard_outbound;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{net::TcpListener, sync::broadcast, sync::watch};
use tools::{ToolContext, ToolRegistry};

mod auth_ext;
mod catalog_ext;
mod config_ext;
#[allow(dead_code)]
#[path = "../../cli/src/custom_agents.rs"]
mod custom_agents;
mod file_ext;
mod find_ext;
#[allow(dead_code)]
#[path = "../../cli/src/lsp_config.rs"]
mod lsp_config;
mod lsp_format_vcs_ext;
mod mcp_ext;
mod openapi;
mod project_ext;
mod pty_ext;
mod session_ext;
mod tui_control;

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
    cwd: PathBuf,
    audit_path: Option<PathBuf>,
    checkpoint_base: Option<PathBuf>,
    config: ConfigContext,
    provider: ProviderSource,
    auth: ServerAuth,
    active_turns: TurnQueue,
    remote_turns: RemoteTurns,
    event_bus: Arc<EventBus>,
    prompt_buffers: PromptBuffers,
    local_shares: LocalShares,
    permissions: PendingPermissions,
    pending_auth: auth_ext::PendingAuth,
    ptys: pty_ext::PtyRegistry,
    tui_control: tui_control::TuiControl,
    lsp_status_cache: lsp_format_vcs_ext::LspStatusCache,
}

impl ServerState {
    /// Build server state for an explicit SQLite database file.
    #[must_use]
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            audit_path: None,
            checkpoint_base: None,
            config: ConfigContext::from_process(),
            provider: ProviderSource::OpenRouter,
            auth: ServerAuth::trusted_loopback(),
            active_turns: TurnQueue::default(),
            remote_turns: RemoteTurns::default(),
            event_bus: Arc::new(EventBus::new()),
            prompt_buffers: PromptBuffers::default(),
            local_shares: LocalShares::default(),
            permissions: PendingPermissions::default(),
            pending_auth: auth_ext::PendingAuth::default(),
            ptys: pty_ext::PtyRegistry::default(),
            tui_control: tui_control::TuiControl::default(),
            lsp_status_cache: lsp_format_vcs_ext::LspStatusCache::default(),
        }
    }

    /// Build server state with a preconstructed provider, used by in-process route tests.
    #[must_use]
    pub fn with_provider(db_path: impl Into<PathBuf>, provider: Arc<dyn Provider>) -> Self {
        Self {
            db_path: db_path.into(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            audit_path: None,
            checkpoint_base: None,
            config: ConfigContext::from_process(),
            provider: ProviderSource::Fixed { provider },
            auth: ServerAuth::trusted_loopback(),
            active_turns: TurnQueue::default(),
            remote_turns: RemoteTurns::default(),
            event_bus: Arc::new(EventBus::new()),
            prompt_buffers: PromptBuffers::default(),
            local_shares: LocalShares::default(),
            permissions: PendingPermissions::default(),
            pending_auth: auth_ext::PendingAuth::default(),
            ptys: pty_ext::PtyRegistry::default(),
            tui_control: tui_control::TuiControl::default(),
            lsp_status_cache: lsp_format_vcs_ext::LspStatusCache::default(),
        }
    }

    /// Attach the event bus that backs `/event` and turn lifecycle publication.
    #[must_use]
    pub fn with_event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        self.event_bus = event_bus;
        self
    }

    /// Override the working directory used by tool execution and checkpoints.
    ///
    /// The process server path leaves this at the process current directory; in-process tests use
    /// it to avoid global `set_current_dir` races while still exercising real tools.
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = cwd.into();
        self
    }

    /// Override the audit JSONL path used by server-originated turns and commands.
    ///
    /// The process path normally resolves `CORTEX_AUDIT_LOG` on demand; tests inject a path to avoid
    /// mutating global environment while still proving the same durable sink behavior.
    #[must_use]
    pub fn with_audit_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.audit_path = Some(path.into());
        self
    }

    /// Override the environment snapshot used for layered config resolution.
    ///
    /// The process server captures its environment at startup; in-process tests use this to prove
    /// `CORTEX_*` precedence without mutating global process environment.
    #[must_use]
    pub fn with_config_env(mut self, env: Vec<(String, String)>) -> Self {
        self.config.env = env;
        self
    }

    /// Apply the CLI-level plugin pure-mode kill switch captured at server startup.
    ///
    /// This is separate from the environment snapshot so `cortex --pure serve` disables plugin
    /// resolution and hooks without requiring the caller to also export `CORTEX_PURE=1`.
    #[must_use]
    pub fn with_pure_plugins(mut self, pure_plugins: bool) -> Self {
        self.config.cli_pure_plugins = pure_plugins;
        self
    }

    /// Override the base directory for the shadow checkpoint store.
    ///
    /// The process server path uses the standard data-dir resolution. Tests can inject a hermetic
    /// base directory and later reopen the same store from another surface.
    #[must_use]
    pub fn with_checkpoint_base(mut self, base: impl Into<PathBuf>) -> Self {
        self.checkpoint_base = Some(base.into());
        self
    }

    /// Configure optional bearer-token auth and browser CORS policy for protected routes.
    #[must_use]
    pub fn with_auth(mut self, auth: ServerAuth) -> Self {
        self.auth = auth;
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
            .field("cwd", &self.cwd)
            .field("audit_path", &self.audit_path)
            .field("auth", &self.auth)
            .field("event_subscribers", &self.event_bus.subscriber_count())
            .field("prompt_buffers", &self.prompt_buffers.len())
            .field("local_shares", &self.local_shares.len())
            .field("pending_permissions", &self.permissions.len())
            .finish_non_exhaustive()
    }
}

/// Optional request-auth policy for `cortex serve`.
///
/// The default mode trusts the loopback listener and requires no request credential. Supplying a
/// token switches protected routes to `Authorization: Bearer <token>` validation; the raw token is
/// intentionally omitted from `Debug`, responses, and OpenAPI.
#[derive(Clone)]
pub struct ServerAuth {
    bearer_token: Option<Arc<str>>,
    cors: CorsPolicy,
}

impl ServerAuth {
    /// Trusted-loopback default: no token required and no browser CORS headers emitted.
    #[must_use]
    pub fn trusted_loopback() -> Self {
        Self {
            bearer_token: None,
            cors: CorsPolicy::Absent,
        }
    }

    /// Token-gated mode using the documented `Authorization: Bearer <token>` scheme.
    pub fn bearer_token(
        token: impl Into<String>,
        allow_loopback_cors: bool,
    ) -> Result<Self, ServeError> {
        let token = token.into();
        let token = token.trim();
        if token.is_empty() {
            return Err(ServeError::EmptyAuthToken);
        }
        Ok(Self {
            bearer_token: Some(Arc::<str>::from(token)),
            cors: if allow_loopback_cors {
                CorsPolicy::LoopbackOrigins
            } else {
                CorsPolicy::Absent
            },
        })
    }

    fn requires_auth(&self, method: &Method, path: &str) -> bool {
        self.bearer_token.is_some() && *method != Method::OPTIONS && !is_public_path(path)
    }

    fn accepts(&self, headers: &HeaderMap) -> bool {
        let Some(expected) = self.bearer_token.as_deref() else {
            return true;
        };
        let Some(header_value) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        else {
            return false;
        };
        let Some(token) = bearer_token_from_header(header_value) else {
            return false;
        };
        constant_time_eq(token.as_bytes(), expected.as_bytes())
    }

    fn cors_origin(&self, headers: &HeaderMap) -> Option<HeaderValue> {
        if self.cors != CorsPolicy::LoopbackOrigins {
            return None;
        }
        let origin = headers.get(header::ORIGIN)?;
        let origin_str = origin.to_str().ok()?;
        is_loopback_origin(origin_str).then(|| origin.clone())
    }
}

impl Default for ServerAuth {
    fn default() -> Self {
        Self::trusted_loopback()
    }
}

impl std::fmt::Debug for ServerAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerAuth")
            .field("token_configured", &self.bearer_token.is_some())
            .field(
                "token",
                &self.bearer_token.as_ref().map(|_| redaction::PLACEHOLDER),
            )
            .field("cors", &self.cors)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CorsPolicy {
    Absent,
    LoopbackOrigins,
}

#[derive(Clone)]
enum ProviderSource {
    OpenRouter,
    Fixed { provider: Arc<dyn Provider> },
}

#[derive(Clone)]
struct ConfigContext {
    home: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    env: Vec<(String, String)>,
    cli_pure_plugins: bool,
}

impl ConfigContext {
    fn from_process() -> Self {
        Self {
            home: std::env::var_os("HOME").map(PathBuf::from),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty()),
            env: std::env::vars().collect(),
            cli_pure_plugins: false,
        }
    }

    fn inputs(
        &self,
        cwd: &Path,
        cli_overrides: BTreeMap<String, String>,
    ) -> settings::ConfigInputs {
        settings::ConfigInputs {
            home: self.home.clone(),
            xdg_config_home: self.xdg_config_home.clone(),
            cwd: cwd.to_path_buf(),
            env: self.env.clone(),
            cli_overrides,
        }
    }

    fn pure_plugins(&self) -> bool {
        self.cli_pure_plugins
            || self
                .env
                .iter()
                .any(|(name, value)| name == "CORTEX_PURE" && value == "1")
    }
}

#[derive(Clone, Default)]
struct RemoteTurns {
    inner: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone, Default)]
struct PromptBuffers {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

#[derive(Clone, Default)]
struct LocalShares {
    inner: Arc<Mutex<HashMap<String, String>>>,
}

#[derive(Clone, Default)]
struct PendingPermissions {
    inner: Arc<Mutex<PendingPermissionState>>,
}

#[derive(Default)]
struct PendingPermissionState {
    next_id: u64,
    by_id: HashMap<String, Arc<PendingPermission>>,
}

struct PendingPermission {
    id: String,
    session_id: String,
    message_id: String,
    tool: String,
    subject: String,
    scope_tool: String,
    scope_pattern: String,
    created_at: Instant,
    reply: Mutex<Option<permissions::ScopedDecision>>,
    ready: Condvar,
}

impl PendingPermission {
    fn new(
        id: String,
        session_id: String,
        message_id: String,
        request: &permissions::PermissionRequest,
        scope_pattern: String,
    ) -> Self {
        Self {
            id,
            session_id,
            message_id,
            tool: request.tool.clone(),
            subject: request.subject.clone(),
            scope_tool: permissions::canonical_tool_name(&request.tool).to_owned(),
            scope_pattern,
            created_at: Instant::now(),
            reply: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    fn wait(&self) -> permissions::ScopedDecision {
        let mut guard = self.reply.lock().expect("pending permission lock");
        loop {
            if let Some(reply) = guard.clone() {
                return reply;
            }
            guard = self.ready.wait(guard).expect("pending permission wait");
        }
    }

    fn resolve(&self, reply: permissions::ScopedDecision) -> bool {
        let mut changed = false;
        if let Ok(mut guard) = self.reply.lock()
            && guard.is_none()
        {
            *guard = Some(reply);
            changed = true;
        }
        self.ready.notify_all();
        changed
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
    /// Token auth was requested with an empty token value.
    #[error("auth token cannot be empty")]
    EmptyAuthToken,
}

impl ServeError {
    /// Whether this failure came from invalid CLI input rather than runtime conditions.
    #[must_use]
    pub const fn is_usage_error(&self) -> bool {
        matches!(
            self,
            ServeError::NonLoopbackHost { .. }
                | ServeError::PortOutOfRange { .. }
                | ServeError::EmptyAuthToken
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
    let auth = state.auth.clone();
    Router::new()
        .route("/health", get(health).options(preflight))
        .route(
            "/openapi.json",
            get(openapi::openapi_json).options(preflight),
        )
        .route(
            "/event",
            get(event_stream).post(publish_event).options(preflight),
        )
        .route(
            "/session",
            get(list_sessions).post(create_session).options(preflight),
        )
        .route(
            "/session/status",
            get(session_ext::session_status_map).options(preflight),
        )
        .route(
            "/session/{id}",
            get(get_session)
                .patch(session_ext::update_session)
                .delete(delete_session)
                .options(preflight),
        )
        .route(
            "/session/{id}/status",
            get(session_ext::session_status).options(preflight),
        )
        .route(
            "/session/{id}/wait",
            get(session_ext::wait_session).options(preflight),
        )
        .route(
            "/session/{id}/context",
            get(session_ext::session_context).options(preflight),
        )
        .route(
            "/session/{id}/children",
            get(session_ext::list_children).options(preflight),
        )
        .route(
            "/session/{id}/diff",
            get(session_ext::session_diff).options(preflight),
        )
        .route(
            "/session/{id}/fork",
            post(session_ext::fork_session).options(preflight),
        )
        .route(
            "/session/{id}/init",
            post(session_ext::init_session).options(preflight),
        )
        .route(
            "/session/{id}/summarize",
            post(session_ext::summarize_session).options(preflight),
        )
        .route(
            "/session/{id}/compact",
            post(session_ext::compact_session).options(preflight),
        )
        .route(
            "/session/{id}/prompt_async",
            post(session_ext::prompt_async).options(preflight),
        )
        .route(
            "/session/{id}/shell",
            post(session_ext::shell_session).options(preflight),
        )
        .route(
            "/session/{id}/revert",
            post(session_ext::revert_session).options(preflight),
        )
        .route(
            "/session/{id}/unrevert",
            post(session_ext::unrevert_session).options(preflight),
        )
        .route(
            "/session/{id}/message",
            get(list_messages).post(post_message).options(preflight),
        )
        .route(
            "/session/{id}/message/{message_id}",
            get(session_ext::get_message)
                .delete(session_ext::delete_message)
                .options(preflight),
        )
        .route(
            "/session/{id}/message/{message_id}/part/{part_id}",
            patch(session_ext::update_part)
                .delete(session_ext::delete_part)
                .options(preflight),
        )
        .route("/session/{id}/todo", get(list_todos).options(preflight))
        .route("/session/{id}/todos", get(list_todos).options(preflight))
        .route(
            "/session/{id}/prompt",
            get(get_prompt_draft).options(preflight),
        )
        .route(
            "/session/{id}/prompt/append",
            post(append_prompt).options(preflight),
        )
        .route(
            "/session/{id}/prompt/submit",
            post(submit_prompt).options(preflight),
        )
        .route(
            "/session/{id}/prompt/clear",
            post(clear_prompt).options(preflight),
        )
        .route(
            "/session/{id}/command",
            post(execute_command).options(preflight),
        )
        .route(
            "/permission/pending",
            get(list_pending_permissions).options(preflight),
        )
        .route(
            "/permission/reply",
            post(reply_pending_permission).options(preflight),
        )
        .route(
            "/session/{id}/permission/pending",
            get(list_pending_permissions_for_session).options(preflight),
        )
        .route(
            "/session/{id}/permission/reply",
            post(reply_pending_permission_for_session).options(preflight),
        )
        .route(
            "/session/{id}/abort",
            post(abort_session).options(preflight),
        )
        .route("/session/{id}/share", post(create_share).options(preflight))
        .route(
            "/session/{id}/unshare",
            post(unshare_session).options(preflight),
        )
        .route("/share/{share_id}", get(get_share).options(preflight))
        .route(
            "/project/discovery",
            get(project_discovery).options(preflight),
        )
        .route(
            "/experimental/workspace/adapter",
            get(list_workspace_adapter_discovery).options(preflight),
        )
        .route(
            "/config",
            get(config_ext::get_config)
                .patch(config_ext::patch_config)
                .options(preflight),
        )
        .route(
            "/config/providers",
            get(config_ext::list_config_providers).options(preflight),
        )
        .route(
            "/provider",
            get(config_ext::list_providers).options(preflight),
        )
        .route("/auth", get(auth_ext::list_auth).options(preflight))
        .route(
            "/auth/callback",
            get(auth_ext::oauth_callback).options(preflight),
        )
        .route(
            "/auth/{provider}",
            put(auth_ext::set_auth)
                .delete(auth_ext::remove_auth)
                .options(preflight),
        )
        .route(
            "/auth/{provider}/authorize",
            post(auth_ext::oauth_authorize).options(preflight),
        )
        .route("/file", get(file_ext::list_files).options(preflight))
        .route("/file/content", get(file_ext::read_file).options(preflight))
        .route(
            "/file/status",
            get(file_ext::file_status).options(preflight),
        )
        .route("/find", get(find_ext::find_text).options(preflight))
        .route("/find/file", get(find_ext::find_file).options(preflight))
        .route(
            "/find/symbol",
            get(find_ext::find_symbol).options(preflight),
        )
        .route("/agent", get(catalog_ext::list_agents).options(preflight))
        .route("/model", get(catalog_ext::list_models).options(preflight))
        .route(
            "/command",
            get(catalog_ext::list_commands).options(preflight),
        )
        .route("/skill", get(catalog_ext::list_skills).options(preflight))
        .route("/mcp", get(mcp_ext::list_mcp).options(preflight))
        .route(
            "/lsp",
            get(lsp_format_vcs_ext::lsp_status).options(preflight),
        )
        .route(
            "/formatter",
            get(lsp_format_vcs_ext::formatter_status).options(preflight),
        )
        .route(
            "/vcs/diff",
            get(lsp_format_vcs_ext::vcs_diff).options(preflight),
        )
        .route(
            "/vcs/apply",
            post(lsp_format_vcs_ext::vcs_apply).options(preflight),
        )
        .route(
            "/project",
            get(project_ext::list_projects).options(preflight),
        )
        .route(
            "/project/current",
            get(project_ext::current_project_route).options(preflight),
        )
        .route(
            "/workspace",
            get(project_ext::current_workspace).options(preflight),
        )
        .route(
            "/worktree",
            get(project_ext::list_worktrees)
                .post(project_ext::create_worktree)
                .delete(project_ext::remove_worktree)
                .options(preflight),
        )
        .route("/path", get(project_ext::path_info).options(preflight))
        .route(
            "/pty",
            get(pty_ext::list_ptys)
                .post(pty_ext::create_pty)
                .options(preflight),
        )
        .route(
            "/pty/{id}",
            get(pty_ext::get_pty)
                .delete(pty_ext::close_pty)
                .options(preflight),
        )
        .route(
            "/pty/{id}/input",
            post(pty_ext::pty_input).options(preflight),
        )
        .route(
            "/pty/{id}/resize",
            post(pty_ext::pty_resize).options(preflight),
        )
        .route(
            "/pty/{id}/stream",
            get(pty_ext::pty_stream).options(preflight),
        )
        .route(
            "/tui/append-prompt",
            post(tui_control::append_prompt).options(preflight),
        )
        .route(
            "/tui/submit-prompt",
            post(tui_control::submit_prompt).options(preflight),
        )
        .route(
            "/tui/clear-prompt",
            post(tui_control::clear_prompt).options(preflight),
        )
        .route(
            "/tui/open-models",
            post(tui_control::open_models).options(preflight),
        )
        .route(
            "/tui/execute-command",
            post(tui_control::execute_command).options(preflight),
        )
        .route(
            "/tui/show-toast",
            post(tui_control::show_toast).options(preflight),
        )
        .route(
            "/tui/select-session",
            post(tui_control::select_session).options(preflight),
        )
        .route(
            "/tui/control/next",
            get(tui_control::control_next).options(preflight),
        )
        .route(
            "/tui/control/response",
            post(tui_control::control_response).options(preflight),
        )
        .with_state(state)
        .layer(middleware::from_fn_with_state(auth, auth_and_cors))
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
    let ptys = state.ptys.clone();
    let result = serve_with_shutdown(
        listener,
        app_with_state(state.with_event_bus(bus.clone())),
        shutdown_signal(bus, version),
    )
    .await;
    // Kill and reap every PTY child by its exact handle so no terminal child is orphaned on exit.
    ptys.shutdown();
    result
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn auth_and_cors(State(auth): State<ServerAuth>, request: Request, next: Next) -> Response {
    let cors_origin = auth.cors_origin(request.headers());

    if request.method() == Method::OPTIONS {
        return preflight_response(cors_origin).into_response();
    }

    if auth.requires_auth(request.method(), request.uri().path())
        && !auth.accepts(request.headers())
    {
        return unauthorized_response(cors_origin);
    }

    let mut response = next.run(request).await;
    if let Some(origin) = cors_origin {
        add_cors_headers(response.headers_mut(), origin, false);
    }
    response
}

fn preflight_response(cors_origin: Option<HeaderValue>) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Some(origin) = cors_origin {
        add_cors_headers(response.headers_mut(), origin, true);
    }
    response
}

fn unauthorized_response(cors_origin: Option<HeaderValue>) -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"cortex\"")],
        Json(ErrorResponse {
            error: "authentication required".to_owned(),
            kind: Some("auth".to_owned()),
        }),
    )
        .into_response();
    if let Some(origin) = cors_origin {
        add_cors_headers(response.headers_mut(), origin, false);
    }
    response
}

fn add_cors_headers(headers: &mut HeaderMap, origin: HeaderValue, preflight: bool) {
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    if preflight {
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
    }
}

fn is_public_path(path: &str) -> bool {
    matches!(path, "/health" | "/openapi.json" | "/auth/callback")
}

fn bearer_token_from_header(header_value: &str) -> Option<&str> {
    let mut parts = header_value.trim().splitn(2, char::is_whitespace);
    let scheme = parts.next()?;
    let token = parts.next()?.trim();
    if token.is_empty() {
        return None;
    }
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

fn is_loopback_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or(rest);
    if authority.eq_ignore_ascii_case("localhost")
        || authority == "127.0.0.1"
        || authority == "[::1]"
    {
        return true;
    }
    if let Some(portless) = authority.strip_prefix("[::1]:") {
        return portless.parse::<u16>().is_ok();
    }
    let Some((host, port)) = authority.rsplit_once(':') else {
        return false;
    };
    (host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1") && port.parse::<u16>().is_ok()
}

async fn event_stream(
    State(state): State<ServerState>,
) -> Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>> {
    let stream_state = EventStreamState::new(state.event_bus.subscribe_stream());
    let stream = stream::unfold(stream_state, |mut stream_state| async move {
        loop {
            if let Some(frame) = stream_state.pending_frames.pop_front() {
                return Some((Ok(frame), stream_state));
            }

            match stream_state.receiver.recv().await {
                Ok(event) => {
                    stream_state.enqueue(event);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

struct EventStreamState {
    receiver: broadcast::Receiver<Event>,
    redactors: HashMap<StreamRedactionKey, redaction::StreamingRedactor>,
    started: HashSet<StreamRedactionKey>,
    pending_frames: VecDeque<SseEvent>,
}

impl EventStreamState {
    fn new(receiver: broadcast::Receiver<Event>) -> Self {
        Self {
            receiver,
            redactors: HashMap::new(),
            started: HashSet::new(),
            pending_frames: VecDeque::new(),
        }
    }

    fn push(&mut self, event: Event) {
        self.pending_frames.push_back(redacted_sse_frame(event));
    }

    fn enqueue(&mut self, event: Event) {
        match event {
            Event::TurnStarted(payload) => {
                self.clear_message(&payload.session_id, &payload.message_id);
                let session_id = payload.session_id.clone();
                self.push(Event::TurnStarted(payload));
                self.push(Event::session_status(session_id, "busy"));
            }
            Event::AssistantTextDelta(mut payload) => {
                let key = StreamRedactionKey::text(&payload.session_id, &payload.message_id);
                payload.text = self.redactors.entry(key).or_default().push(&payload.text);
                self.emit_stream_fragment(
                    StreamChannel::Text,
                    payload.session_id,
                    payload.message_id,
                    payload.text,
                );
            }
            Event::AssistantReasoningDelta(mut payload) => {
                let key = StreamRedactionKey::reasoning(&payload.session_id, &payload.message_id);
                payload.text = self.redactors.entry(key).or_default().push(&payload.text);
                self.emit_stream_fragment(
                    StreamChannel::Reasoning,
                    payload.session_id,
                    payload.message_id,
                    payload.text,
                );
            }
            Event::ToolCall(payload) => {
                let compat = match (&payload.session_id, &payload.message_id) {
                    (Some(session_id), Some(message_id)) => {
                        Some(Event::SessionNextToolStarted(ToolStarted {
                            session_id: session_id.clone(),
                            message_id: message_id.clone(),
                            id: payload.id.clone(),
                            name: payload.name.clone(),
                        }))
                    }
                    _ => None,
                };
                self.push(Event::ToolCall(payload));
                if let Some(compat) = compat {
                    self.push(compat);
                }
            }
            Event::ToolResult(payload) => {
                let compat = match (&payload.session_id, &payload.message_id) {
                    (Some(session_id), Some(message_id)) => {
                        Some(Event::SessionNextToolEnded(ToolEnded {
                            session_id: session_id.clone(),
                            message_id: message_id.clone(),
                            id: payload.id.clone(),
                            name: payload.name.clone(),
                            is_error: payload.is_error,
                        }))
                    }
                    _ => None,
                };
                self.push(Event::ToolResult(payload));
                if let Some(compat) = compat {
                    self.push(compat);
                }
            }
            Event::TurnError(payload) => {
                self.flush_message(&payload.session_id, &payload.message_id);
                let session_id = payload.session_id.clone();
                let message_id = payload.message_id.clone();
                let error_kind = payload.error_kind.clone();
                let message = payload.message.clone();
                self.push(Event::TurnError(payload));
                self.push(Event::session_error(
                    session_id.clone(),
                    Some(message_id),
                    error_kind,
                    message,
                ));
                self.push(Event::session_idle(session_id.clone()));
                self.push(Event::session_status(session_id, "idle"));
            }
            Event::TurnDone(payload) => {
                self.flush_message(&payload.session_id, &payload.message_id);
                let session_id = payload.session_id.clone();
                self.push(Event::TurnDone(payload));
                self.push(Event::session_idle(session_id.clone()));
                self.push(Event::session_status(session_id, "idle"));
            }
            other => self.push(other),
        }
    }

    /// Emit one (already stream-redacted) text/reasoning fragment as both the legacy
    /// `assistant.*_delta` vocabulary and the compatible `session.next.*.delta` vocabulary,
    /// emitting the channel's `start` marker once before the first visible fragment.
    fn emit_stream_fragment(
        &mut self,
        channel: StreamChannel,
        session_id: String,
        message_id: String,
        text: String,
    ) {
        if text.is_empty() {
            return;
        }
        let key = StreamRedactionKey::new(&session_id, &message_id, channel);
        if self.started.insert(key) {
            let boundary = StreamBoundary {
                session_id: session_id.clone(),
                message_id: message_id.clone(),
            };
            let start = match channel {
                StreamChannel::Text => Event::SessionNextTextStart(boundary),
                StreamChannel::Reasoning => Event::SessionNextReasoningStart(boundary),
            };
            self.push(start);
        }
        let legacy = match channel {
            StreamChannel::Text => {
                Event::assistant_text_delta(&session_id, &message_id, text.clone())
            }
            StreamChannel::Reasoning => {
                Event::assistant_reasoning_delta(&session_id, &message_id, text.clone())
            }
        };
        self.push(legacy);
        let delta = StreamDelta {
            session_id,
            message_id,
            text,
        };
        let next = match channel {
            StreamChannel::Text => Event::SessionNextTextDelta(delta),
            StreamChannel::Reasoning => Event::SessionNextReasoningDelta(delta),
        };
        self.push(next);
    }

    fn flush_message(&mut self, session_id: &str, message_id: &str) {
        for channel in [StreamChannel::Text, StreamChannel::Reasoning] {
            let key = StreamRedactionKey::new(session_id, message_id, channel);
            if let Some(mut redactor) = self.redactors.remove(&key) {
                let text = redactor.finish();
                if !text.is_empty() {
                    self.emit_stream_fragment(
                        channel,
                        session_id.to_owned(),
                        message_id.to_owned(),
                        text,
                    );
                }
            }
            if self.started.remove(&key) {
                let boundary = StreamBoundary {
                    session_id: session_id.to_owned(),
                    message_id: message_id.to_owned(),
                };
                let end = match channel {
                    StreamChannel::Text => Event::SessionNextTextEnd(boundary),
                    StreamChannel::Reasoning => Event::SessionNextReasoningEnd(boundary),
                };
                self.push(end);
            }
        }
    }

    fn clear_message(&mut self, session_id: &str, message_id: &str) {
        for channel in [StreamChannel::Text, StreamChannel::Reasoning] {
            let key = StreamRedactionKey::new(session_id, message_id, channel);
            self.redactors.remove(&key);
            self.started.remove(&key);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StreamRedactionKey {
    session_id: String,
    message_id: String,
    channel: StreamChannel,
}

impl StreamRedactionKey {
    fn new(session_id: &str, message_id: &str, channel: StreamChannel) -> Self {
        Self {
            session_id: session_id.to_owned(),
            message_id: message_id.to_owned(),
            channel,
        }
    }

    fn text(session_id: &str, message_id: &str) -> Self {
        Self::new(session_id, message_id, StreamChannel::Text)
    }

    fn reasoning(session_id: &str, message_id: &str) -> Self {
        Self::new(session_id, message_id, StreamChannel::Reasoning)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum StreamChannel {
    Text,
    Reasoning,
}

fn redacted_sse_frame(event: Event) -> SseEvent {
    let kind = event.kind();
    let data = serde_json::to_string(&event).unwrap_or_else(|err| {
        serde_json::json!({
            "kind": "event.serialization_error",
            "message": err.to_string(),
        })
        .to_string()
    });
    let data = redaction::redact(&data);
    SseEvent::default().event(kind).data(data)
}

async fn publish_event(
    State(state): State<ServerState>,
    Json(event): Json<Event>,
) -> Json<EventPublishResponse> {
    state.remote_turns.observe(&event);
    let kind = event.kind();
    state.event_bus.publish(&event);
    Json(EventPublishResponse {
        status: "published",
        kind,
    })
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
    model: Option<String>,
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
    fn model_override(&self) -> Option<String> {
        self.model.as_deref().and_then(non_empty)
    }

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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageListQuery {
    /// Maximum number of messages to return; omitted means all remaining.
    #[serde(default)]
    limit: Option<usize>,
    /// Return only messages that appear strictly after this message id (forward paging).
    #[serde(default)]
    cursor: Option<String>,
    /// Reference-compatible alias for `cursor`: return messages strictly after this id.
    #[serde(default)]
    after: Option<String>,
}

#[derive(Debug, Serialize)]
struct MessageListResponse {
    session: Session,
    messages: Vec<Message>,
    usage: UsageTotals,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct TodoListResponse {
    session: Session,
    todos: Vec<TodoItem>,
}

#[derive(Debug, Serialize)]
struct ProjectDiscoveryResponse {
    directory: String,
    workspace_adapters: Vec<WorkspaceAdapterDiscovery>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct WorkspaceAdapterDiscovery {
    plugin: String,
    #[serde(rename = "type")]
    workspace_type: String,
    name: String,
    description: String,
}

impl From<plugins::PluginWorkspaceAdapterEntry> for WorkspaceAdapterDiscovery {
    fn from(entry: plugins::PluginWorkspaceAdapterEntry) -> Self {
        Self {
            plugin: redaction::redact(&entry.plugin),
            workspace_type: redaction::redact(&entry.workspace.workspace_type),
            name: redaction::redact(&entry.workspace.name),
            description: redaction::redact(&entry.workspace.description),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingPermissionResponse {
    pending: Vec<PendingPermissionItem>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingPermissionItem {
    id: String,
    session_id: String,
    message_id: String,
    tool: String,
    subject: String,
    waited_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PermissionReplyRequest {
    id: String,
    decision: String,
    #[serde(default)]
    always: bool,
    #[serde(default)]
    cascade: bool,
    #[serde(default)]
    feedback: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionReplyResponse {
    status: &'static str,
    id: String,
    resolved: usize,
}

#[derive(Debug, Serialize)]
struct PromptResponse {
    status: &'static str,
    model: String,
    agent: String,
    queued: bool,
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
struct EventPublishResponse {
    status: &'static str,
    kind: &'static str,
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
    axum::extract::Query(query): axum::extract::Query<MessageListQuery>,
) -> Result<Json<MessageListResponse>, ApiError> {
    let db = open_database(&state)?;
    let session = db.sessions().require(&id).map_err(ApiError::from_db)?;
    let mut messages = db
        .messages()
        .list_for_session(&id)
        .map_err(ApiError::from_db)?;
    let usage = db.usage().session_totals(&id).map_err(ApiError::from_db)?;

    let cursor = query.cursor.as_deref().or(query.after.as_deref());
    if let Some(cursor) = cursor {
        match messages.iter().position(|message| message.id == cursor) {
            Some(index) => {
                messages.drain(..=index);
            }
            None => {
                return Err(ApiError::bad_request(format!(
                    "cursor `{cursor}` does not match any message in this session"
                )));
            }
        }
    }

    let mut next_cursor = None;
    if let Some(limit) = query.limit
        && messages.len() > limit
    {
        messages.truncate(limit);
        next_cursor = messages.last().map(|message| message.id.clone());
    }

    Ok(Json(MessageListResponse {
        session,
        messages,
        usage,
        next_cursor,
    }))
}

async fn list_todos(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<TodoListResponse>, ApiError> {
    let db = open_database(&state)?;
    let session = db.sessions().require(&id).map_err(ApiError::from_db)?;
    let todos = db
        .todos()
        .list_for_session(&id)
        .map_err(ApiError::from_db)?;
    Ok(Json(TodoListResponse { session, todos }))
}

async fn project_discovery(
    State(state): State<ServerState>,
) -> Result<Json<ProjectDiscoveryResponse>, ApiError> {
    let workspace_adapters = load_workspace_adapter_discovery(&state)?;
    Ok(Json(ProjectDiscoveryResponse {
        directory: redaction::redact(&state.cwd.to_string_lossy()),
        workspace_adapters,
    }))
}

async fn list_workspace_adapter_discovery(
    State(state): State<ServerState>,
) -> Result<Json<Vec<WorkspaceAdapterDiscovery>>, ApiError> {
    Ok(Json(load_workspace_adapter_discovery(&state)?))
}

async fn list_pending_permissions(
    State(state): State<ServerState>,
) -> Result<Json<PendingPermissionResponse>, ApiError> {
    let pending = state.permissions.list(None)?;
    Ok(Json(PendingPermissionResponse { pending }))
}

async fn list_pending_permissions_for_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PendingPermissionResponse>, ApiError> {
    require_session(&state, &id)?;
    let pending = state.permissions.list(Some(id.as_str()))?;
    Ok(Json(PendingPermissionResponse { pending }))
}

async fn reply_pending_permission(
    State(state): State<ServerState>,
    Json(payload): Json<PermissionReplyRequest>,
) -> Result<Json<PermissionReplyResponse>, ApiError> {
    let (decision, scope) = parse_permission_reply(&payload)?;
    let id = payload.id.clone();
    let resolved = state.permissions.reply(
        &payload.id,
        None,
        permissions::ScopedDecision {
            decision,
            scope,
            feedback: payload.feedback.clone(),
        },
        payload.cascade,
    )?;
    Ok(Json(PermissionReplyResponse {
        status: "resolved",
        id,
        resolved,
    }))
}

async fn reply_pending_permission_for_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<PermissionReplyRequest>,
) -> Result<Json<PermissionReplyResponse>, ApiError> {
    require_session(&state, &id)?;
    let (decision, scope) = parse_permission_reply(&payload)?;
    let request_id = payload.id.clone();
    let resolved = state.permissions.reply(
        &payload.id,
        Some(id.as_str()),
        permissions::ScopedDecision {
            decision,
            scope,
            feedback: payload.feedback.clone(),
        },
        payload.cascade,
    )?;
    Ok(Json(PermissionReplyResponse {
        status: "resolved",
        id: request_id,
        resolved,
    }))
}

fn parse_permission_reply(
    payload: &PermissionReplyRequest,
) -> Result<(permissions::Decision, permissions::DecisionScope), ApiError> {
    let decision = match payload.decision.trim().to_ascii_lowercase().as_str() {
        "allow" => permissions::Decision::Allow,
        "deny" => permissions::Decision::Deny,
        other => {
            return Err(ApiError::bad_request(format!(
                "permission reply decision must be allow or deny, got `{other}`"
            )));
        }
    };
    let scope = if payload.always {
        permissions::DecisionScope::Always
    } else {
        permissions::DecisionScope::Once
    };
    Ok((decision, scope))
}

async fn post_message(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<MessageRequest>,
) -> Result<Json<PromptResponse>, ApiError> {
    let model = payload.model_override();
    let prompt = payload
        .into_prompt()
        .ok_or_else(|| ApiError::bad_request("message body must include non-empty text"))?;
    run_message_turn(state, id, prompt, model).await.map(Json)
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
    run_message_turn(state, id, prompt, None).await.map(Json)
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
    let output = run_session_bash(&state, &id, &command).await?;
    let is_error = output.is_error;
    state.event_bus.publish(&Event::command_executed(
        Some(id.clone()),
        redaction::redact(&command),
        Some(i32::from(is_error)),
    ));
    Ok(Json(CommandResponse {
        status: if is_error { "error" } else { "completed" },
        command: redaction::redact(&command),
        output: redaction::redact(&output.model_content()),
        is_error,
    }))
}

/// Run a single bash command in the session's working directory through the real tool registry,
/// publishing tool-call/tool-result events and honoring the session's permission ruleset and audit
/// sink. Shared by the `/command` and `/shell` routes.
pub(crate) async fn run_session_bash(
    state: &ServerState,
    id: &str,
    command: &str,
) -> Result<tools::ToolOutput, ApiError> {
    let call_id = format!(
        "remote-command-{}-{}",
        std::process::id(),
        REMOTE_COMMAND_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let arguments = serde_json::json!({ "command": command }).to_string();
    state.event_bus.publish(&Event::tool_call_for_turn(
        id.to_owned(),
        call_id.clone(),
        call_id.clone(),
        "bash",
        arguments.clone(),
    ));

    let config_inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let effective_config = settings::load(config_inputs.clone()).map_err(config_api_error)?;
    let selected_agent = resolve_turn_agent(&config_inputs, effective_config.agent(), "serve")
        .map_err(ApiError::bad_request)?;
    let turn_permissions = permission_engine_for_turn(
        state.event_bus.clone(),
        state.permissions.clone(),
        id,
        &call_id,
        &config_inputs,
        &selected_agent,
    )?;
    let audit_sink = audit_sink_for(state, id);
    let mut context = ToolContext::new(state.cwd.clone())
        .with_session(id.to_owned())
        .with_todo_store(Arc::new(ServerTodoStore {
            db_path: state.db_path.clone(),
        }))
        .with_audit(audit_sink)
        .with_shell_env(scoped_shell_env(&state.cwd));
    if let Some(permission_engine) = turn_permissions.engine() {
        context = context.with_permissions(permission_engine);
    }
    let output = ToolRegistry::builtin()
        .execute("bash", &arguments, &context)
        .await;
    context.background_jobs().shutdown().await;
    turn_permissions.persist_if_changed();
    state.event_bus.publish(&Event::tool_result_for_turn(
        id.to_owned(),
        call_id.clone(),
        call_id,
        "bash",
        output.is_error,
    ));
    Ok(output)
}

/// Load the project-scoped environment store for `cwd` as owned `(OsString, OsString)` pairs, ready
/// to merge into a tool/command child environment. Keyed by the canonical workspace key (the same
/// key the CLI uses), so a value set with `cortex env set` reaches the server `shell`/`command`
/// routes and `cortex env remove` makes it no longer appear.
fn scoped_shell_env(cwd: &std::path::Path) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    settings::env_store::load_for(&checkpoints::workspace_key(cwd))
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
}

async fn abort_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<AbortResponse>, ApiError> {
    let db = open_database(&state)?;
    db.sessions().require(&id).map_err(ApiError::from_db)?;
    let server_active = state
        .active_turns
        .abort(&id)
        .map_err(|message| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, message))?;
    let _resolved_pending = state.permissions.cancel_session(
        &id,
        permissions::ScopedDecision::once(permissions::Decision::Deny),
    )?;
    let remote_active = state.remote_turns.is_active(&id)?;
    if remote_active && let Err(err) = write_remote_abort_request(&id) {
        eprintln!("cortex serve: failed to write remote abort marker for session {id}: {err}");
    }
    Ok(Json(AbortResponse {
        status: "abort_requested",
        active: server_active || remote_active,
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
    headers: HeaderMap,
) -> Result<Response, ApiError> {
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
    if wants_share_json(&headers) {
        return Ok(Json(share_artifact(&session, &messages)).into_response());
    }
    Ok(Html(render_share_page(&session, &messages)).into_response())
}

/// Whether the client prefers the importable JSON artifact over the rendered HTML share page.
///
/// A share link opened in a browser sends `Accept: text/html` and gets the HTML view; the
/// `cortex import <url>` client sends `Accept: application/json` and gets the artifact that
/// re-imports into a local database.
fn wants_share_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.to_ascii_lowercase().contains("application/json"))
}

/// The importable share artifact, wire-compatible with a `cortex export` document so the headless
/// `cortex import` reader accepts it unchanged. Secret-shaped values are redacted at the boundary.
#[derive(serde::Serialize)]
struct ShareArtifact {
    version: u32,
    session: ShareArtifactSession,
    messages: Vec<ShareArtifactMessage>,
}

#[derive(serde::Serialize)]
struct ShareArtifactSession {
    id: String,
    title: Option<String>,
    model: Option<String>,
}

#[derive(serde::Serialize)]
struct ShareArtifactMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    parts: Vec<RichMessagePart>,
}

/// Version stamp the headless `cortex export`/`import` artifact contract expects.
const SHARE_ARTIFACT_VERSION: u32 = 1;

fn share_artifact(session: &Session, messages: &[Message]) -> ShareArtifact {
    ShareArtifact {
        version: SHARE_ARTIFACT_VERSION,
        session: ShareArtifactSession {
            id: session.id.clone(),
            title: session.title.as_deref().map(redaction::redact),
            model: session.model.as_deref().map(redaction::redact),
        },
        messages: messages
            .iter()
            .map(|message| ShareArtifactMessage {
                role: redaction::redact(&message.role),
                content: redaction::redact(&message.content),
                parts: message.parts.clone(),
            })
            .collect(),
    }
}

fn open_database(state: &ServerState) -> Result<Database, ApiError> {
    Database::open(state.db_path()).map_err(ApiError::from_db)
}

fn audit_sink_for(state: &ServerState, session_id: &str) -> Arc<audit::AuditSink> {
    let sink = match &state.audit_path {
        Some(path) => audit::AuditSink::new(path, session_id),
        None => audit::AuditSink::from_env_or_cwd(session_id, &state.cwd),
    };
    Arc::new(sink.with_surface_and_source("serve", "server"))
}

fn require_session(state: &ServerState, id: &str) -> Result<Session, ApiError> {
    let db = open_database(state)?;
    db.sessions().require(id).map_err(ApiError::from_db)
}

async fn run_message_turn(
    state: ServerState,
    session_id: String,
    prompt: String,
    request_model: Option<String>,
) -> Result<PromptResponse, ApiError> {
    let db = open_database(&state)?;
    let session = db
        .sessions()
        .require(&session_id)
        .map_err(ApiError::from_db)?;
    let config_inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let effective_config = settings::load(config_inputs.clone()).map_err(config_api_error)?;
    let project_memory = memory::discover(&config_inputs);
    for warning in project_memory.warnings() {
        eprintln!("cortex serve: {warning}");
    }
    if project_memory.net_blocked() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "network is disabled by CORTEX_NO_NET=1; remote instruction URLs cannot be fetched"
                .to_owned(),
        ));
    }
    let selected_agent = resolve_turn_agent(&config_inputs, effective_config.agent(), "serve")
        .map_err(ApiError::bad_request)?;
    let selected_model = select_turn_model(
        request_model.as_deref(),
        selected_agent.model(),
        session.model.as_deref(),
        effective_config.model(),
    )
    .map_err(ApiError::bad_request)?;
    let provider_config = providers::ProviderConfigSet::from_table(effective_config.table())
        .map_err(provider_config_api_error)?;
    let resolved_for_guard = provider_config
        .resolve_model(providers::Catalogue::embedded(), Some(&selected_model))
        .map_err(provider_config_api_error)?;
    let audit_sink = audit_sink_for(&state, &session_id);

    // The kill-switch wins before credential resolution, prompt persistence, or any outbound
    // provider setup, so `CORTEX_NO_NET=1` leaves the server healthy but produces no assistant.
    if let Err(diagnostic) = guard_outbound(OUTBOUND_OPERATION) {
        let endpoint = provider_config
            .resolve_stream_endpoint(
                &resolved_for_guard.provider_id,
                &resolved_for_guard.model_id,
            )
            .unwrap_or_else(|_| {
                format!(
                    "{}/chat/completions",
                    providers::openrouter_base_url().trim_end_matches('/')
                )
            });
        let host = audit::host_from_endpoint(&endpoint).unwrap_or_default();
        audit_sink.record_network_call(
            "llm",
            Some(&resolved_for_guard.provider_id),
            Some(&selected_model),
            "POST",
            &host,
            &endpoint,
            None,
            diagnostic.kind(),
        );
        return Err(ApiError::diagnostic(
            StatusCode::SERVICE_UNAVAILABLE,
            diagnostic,
        ));
    }

    let plugin_config =
        plugins::PluginConfig::from_toml_value(effective_config.get("plugin"), &state.cwd)
            .map_err(plugin_config_error)?;
    // Attach the shared audit sink so the server's plugin lifecycle/event-hook activity lands in
    // the same sink as headless and the TUI, attributed to the "serve"/"server" surface.
    let plugin_hooks = plugins::PluginEventHooks::load_or_disabled(
        &plugin_config,
        state.config.pure_plugins(),
        "serve",
    )
    .map_err(plugin_runtime_error)?
    .with_audit(audit_sink.clone());
    let resolved = resolve_turn_provider(
        &state.provider,
        &selected_model,
        &provider_config,
        &config_inputs,
    )?;
    let history = load_history(&db, &session_id)?;
    let active_turn = state
        .active_turns
        .acquire(session_id.clone())
        .await
        .map_err(|message| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, message))?;
    let queued = active_turn.queued();
    let user_message = db
        .messages()
        .create_with_parts(
            &session_id,
            "user",
            &prompt,
            &[RichMessagePart::text(prompt.clone())],
        )
        .map_err(ApiError::from_db)?;
    let started_event = Event::turn_started(&session_id, &user_message.id);
    state.event_bus.publish(&started_event);
    plugin_hooks.invoke_event(&started_event);
    state.event_bus.publish(&Event::message_updated(
        &session_id,
        &user_message.id,
        &user_message.role,
        Some(user_message.created_at.clone()),
    ));
    state.event_bus.publish(&Event::session_model_switched(
        &session_id,
        &user_message.id,
        &resolved.model_id,
    ));
    state.event_bus.publish(&Event::session_agent_switched(
        &session_id,
        &user_message.id,
        selected_agent.name(),
    ));
    state.event_bus.publish(&Event::session_step_started(
        &session_id,
        &user_message.id,
        1,
    ));

    let system =
        render_system_context(&resolved.model_id, &selected_agent, project_memory.render());
    let mut turn = Turn::new(&resolved.model_id, system, prompt)
        .with_history(history)
        .with_compaction(compaction_policy(&resolved.model));
    if !resolved.request_params.is_empty() {
        turn = turn.with_extra_params(resolved.request_params.clone());
    }
    if let Some(temperature) = selected_agent.temperature() {
        turn = turn.with_temperature(temperature);
    }
    let turn_permissions = permission_engine_for_turn(
        state.event_bus.clone(),
        state.permissions.clone(),
        &session_id,
        &user_message.id,
        &config_inputs,
        &selected_agent,
    )?;
    let dispatcher = ServerToolDispatcher::for_model(
        state.cwd.clone(),
        &resolved.model_id,
        session_id.clone(),
        state.db_path.clone(),
        state.checkpoint_base.clone(),
        turn_permissions.engine(),
        audit_sink,
    );
    let result = run_cancellable_turn(
        resolved.provider.as_ref(),
        turn,
        &dispatcher,
        active_turn.abort_receiver(),
        TurnEventContext {
            bus: state.event_bus.clone(),
            session_id: session_id.clone(),
            message_id: user_message.id.clone(),
            cwd: state.cwd.clone(),
        },
    )
    .await;
    dispatcher.shutdown().await;
    turn_permissions.persist_if_changed();

    let Some(result) = result else {
        state.event_bus.publish(&Event::turn_done(
            &session_id,
            &user_message.id,
            "aborted",
            0,
        ));
        return Ok(PromptResponse {
            status: "aborted",
            model: resolved.model_id,
            agent: selected_agent.name().to_owned(),
            queued,
            role: None,
            content: None,
            message: None,
            usage: None,
            finish_reason: None,
        });
    };
    let TurnRunResult {
        mut outcome,
        runner_events,
    } = result;
    apply_estimated_usage_cost(&resolved.model, &mut outcome);

    if let Some(error) = &outcome.error {
        return Err(ApiError::from_run_error(error));
    }

    // Let plugins observe/transform compaction summaries and continuation markers before they are
    // persisted, streamed, or echoed, mirroring the headless `run` path through the shared engine
    // record <-> hook-value seam so a server-driven compaction is identical across surfaces.
    if !outcome.compactions.is_empty() {
        let mut values: Vec<serde_json::Value> = outcome
            .compactions
            .iter()
            .map(|compaction| compaction.to_hook_value(&resolved.model_id))
            .collect();
        plugin_hooks.apply_compaction_hooks(&mut values);
        for (compaction, value) in outcome.compactions.iter_mut().zip(&values) {
            compaction.apply_hook_value(value);
        }
    }

    persist_usage(&db, &session_id, &outcome);
    let mut lifecycle = PartLifecycleTracker::new(&user_message.id, &resolved.model_id);
    for event in &runner_events {
        let _ = lifecycle.observe(event);
    }
    let part_timestamps = lifecycle.part_timestamps();
    let assistant_parts = build_assistant_message_parts(
        &resolved.model_id,
        selected_agent.name(),
        &user_message.id,
        &runner_events,
        &outcome,
        &part_timestamps,
    );
    let assistant = db
        .messages()
        .create_with_parts(&session_id, "assistant", outcome.answer(), &assistant_parts)
        .map_err(ApiError::from_db)?;
    state.event_bus.publish(&Event::message_parts(
        session_id.clone(),
        assistant.id.clone(),
        assistant_parts.clone(),
    ));
    state.event_bus.publish(&Event::message_updated(
        &session_id,
        &assistant.id,
        &assistant.role,
        Some(assistant.created_at.clone()),
    ));
    for part in &assistant_parts {
        state.event_bus.publish(&Event::message_part_updated(
            &session_id,
            &assistant.id,
            part.clone(),
        ));
    }
    if let Some(usage) = outcome.usage.as_ref() {
        let context_limit = resolved
            .model
            .info
            .as_ref()
            .and_then(|info| u32::try_from(info.limit.context).ok());
        state.event_bus.publish(&Event::session_context_updated(
            &session_id,
            &user_message.id,
            usage.total_tokens,
            context_limit,
        ));
    }
    for compaction in &outcome.compactions {
        state.event_bus.publish(&Event::session_compacted(
            &session_id,
            &user_message.id,
            &compaction.tail_start_id,
            compaction.model.clone(),
        ));
    }
    state.event_bus.publish(&Event::session_step_finished(
        &session_id,
        &user_message.id,
        outcome.steps,
    ));
    if let Ok(todos) = db.todos().list_for_session(&session_id)
        && !todos.is_empty()
        && let Ok(todos_json) = serde_json::to_value(&todos)
    {
        state
            .event_bus
            .publish(&Event::todo_updated(&session_id, todos_json));
    }
    let content = redaction::redact(outcome.answer());

    Ok(PromptResponse {
        status: "completed",
        model: resolved.model_id,
        agent: selected_agent.name().to_owned(),
        queued,
        role: Some("assistant"),
        content: Some(content),
        message: Some(assistant),
        usage: outcome.usage,
        finish_reason: Some(outcome.finish_reason),
    })
}

fn load_workspace_adapter_discovery(
    state: &ServerState,
) -> Result<Vec<WorkspaceAdapterDiscovery>, ApiError> {
    let plugin_hooks = load_plugin_hooks(state)?;
    Ok(plugin_hooks
        .workspace_adapter_entries()
        .into_iter()
        .map(WorkspaceAdapterDiscovery::from)
        .collect())
}

/// Load the configured plugin registry for discovery routes, honoring `--pure`/`CORTEX_PURE`.
///
/// This is the shared seam behind workspace-adapter and catalog discovery so plugin-registered
/// providers, models, agents, commands, and skills surface from a single, consistent load rather
/// than each route rolling its own plugin bootstrap.
///
/// Loading is LENIENT: a plugin that cannot be resolved or loaded (e.g. an offline package under
/// `CORTEX_NO_NET`, a missing local file, or no JavaScript runtime on `PATH`) is skipped, with a
/// `cortex serve:` warning, and the resolvable subset is returned. This keeps the read-only
/// catalog/discovery GET routes available (`200` with the resolvable subset) instead of failing
/// the whole route with a `500` when one configured plugin is unreachable.
pub(crate) fn load_plugin_hooks(
    state: &ServerState,
) -> Result<plugins::PluginEventHooks, ApiError> {
    let config_inputs = state.config.inputs(&state.cwd, BTreeMap::new());
    let effective_config = settings::load(config_inputs).map_err(config_api_error)?;
    let plugin_config =
        plugins::PluginConfig::from_toml_value(effective_config.get("plugin"), &state.cwd)
            .map_err(plugin_config_error)?;
    let resolved_config =
        serde_json::to_value(effective_config.table()).unwrap_or_else(|_| serde_json::json!({}));
    let resolved_config = match resolved_config {
        serde_json::Value::Object(object) => serde_json::Value::Object(object),
        _ => serde_json::json!({}),
    };
    let (hooks, skipped) = plugins::PluginEventHooks::load_lenient_with_config(
        &plugin_config,
        state.config.pure_plugins(),
        "serve",
        resolved_config,
    );
    for skip in &skipped {
        eprintln!(
            "cortex serve: skipping plugin `{}` for discovery: {}",
            skip.spec, skip.reason
        );
    }
    Ok(hooks)
}

struct ResolvedTurnProvider {
    model_id: String,
    model: ResolvedModel,
    provider: Arc<dyn Provider>,
    request_params: serde_json::Map<String, serde_json::Value>,
}

fn resolve_turn_provider(
    source: &ProviderSource,
    model_id: &str,
    provider_config: &providers::ProviderConfigSet,
    config_inputs: &settings::ConfigInputs,
) -> Result<ResolvedTurnProvider, ApiError> {
    match source {
        ProviderSource::OpenRouter => {
            let resolved = provider_config
                .resolve_model(providers::Catalogue::embedded(), Some(model_id))
                .map_err(provider_config_api_error)?;
            policy::enforce_provider_use(config_inputs, &resolved.provider_id).map_err(|err| {
                if err.is_config() {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("cannot load provider policy: {err}"),
                    )
                } else {
                    ApiError::new(StatusCode::FORBIDDEN, err.to_string())
                }
            })?;
            let provider = providers::resolve_runtime_provider_with_options(
                provider_config,
                &resolved.provider_id,
                &resolved.model_id,
                None,
            )
            .map_err(|error| {
                if let Some(config_error) = error.as_config() {
                    return provider_config_api_error(config_error.clone());
                }
                if let Some(diagnostic) = error.as_diagnostic() {
                    let status = match diagnostic.kind() {
                        "no-credential" => StatusCode::UNAUTHORIZED,
                        "net-disabled" => StatusCode::SERVICE_UNAVAILABLE,
                        _ => StatusCode::BAD_GATEWAY,
                    };
                    return ApiError::diagnostic(status, diagnostic.clone());
                }
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "failed to resolve runtime provider",
                )
            })?;
            let request_params = provider_config.provider_params(&resolved.provider_id);
            Ok(ResolvedTurnProvider {
                model_id: resolved.model_id.clone(),
                model: resolved,
                provider: provider.provider,
                request_params,
            })
        }
        ProviderSource::Fixed { provider } => {
            let resolved = provider_config
                .resolve_model(providers::Catalogue::embedded(), Some(model_id))
                .map_err(provider_config_api_error)?;
            policy::enforce_provider_use(config_inputs, &resolved.provider_id).map_err(|err| {
                if err.is_config() {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("cannot load provider policy: {err}"),
                    )
                } else {
                    ApiError::new(StatusCode::FORBIDDEN, err.to_string())
                }
            })?;
            let request_params = provider_config.provider_params(&resolved.provider_id);
            Ok(ResolvedTurnProvider {
                model_id: resolved.model_id.clone(),
                model: resolved,
                provider: provider.clone(),
                request_params,
            })
        }
    }
}

fn select_turn_model(
    request_model: Option<&str>,
    agent_model: Option<&str>,
    session_model: Option<&str>,
    config_model: &str,
) -> Result<String, String> {
    let model = request_model
        .and_then(non_empty)
        .or_else(|| agent_model.and_then(non_empty))
        .or_else(|| session_model.and_then(non_empty))
        .or_else(|| non_empty(config_model))
        .unwrap_or_else(|| providers::OPENROUTER_DEFAULT_MODEL.to_owned());
    providers::validate_model_id(&model)?;
    Ok(model)
}

fn resolve_turn_agent(
    inputs: &settings::ConfigInputs,
    config_agent: &str,
    surface: &str,
) -> Result<custom_agents::ActiveAgent, String> {
    let discovery = custom_agents::discover(inputs);
    for warning in &discovery.warnings {
        eprintln!("cortex {surface}: {warning}");
    }
    let requested = non_empty(config_agent);
    custom_agents::resolve(requested.as_deref(), &discovery).map_err(|err| err.to_string())
}

fn render_system_context(
    model_id: &str,
    agent: &custom_agents::ActiveAgent,
    project_memory: Option<String>,
) -> String {
    let mut system = SystemContext::for_model(model_id, EnvironmentBlock::from_process())
        .with_project_memory(project_memory)
        .with_agent(agent.base_agent())
        .render();
    if let Some(prompt) = agent.custom_prompt() {
        system.push_str("\n\nCustom agent instructions:\n");
        system.push_str(prompt);
    }
    system
}

async fn run_cancellable_turn(
    provider: &dyn Provider,
    turn: Turn,
    dispatcher: &dyn ToolDispatcher,
    mut abort_rx: watch::Receiver<bool>,
    ctx: TurnEventContext,
) -> Option<TurnRunResult> {
    let runner = SessionRunner::default();
    let events: Arc<Mutex<Vec<RunnerEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_for_sink = events.clone();
    // Correlate each tool call's path/question arguments (keyed by call id) so the matching tool
    // RESULT can emit the subsystem family (`file.edited`, `question.*`) with the right detail.
    let mut pending_tool_calls: HashMap<String, RunnerEvent> = HashMap::new();
    tokio::select! {
        outcome = runner.run_with_tools(provider, turn, dispatcher, move |event| {
            publish_runner_event(&ctx, &mut pending_tool_calls, &event);
            if let Ok(mut captured) = events_for_sink.lock() {
                captured.push(event.clone());
            }
        }) => Some(outcome),
        () = wait_for_abort(&mut abort_rx) => None,
    }
    .map(|outcome| TurnRunResult {
        outcome,
        runner_events: events
            .lock()
            .map(|captured| captured.clone())
            .unwrap_or_default(),
    })
}

struct TurnRunResult {
    outcome: RunOutcome,
    runner_events: Vec<RunnerEvent>,
}

/// The shared context for publishing a turn's runner events to the bus: the event bus plus the
/// session/turn ids and workspace cwd used to build subsystem event payloads.
struct TurnEventContext {
    bus: Arc<EventBus>,
    session_id: String,
    message_id: String,
    cwd: PathBuf,
}

fn publish_runner_event(
    ctx: &TurnEventContext,
    pending_tool_calls: &mut HashMap<String, RunnerEvent>,
    event: &RunnerEvent,
) {
    let TurnEventContext {
        bus,
        session_id,
        message_id,
        cwd,
    } = ctx;
    let session_id = session_id.as_str();
    let message_id = message_id.as_str();
    let cwd = cwd.as_path();
    match event {
        RunnerEvent::TextDelta { text } => bus.publish(&Event::assistant_text_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::Reasoning { text, .. } => bus.publish(&Event::assistant_reasoning_delta(
            session_id.to_owned(),
            message_id.to_owned(),
            text.clone(),
        )),
        RunnerEvent::ToolCall {
            id,
            name,
            arguments,
        } => {
            bus.publish(&Event::tool_call_for_turn(
                session_id.to_owned(),
                message_id.to_owned(),
                id.clone(),
                name.clone(),
                arguments.clone(),
            ));
            // A `question` tool call drives a real user prompt; surface it on the live
            // `question.asked` family (distinct from the permission engine's `permission.asked`).
            // A single call may carry several prompts, so emit one event per question.
            if name == "question" {
                for asked in question_asked_events(session_id, id, arguments) {
                    bus.publish(&asked);
                }
            }
            pending_tool_calls.insert(id.clone(), event.clone());
        }
        RunnerEvent::ToolResult {
            id,
            name,
            is_error,
            parts,
        } => {
            bus.publish(&Event::tool_result_for_turn(
                session_id.to_owned(),
                message_id.to_owned(),
                id.clone(),
                name.clone(),
                *is_error,
            ));
            let call = pending_tool_calls.remove(id);
            publish_tool_subsystem_events(
                bus,
                session_id,
                cwd,
                name,
                *is_error,
                call.as_ref(),
                parts,
            );
        }
        RunnerEvent::Usage(usage) => bus.publish(&Event::turn_usage(
            session_id.to_owned(),
            message_id.to_owned(),
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.cache_creation_input_tokens,
            usage.cache_read_input_tokens,
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
        RunnerEvent::Retry {
            attempt,
            max_attempts,
            delay_ms,
            classification,
            ..
        } => bus.publish(&Event::session_retried(
            session_id.to_owned(),
            message_id.to_owned(),
            *attempt,
            *max_attempts,
            *delay_ms,
            classification.clone(),
        )),
    }
}

/// Emit the subsystem event family for a completed tool call from a real turn action: `file.edited`
/// for a successful `write`/`edit`/`apply_patch`, and `question.replied`/`question.rejected` for a
/// `question` tool result. `call` is the correlated [`RunnerEvent::ToolCall`] (carrying the
/// arguments) when available. These are the live-emission counterparts of the otherwise
/// forward-compat-only families, fired from the actual subsystem rather than a `/event` POST.
fn publish_tool_subsystem_events(
    bus: &EventBus,
    session_id: &str,
    cwd: &Path,
    name: &str,
    is_error: bool,
    call: Option<&RunnerEvent>,
    parts: &[protocol::ToolResultPart],
) {
    let arguments = call.and_then(|event| match event {
        RunnerEvent::ToolCall { arguments, .. } => {
            serde_json::from_str::<serde_json::Value>(arguments).ok()
        }
        _ => None,
    });
    match name {
        "write" | "edit" | "apply_patch" if !is_error => {
            for path in edited_paths(name, cwd, arguments.as_ref()) {
                bus.publish(&Event::file_edited(
                    Some(session_id.to_owned()),
                    path,
                    Some(name.to_owned()),
                ));
            }
        }
        "question" => {
            let id = call
                .and_then(|event| match event {
                    RunnerEvent::ToolCall { id, .. } => Some(id.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            if is_error {
                bus.publish(&Event::question_rejected(
                    Some(session_id.to_owned()),
                    id,
                    Some(tool_result_text(parts)),
                ));
            } else {
                bus.publish(&Event::question_replied(
                    Some(session_id.to_owned()),
                    id,
                    tool_result_text(parts),
                ));
            }
        }
        _ => {}
    }
}

/// The absolute path(s) a successful file-mutating tool touched, derived from its call arguments.
///
/// For `apply_patch` the paths come from the parsed envelope hunks via [`tools::patch_edited_paths`],
/// NOT from re-applying the patch against the on-disk file: this runs after the tool RESULT, when the
/// file is already mutated, so re-locating an `Update` hunk's chunks would fail and silently drop the
/// path (the common "edit an existing file" case).
fn edited_paths(name: &str, cwd: &Path, arguments: Option<&serde_json::Value>) -> Vec<String> {
    let Some(arguments) = arguments else {
        return Vec::new();
    };
    if name == "apply_patch" {
        let patch = ["patch", "patchText", "patch_text", "input"]
            .iter()
            .find_map(|key| arguments.get(*key).and_then(serde_json::Value::as_str));
        return patch
            .map(|patch| {
                tools::patch_edited_paths(patch, cwd)
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
    }
    arguments
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(|path| {
            let candidate = Path::new(path);
            let resolved = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                cwd.join(candidate)
            };
            vec![resolved.to_string_lossy().into_owned()]
        })
        .unwrap_or_default()
}

/// Build the `question.asked` events from a `question` tool call's arguments, one per question
/// prompt (a single tool call may carry several questions). Returns an empty vec when the arguments
/// carry no usable prompt.
fn question_asked_events(session_id: &str, id: &str, arguments: &str) -> Vec<Event> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return Vec::new();
    };
    let Some(questions) = value.get("questions").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    questions
        .iter()
        .filter_map(|question| {
            let prompt = question.get("prompt").and_then(serde_json::Value::as_str)?;
            let options = question
                .get("options")
                .and_then(serde_json::Value::as_array)
                .map(|options| {
                    options
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Some(Event::question_asked(
                Some(session_id.to_owned()),
                id.to_owned(),
                prompt.to_owned(),
                options,
            ))
        })
        .collect()
}

/// Concatenate the text parts of a tool result into a single string.
fn tool_result_text(parts: &[protocol::ToolResultPart]) -> String {
    parts
        .iter()
        .filter_map(|part| part.as_text())
        .collect::<Vec<_>>()
        .join("\n")
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
        .map(|message| {
            let content_parts = extract_message_content_parts(&message.parts);
            let content = if message.content.trim().is_empty() {
                message
                    .parts
                    .iter()
                    .filter_map(|part| match &part.body {
                        protocol::MessagePartBody::Text { text }
                        | protocol::MessagePartBody::Reasoning { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                message.content.clone()
            };
            HistoryMessage::new(message.role, content).with_parts(content_parts)
        })
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

fn apply_estimated_usage_cost(model: &ResolvedModel, outcome: &mut RunOutcome) {
    let Some(usage) = outcome.usage.as_mut() else {
        return;
    };
    if usage.cost.is_some() {
        return;
    }
    let Some(info) = model.info.as_ref() else {
        return;
    };
    usage.cost = providers::estimate_usage_cost(info, usage);
}

fn compaction_policy(model: &ResolvedModel) -> CompactionPolicy {
    // The effective context window can be overridden via `CORTEX_CONTEXT_LIMIT_TOKENS` (an
    // operational tuning/testing knob), mirroring the headless `run` path so a long server session
    // compacts deterministically. An explicit override drives the budget purely from the requested
    // window; otherwise the model's catalogue window (or the engine default) stands.
    if let Some(context) = context_limit_override() {
        return CompactionPolicy::for_context_window(context, 0);
    }
    let (context, output) = match &model.info {
        Some(info) => (
            usize::try_from(info.limit.context).unwrap_or(usize::MAX),
            usize::try_from(info.limit.output).unwrap_or(0),
        ),
        None => (engine::DEFAULT_CONTEXT_LIMIT_TOKENS, 0),
    };
    CompactionPolicy::for_context_window(context, output)
}

/// Parse the `CORTEX_CONTEXT_LIMIT_TOKENS` override, accepting only a positive integer (trimmed) so
/// an absent, blank, zero, or unparseable value leaves the model-derived window in place.
fn context_limit_override() -> Option<usize> {
    let raw = std::env::var("CORTEX_CONTEXT_LIMIT_TOKENS").ok()?;
    let parsed: usize = raw.trim().parse().ok()?;
    (parsed >= 1).then_some(parsed)
}

fn config_api_error(err: settings::ConfigError) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("invalid server configuration: {err}"),
    )
}

fn provider_config_api_error(err: providers::ProviderConfigError) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("invalid provider configuration: {err}"),
    )
}

fn plugin_config_error(message: String) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("invalid plugin configuration: {message}"),
    )
}

fn plugin_runtime_error(err: plugins::PluginRuntimeError) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("plugin runtime error: {err}"),
    )
}

struct ServerPermissionResponder {
    pending: PendingPermissions,
    ruleset: permissions::Ruleset,
    session_id: String,
    message_id: String,
}

struct ServerTurnPermissions {
    engine: Option<Arc<PermissionEngine>>,
    saved: Arc<permissions::SavedDecisions>,
    loaded: Vec<permissions::SavedDecision>,
    workspace_key: String,
}

impl ServerTurnPermissions {
    fn new(
        engine: Option<Arc<PermissionEngine>>,
        saved: Arc<permissions::SavedDecisions>,
        loaded: Vec<permissions::SavedDecision>,
        workspace_key: String,
    ) -> Self {
        Self {
            engine,
            saved,
            loaded,
            workspace_key,
        }
    }

    fn engine(&self) -> Option<Arc<PermissionEngine>> {
        self.engine.clone()
    }

    fn persist_if_changed(&self) {
        let snapshot = self.saved.snapshot();
        if snapshot.as_slice() == self.loaded.as_slice() {
            return;
        }
        if let Err(err) = permissions::store::save_for(&self.workspace_key, &snapshot) {
            eprintln!(
                "cortex serve: failed to persist saved permission decisions for workspace `{}`: {err}",
                self.workspace_key
            );
        }
    }
}

impl ServerPermissionResponder {
    fn new(
        pending: PendingPermissions,
        ruleset: permissions::Ruleset,
        session_id: &str,
        message_id: &str,
    ) -> Self {
        Self {
            pending,
            ruleset,
            session_id: session_id.to_owned(),
            message_id: message_id.to_owned(),
        }
    }
}

impl permissions::Responder for ServerPermissionResponder {
    fn respond(&self, request: &permissions::PermissionRequest) -> permissions::Decision {
        self.respond_scoped(request).decision
    }

    fn respond_scoped(
        &self,
        request: &permissions::PermissionRequest,
    ) -> permissions::ScopedDecision {
        let (_, matched_rule) = self.ruleset.resolve(&request.tool, &request.subject);
        let scope_pattern = matched_rule
            .map(|rule| rule.pattern.clone())
            .unwrap_or_else(|| "*".to_owned());
        let pending =
            self.pending
                .enqueue(&self.session_id, &self.message_id, request, scope_pattern);
        let reply = wait_for_pending_reply(&pending);
        self.pending.remove(&pending.id);
        reply
    }
}

fn wait_for_pending_reply(pending: &PendingPermission) -> permissions::ScopedDecision {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| pending.wait())
        }
        _ => pending.wait(),
    }
}

fn permission_engine_for_turn(
    bus: Arc<EventBus>,
    pending_permissions: PendingPermissions,
    session_id: &str,
    message_id: &str,
    inputs: &settings::ConfigInputs,
    agent: &custom_agents::ActiveAgent,
) -> Result<ServerTurnPermissions, ApiError> {
    let workspace_key = checkpoints::workspace_key(&inputs.cwd);
    let loaded = permissions::store::load_for(&workspace_key);
    let saved = Arc::new(permissions::SavedDecisions::from_decisions(loaded.clone()));
    let base_agent = agent.base_agent();
    let config_ruleset = permissions::load_agent_rulesets(inputs)
        .map(|rulesets| rulesets.for_agent(agent.name()))
        .map_err(permission_config_error)?;
    let mut ruleset = permissions::Ruleset::new(readonly_agent_base_rules(base_agent));
    for rule in config_ruleset.rules() {
        ruleset.push(rule.clone());
    }
    for rule in agent.permissions() {
        ruleset.push(rule.clone());
    }
    for rule in readonly_agent_restriction_rules(base_agent) {
        ruleset.push(rule);
    }
    let trust = policy::resolve_trust(inputs).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot load trust policy: {err}"),
        )
    })?;
    let mut trust_hard_denies = Vec::new();
    for rule in policy::trust_ruleset(trust).rules() {
        if rule.action == permissions::Action::Deny {
            trust_hard_denies.push(rule.clone());
        } else {
            ruleset.push(rule.clone());
        }
    }
    let mut hard_denies = readonly_agent_restriction_rules(base_agent);
    hard_denies.extend(trust_hard_denies);
    if ruleset.is_empty() && hard_denies.is_empty() && saved.is_empty() {
        return Ok(ServerTurnPermissions::new(
            None,
            saved,
            loaded,
            workspace_key,
        ));
    }
    let responder = Arc::new(ServerPermissionResponder::new(
        pending_permissions,
        ruleset.clone(),
        session_id,
        message_id,
    ));
    Ok(ServerTurnPermissions::new(
        Some(Arc::new(
            PermissionEngine::new(ruleset)
                .with_hard_denies(permissions::Ruleset::new(hard_denies))
                .with_responder(responder)
                .with_saved_decisions(saved.clone())
                .with_bus(bus)
                .with_event_context(session_id.to_owned(), message_id.to_owned()),
        )),
        saved,
        loaded,
        workspace_key,
    ))
}

fn readonly_agent_base_rules(agent: Agent) -> Vec<permissions::Rule> {
    match agent {
        Agent::Plan | Agent::Explore => {
            let mut rules: Vec<permissions::Rule> =
                ["read", "glob", "grep", "lsp", "webfetch", "websearch"]
                    .into_iter()
                    .map(|tool| permissions::Rule::new(tool, "*", permissions::Action::Allow))
                    .collect();
            rules.extend(permissions::default_external_and_env_rules());
            rules
        }
        Agent::Build | Agent::General => permissions::permissive_default_rules(),
    }
}

fn readonly_agent_restriction_rules(agent: Agent) -> Vec<permissions::Rule> {
    match agent {
        Agent::Plan | Agent::Explore => ["write", "edit", "apply_patch", "format", "bash"]
            .into_iter()
            .map(|tool| permissions::Rule::new(tool, "*", permissions::Action::Deny))
            .collect(),
        Agent::Build | Agent::General => Vec::new(),
    }
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
    checkpoint_recorder: Option<Arc<checkpoints::TurnRecorder>>,
}

impl ServerToolDispatcher {
    fn for_model(
        cwd: PathBuf,
        model_id: &str,
        session_id: String,
        db_path: PathBuf,
        checkpoint_base: Option<PathBuf>,
        permissions: Option<Arc<PermissionEngine>>,
        audit: Arc<audit::AuditSink>,
    ) -> Self {
        let registry = match ModelFamily::from_model_id(model_id) {
            ModelFamily::Gpt | ModelFamily::Codex => ToolRegistry::gpt(),
            ModelFamily::Anthropic | ModelFamily::Gemini | ModelFamily::Default => {
                ToolRegistry::builtin()
            }
        };
        let checkpoint_recorder = open_checkpoint_recorder(&cwd, checkpoint_base, &session_id);
        let mut context = ToolContext::new(cwd)
            .with_session(session_id)
            .with_todo_store(Arc::new(ServerTodoStore { db_path }))
            .with_audit(audit)
            .with_interactive_questions(true);
        if let Some(permissions) = permissions {
            context = context.with_permissions(permissions);
        }
        if let Some(recorder) = &checkpoint_recorder {
            context = context.with_checkpoints(Arc::new(ServerCheckpointRecorder {
                inner: recorder.clone(),
            }));
        }
        Self {
            registry,
            context,
            checkpoint_recorder,
        }
    }

    async fn shutdown(&self) {
        self.context.background_jobs().shutdown().await;
        if let Some(recorder) = &self.checkpoint_recorder
            && let Err(err) = recorder.finalize()
        {
            eprintln!("cortex serve: checkpoint finalization failed: {err}");
        }
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

struct ServerCheckpointRecorder {
    inner: Arc<checkpoints::TurnRecorder>,
}

impl tools::CheckpointRecorder for ServerCheckpointRecorder {
    fn record_pre_image(&self, path: &Path) {
        self.inner.record_pre_image(path);
    }
}

fn open_checkpoint_recorder(
    cwd: &Path,
    checkpoint_base: Option<PathBuf>,
    session_id: &str,
) -> Option<Arc<checkpoints::TurnRecorder>> {
    let store = match checkpoint_base {
        Some(base) => {
            let git_dir = base.join(checkpoints::workspace_key(cwd));
            checkpoints::ShadowStore::open(cwd, &git_dir).ok()
        }
        None => checkpoints::ShadowStore::open_default(cwd).ok().flatten(),
    }?;
    Some(Arc::new(store.begin_turn(session_id.to_owned())))
}

/// Open the shadow checkpoint store for the server's workspace, mirroring the recorder's location
/// resolution. Used by the diff/revert/unrevert routes.
pub(crate) fn open_checkpoint_store(state: &ServerState) -> Option<checkpoints::ShadowStore> {
    match &state.checkpoint_base {
        Some(base) => {
            let git_dir = base.join(checkpoints::workspace_key(&state.cwd));
            checkpoints::ShadowStore::open(&state.cwd, &git_dir).ok()
        }
        None => checkpoints::ShadowStore::open_default(&state.cwd)
            .ok()
            .flatten(),
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

fn write_remote_abort_request(session_id: &str) -> std::io::Result<()> {
    let dir = data_subdir("remote-abort");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(format!("{session_id}.abort")), b"abort\n")
}

fn data_subdir(name: &str) -> PathBuf {
    if let Some(dir) = env_non_empty("CORTEX_DATA_DIR") {
        return PathBuf::from(dir).join(name);
    }
    if let Some(dir) = env_non_empty("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("cortex").join(name);
    }
    if let Some(home) = env_non_empty("HOME") {
        return PathBuf::from(home).join(".local/share/cortex").join(name);
    }
    std::env::temp_dir().join("cortex").join(name)
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

impl RemoteTurns {
    fn observe(&self, event: &Event) {
        match event {
            Event::TurnStarted(turn) => self.insert(&turn.session_id),
            Event::TurnDone(turn) => self.remove(&turn.session_id),
            Event::TurnError(turn) => self.remove(&turn.session_id),
            _ => {}
        }
    }

    fn insert(&self, session_id: &str) {
        if let Ok(mut active) = self.inner.lock() {
            active.insert(session_id.to_owned());
        }
    }

    fn remove(&self, session_id: &str) {
        if let Ok(mut active) = self.inner.lock() {
            active.remove(session_id);
        }
    }

    fn is_active(&self, session_id: &str) -> Result<bool, ApiError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "remote turn registry is unavailable",
                )
            })?
            .contains(session_id))
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

impl PendingPermissions {
    fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|state| state.by_id.len())
            .unwrap_or(0)
    }

    fn enqueue(
        &self,
        session_id: &str,
        message_id: &str,
        request: &permissions::PermissionRequest,
        scope_pattern: String,
    ) -> Arc<PendingPermission> {
        let mut state = self.inner.lock().expect("pending permission registry lock");
        let id = format!("perm-{}-{}", std::process::id(), state.next_id);
        state.next_id = state.next_id.saturating_add(1);
        let pending = Arc::new(PendingPermission::new(
            id.clone(),
            session_id.to_owned(),
            message_id.to_owned(),
            request,
            scope_pattern,
        ));
        state.by_id.insert(id, pending.clone());
        pending
    }

    fn remove(&self, id: &str) {
        if let Ok(mut state) = self.inner.lock() {
            state.by_id.remove(id);
        }
    }

    fn list(&self, session_id: Option<&str>) -> Result<Vec<PendingPermissionItem>, ApiError> {
        let state = self.inner.lock().map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "permission registry is unavailable",
            )
        })?;
        let mut pending: Vec<PendingPermissionItem> = state
            .by_id
            .values()
            .filter(|entry| session_id.is_none_or(|session| entry.session_id == session))
            .filter(|entry| {
                entry
                    .reply
                    .lock()
                    .map(|reply| reply.is_none())
                    .unwrap_or(false)
            })
            .map(|entry| PendingPermissionItem {
                id: entry.id.clone(),
                session_id: entry.session_id.clone(),
                message_id: entry.message_id.clone(),
                tool: entry.tool.clone(),
                subject: entry.subject.clone(),
                waited_ms: entry.created_at.elapsed().as_millis() as u64,
            })
            .collect();
        pending.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(pending)
    }

    fn reply(
        &self,
        id: &str,
        session_id: Option<&str>,
        reply: permissions::ScopedDecision,
        cascade: bool,
    ) -> Result<usize, ApiError> {
        let state = self.inner.lock().map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "permission registry is unavailable",
            )
        })?;
        let Some(target) = state.by_id.get(id).cloned() else {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("pending permission `{id}` was not found"),
            ));
        };
        if let Some(expected_session) = session_id
            && target.session_id != expected_session
        {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("pending permission `{id}` was not found for session `{expected_session}`"),
            ));
        }
        let mut resolved = 0;
        if target.resolve(reply.clone()) {
            resolved += 1;
        }
        if reply.scope == permissions::DecisionScope::Always {
            for pending in state.by_id.values() {
                if pending.id == target.id {
                    continue;
                }
                if same_saved_decision_scope(&target, pending) && pending.resolve(reply.clone()) {
                    resolved += 1;
                }
            }
        } else if reply.decision == permissions::Decision::Deny && cascade {
            for pending in state.by_id.values() {
                if pending.id == target.id {
                    continue;
                }
                if pending.session_id == target.session_id
                    && pending.resolve(permissions::ScopedDecision::once(
                        permissions::Decision::Deny,
                    ))
                {
                    resolved += 1;
                }
            }
        }
        Ok(resolved)
    }

    fn cancel_session(
        &self,
        session_id: &str,
        reply: permissions::ScopedDecision,
    ) -> Result<usize, ApiError> {
        let state = self.inner.lock().map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "permission registry is unavailable",
            )
        })?;
        let mut resolved = 0;
        for pending in state.by_id.values() {
            if pending.session_id == session_id && pending.resolve(reply.clone()) {
                resolved += 1;
            }
        }
        Ok(resolved)
    }
}

fn same_saved_decision_scope(left: &PendingPermission, right: &PendingPermission) -> bool {
    left.scope_tool == right.scope_tool && left.scope_pattern == right.scope_pattern
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

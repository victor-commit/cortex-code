//! Remote TUI control routes.
//!
//! An external client (editor, script, or the SDK) drives an *attached* TUI through these routes.
//! Issuing routes (`/tui/append-prompt`, `/tui/open-models`, `/tui/show-toast`,
//! `/tui/execute-command`, `/tui/select-session`, `/tui/submit-prompt`, `/tui/clear-prompt`) enqueue
//! a control request and wait for the attached TUI to apply it. The attached TUI polls
//! `GET /tui/control/next` for the next request, applies it locally, and reports the result via
//! `POST /tui/control/response`. The issuing route then returns the success/error status the TUI
//! reported. When no TUI claims a request in time the issuing route fails closed with a clear
//! "no attached TUI" error rather than hanging forever.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse, response::Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};

use super::{ApiError, ServerState};

/// How long an issuing route waits for the attached TUI to apply and acknowledge a control request.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);

static CONTROL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One control request delivered to the attached TUI.
#[derive(Clone, Debug, Serialize)]
pub(super) struct ControlRequest {
    id: String,
    action: &'static str,
    body: Value,
}

/// The TUI's report for a delivered control request.
#[derive(Debug)]
struct ControlOutcome {
    success: bool,
    error: Option<String>,
}

/// Shared TUI control registry. Cloned into [`ServerState`].
#[derive(Clone, Default)]
pub(super) struct TuiControl {
    inner: Arc<TuiControlInner>,
}

#[derive(Default)]
struct TuiControlInner {
    queue: Mutex<TuiControlQueue>,
    notify: Notify,
}

#[derive(Default)]
struct TuiControlQueue {
    pending_delivery: VecDeque<ControlRequest>,
    awaiting_response: HashMap<String, oneshot::Sender<ControlOutcome>>,
}

impl TuiControl {
    async fn enqueue_and_wait(
        &self,
        action: &'static str,
        body: Value,
    ) -> Result<Json<ControlAck>, ApiError> {
        let id = format!(
            "ctl-{}-{}",
            std::process::id(),
            CONTROL_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let request = ControlRequest {
            id: id.clone(),
            action,
            body,
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut queue = self.lock()?;
            queue.pending_delivery.push_back(request);
            queue.awaiting_response.insert(id.clone(), tx);
        }
        self.inner.notify.notify_waiters();

        match tokio::time::timeout(CONTROL_TIMEOUT, rx).await {
            Ok(Ok(outcome)) => Ok(Json(ControlAck {
                status: if outcome.success { "ok" } else { "error" },
                action,
                error: outcome.error,
            })),
            // Sender dropped without a response (request consumed but never acknowledged).
            Ok(Err(_)) => {
                self.discard(&id);
                Err(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "attached TUI dropped the control request without responding",
                ))
            }
            Err(_) => {
                self.discard(&id);
                Err(ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "no attached TUI responded to the control request",
                ))
            }
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, TuiControlQueue>, ApiError> {
        self.inner.queue.lock().map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "tui control registry is unavailable",
            )
        })
    }

    fn discard(&self, id: &str) {
        if let Ok(mut queue) = self.inner.queue.lock() {
            queue.awaiting_response.remove(id);
            queue.pending_delivery.retain(|request| request.id != id);
        }
    }

    fn take_next(&self) -> Result<Option<ControlRequest>, ApiError> {
        Ok(self.lock()?.pending_delivery.pop_front())
    }

    fn resolve(&self, id: &str, outcome: ControlOutcome) -> Result<bool, ApiError> {
        let sender = self.lock()?.awaiting_response.remove(id);
        match sender {
            Some(sender) => Ok(sender.send(outcome).is_ok()),
            None => Ok(false),
        }
    }
}

/// Acknowledgement returned to the client that issued a control request.
#[derive(Debug, Serialize)]
pub(super) struct ControlAck {
    status: &'static str,
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Body for `POST /tui/append-prompt`.
#[derive(Debug, Deserialize)]
pub(super) struct AppendPromptRequest {
    text: String,
}

/// `POST /tui/append-prompt` — append text to the attached TUI's prompt composer.
pub(super) async fn append_prompt(
    State(state): State<ServerState>,
    Json(payload): Json<AppendPromptRequest>,
) -> Result<Json<ControlAck>, ApiError> {
    if payload.text.is_empty() {
        return Err(ApiError::bad_request(
            "append-prompt body must include text",
        ));
    }
    state
        .tui_control
        .enqueue_and_wait("append-prompt", json!({ "text": payload.text }))
        .await
}

/// `POST /tui/submit-prompt` — submit the attached TUI's current prompt.
pub(super) async fn submit_prompt(
    State(state): State<ServerState>,
) -> Result<Json<ControlAck>, ApiError> {
    state
        .tui_control
        .enqueue_and_wait("submit-prompt", json!({}))
        .await
}

/// `POST /tui/clear-prompt` — clear the attached TUI's prompt composer.
pub(super) async fn clear_prompt(
    State(state): State<ServerState>,
) -> Result<Json<ControlAck>, ApiError> {
    state
        .tui_control
        .enqueue_and_wait("clear-prompt", json!({}))
        .await
}

/// `POST /tui/open-models` — open the model picker in the attached TUI.
pub(super) async fn open_models(
    State(state): State<ServerState>,
) -> Result<Json<ControlAck>, ApiError> {
    state
        .tui_control
        .enqueue_and_wait("open-models", json!({}))
        .await
}

/// Body for `POST /tui/execute-command`.
#[derive(Debug, Deserialize)]
pub(super) struct ExecuteCommandRequest {
    command: String,
}

/// `POST /tui/execute-command` — run a named TUI command in the attached TUI.
pub(super) async fn execute_command(
    State(state): State<ServerState>,
    Json(payload): Json<ExecuteCommandRequest>,
) -> Result<Json<ControlAck>, ApiError> {
    let command = payload.command.trim();
    if command.is_empty() {
        return Err(ApiError::bad_request(
            "execute-command body must include a command",
        ));
    }
    state
        .tui_control
        .enqueue_and_wait("execute-command", json!({ "command": command }))
        .await
}

/// Body for `POST /tui/show-toast`.
#[derive(Debug, Deserialize)]
pub(super) struct ShowToastRequest {
    message: String,
    #[serde(default)]
    variant: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

/// `POST /tui/show-toast` — show a toast notification in the attached TUI.
pub(super) async fn show_toast(
    State(state): State<ServerState>,
    Json(payload): Json<ShowToastRequest>,
) -> Result<Json<ControlAck>, ApiError> {
    if payload.message.trim().is_empty() {
        return Err(ApiError::bad_request(
            "show-toast body must include a message",
        ));
    }
    let mut body = json!({ "message": payload.message });
    if let Some(variant) = payload.variant {
        body["variant"] = Value::String(variant);
    }
    if let Some(title) = payload.title {
        body["title"] = Value::String(title);
    }
    state.tui_control.enqueue_and_wait("show-toast", body).await
}

/// Body for `POST /tui/select-session`.
#[derive(Debug, Deserialize)]
pub(super) struct SelectSessionRequest {
    session_id: String,
}

/// `POST /tui/select-session` — switch the attached TUI to a different session.
pub(super) async fn select_session(
    State(state): State<ServerState>,
    Json(payload): Json<SelectSessionRequest>,
) -> Result<Json<ControlAck>, ApiError> {
    let session_id = payload.session_id.trim();
    if session_id.is_empty() {
        return Err(ApiError::bad_request(
            "select-session body must include a session_id",
        ));
    }
    state
        .tui_control
        .enqueue_and_wait("select-session", json!({ "session_id": session_id }))
        .await
}

/// `GET /tui/control/next` — the attached TUI polls for the next control request.
///
/// Returns `200` with the request when one is queued, or `204 No Content` when the queue is empty so
/// the TUI can poll cheaply without long-lived blocking.
pub(super) async fn control_next(State(state): State<ServerState>) -> Result<Response, ApiError> {
    match state.tui_control.take_next()? {
        Some(request) => Ok(Json(request).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

/// Body for `POST /tui/control/response`.
#[derive(Debug, Deserialize)]
pub(super) struct ControlResponseRequest {
    id: String,
    #[serde(default = "default_success")]
    success: bool,
    #[serde(default)]
    error: Option<String>,
}

const fn default_success() -> bool {
    true
}

/// Acknowledgement for `POST /tui/control/response`.
#[derive(Debug, Serialize)]
pub(super) struct ControlResponseAck {
    status: &'static str,
    matched: bool,
}

/// `POST /tui/control/response` — the attached TUI reports the outcome of a control request.
pub(super) async fn control_response(
    State(state): State<ServerState>,
    Json(payload): Json<ControlResponseRequest>,
) -> Result<Json<ControlResponseAck>, ApiError> {
    if payload.id.trim().is_empty() {
        return Err(ApiError::bad_request(
            "control response body must include the request id",
        ));
    }
    let matched = state.tui_control.resolve(
        &payload.id,
        ControlOutcome {
            success: payload.success,
            error: payload.error,
        },
    )?;
    Ok(Json(ControlResponseAck {
        status: "ok",
        matched,
    }))
}

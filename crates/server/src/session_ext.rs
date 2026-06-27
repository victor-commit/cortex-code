//! Wire-compatible session route handlers layered over the core engine and store.
//!
//! These handlers extend the base session surface (`lib.rs`) with the reference-compatible route
//! family: session update, cursor-paged message access, single message/part get/delete/update,
//! status/wait/context, children/fork/diff, init/summarize/compact, async prompt, shell, and
//! revert/unrevert. They reuse the same database repositories, durable runner, checkpoint store,
//! permission engine, and audit sink as the base routes, so the server stays a thin shell over the
//! engine rather than a parallel implementation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
};
use db::{Message, Session};
use engine::{SessionRunner, Turn, estimate_messages_tokens};
use events::Event;
use protocol::{MessagePart, MessagePartBody};
use serde::{Deserialize, Serialize};

use super::{
    ApiError, CommandRequest, MessageRequest, ServerState, apply_estimated_usage_cost,
    compaction_policy, config_api_error, load_history, non_empty, open_checkpoint_store,
    open_database, persist_usage, provider_config_api_error, render_system_context,
    require_session, resolve_turn_agent, resolve_turn_provider, run_message_turn, run_session_bash,
    select_turn_model,
};

static ASYNC_OPERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Default upper bound on how long `GET /session/{id}/wait` blocks before returning the current
/// status, so a stuck turn never hangs a client indefinitely.
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;
/// Hard ceiling for the `wait` timeout, regardless of the requested value.
const MAX_WAIT_TIMEOUT_MS: u64 = 120_000;
/// Polling interval while waiting for a session to become idle.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateSessionRequest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// `PATCH /session/{id}` — update a session's title and/or model selection.
pub(super) async fn update_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<UpdateSessionRequest>,
) -> Result<Json<Session>, ApiError> {
    let db = open_database(&state)?;
    db.sessions().require(&id).map_err(ApiError::from_db)?;

    let title = payload.title.as_deref().and_then(non_empty);
    let model = payload.model.as_deref().and_then(non_empty);
    if title.is_none() && model.is_none() {
        return Err(ApiError::bad_request(
            "update body must include a non-empty title or model",
        ));
    }

    let mut session = None;
    if let Some(model) = &model {
        providers::validate_model_id(model).map_err(ApiError::bad_request)?;
        session = Some(
            db.sessions()
                .set_model(&id, Some(model))
                .map_err(ApiError::from_db)?,
        );
    }
    if let Some(title) = &title {
        session = Some(
            db.sessions()
                .set_title(&id, Some(title))
                .map_err(ApiError::from_db)?,
        );
    }

    let session = match session {
        Some(session) => session,
        None => db.sessions().require(&id).map_err(ApiError::from_db)?,
    };
    Ok(Json(session))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionStatus {
    id: String,
    status: &'static str,
    busy: bool,
    reverted: bool,
    pending_permissions: usize,
}

fn compute_status(state: &ServerState, session: &Session) -> Result<SessionStatus, ApiError> {
    let busy = state
        .active_turns
        .is_active(&session.id)
        .map_err(|message| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, message))?
        || state.remote_turns.is_active(&session.id)?;
    let pending = state.permissions.list(Some(session.id.as_str()))?.len();
    let status = if busy {
        "running"
    } else if pending > 0 {
        "waiting"
    } else {
        "idle"
    };
    Ok(SessionStatus {
        id: session.id.clone(),
        status,
        busy,
        reverted: session.reverted_message_id.is_some(),
        pending_permissions: pending,
    })
}

/// `GET /session/{id}/status` — report whether one session is running, waiting, or idle.
pub(super) async fn session_status(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<SessionStatus>, ApiError> {
    let session = require_session(&state, &id)?;
    Ok(Json(compute_status(&state, &session)?))
}

/// `GET /session/status` — report the status of every persisted session, keyed by session id.
pub(super) async fn session_status_map(
    State(state): State<ServerState>,
) -> Result<Json<std::collections::BTreeMap<String, SessionStatus>>, ApiError> {
    let db = open_database(&state)?;
    let sessions = db.sessions().list().map_err(ApiError::from_db)?;
    let mut map = std::collections::BTreeMap::new();
    for session in sessions {
        let status = compute_status(&state, &session)?;
        map.insert(session.id.clone(), status);
    }
    Ok(Json(map))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WaitQuery {
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// `GET /session/{id}/wait` — block until the session is no longer actively running, or until the
/// bounded timeout elapses, then return its final status.
pub(super) async fn wait_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<WaitQuery>,
) -> Result<Json<SessionStatus>, ApiError> {
    let session = require_session(&state, &id)?;
    let timeout_ms = query
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
        .min(MAX_WAIT_TIMEOUT_MS);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let busy = state
            .active_turns
            .is_active(&id)
            .map_err(|message| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, message))?
            || state.remote_turns.is_active(&id)?;
        if !busy || Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(WAIT_POLL_INTERVAL).await;
    }
    // Re-read the session so the reverted flag reflects any concurrent change.
    let session = require_session(&state, &session.id)?;
    Ok(Json(compute_status(&state, &session)?))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionContext {
    id: String,
    model: String,
    agent: String,
    message_count: usize,
    estimated_tokens: usize,
    reverted: bool,
    busy: bool,
}

/// `GET /session/{id}/context` — report the assembled turn context (model/agent/token estimate)
/// for the session without running a turn. This path opens no outbound socket.
pub(super) async fn session_context(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<SessionContext>, ApiError> {
    let db = open_database(&state)?;
    let session = db.sessions().require(&id).map_err(ApiError::from_db)?;
    let config_inputs = state
        .config
        .inputs(&state.cwd, std::collections::BTreeMap::new());
    let effective_config = settings::load(config_inputs.clone()).map_err(config_api_error)?;
    let selected_agent = resolve_turn_agent(&config_inputs, effective_config.agent(), "serve")
        .map_err(ApiError::bad_request)?;
    let model = select_turn_model(
        None,
        selected_agent.model(),
        session.model.as_deref(),
        effective_config.model(),
    )
    .map_err(ApiError::bad_request)?;
    let history = load_history(&db, &id)?;
    let busy = state
        .active_turns
        .is_active(&id)
        .map_err(|message| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, message))?
        || state.remote_turns.is_active(&id)?;
    Ok(Json(SessionContext {
        id: session.id.clone(),
        model,
        agent: selected_agent.name().to_owned(),
        message_count: history.len(),
        estimated_tokens: estimate_messages_tokens(&history),
        reverted: session.reverted_message_id.is_some(),
        busy,
    }))
}

/// `GET /session/{id}/children` — list the sessions forked from this one.
pub(super) async fn list_children(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<Session>>, ApiError> {
    let db = open_database(&state)?;
    db.sessions().require(&id).map_err(ApiError::from_db)?;
    let children = db.sessions().children(&id).map_err(ApiError::from_db)?;
    Ok(Json(children))
}

/// `GET /session/{id}/diff` — report the net file changes produced by this session's turns.
pub(super) async fn session_diff(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<checkpoints::FileDiff>>, ApiError> {
    require_session(&state, &id)?;
    let Some(store) = open_checkpoint_store(&state) else {
        return Ok(Json(Vec::new()));
    };
    let diffs = store.session_file_diffs(&id).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot compute session diff: {err}"),
        )
    })?;
    Ok(Json(diffs))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ForkRequest {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

/// `POST /session/{id}/fork` — create a child session that inherits this session's transcript
/// (optionally truncated at `messageID`).
pub(super) async fn fork_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    payload: Option<Json<ForkRequest>>,
) -> Result<(StatusCode, Json<Session>), ApiError> {
    let payload = payload.map(|Json(body)| body).unwrap_or_default();
    let db = open_database(&state)?;
    let parent = db.sessions().require(&id).map_err(ApiError::from_db)?;
    let mut messages = db
        .messages()
        .list_for_session(&id)
        .map_err(ApiError::from_db)?;

    if let Some(cutoff) = payload.message_id.as_deref().and_then(non_empty) {
        match messages.iter().position(|message| message.id == cutoff) {
            Some(index) => messages.truncate(index + 1),
            None => {
                return Err(ApiError::bad_request(format!(
                    "message `{cutoff}` does not belong to session `{id}`"
                )));
            }
        }
    }

    let title = payload
        .title
        .as_deref()
        .and_then(non_empty)
        .or_else(|| parent.title.clone());
    let child = db
        .sessions()
        .create_child(&id, title.as_deref())
        .map_err(ApiError::from_db)?;
    if let Some(model) = parent.model.as_deref() {
        db.sessions()
            .set_model(&child.id, Some(model))
            .map_err(ApiError::from_db)?;
    }

    for message in &messages {
        let parts: Vec<MessagePart> = message
            .parts
            .iter()
            .map(|part| {
                let mut part = part.clone();
                part.id = String::new();
                part.created_at = None;
                part
            })
            .collect();
        db.messages()
            .create_with_parts(&child.id, &message.role, &message.content, &parts)
            .map_err(ApiError::from_db)?;
    }

    let child = db
        .sessions()
        .require(&child.id)
        .map_err(ApiError::from_db)?;
    Ok((StatusCode::CREATED, Json(child)))
}

/// `POST /session/{id}/init` — run a real turn that analyzes the project and creates/updates an
/// AGENTS.md file using the agent's tools.
pub(super) async fn init_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<super::PromptResponse>, ApiError> {
    const INIT_INSTRUCTION: &str = "Analyze this project's structure, build/test commands, and \
        conventions, then create or update an AGENTS.md file at the project root capturing what a \
        coding agent needs to know to work here. Keep it concise and accurate.";
    run_message_turn(state, id, INIT_INSTRUCTION.to_owned(), None)
        .await
        .map(Json)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SummaryResponse {
    status: &'static str,
    model: String,
    summary: String,
    message: Message,
}

/// Run a tool-free auxiliary LLM turn over the session history with `instruction`, persist the
/// result as an assistant message (optionally marked as a compaction part), and return it.
async fn run_auxiliary_summary(
    state: &ServerState,
    session_id: &str,
    instruction: &str,
    as_compaction: bool,
) -> Result<SummaryResponse, ApiError> {
    let db = open_database(state)?;
    let session = db
        .sessions()
        .require(session_id)
        .map_err(ApiError::from_db)?;
    let config_inputs = state
        .config
        .inputs(&state.cwd, std::collections::BTreeMap::new());
    let effective_config = settings::load(config_inputs.clone()).map_err(config_api_error)?;
    let selected_agent = resolve_turn_agent(&config_inputs, effective_config.agent(), "serve")
        .map_err(ApiError::bad_request)?;
    let selected_model = select_turn_model(
        None,
        selected_agent.model(),
        session.model.as_deref(),
        effective_config.model(),
    )
    .map_err(ApiError::bad_request)?;
    let provider_config = providers::ProviderConfigSet::from_table(effective_config.table())
        .map_err(provider_config_api_error)?;

    if let Err(diagnostic) = redaction::guard_outbound(super::OUTBOUND_OPERATION) {
        return Err(ApiError::diagnostic(
            StatusCode::SERVICE_UNAVAILABLE,
            diagnostic,
        ));
    }

    let resolved = resolve_turn_provider(
        &state.provider,
        &selected_model,
        &provider_config,
        &config_inputs,
    )?;
    let history = load_history(&db, session_id)?;
    let last_message_id = db
        .messages()
        .list_for_session(session_id)
        .map_err(ApiError::from_db)?
        .last()
        .map(|message| message.id.clone())
        .unwrap_or_default();
    let system = render_system_context(&resolved.model_id, &selected_agent, None);
    let mut turn = Turn::new(&resolved.model_id, system, instruction.to_owned())
        .with_history(history)
        .with_compaction(compaction_policy(&resolved.model));
    if !resolved.request_params.is_empty() {
        turn = turn.with_extra_params(resolved.request_params.clone());
    }

    let runner = SessionRunner::default();
    let mut outcome = runner
        .run(resolved.provider.as_ref(), turn, |_event| {})
        .await;
    apply_estimated_usage_cost(&resolved.model, &mut outcome);
    if let Some(error) = &outcome.error {
        return Err(ApiError::from_run_error(error));
    }
    persist_usage(&db, session_id, &outcome);

    let summary = outcome.answer().to_owned();
    let mut parts = Vec::new();
    if as_compaction {
        parts.push(MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::Compaction {
                summary: summary.clone(),
                tail_start_id: last_message_id,
                continued_request: false,
            },
        });
    }
    parts.push(MessagePart::text(summary.clone()));
    let message = db
        .messages()
        .create_with_parts(session_id, "assistant", &summary, &parts)
        .map_err(ApiError::from_db)?;
    state.event_bus.publish(&Event::message_parts(
        session_id.to_owned(),
        message.id.clone(),
        message.parts.clone(),
    ));

    Ok(SummaryResponse {
        status: if as_compaction {
            "compacted"
        } else {
            "summarized"
        },
        model: resolved.model_id,
        summary: redaction::redact(&summary),
        message,
    })
}

/// `POST /session/{id}/summarize` — produce a concise AI summary of the session, persisted as a
/// visible assistant message.
pub(super) async fn summarize_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<SummaryResponse>, ApiError> {
    const SUMMARIZE_INSTRUCTION: &str = "Summarize this conversation so far. Preserve key facts, \
        decisions, and open work. Be concise and factual. Respond with only the summary text.";
    run_auxiliary_summary(&state, &id, SUMMARIZE_INSTRUCTION, false)
        .await
        .map(Json)
}

/// `POST /session/{id}/compact` — compact the session history into an AI summary, persisted as a
/// first-class compaction part plus visible summary text.
pub(super) async fn compact_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<SummaryResponse>, ApiError> {
    const COMPACT_INSTRUCTION: &str = "Compact this conversation by summarizing the removed context \
        so future turns preserve key facts, decisions, and open tasks. Be concise and factual. \
        Respond with only the summary text.";
    run_auxiliary_summary(&state, &id, COMPACT_INSTRUCTION, true)
        .await
        .map(Json)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AsyncPromptResponse {
    status: &'static str,
    operation_id: String,
    session_id: String,
    queued: bool,
}

/// `POST /session/{id}/prompt_async` — admit a prompt and return immediately with an operation id;
/// the turn runs in the background and streams progress over `/event`.
pub(super) async fn prompt_async(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<MessageRequest>,
) -> Result<(StatusCode, Json<AsyncPromptResponse>), ApiError> {
    require_session(&state, &id)?;
    let model = payload.model_override();
    let prompt = payload
        .into_prompt()
        .ok_or_else(|| ApiError::bad_request("prompt body must include non-empty text"))?;
    let operation_id = format!(
        "op-{}-{}",
        std::process::id(),
        ASYNC_OPERATION_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let busy = state
        .active_turns
        .is_active(&id)
        .map_err(|message| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, message))?;

    let task_state = state.clone();
    let task_session = id.clone();
    tokio::spawn(async move {
        if let Err(err) = run_message_turn(task_state, task_session.clone(), prompt, model).await {
            eprintln!("cortex serve: async prompt for session {task_session} failed: {err:?}");
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(AsyncPromptResponse {
            status: "accepted",
            operation_id,
            session_id: id,
            queued: busy,
        }),
    ))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ShellResponse {
    status: &'static str,
    command: String,
    output: String,
    is_error: bool,
    message: Message,
}

/// `POST /session/{id}/shell` — run a shell command in the session context and persist the command
/// and its output into the session transcript.
pub(super) async fn shell_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<CommandRequest>,
) -> Result<Json<ShellResponse>, ApiError> {
    require_session(&state, &id)?;
    let command = non_empty(&payload.command)
        .ok_or_else(|| ApiError::bad_request("shell body must include a non-empty command"))?;
    let db = open_database(&state)?;
    db.messages()
        .create_with_parts(
            &id,
            "user",
            &format!("$ {command}"),
            &[MessagePart::text(format!("$ {command}"))],
        )
        .map_err(ApiError::from_db)?;

    let output = run_session_bash(&state, &id, &command).await?;
    let is_error = output.is_error;
    let rendered = output.model_content();
    let message = db
        .messages()
        .create_with_parts(
            &id,
            "assistant",
            &rendered,
            &[MessagePart::text(rendered.clone())],
        )
        .map_err(ApiError::from_db)?;

    Ok(Json(ShellResponse {
        status: if is_error { "error" } else { "completed" },
        command: redaction::redact(&command),
        output: redaction::redact(&rendered),
        is_error,
        message,
    }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RevertRequest {
    #[serde(default)]
    message_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RevertResponse {
    status: &'static str,
    session: Session,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    conflicts: Vec<String>,
}

/// `POST /session/{id}/revert` — undo the session's most recent file changes via the shadow
/// checkpoint store and mark the session reverted.
pub(super) async fn revert_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    payload: Option<Json<RevertRequest>>,
) -> Result<Json<RevertResponse>, ApiError> {
    let payload = payload.map(|Json(body)| body).unwrap_or_default();
    let db = open_database(&state)?;
    db.sessions().require(&id).map_err(ApiError::from_db)?;

    let Some(store) = open_checkpoint_store(&state) else {
        let session = db.sessions().require(&id).map_err(ApiError::from_db)?;
        return Ok(Json(RevertResponse {
            status: "nothing",
            session,
            paths: Vec::new(),
            conflicts: Vec::new(),
        }));
    };

    let outcome = store.undo().map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot revert session: {err}"),
        )
    })?;
    match outcome {
        checkpoints::RestoreOutcome::Restored { paths, .. } => {
            let marker = payload
                .message_id
                .as_deref()
                .and_then(non_empty)
                .or_else(|| {
                    db.messages()
                        .list_for_session(&id)
                        .ok()
                        .and_then(|messages| messages.last().map(|message| message.id.clone()))
                });
            let session = db
                .sessions()
                .set_reverted_message(&id, marker.as_deref())
                .map_err(ApiError::from_db)?;
            Ok(Json(RevertResponse {
                status: "reverted",
                session,
                paths,
                conflicts: Vec::new(),
            }))
        }
        checkpoints::RestoreOutcome::Nothing => {
            let session = db.sessions().require(&id).map_err(ApiError::from_db)?;
            Ok(Json(RevertResponse {
                status: "nothing",
                session,
                paths: Vec::new(),
                conflicts: Vec::new(),
            }))
        }
        checkpoints::RestoreOutcome::Conflict { conflicts, .. } => Err(ApiError {
            status: StatusCode::CONFLICT,
            message: "revert refused: files changed outside the session since the snapshot"
                .to_owned(),
            kind: Some("conflict".to_owned()),
        })
        .map_err(|mut err: ApiError| {
            err.message = format!("{}: {}", err.message, conflicts.join(", "));
            err
        }),
    }
}

/// `POST /session/{id}/unrevert` — redo previously reverted file changes and clear the session's
/// reverted state.
pub(super) async fn unrevert_session(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<RevertResponse>, ApiError> {
    let db = open_database(&state)?;
    db.sessions().require(&id).map_err(ApiError::from_db)?;

    let Some(store) = open_checkpoint_store(&state) else {
        let session = db
            .sessions()
            .set_reverted_message(&id, None)
            .map_err(ApiError::from_db)?;
        return Ok(Json(RevertResponse {
            status: "nothing",
            session,
            paths: Vec::new(),
            conflicts: Vec::new(),
        }));
    };

    let outcome = store.redo().map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("cannot unrevert session: {err}"),
        )
    })?;
    match outcome {
        checkpoints::RestoreOutcome::Restored { paths, .. } => {
            let session = db
                .sessions()
                .set_reverted_message(&id, None)
                .map_err(ApiError::from_db)?;
            Ok(Json(RevertResponse {
                status: "unreverted",
                session,
                paths,
                conflicts: Vec::new(),
            }))
        }
        checkpoints::RestoreOutcome::Nothing => {
            let session = db
                .sessions()
                .set_reverted_message(&id, None)
                .map_err(ApiError::from_db)?;
            Ok(Json(RevertResponse {
                status: "nothing",
                session,
                paths: Vec::new(),
                conflicts: Vec::new(),
            }))
        }
        checkpoints::RestoreOutcome::Conflict { conflicts, .. } => Err(ApiError {
            status: StatusCode::CONFLICT,
            message: format!(
                "unrevert refused: files changed outside the session since the snapshot: {}",
                conflicts.join(", ")
            ),
            kind: Some("conflict".to_owned()),
        }),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeleteResponse {
    status: &'static str,
    deleted: bool,
}

/// `GET /session/{id}/message/{message_id}` — fetch a single message with its rich parts.
pub(super) async fn get_message(
    State(state): State<ServerState>,
    AxumPath((id, message_id)): AxumPath<(String, String)>,
) -> Result<Json<Message>, ApiError> {
    let db = open_database(&state)?;
    db.sessions().require(&id).map_err(ApiError::from_db)?;
    let message = db
        .messages()
        .get(&message_id)
        .map_err(ApiError::from_db)?
        .filter(|message| message.session_id == id)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no such message `{message_id}` in session `{id}`"),
            )
        })?;
    Ok(Json(message))
}

/// `DELETE /session/{id}/message/{message_id}` — delete a single message and its parts.
pub(super) async fn delete_message(
    State(state): State<ServerState>,
    AxumPath((id, message_id)): AxumPath<(String, String)>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let db = open_database(&state)?;
    db.sessions().require(&id).map_err(ApiError::from_db)?;
    let message = db
        .messages()
        .get(&message_id)
        .map_err(ApiError::from_db)?
        .filter(|message| message.session_id == id)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no such message `{message_id}` in session `{id}`"),
            )
        })?;
    let deleted = db
        .messages()
        .delete(&message.id)
        .map_err(ApiError::from_db)?;
    Ok(Json(DeleteResponse {
        status: "deleted",
        deleted,
    }))
}

fn require_message_part(
    db: &db::Database,
    session_id: &str,
    message_id: &str,
    part_id: &str,
) -> Result<(), ApiError> {
    db.sessions()
        .require(session_id)
        .map_err(ApiError::from_db)?;
    let message = db
        .messages()
        .get(message_id)
        .map_err(ApiError::from_db)?
        .filter(|message| message.session_id == session_id)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no such message `{message_id}` in session `{session_id}`"),
            )
        })?;
    if !message.parts.iter().any(|part| part.id == part_id) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no such part `{part_id}` in message `{message_id}`"),
        ));
    }
    Ok(())
}

/// `PATCH /session/{id}/message/{message_id}/part/{part_id}` — replace one rich message part.
pub(super) async fn update_part(
    State(state): State<ServerState>,
    AxumPath((id, message_id, part_id)): AxumPath<(String, String, String)>,
    Json(part): Json<MessagePart>,
) -> Result<Json<MessagePart>, ApiError> {
    let db = open_database(&state)?;
    require_message_part(&db, &id, &message_id, &part_id)?;
    let updated = db
        .messages()
        .update_part(&part_id, &part)
        .map_err(ApiError::from_db)?;
    Ok(Json(updated))
}

/// `DELETE /session/{id}/message/{message_id}/part/{part_id}` — delete one rich message part.
pub(super) async fn delete_part(
    State(state): State<ServerState>,
    AxumPath((id, message_id, part_id)): AxumPath<(String, String, String)>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let db = open_database(&state)?;
    require_message_part(&db, &id, &message_id, &part_id)?;
    let deleted = db
        .messages()
        .delete_part(&part_id)
        .map_err(ApiError::from_db)?;
    Ok(Json(DeleteResponse {
        status: "deleted",
        deleted,
    }))
}

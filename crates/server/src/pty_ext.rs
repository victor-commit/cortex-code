//! PTY lifecycle routes backed by a real pseudo-terminal.
//!
//! Clients create a PTY (`POST /pty`), stream its output (`GET /pty/{id}/stream` as SSE), send input
//! (`POST /pty/{id}/input`), resize it (`POST /pty/{id}/resize`), and close it (`DELETE /pty/{id}`).
//! Each PTY runs a real child process on a real pseudo-terminal via `portable-pty`. The spawning
//! child is tracked by its exact OS PID and is killed and reaped on close (or on server shutdown),
//! so no orphaned terminal child remains.

use std::collections::HashMap;
use std::convert::Infallible;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
};
use base64::Engine as _;
use futures_util::stream;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use super::{ApiError, ServerState};
use crate::file_ext::resolve_in_workspace;

/// Upper bound on buffered PTY output retained for replay to late stream subscribers.
const MAX_BUFFER_BYTES: usize = 4 * 1024 * 1024;
/// Poll interval for the SSE output stream.
const STREAM_POLL: Duration = Duration::from_millis(60);

static PTY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Registry of live PTY sessions shared across handlers. Cloned into [`ServerState`].
#[derive(Clone, Default)]
pub(super) struct PtyRegistry {
    inner: Arc<Mutex<HashMap<String, PtySession>>>,
}

struct PtySession {
    pid: Option<u32>,
    cols: u16,
    rows: u16,
    command: String,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    output: Arc<Mutex<Vec<u8>>>,
    exited: Arc<AtomicBool>,
}

impl PtySession {
    fn summary(&self, id: &str) -> PtySummary {
        PtySummary {
            id: id.to_owned(),
            pid: self.pid,
            cols: self.cols,
            rows: self.rows,
            command: self.command.clone(),
            running: !self.exited.load(Ordering::Relaxed),
        }
    }
}

impl PtyRegistry {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, PtySession>>, ApiError> {
        self.inner.lock().map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "pty registry is unavailable",
            )
        })
    }

    /// Kill and reap every live PTY child by its exact handle. Used on server shutdown.
    pub(super) fn shutdown(&self) {
        if let Ok(mut sessions) = self.inner.lock() {
            for (_, mut session) in sessions.drain() {
                let _ = session.child.kill();
                let _ = session.child.wait();
            }
        }
    }
}

/// A PTY session's observable state.
#[derive(Debug, Serialize)]
pub(super) struct PtySummary {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    cols: u16,
    rows: u16,
    command: String,
    running: bool,
}

/// Request body for `POST /pty`.
#[derive(Debug, Deserialize)]
pub(super) struct CreatePtyRequest {
    /// Program to run; defaults to the login shell (`$SHELL` or `/bin/sh`).
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    /// Workspace-relative working directory; defaults to the server workspace.
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    env: HashMap<String, String>,
}

/// `POST /pty` — create a PTY running a real child process and return its id and PID.
pub(super) async fn create_pty(
    State(state): State<ServerState>,
    Json(payload): Json<CreatePtyRequest>,
) -> Result<(StatusCode, Json<PtySummary>), ApiError> {
    let cols = payload.cols.unwrap_or(80).clamp(1, 1000);
    let rows = payload.rows.unwrap_or(24).clamp(1, 1000);
    let cwd = match payload
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(requested) => resolve_in_workspace(&state.cwd, requested)?,
        None => state.cwd.clone(),
    };

    let program = payload
        .command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(default_shell);

    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not open pty: {err}"),
            )
        })?;

    let mut builder = CommandBuilder::new(&program);
    for arg in &payload.args {
        builder.arg(arg);
    }
    builder.cwd(&cwd);
    builder.env("TERM", "xterm-256color");
    for (key, value) in &payload.env {
        builder.env(key, value);
    }

    let child = pair.slave.spawn_command(builder).map_err(|err| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "could not spawn `{}` in a pty: {err}",
                redaction::redact(&program)
            ),
        )
    })?;
    // Drop the slave so the master read side observes EOF once the child exits.
    drop(pair.slave);

    let pid = child.process_id();
    let reader = pair.master.try_clone_reader().map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not read pty output: {err}"),
        )
    })?;
    let writer = pair.master.take_writer().map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not write pty input: {err}"),
        )
    })?;

    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let exited = Arc::new(AtomicBool::new(false));
    spawn_reader(reader, output.clone(), exited.clone());

    let id = format!(
        "pty-{}-{}",
        std::process::id(),
        PTY_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let session = PtySession {
        pid,
        cols,
        rows,
        command: redaction::redact(&program),
        master: pair.master,
        writer,
        child,
        output,
        exited,
    };
    let summary = session.summary(&id);
    state.ptys.lock()?.insert(id.clone(), session);
    state
        .event_bus
        .publish(&events::Event::PtyCreated(events::PtyEvent {
            id: id.clone(),
            shell: Some(redaction::redact(&program)),
        }));
    Ok((StatusCode::CREATED, Json(summary)))
}

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    exited: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut sink) = output.lock() {
                        if sink.len() < MAX_BUFFER_BYTES {
                            let room = MAX_BUFFER_BYTES - sink.len();
                            sink.extend_from_slice(&buf[..n.min(room)]);
                        }
                    }
                }
                Err(_) => break,
            }
        }
        exited.store(true, Ordering::Relaxed);
    });
}

/// `GET /pty` — list live PTY sessions.
pub(super) async fn list_ptys(
    State(state): State<ServerState>,
) -> Result<Json<Vec<PtySummary>>, ApiError> {
    let sessions = state.ptys.lock()?;
    let mut summaries: Vec<PtySummary> = sessions
        .iter()
        .map(|(id, session)| session.summary(id))
        .collect();
    summaries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(Json(summaries))
}

/// `GET /pty/{id}` — report one PTY's current state.
pub(super) async fn get_pty(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PtySummary>, ApiError> {
    let sessions = state.ptys.lock()?;
    let session = sessions.get(&id).ok_or_else(|| pty_not_found(&id))?;
    Ok(Json(session.summary(&id)))
}

/// Request body for `POST /pty/{id}/input`.
#[derive(Debug, Deserialize)]
pub(super) struct PtyInputRequest {
    data: String,
    /// Interpret `data` as base64 (for raw control bytes) rather than UTF-8 text.
    #[serde(default)]
    base64: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct PtyInputResponse {
    status: &'static str,
    bytes: usize,
}

/// `POST /pty/{id}/input` — write input bytes to the PTY.
pub(super) async fn pty_input(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<PtyInputRequest>,
) -> Result<Json<PtyInputResponse>, ApiError> {
    let bytes = if payload.base64 {
        base64::engine::general_purpose::STANDARD
            .decode(payload.data.as_bytes())
            .map_err(|err| ApiError::bad_request(format!("invalid base64 input: {err}")))?
    } else {
        payload.data.into_bytes()
    };
    let mut sessions = state.ptys.lock()?;
    let session = sessions.get_mut(&id).ok_or_else(|| pty_not_found(&id))?;
    session.writer.write_all(&bytes).map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not write to pty: {err}"),
        )
    })?;
    session.writer.flush().map_err(|err| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not flush pty input: {err}"),
        )
    })?;
    Ok(Json(PtyInputResponse {
        status: "written",
        bytes: bytes.len(),
    }))
}

/// Request body for `POST /pty/{id}/resize`.
#[derive(Debug, Deserialize)]
pub(super) struct PtyResizeRequest {
    cols: u16,
    rows: u16,
}

/// `POST /pty/{id}/resize` — resize the PTY and update its stored dimensions.
pub(super) async fn pty_resize(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<PtyResizeRequest>,
) -> Result<Json<PtySummary>, ApiError> {
    let cols = payload.cols.clamp(1, 1000);
    let rows = payload.rows.clamp(1, 1000);
    let mut sessions = state.ptys.lock()?;
    let session = sessions.get_mut(&id).ok_or_else(|| pty_not_found(&id))?;
    session
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|err| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not resize pty: {err}"),
            )
        })?;
    session.cols = cols;
    session.rows = rows;
    Ok(Json(session.summary(&id)))
}

#[derive(Debug, Serialize)]
pub(super) struct PtyCloseResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    exited: bool,
}

/// `DELETE /pty/{id}` — kill the PTY child by its exact handle, reap it, and drop the session.
pub(super) async fn close_pty(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PtyCloseResponse>, ApiError> {
    let mut session = {
        let mut sessions = state.ptys.lock()?;
        sessions.remove(&id).ok_or_else(|| pty_not_found(&id))?
    };
    let pid = session.pid;
    let _ = session.child.kill();
    let exited = session.child.wait().is_ok();
    state
        .event_bus
        .publish(&events::Event::PtyDeleted(events::PtyRef {
            id: id.clone(),
        }));
    state
        .event_bus
        .publish(&events::Event::PtyExited(events::PtyExited {
            id,
            exit_code: None,
        }));
    Ok(Json(PtyCloseResponse {
        status: "closed",
        pid,
        exited,
    }))
}

/// `GET /pty/{id}/stream` — stream PTY output as SSE frames carrying base64 chunks.
///
/// Buffered output produced before the client connects is replayed first, then live output is
/// streamed until the child exits, after which a terminal `exit` event is emitted and the stream
/// ends.
pub(super) async fn pty_stream(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let (output, exited) = {
        let sessions = state.ptys.lock()?;
        let session = sessions.get(&id).ok_or_else(|| pty_not_found(&id))?;
        (session.output.clone(), session.exited.clone())
    };

    let stream_state = PtyStreamState {
        output,
        exited,
        cursor: 0,
        finished: false,
    };
    let stream = stream::unfold(stream_state, |mut stream_state| async move {
        loop {
            if stream_state.finished {
                return None;
            }
            let (chunk, len) = {
                let sink = stream_state
                    .output
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let len = sink.len();
                if len > stream_state.cursor {
                    (Some(sink[stream_state.cursor..].to_vec()), len)
                } else {
                    (None, len)
                }
            };
            if let Some(chunk) = chunk {
                stream_state.cursor = len;
                let encoded = base64::engine::general_purpose::STANDARD.encode(&chunk);
                let frame = SseEvent::default()
                    .event("output")
                    .json_data(serde_json::json!({ "data": encoded }))
                    .unwrap_or_else(|_| SseEvent::default().data(""));
                return Some((Ok(frame), stream_state));
            }
            if stream_state.exited.load(Ordering::Relaxed) {
                // Drain any final bytes appended between the snapshot and the exit flag.
                let remaining = {
                    let sink = stream_state
                        .output
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    sink.len() > stream_state.cursor
                };
                if remaining {
                    continue;
                }
                stream_state.finished = true;
                let frame = SseEvent::default()
                    .event("exit")
                    .json_data(serde_json::json!({ "status": "exited" }))
                    .unwrap_or_else(|_| SseEvent::default().data(""));
                return Some((Ok(frame), stream_state));
            }
            tokio::time::sleep(STREAM_POLL).await;
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

struct PtyStreamState {
    output: Arc<Mutex<Vec<u8>>>,
    exited: Arc<AtomicBool>,
    cursor: usize,
    finished: bool,
}

fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".to_owned())
}

fn pty_not_found(id: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, format!("no such pty: {id}"))
}

//! `cortex attach`: a read-only observer that streams a session's messages until you detach.
//!
//! Attach prints a session's existing transcript and then follows newly persisted messages from
//! the shared SQLite store, which both headless `cortex run` and `cortex serve` write through. It
//! never starts or aborts a turn: detaching (Ctrl-C, the end of `--timeout`, or `--once`) just
//! stops observing, leaving any in-flight turn running in its own process. With `--server <URL>`
//! the existing transcript is fetched from a loopback `cortex serve` over HTTP (proving the server
//! attach path) before the live tail continues against the shared database.

use std::collections::HashSet;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use db::{Database, DbError, DbPathInputs, Message};
use errors::Exit;
use serde::{Deserialize, Serialize};

use crate::cli::{AttachArgs, OutputFormat};

/// Minimum poll interval, so a `--poll-ms 0` cannot spin the CPU.
const MIN_POLL: Duration = Duration::from_millis(10);

pub fn run(args: AttachArgs) -> Exit {
    let db = match open_database() {
        Ok(db) => db,
        Err(exit) => return exit,
    };

    let session_id = match resolve_session(&db, &args) {
        Ok(id) => id,
        Err(exit) => return exit,
    };

    let initial = match args.server.as_deref() {
        Some(server) => match fetch_server_messages(server, &session_id) {
            Ok(messages) => messages,
            Err(exit) => return exit,
        },
        None => match db.messages().list_for_session(&session_id) {
            Ok(messages) => messages,
            Err(err) => return db_error(&err),
        },
    };

    let format = args.format.format;
    let mut stdout = io::stdout();
    let mut seen: HashSet<String> = HashSet::new();
    for message in &initial {
        if emit_message(&mut stdout, message, format).is_err() {
            return Exit::Failure;
        }
        seen.insert(message.id.clone());
    }

    if args.once {
        return Exit::Success;
    }

    follow(&db, &session_id, format, &args, &mut seen, &mut stdout)
}

/// Poll the shared database for newly persisted messages and stream them until the bounded
/// `--timeout` elapses (or, when unbounded, until the process is interrupted). Reading is
/// non-mutating, so detaching here never disturbs the turn that is producing the messages.
fn follow(
    db: &Database,
    session_id: &str,
    format: OutputFormat,
    args: &AttachArgs,
    seen: &mut HashSet<String>,
    stdout: &mut io::Stdout,
) -> Exit {
    let poll = Duration::from_millis(args.poll_ms).max(MIN_POLL);
    let deadline = (args.timeout > 0).then(|| Instant::now() + Duration::from_secs(args.timeout));
    loop {
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            return Exit::Success;
        }
        std::thread::sleep(poll);
        match db.messages().list_for_session(session_id) {
            Ok(messages) => {
                for message in &messages {
                    if seen.insert(message.id.clone())
                        && emit_message(stdout, message, format).is_err()
                    {
                        return Exit::Failure;
                    }
                }
            }
            Err(err) => return db_error(&err),
        }
    }
}

/// Resolve the session to attach to: an explicit id, else the most recently updated session
/// (looked up through the server when `--server` is set, otherwise from the local store).
fn resolve_session(db: &Database, args: &AttachArgs) -> Result<String, Exit> {
    if let Some(id) = args.session.clone() {
        return Ok(id);
    }
    if let Some(server) = args.server.as_deref() {
        return fetch_latest_server_session(server);
    }
    match db.sessions().most_recent() {
        Ok(Some(session)) => Ok(session.id),
        Ok(None) => {
            eprintln!(
                "cortex attach: no sessions exist yet; pass a session id once one is created"
            );
            Err(Exit::Failure)
        }
        Err(err) => Err(db_error_exit(&err)),
    }
}

fn open_database() -> Result<Database, Exit> {
    let inputs = DbPathInputs::from_process();
    let Some(path) = inputs.resolve() else {
        eprintln!(
            "cortex attach: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Err(Exit::Failure);
    };
    Database::open(&path).map_err(|err| db_error_exit(&err))
}

/// A message rendered for output, with role/content scrubbed of API-key-shaped secrets so an
/// attached observer never sees a credential in cleartext.
#[derive(Serialize)]
struct AttachMessage<'a> {
    id: &'a str,
    session_id: &'a str,
    role: String,
    content: String,
    created_at: &'a str,
}

fn emit_message(
    stdout: &mut io::Stdout,
    message: &Message,
    format: OutputFormat,
) -> io::Result<()> {
    let role = redaction::redact(&message.role);
    let content = redaction::redact(&message.content);
    let mut handle = stdout.lock();
    match format {
        OutputFormat::Text => {
            writeln!(handle, "[{role}] {content}")?;
        }
        OutputFormat::Json | OutputFormat::Jsonl => {
            let projected = AttachMessage {
                id: &message.id,
                session_id: &message.session_id,
                role,
                content,
                created_at: &message.created_at,
            };
            let line = serde_json::to_string(&projected).map_err(io::Error::other)?;
            writeln!(handle, "{line}")?;
        }
    }
    handle.flush()
}

#[derive(Deserialize)]
struct ServerMessageList {
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct ServerSession {
    id: String,
}

/// Fetch the current transcript for a session through a loopback `cortex serve` (the server-attach
/// path), behind the `CORTEX_NO_NET` outbound guard and the network feature gate.
fn fetch_server_messages(server: &str, session_id: &str) -> Result<Vec<Message>, Exit> {
    let url = format!(
        "{}/session/{}/message",
        server.trim_end_matches('/'),
        session_id
    );
    let body = fetch_server_json(&url)?;
    let parsed: ServerMessageList = serde_json::from_str(&body).map_err(|err| {
        eprintln!("cortex attach: could not parse server message list: {err}");
        Exit::Failure
    })?;
    Ok(parsed.messages)
}

/// Resolve the most recent session id through a loopback `cortex serve`.
fn fetch_latest_server_session(server: &str) -> Result<String, Exit> {
    let url = format!("{}/session", server.trim_end_matches('/'));
    let body = fetch_server_json(&url)?;
    let sessions: Vec<ServerSession> = serde_json::from_str(&body).map_err(|err| {
        eprintln!("cortex attach: could not parse server session list: {err}");
        Exit::Failure
    })?;
    match sessions.into_iter().next() {
        Some(session) => Ok(session.id),
        None => {
            eprintln!("cortex attach: the server reports no sessions yet");
            Err(Exit::Failure)
        }
    }
}

/// Perform a real GET against a loopback server, fail-closed under `CORTEX_NO_NET`.
fn fetch_server_json(url: &str) -> Result<String, Exit> {
    if let Err(diag) = redaction::guard_outbound("attach server fetch") {
        eprintln!("cortex attach: {diag}");
        return Err(diag.exit());
    }
    if !crate::feature_gate::network_enabled() {
        return Err(crate::feature_gate::network_unavailable("attach --server"));
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex attach: failed to start the async runtime: {err}");
            return Err(Exit::Failure);
        }
    };
    runtime.block_on(async {
        let client = reqwest::Client::builder().build().map_err(|err| {
            eprintln!(
                "cortex attach: cannot build HTTP client: {}",
                err.without_url()
            );
            Exit::Failure
        })?;
        let response = client.get(url).send().await.map_err(|err| {
            eprintln!("cortex attach: request failed: {}", err.without_url());
            Exit::Failure
        })?;
        let status = response.status();
        if !status.is_success() {
            eprintln!("cortex attach: server returned HTTP {}", status.as_u16());
            return Err(Exit::Failure);
        }
        response.text().await.map_err(|err| {
            eprintln!(
                "cortex attach: cannot read server response: {}",
                err.without_url()
            );
            Exit::Failure
        })
    })
}

fn db_error(err: &DbError) -> Exit {
    db_error_exit(err)
}

fn db_error_exit(err: &DbError) -> Exit {
    eprintln!("cortex attach: {err}");
    Exit::Failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, role: &str, content: &str) -> Message {
        Message {
            id: id.to_owned(),
            session_id: "ses_1".to_owned(),
            role: role.to_owned(),
            content: content.to_owned(),
            parts: Vec::new(),
            created_at: "2026-01-01 00:00:00".to_owned(),
        }
    }

    #[test]
    fn emit_text_renders_role_and_content() {
        let msg = message("msg_1", "assistant", "hello there");
        // Render through the same redaction the streamer uses.
        let role = redaction::redact(&msg.role);
        let content = redaction::redact(&msg.content);
        assert_eq!(format!("[{role}] {content}"), "[assistant] hello there");
    }

    #[test]
    fn emit_json_scrubs_a_secret_in_content() {
        // Assembled from fragments so no contiguous key-shaped literal lives in source.
        let secret = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(4));
        let msg = message("msg_2", "user", &format!("my key is {secret}"));
        let projected = AttachMessage {
            id: &msg.id,
            session_id: &msg.session_id,
            role: redaction::redact(&msg.role),
            content: redaction::redact(&msg.content),
            created_at: &msg.created_at,
        };
        let line = serde_json::to_string(&projected).expect("serialize");
        assert!(!line.contains(&secret), "secret must be redacted: {line}");
        assert!(
            line.contains(redaction::PLACEHOLDER),
            "placeholder present: {line}"
        );
    }

    #[test]
    fn server_message_list_parses_messages_array() {
        let json = r#"{"session":{"id":"ses_1"},"messages":[{"id":"msg_1","session_id":"ses_1","role":"user","content":"hi","created_at":"2026-01-01 00:00:00"}],"usage":{}}"#;
        let parsed: ServerMessageList = serde_json::from_str(json).expect("parse");
        assert_eq!(parsed.messages.len(), 1);
        assert_eq!(parsed.messages[0].content, "hi");
    }
}

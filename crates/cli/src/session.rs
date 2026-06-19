//! The `cortex session` subcommands: a minimal, persistence-touching CLI surface that makes
//! durability observable across process restarts.
//!
//! The database is opened (created and migrated on first use) at the path resolved by
//! [`db::DbPathInputs`]: an explicit `CORTEX_DB_PATH`, else `CORTEX_DATA_DIR/cortex.db`, else
//! the default under the user's data home. All diagnostics go to stderr; machine output
//! (`--format json`) is a single strict JSON document on stdout.

use std::io::{self, IsTerminal, Read, Write};

use db::{Database, DbError, DbPathInputs, Message, Session};
use errors::Exit;
use serde::Serialize;

use crate::cli::{
    OutputFormat, SessionArgs, SessionCommand, SessionListArgs, SessionMessageArgs, SessionNewArgs,
    SessionShowArgs,
};

/// Dispatch a `cortex session` invocation.
pub fn run(args: SessionArgs) -> Exit {
    let db = match open_database() {
        Ok(db) => db,
        Err(exit) => return exit,
    };

    match args.command {
        SessionCommand::New(args) => new(&db, args),
        SessionCommand::List(args) => list(&db, args),
        SessionCommand::Show(args) => show(&db, args),
        SessionCommand::Message(args) => message(&db, args),
    }
}

/// Resolve the database path and open it, mapping any persistence failure to a clear stderr
/// diagnostic and a non-zero exit (never a panic).
fn open_database() -> Result<Database, Exit> {
    let inputs = DbPathInputs::from_process();
    let Some(path) = inputs.resolve() else {
        eprintln!(
            "cortex session: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Err(Exit::Failure);
    };
    Database::open(&path).map_err(|err| db_error(&err))
}

fn new(db: &Database, args: SessionNewArgs) -> Exit {
    match db.sessions().create(args.title.as_deref()) {
        Ok(session) => match args.format.format {
            OutputFormat::Json | OutputFormat::Jsonl => emit_json(&session),
            OutputFormat::Text => emit_text(&format!("{}\n", session.id)),
        },
        Err(err) => db_error(&err),
    }
}

fn list(db: &Database, args: SessionListArgs) -> Exit {
    match db.sessions().list() {
        Ok(sessions) => match args.format.format {
            OutputFormat::Json | OutputFormat::Jsonl => emit_json(&SessionList { sessions }),
            OutputFormat::Text => emit_text(&render_session_list(&sessions)),
        },
        Err(err) => db_error(&err),
    }
}

fn show(db: &Database, args: SessionShowArgs) -> Exit {
    let session = match db.sessions().require(&args.id) {
        Ok(session) => session,
        Err(err) => return db_error(&err),
    };
    let messages = match db.messages().list_for_session(&session.id) {
        Ok(messages) => messages,
        Err(err) => return db_error(&err),
    };

    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&SessionDetail { session, messages }),
        OutputFormat::Text => emit_text(&render_session_detail(&session, &messages)),
    }
}

fn message(db: &Database, args: SessionMessageArgs) -> Exit {
    let content = match resolve_content(args.content) {
        Ok(Some(content)) => content,
        Ok(None) => {
            eprintln!(
                "cortex session: no message content (pass an argument or pipe text on stdin)"
            );
            return Exit::MissingInput;
        }
        Err(err) => {
            eprintln!("cortex session: failed to read stdin: {err}");
            return Exit::Failure;
        }
    };

    match db.messages().create(&args.id, &args.role, &content) {
        Ok(message) => match args.format.format {
            OutputFormat::Json | OutputFormat::Jsonl => emit_json(&message),
            OutputFormat::Text => emit_text(&format!("{}\n", message.id)),
        },
        Err(err) => db_error(&err),
    }
}

/// Resolve the message content: a positional argument wins; otherwise stdin is read (an
/// interactive terminal with nothing piped is treated as "no content" rather than blocking).
fn resolve_content(arg: Option<String>) -> io::Result<Option<String>> {
    if let Some(content) = arg {
        return Ok(Some(content));
    }
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }
    let mut buffer = String::new();
    stdin.lock().read_to_string(&mut buffer)?;
    let trimmed = buffer.strip_suffix('\n').unwrap_or(&buffer);
    let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

/// A session listing as a single JSON document.
#[derive(Serialize)]
struct SessionList {
    sessions: Vec<Session>,
}

/// A session plus its messages as a single JSON document.
#[derive(Serialize)]
struct SessionDetail {
    session: Session,
    messages: Vec<Message>,
}

fn render_session_list(sessions: &[Session]) -> String {
    if sessions.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for session in sessions {
        let title = session.title.as_deref().unwrap_or("");
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            session.id, session.created_at, title
        ));
    }
    out
}

fn render_session_detail(session: &Session, messages: &[Message]) -> String {
    let mut out = String::new();
    out.push_str(&format!("session {}\n", session.id));
    if let Some(title) = session.title.as_deref() {
        out.push_str(&format!("title: {title}\n"));
    }
    out.push_str(&format!("created: {}\n", session.created_at));
    out.push_str(&format!("messages: {}\n", messages.len()));
    for message in messages {
        out.push_str(&format!("[{}] {}\n", message.role, message.content));
    }
    out
}

/// Serialize a value to a single-line strict JSON document terminated with a newline.
fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex session: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

/// Write a body to stdout, mapping a broken pipe to a generic failure exit.
fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

/// Render a [`DbError`] to stderr and map it to a non-zero exit.
fn db_error(err: &DbError) -> Exit {
    eprintln!("cortex session: {err}");
    Exit::Failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, title: Option<&str>) -> Session {
        Session {
            id: id.to_owned(),
            title: title.map(str::to_owned),
            created_at: "2026-01-01 00:00:00".to_owned(),
            updated_at: "2026-01-01 00:00:00".to_owned(),
            model: None,
        }
    }

    #[test]
    fn empty_session_list_renders_empty_text() {
        assert_eq!(render_session_list(&[]), "");
    }

    #[test]
    fn session_list_text_is_one_tab_separated_line_per_session() {
        let rendered = render_session_list(&[session("ses_a", Some("hi")), session("ses_b", None)]);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("ses_a\t"));
        assert!(lines[0].ends_with("\thi"));
        assert!(lines[1].ends_with('\t'), "absent title renders empty");
    }

    #[test]
    fn detail_text_includes_messages_with_role_and_content() {
        let s = session("ses_a", Some("t"));
        let messages = vec![Message {
            id: "msg_1".to_owned(),
            session_id: "ses_a".to_owned(),
            role: "user".to_owned(),
            content: "marmalade".to_owned(),
            created_at: "2026-01-01 00:00:00".to_owned(),
        }];
        let rendered = render_session_detail(&s, &messages);
        assert!(rendered.contains("session ses_a"));
        assert!(rendered.contains("[user] marmalade"));
    }

    #[test]
    fn content_argument_takes_precedence() {
        let resolved = resolve_content(Some("from-arg".to_owned())).expect("resolve");
        assert_eq!(resolved, Some("from-arg".to_owned()));
    }
}

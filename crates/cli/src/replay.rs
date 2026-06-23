//! The `cortex replay` subcommand: reconstruct a persisted transcript from SQLite.
//!
//! Replay is a local persistence read only. It never calls the model or opens network sockets, and
//! it works identically under `CORTEX_NO_NET=1`.

use std::io::{self, Write};

use db::{Database, DbError, DbPathInputs, Message};
use errors::Exit;
use serde::Serialize;

use crate::cli::{OutputFormat, ReplayArgs};

/// Dispatch a `cortex replay` invocation.
pub fn run(args: ReplayArgs) -> Exit {
    let db = match open_database() {
        Ok(db) => db,
        Err(exit) => return exit,
    };

    let session = match db.sessions().require(&args.id) {
        Ok(session) => session,
        Err(err) => return db_error(&err),
    };
    let messages = match db.messages().list_for_session(&session.id) {
        Ok(messages) => messages,
        Err(err) => return db_error(&err),
    };

    match args.format.format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&ReplayJson {
            session_id: session.id,
            messages: messages
                .iter()
                .map(|message| ReplayMessage {
                    role: message.role.clone(),
                    text: message.content.clone(),
                })
                .collect(),
        }),
        OutputFormat::Text => emit_text(&render_text(&messages)),
    }
}

fn open_database() -> Result<Database, Exit> {
    let inputs = DbPathInputs::from_process();
    let Some(path) = inputs.resolve() else {
        eprintln!(
            "cortex replay: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Err(Exit::Failure);
    };
    Database::open(&path).map_err(|err| db_error(&err))
}

#[derive(Serialize)]
struct ReplayJson {
    session_id: String,
    messages: Vec<ReplayMessage>,
}

#[derive(Serialize)]
struct ReplayMessage {
    role: String,
    text: String,
}

fn render_text(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        out.push_str(&format!("[{}] {}\n", message.role, message.content));
    }
    out
}

fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex replay: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

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

fn db_error(err: &DbError) -> Exit {
    eprintln!("cortex replay: {err}");
    Exit::Failure
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_output_preserves_role_labels_and_order() {
        let messages = vec![
            Message {
                id: "msg_1".to_owned(),
                session_id: "ses_1".to_owned(),
                role: "user".to_owned(),
                content: "first".to_owned(),
                created_at: "2026-01-01 00:00:00".to_owned(),
            },
            Message {
                id: "msg_2".to_owned(),
                session_id: "ses_1".to_owned(),
                role: "assistant".to_owned(),
                content: "second".to_owned(),
                created_at: "2026-01-01 00:00:01".to_owned(),
            },
        ];
        let rendered = render_text(&messages);
        assert_eq!(rendered, "[user] first\n[assistant] second\n");
    }
}

//! `cortex export` / `cortex import` session artifact handling.

use std::fs;
use std::io::{self, Read, Write};

use db::{Database, DbError, DbPathInputs};
use errors::Exit;
use protocol::MessagePart;
use serde::{Deserialize, Serialize};

use crate::cli::{ExportArgs, ImportArgs};

const ARTIFACT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SessionArtifact {
    version: u32,
    session: ArtifactSession,
    messages: Vec<ArtifactMessage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ArtifactSession {
    id: String,
    title: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ArtifactMessage {
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parts: Vec<MessagePart>,
}

pub fn run_export(args: ExportArgs) -> Exit {
    let db = match open_database("export") {
        Ok(db) => db,
        Err(exit) => return exit,
    };

    let session = match db.sessions().require(&args.session_id) {
        Ok(session) => session,
        Err(err) => return db_error("export", &err),
    };
    let messages = match db.messages().list_for_session(&session.id) {
        Ok(messages) => messages,
        Err(err) => return db_error("export", &err),
    };

    let artifact = SessionArtifact {
        version: ARTIFACT_VERSION,
        session: ArtifactSession {
            id: session.id,
            title: session.title.map(|value| redaction::redact(&value)),
            model: session.model.map(|value| redaction::redact(&value)),
        },
        messages: messages
            .into_iter()
            .map(|message| ArtifactMessage {
                role: redaction::redact(&message.role),
                content: redaction::redact(&message.content),
                parts: message.parts,
            })
            .collect(),
    };

    emit_json("export", &artifact)
}

pub fn run_import(args: ImportArgs) -> Exit {
    let artifact_source = match read_artifact_source(&args.artifact) {
        Ok(source) => source,
        Err(exit) => return exit,
    };
    let artifact = match parse_artifact(&artifact_source) {
        Ok(artifact) => artifact,
        Err(message) => {
            eprintln!("cortex import: {message}");
            return Exit::Failure;
        }
    };
    if let Err(message) = validate_artifact(&artifact) {
        eprintln!("cortex import: {message}");
        return Exit::Failure;
    }

    let db = match open_database("import") {
        Ok(db) => db,
        Err(exit) => return exit,
    };
    match db.sessions().get(&artifact.session.id) {
        Ok(Some(_)) => {
            eprintln!(
                "cortex import: session already exists: {}",
                artifact.session.id
            );
            return Exit::Failure;
        }
        Ok(None) => {}
        Err(err) => return db_error("import", &err),
    }

    let title = artifact.session.title.as_deref();
    if let Err(err) = db.sessions().ensure(&artifact.session.id, title) {
        return db_error("import", &err);
    }

    let model = artifact.session.model.as_deref();
    if let Err(err) = db.sessions().set_model(&artifact.session.id, model) {
        cleanup_import_failure(&db, &artifact.session.id);
        return db_error("import", &err);
    }

    for message in &artifact.messages {
        let role = redaction::redact(&message.role);
        let content = redaction::redact(&message.content);
        if let Err(err) =
            db.messages()
                .create_with_parts(&artifact.session.id, &role, &content, &message.parts)
        {
            cleanup_import_failure(&db, &artifact.session.id);
            return db_error("import", &err);
        }
    }

    emit_text(&format!("{}\n", artifact.session.id))
}

fn cleanup_import_failure(db: &Database, session_id: &str) {
    let _ = db.sessions().delete(session_id);
}

fn read_artifact_source(path: &str) -> Result<String, Exit> {
    if path == "-" {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|err| read_error("stdin", err))?;
        return Ok(input);
    }
    fs::read_to_string(path).map_err(|err| read_error(path, err))
}

fn parse_artifact(source: &str) -> Result<SessionArtifact, String> {
    serde_json::from_str(source).map_err(|err| format!("invalid or unparseable artifact: {}", err))
}

fn validate_artifact(artifact: &SessionArtifact) -> Result<(), String> {
    if artifact.version != ARTIFACT_VERSION {
        return Err(format!(
            "invalid artifact version {}; expected {}",
            artifact.version, ARTIFACT_VERSION
        ));
    }
    if artifact.session.id.trim().is_empty() {
        return Err("invalid artifact: session.id is empty".to_owned());
    }
    for (index, message) in artifact.messages.iter().enumerate() {
        if message.role.trim().is_empty() {
            return Err(format!("invalid artifact: messages[{index}].role is empty"));
        }
    }
    Ok(())
}

fn read_error(path: &str, err: io::Error) -> Exit {
    eprintln!("cortex import: failed to read artifact {path:?}: {err}");
    Exit::Failure
}

fn open_database(command: &str) -> Result<Database, Exit> {
    let inputs = DbPathInputs::from_process();
    let Some(path) = inputs.resolve() else {
        eprintln!(
            "cortex {command}: cannot determine a database location; set HOME, XDG_DATA_HOME, \
             CORTEX_DATA_DIR, or CORTEX_DB_PATH"
        );
        return Err(Exit::Failure);
    };
    Database::open(&path).map_err(|err| db_error(command, &err))
}

fn emit_json<T: Serialize>(command: &str, value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex {command}: failed to serialize output: {err}");
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

fn db_error(command: &str, err: &DbError) -> Exit {
    eprintln!("cortex {command}: {err}");
    Exit::Failure
}

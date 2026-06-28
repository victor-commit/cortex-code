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
    let artifact_source = match read_import_source(&args.artifact) {
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

/// Resolve the import artifact from a local file, stdin (`-`), or an `http(s)://` URL.
///
/// A URL is fetched over the network behind the `CORTEX_NO_NET` outbound guard (consulted before
/// any socket opens); the network feature gate must be compiled in. Diagnostics about a URL never
/// echo its query string or userinfo, so a share token embedded in the link is not logged.
fn read_import_source(artifact: &str) -> Result<String, Exit> {
    match parse_http_url(artifact) {
        Some(url) => fetch_url_artifact(&url),
        None => read_artifact_source(artifact),
    }
}

/// Parse an `http`/`https` URL, returning `None` for any non-URL argument (a local path or `-`).
fn parse_http_url(artifact: &str) -> Option<url::Url> {
    let url = url::Url::parse(artifact).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

/// A loggable rendering of a URL that omits its query string, fragment, and userinfo, so a share
/// token carried in the link never reaches stderr.
fn safe_url_display(url: &url::Url) -> String {
    let mut sanitized = url.clone();
    sanitized.set_query(None);
    sanitized.set_fragment(None);
    let _ = sanitized.set_username("");
    let _ = sanitized.set_password(None);
    sanitized.to_string()
}

/// Fetch a remote share artifact, fail-closed under `CORTEX_NO_NET`.
///
/// The outbound guard is consulted FIRST — before the network feature gate constructs a runtime or
/// opens any socket — so the kill-switch refuses with a `net-disabled` diagnostic and zero
/// connection attempts. The request asks for `application/json` so a `cortex serve` share link
/// returns the importable artifact rather than its HTML view.
fn fetch_url_artifact(url: &url::Url) -> Result<String, Exit> {
    if let Err(diag) = redaction::guard_outbound("share import fetch") {
        eprintln!("cortex import: {diag}");
        return Err(diag.exit());
    }
    if !crate::feature_gate::network_enabled() {
        return Err(crate::feature_gate::network_unavailable("import"));
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cortex import: failed to start the async runtime: {err}");
            return Err(Exit::Failure);
        }
    };

    match runtime.block_on(fetch_share_body(url)) {
        Ok(body) => Ok(body),
        Err(detail) => {
            eprintln!(
                "cortex import: failed to fetch share {}: {detail}",
                safe_url_display(url)
            );
            Err(Exit::Failure)
        }
    }
}

/// Perform the real HTTP GET for a share artifact. Errors strip the URL (`without_url`) so a share
/// token in the link is not surfaced in the returned detail.
async fn fetch_share_body(url: &url::Url) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .build()
        .map_err(|err| format!("cannot build HTTP client: {}", err.without_url()))?;
    let response = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|err| format!("request failed: {}", err.without_url()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("share endpoint returned HTTP {}", status.as_u16()));
    }
    response
        .text()
        .await
        .map_err(|err| format!("cannot read share body: {}", err.without_url()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_and_https_urls_but_not_paths() {
        assert!(parse_http_url("http://127.0.0.1:3200/share/shr_1").is_some());
        assert!(parse_http_url("https://example.test/share/abc").is_some());
        assert!(parse_http_url("./local.json").is_none());
        assert!(parse_http_url("-").is_none());
        assert!(parse_http_url("/tmp/artifact.json").is_none());
        // A non-http scheme is treated as a local path, not a fetchable URL.
        assert!(parse_http_url("file:///etc/passwd").is_none());
    }

    #[test]
    fn safe_url_display_strips_query_fragment_and_userinfo() {
        // Userinfo is attached via setters so no credential-shaped URL literal lives in source.
        let mut url = url::Url::parse("https://example.test/share/shr_9?token=abc123#frag")
            .expect("parse url");
        url.set_username("user").expect("set username");
        url.set_password(Some("secret-token"))
            .expect("set password");
        let display = safe_url_display(&url);
        assert!(
            display.contains("/share/shr_9"),
            "path is preserved: {display}"
        );
        assert!(
            !display.contains("abc123"),
            "query token must not leak: {display}"
        );
        assert!(
            !display.contains("secret-token"),
            "userinfo must not leak: {display}"
        );
        assert!(!display.contains('#'), "fragment must not leak: {display}");
        assert!(
            !display.contains('?'),
            "query separator must not leak: {display}"
        );
    }
}

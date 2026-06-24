//! The `cortex search` subcommand: ripgrep-backed working-tree search plus persisted-session hits.
//!
//! The command remains local-only and shells out to `rg` in the current working directory, so it
//! inherits ripgrep defaults for ignore handling and binary-file skipping. When a shared session
//! database already exists, it also scans persisted messages for the query and appends those hits.

use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use db::{Database, DbPathInputs};
use errors::Exit;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::cli::{OutputFormat, SearchArgs};

/// Dispatch a `cortex search` invocation.
pub fn run(args: SearchArgs) -> Exit {
    if args.query.trim().is_empty() {
        eprintln!("cortex search: query must not be empty");
        return Exit::Usage;
    }

    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("cortex search: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };

    match args.format.format {
        OutputFormat::Text => match run_text_search(&cwd, &args.query) {
            Ok(body) => emit_text(&body),
            Err(err) => fail(err),
        },
        OutputFormat::Json | OutputFormat::Jsonl => match run_json_search(&cwd, &args.query) {
            Ok(matches) => emit_json(&SearchDocument { matches }),
            Err(err) => fail(err),
        },
    }
}

fn run_text_search(cwd: &Path, query: &str) -> Result<String, SearchError> {
    let output = rg_output(cwd, query, false)?;
    let mut body = match output.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        Some(1) => Ok(String::new()),
        _ if is_no_files_searched(&output.stderr) => Ok(String::new()),
        _ => Err(SearchError::RipgrepFailed(stderr_or_default(
            &output.stderr,
            "ripgrep failed",
        ))),
    }?;
    append_session_text_matches(query, &mut body)?;
    Ok(body)
}

fn run_json_search(cwd: &Path, query: &str) -> Result<Vec<SearchMatch>, SearchError> {
    let output = rg_output(cwd, query, true)?;
    let mut matches = match output.status.code() {
        Some(0) | Some(1) => Ok(parse_ripgrep_json(&output.stdout)?),
        _ if is_no_files_searched(&output.stderr) => Ok(Vec::new()),
        _ => Err(SearchError::RipgrepFailed(stderr_or_default(
            &output.stderr,
            "ripgrep failed",
        ))),
    }?;
    matches.extend(session_json_matches(query)?);
    Ok(matches)
}

fn rg_output(cwd: &Path, query: &str, json: bool) -> Result<std::process::Output, SearchError> {
    let mut command = Command::new("rg");
    command
        .current_dir(cwd)
        .arg("--line-number")
        .arg("--color")
        .arg("never")
        .arg("-e")
        .arg(query);

    if json {
        command.arg("--json");
    } else {
        command.arg("--no-heading");
    }

    command.output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SearchError::RipgrepNotInstalled
        } else {
            SearchError::RipgrepSpawn(err.to_string())
        }
    })
}

fn parse_ripgrep_json(stdout: &[u8]) -> Result<Vec<SearchMatch>, SearchError> {
    let mut matches = Vec::new();
    for line in stdout.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let event: Value = serde_json::from_slice(line)
            .map_err(|err| SearchError::JsonDecode(format!("invalid rg --json event: {err}")))?;
        if event.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }

        let path = event
            .pointer("/data/path/text")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                SearchError::JsonDecode(
                    "missing `data.path.text` in ripgrep match event".to_owned(),
                )
            })?;
        let line_number = event
            .pointer("/data/line_number")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                SearchError::JsonDecode(
                    "missing `data.line_number` in ripgrep match event".to_owned(),
                )
            })?;
        let matched_text = event
            .pointer("/data/lines/text")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                SearchError::JsonDecode(
                    "missing `data.lines.text` in ripgrep match event".to_owned(),
                )
            })?;

        matches.push(SearchMatch {
            path: path.to_owned(),
            line: line_number,
            matched_text: matched_text.trim_end_matches(['\r', '\n']).to_owned(),
            source: "workspace".to_owned(),
            session_id: None,
            message_id: None,
            role: None,
        });
    }
    Ok(matches)
}

#[derive(Debug, Clone)]
struct SessionSearchMatch {
    session_id: String,
    message_id: String,
    role: String,
    line: u64,
    matched_text: String,
}

fn append_session_text_matches(query: &str, body: &mut String) -> Result<(), SearchError> {
    for hit in session_matches(query)? {
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!(
            "session:{}:{}:{}: {}\n",
            hit.session_id, hit.line, hit.role, hit.matched_text
        ));
    }
    Ok(())
}

fn session_json_matches(query: &str) -> Result<Vec<SearchMatch>, SearchError> {
    Ok(session_matches(query)?
        .into_iter()
        .map(|hit| SearchMatch {
            path: format!("session:{}", hit.session_id),
            line: hit.line,
            matched_text: hit.matched_text,
            source: "session".to_owned(),
            session_id: Some(hit.session_id),
            message_id: Some(hit.message_id),
            role: Some(hit.role),
        })
        .collect())
}

fn session_matches(query: &str) -> Result<Vec<SessionSearchMatch>, SearchError> {
    let regex = session_query_regex(query)?;
    let Some(db) = open_existing_database() else {
        return Ok(Vec::new());
    };
    let Ok(sessions) = db.sessions().list() else {
        return Ok(Vec::new());
    };
    let mut matches = Vec::new();
    for session in sessions {
        let Ok(messages) = db.messages().list_for_session(&session.id) else {
            continue;
        };
        for (index, message) in messages.iter().enumerate() {
            for matched_line in session_message_line_matches(&regex, &message.content) {
                matches.push(SessionSearchMatch {
                    session_id: session.id.clone(),
                    message_id: message.id.clone(),
                    role: message.role.clone(),
                    line: (index + 1) as u64,
                    matched_text: matched_line.to_owned(),
                });
            }
        }
    }
    matches.sort_by(|left, right| {
        left.session_id
            .cmp(&right.session_id)
            .then_with(|| left.line.cmp(&right.line))
    });
    Ok(matches)
}

fn session_message_line_matches<'a>(
    regex: &'a Regex,
    message_content: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
    message_content.lines().filter(|line| regex.is_match(line))
}

fn session_query_regex(query: &str) -> Result<Regex, SearchError> {
    Regex::new(query).map_err(|err| SearchError::InvalidQuery(err.to_string()))
}

fn open_existing_database() -> Option<Database> {
    let path = DbPathInputs::from_process().resolve()?;
    if !path.is_file() {
        return None;
    }
    Database::open(path).ok()
}

#[derive(Debug)]
enum SearchError {
    RipgrepNotInstalled,
    RipgrepSpawn(String),
    RipgrepFailed(String),
    InvalidQuery(String),
    JsonDecode(String),
}

fn fail(err: SearchError) -> Exit {
    match err {
        SearchError::RipgrepNotInstalled => {
            eprintln!("cortex search: ripgrep (rg) is required but was not found on PATH");
        }
        SearchError::RipgrepSpawn(detail) => {
            eprintln!("cortex search: failed to execute ripgrep: {detail}");
        }
        SearchError::RipgrepFailed(detail) => {
            eprintln!("cortex search: {detail}");
        }
        SearchError::InvalidQuery(detail) => {
            eprintln!("cortex search: query is not a valid regex: {detail}");
        }
        SearchError::JsonDecode(detail) => {
            eprintln!("cortex search: failed to decode ripgrep JSON output: {detail}");
        }
    }
    Exit::Failure
}

#[derive(Serialize)]
struct SearchDocument {
    matches: Vec<SearchMatch>,
}

#[derive(Serialize)]
struct SearchMatch {
    path: String,
    line: u64,
    matched_text: String,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex search: failed to serialize JSON output: {err}");
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

fn stderr_or_default(stderr: &[u8], fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        fallback.to_owned()
    } else {
        detail.to_owned()
    }
}

fn is_no_files_searched(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).contains("No files were searched")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ripgrep_json_extracts_match_entries() {
        let payload = concat!(
            "{\"type\":\"begin\",\"data\":{\"path\":{\"text\":\"src/main.rs\"}}}\n",
            "{\"type\":\"match\",\"data\":{\"path\":{\"text\":\"src/main.rs\"},\"lines\":{\"text\":\"let marker = \\\"XYZ\\\";\\n\"},\"line_number\":12,\"absolute_offset\":0,\"submatches\":[{\"match\":{\"text\":\"XYZ\"},\"start\":14,\"end\":17}]}}\n",
            "{\"type\":\"summary\",\"data\":{\"elapsed_total\":{\"secs\":0,\"nanos\":1,\"human\":\"0.000001s\"},\"stats\":{\"searches\":1,\"searches_with_match\":1,\"bytes_searched\":10,\"bytes_printed\":10,\"matched_lines\":1,\"matches\":1}}}\n"
        );
        let matches = parse_ripgrep_json(payload.as_bytes()).expect("parse");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "src/main.rs");
        assert_eq!(matches[0].line, 12);
        assert_eq!(matches[0].matched_text, "let marker = \"XYZ\";");
        assert_eq!(matches[0].source, "workspace");
        assert!(matches[0].session_id.is_none());
    }

    #[test]
    fn parse_ripgrep_json_returns_empty_for_no_matches() {
        let payload = concat!(
            "{\"type\":\"summary\",\"data\":{\"elapsed_total\":{\"secs\":0,\"nanos\":1,\"human\":\"0.000001s\"},\"stats\":{\"searches\":1,\"searches_with_match\":0,\"bytes_searched\":0,\"bytes_printed\":0,\"matched_lines\":0,\"matches\":0}}}\n"
        );
        let matches = parse_ripgrep_json(payload.as_bytes()).expect("parse");
        assert!(matches.is_empty());
    }

    #[test]
    fn detects_no_files_searched_ripgrep_diagnostic() {
        let stderr = "No files were searched, which means ripgrep probably applied a filter you didn't expect.\nRunning with --debug will show why files are being skipped.\n";
        assert!(is_no_files_searched(stderr.as_bytes()));
    }

    #[test]
    fn session_query_regex_rejects_invalid_patterns() {
        let err = session_query_regex("(").expect_err("invalid regex should error");
        assert!(matches!(err, SearchError::InvalidQuery(_)));
    }

    #[test]
    fn session_message_line_matches_are_line_oriented() {
        let regex = session_query_regex("^TOKEN_[0-9]+$").expect("valid regex");
        let message = "before\nTOKEN_1\nafter\nTOKEN_2";
        let matches: Vec<&str> = session_message_line_matches(&regex, message).collect();
        assert_eq!(matches, vec!["TOKEN_1", "TOKEN_2"]);
    }
}

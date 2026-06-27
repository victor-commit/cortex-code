//! Bounded search routes backed by ripgrep and a lightweight source-symbol scan.
//!
//! These handlers let a server client search the workspace without shelling out to the CLI:
//! `/find` searches file contents for a text pattern, `/find/file` matches file/directory names,
//! and `/find/symbol` extracts source-symbol definitions. All three are bounded, run only inside
//! the workspace `cwd`, and inherit ripgrep's ignore/binary skipping so ignored and binary files
//! never leak into results. Symbol search uses a deterministic, offline source scan rather than a
//! spawned LSP server so it stays fast and honors the no-network validation path.

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use axum::{Json, extract::State, http::StatusCode};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ApiError, ServerState};

/// Default and hard cap on the number of results any find route returns.
const DEFAULT_LIMIT: usize = 200;
const MAX_LIMIT: usize = 1000;

/// Query for `/find` (text search).
#[derive(Debug, Deserialize)]
pub(super) struct FindTextQuery {
    #[serde(default)]
    pattern: String,
    #[serde(default)]
    limit: Option<usize>,
}

/// Query for `/find/file` (filename search).
#[derive(Debug, Deserialize)]
pub(super) struct FindFileQuery {
    #[serde(default)]
    query: String,
    #[serde(default)]
    dirs: Option<String>,
    #[serde(rename = "type", default)]
    entry_type: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// Query for `/find/symbol` (source symbol search).
#[derive(Debug, Deserialize)]
pub(super) struct FindSymbolQuery {
    #[serde(default)]
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

/// One text match, matching the reference ripgrep `Match` wire shape.
#[derive(Debug, Serialize)]
pub(super) struct TextMatch {
    path: TextField,
    lines: TextField,
    line_number: u64,
    absolute_offset: u64,
    submatches: Vec<SubMatch>,
}

#[derive(Debug, Serialize)]
pub(super) struct TextField {
    text: String,
}

#[derive(Debug, Serialize)]
pub(super) struct SubMatch {
    #[serde(rename = "match")]
    matched: TextField,
    start: u64,
    end: u64,
}

/// A workspace symbol, matching the reference `Symbol` wire shape.
#[derive(Debug, Serialize)]
pub(super) struct Symbol {
    name: String,
    kind: u8,
    location: SymbolLocation,
}

#[derive(Debug, Serialize)]
pub(super) struct SymbolLocation {
    uri: String,
    range: Range,
}

#[derive(Debug, Serialize)]
pub(super) struct Range {
    start: Position,
    end: Position,
}

#[derive(Debug, Serialize)]
pub(super) struct Position {
    line: u64,
    character: u64,
}

/// `GET /find?pattern=<text>` — search file contents for a text pattern.
pub(super) async fn find_text(
    State(state): State<ServerState>,
    axum::extract::Query(query): axum::extract::Query<FindTextQuery>,
) -> Result<Json<Vec<TextMatch>>, ApiError> {
    let pattern = query.pattern.trim();
    if pattern.is_empty() {
        return Err(ApiError::bad_request(
            "`pattern` query parameter is required",
        ));
    }
    let limit = clamp_limit(query.limit);
    let output = run_rg(&state.cwd, &["--json", "--", pattern, "."])?;
    let mut matches = Vec::new();
    for line in output.lines() {
        if matches.len() >= limit {
            break;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let Some(data) = value.get("data") else {
            continue;
        };
        let path = redaction::redact(text_field(data.get("path")).trim_end_matches('\n'));
        let lines = text_field(data.get("lines"));
        let line_number = data.get("line_number").and_then(Value::as_u64).unwrap_or(0);
        let absolute_offset = data
            .get("absolute_offset")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let submatches = data
            .get("submatches")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| SubMatch {
                        matched: TextField {
                            text: redaction::redact(&text_field(item.get("match"))),
                        },
                        start: item.get("start").and_then(Value::as_u64).unwrap_or(0),
                        end: item.get("end").and_then(Value::as_u64).unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();
        matches.push(TextMatch {
            path: TextField { text: path },
            lines: TextField {
                text: redaction::redact(&lines),
            },
            line_number,
            absolute_offset,
            submatches,
        });
    }
    Ok(Json(matches))
}

/// `GET /find/file?query=<name>` — search for files (and optionally directories) by name.
pub(super) async fn find_file(
    State(state): State<ServerState>,
    axum::extract::Query(query): axum::extract::Query<FindFileQuery>,
) -> Result<Json<Vec<String>>, ApiError> {
    let needle = query.query.trim().to_ascii_lowercase();
    let limit = clamp_limit(query.limit);
    let output = run_rg(&state.cwd, &["--files", "."])?;

    let want_dirs = matches!(query.entry_type.as_deref(), Some("directory"))
        || matches!(query.dirs.as_deref(), Some("true"));
    let only_dirs = matches!(query.entry_type.as_deref(), Some("directory"));

    let mut seen = std::collections::BTreeSet::new();
    for raw in output.lines() {
        let file = raw.trim_start_matches("./").trim();
        if file.is_empty() {
            continue;
        }
        if !only_dirs {
            seen.insert(file.to_owned());
        }
        if want_dirs {
            let mut ancestor = Path::new(file);
            while let Some(parent) = ancestor.parent() {
                if parent.as_os_str().is_empty() {
                    break;
                }
                seen.insert(parent.to_string_lossy().into_owned());
                ancestor = parent;
            }
        }
    }

    let mut results: Vec<String> = seen
        .into_iter()
        .filter(|path| needle.is_empty() || path.to_ascii_lowercase().contains(&needle))
        .map(|path| redaction::redact(&path))
        .collect();
    results.truncate(limit);
    Ok(Json(results))
}

/// `GET /find/symbol?query=<name>` — search source-symbol definitions across the workspace.
pub(super) async fn find_symbol(
    State(state): State<ServerState>,
    axum::extract::Query(query): axum::extract::Query<FindSymbolQuery>,
) -> Result<Json<Vec<Symbol>>, ApiError> {
    let needle = query.query.trim();
    if needle.is_empty() {
        return Err(ApiError::bad_request("`query` query parameter is required"));
    }
    let limit = clamp_limit(query.limit);
    let lowered = needle.to_ascii_lowercase();
    let rg_pattern = format!(
        r"\b({})\b[^\n]*{}",
        DEFINITION_KEYWORDS.join("|"),
        regex::escape(needle)
    );
    let output = run_rg(&state.cwd, &["--json", "--", &rg_pattern, "."])?;

    let mut symbols = Vec::new();
    for line in output.lines() {
        if symbols.len() >= limit {
            break;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("match") {
            continue;
        }
        let Some(data) = value.get("data") else {
            continue;
        };
        let path = text_field(data.get("path"));
        let path = path.trim_end_matches('\n');
        let line_text = text_field(data.get("lines"));
        let line_number = data.get("line_number").and_then(Value::as_u64).unwrap_or(1);

        for capture in definition_regex().captures_iter(&line_text) {
            if symbols.len() >= limit {
                break;
            }
            let keyword = capture.name("kw").map(|m| m.as_str()).unwrap_or("");
            let Some(name_match) = capture.name("name") else {
                continue;
            };
            let name = name_match.as_str();
            if !name.to_ascii_lowercase().contains(&lowered) {
                continue;
            }
            let character = line_text[..name_match.start()].chars().count() as u64;
            symbols.push(Symbol {
                name: redaction::redact(name),
                kind: symbol_kind(keyword),
                location: SymbolLocation {
                    uri: path_to_uri(&state.cwd, path),
                    range: Range {
                        start: Position {
                            line: line_number.saturating_sub(1),
                            character,
                        },
                        end: Position {
                            line: line_number.saturating_sub(1),
                            character: character + name.chars().count() as u64,
                        },
                    },
                },
            });
        }
    }
    Ok(Json(symbols))
}

/// Definition keywords recognized across the common languages cortex targets.
const DEFINITION_KEYWORDS: &[&str] = &[
    "fn",
    "function",
    "func",
    "def",
    "class",
    "struct",
    "enum",
    "trait",
    "interface",
    "type",
    "const",
    "let",
    "var",
    "impl",
    "mod",
    "module",
    "namespace",
];

fn definition_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        let keywords = DEFINITION_KEYWORDS.join("|");
        Regex::new(&format!(
            r"\b(?P<kw>{keywords})\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
        ))
        .expect("definition regex compiles")
    })
}

fn symbol_kind(keyword: &str) -> u8 {
    match keyword {
        "fn" | "function" | "func" | "def" => 12, // Function
        "class" | "impl" => 5,                    // Class
        "struct" => 23,                           // Struct
        "enum" => 10,                             // Enum
        "trait" | "interface" => 11,              // Interface
        "type" => 26,                             // TypeParameter
        "const" => 14,                            // Constant
        "let" | "var" => 13,                      // Variable
        "mod" | "module" | "namespace" => 2,      // Module
        _ => 13,                                  // Variable fallback
    }
}

fn clamp_limit(requested: Option<usize>) -> usize {
    requested
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_LIMIT)
        .min(MAX_LIMIT)
}

fn text_field(value: Option<&Value>) -> String {
    value
        .and_then(|value| value.get("text"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_default()
}

fn path_to_uri(cwd: &Path, relative: &str) -> String {
    let absolute = cwd.join(relative.trim_start_matches("./"));
    redaction::redact(&format!("file://{}", absolute.to_string_lossy()))
}

/// Run ripgrep inside `cwd`, returning stdout. A missing `rg` binary or a ripgrep error is mapped
/// to a clean server error rather than a panic.
fn run_rg(cwd: &Path, args: &[&str]) -> Result<String, ApiError> {
    let output = Command::new("rg")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "ripgrep (`rg`) is not installed; search routes are unavailable",
                )
            } else {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to run ripgrep: {err}"),
                )
            }
        })?;
    // ripgrep exits 1 when there are no matches; that is a successful empty search, not an error.
    match output.status.code() {
        Some(0) | Some(1) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        _ => Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "ripgrep failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )),
    }
}

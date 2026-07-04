//! The `cortex search` subcommand: ripgrep-backed working-tree search plus persisted-session hits.
//!
//! The command remains local-only and shells out to `rg` in the current working directory, so it
//! inherits ripgrep defaults for ignore handling and binary-file skipping. When a shared session
//! database already exists, it also scans persisted messages for the query and appends those hits.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use db::{Database, DbPathInputs};
use errors::Exit;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::cli::{OutputFormat, SearchArgs};
use crate::style::{self, Style, Styler};

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
    let roots = search_roots(&cwd);

    match args.format.format {
        OutputFormat::Text => match collect_text_parts(&roots, &args.query) {
            // On a TTY the hits are re-rendered with brand coloring + query highlight and a footer;
            // off a TTY the raw ripgrep/session passthrough is emitted byte-identical (VAL-CLI-020..023).
            Ok((workspace_body, session_hits)) => {
                let styler = Styler::for_stdout();
                if styler.is_enabled() {
                    emit_text(&render_search_styled(
                        &workspace_body,
                        &session_hits,
                        &args.query,
                        styler,
                    ))
                } else {
                    emit_text(&plain_text_body(&workspace_body, &session_hits))
                }
            }
            Err(err) => fail(err),
        },
        OutputFormat::Json | OutputFormat::Jsonl => match run_json_search(&roots, &args.query) {
            Ok(matches) => emit_json(&SearchDocument { matches }),
            Err(err) => fail(err),
        },
    }
}

/// The directory roots a search covers.
///
/// By default this is just the working directory, preserving cwd-scoped behavior. When the working
/// directory is registered to a project that spans multiple directories (in an existing database),
/// the search widens to every directory of that project, so one logical project is searchable as a
/// unit regardless of which of its directories you launched from.
fn search_roots(cwd: &Path) -> Vec<PathBuf> {
    let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if let Some(db) = open_existing_database()
        && let Ok(Some(project)) = db
            .projects()
            .find_by_directory(&canonical.to_string_lossy())
    {
        let directories: Vec<PathBuf> = project
            .directories
            .iter()
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect();
        if directories.len() > 1 {
            return directories;
        }
    }
    vec![cwd.to_path_buf()]
}

/// Gather the raw text-search results: the ripgrep passthrough body (concatenated across roots)
/// and the persisted-session hits, kept separate so the piped path can reproduce the exact machine
/// bytes while the TTY path can re-render them with brand styling.
fn collect_text_parts(
    roots: &[PathBuf],
    query: &str,
) -> Result<(String, Vec<SessionSearchMatch>), SearchError> {
    let mut workspace_body = String::new();
    for root in roots {
        let output = rg_output(root, query, false)?;
        let chunk = match output.status.code() {
            Some(0) => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
            Some(1) => Ok(String::new()),
            _ if is_no_files_searched(&output.stderr) => Ok(String::new()),
            _ => Err(SearchError::RipgrepFailed(stderr_or_default(
                &output.stderr,
                "ripgrep failed",
            ))),
        }?;
        workspace_body.push_str(&chunk);
    }
    let session_hits = session_matches(query)?;
    Ok((workspace_body, session_hits))
}

/// Reproduce the historical plain text body byte-for-byte: the ripgrep passthrough followed by one
/// `session:{id}:{line}:{role}: text` line per persisted-session hit. This is the machine/piped
/// path, so its bytes must stay identical to the pre-styling baseline.
fn plain_text_body(workspace_body: &str, session_hits: &[SessionSearchMatch]) -> String {
    let mut body = workspace_body.to_owned();
    for hit in session_hits {
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&format!(
            "session:{}:{}:{}: {}\n",
            hit.session_id, hit.line, hit.role, hit.matched_text
        ));
    }
    body
}

/// A compiled query matcher for highlighting occurrences on a hit line. The query is validated as
/// a regex before styling runs (session matching compiles the same pattern), so the regex arm is
/// the normal case; the literal arm is a defensive fallback.
enum QueryMatcher {
    Regex(Regex),
    Literal(String),
}

impl QueryMatcher {
    fn build(query: &str) -> Self {
        match Regex::new(query) {
            Ok(regex) => Self::Regex(regex),
            Err(_) => Self::Literal(query.to_owned()),
        }
    }

    /// Non-overlapping match byte ranges within `text`, skipping zero-width matches so a pattern
    /// like `.*`/`^` cannot loop or emit empty highlights.
    fn ranges(&self, text: &str) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        match self {
            Self::Regex(regex) => {
                for m in regex.find_iter(text) {
                    if m.end() > m.start() {
                        ranges.push((m.start(), m.end()));
                    }
                }
            }
            Self::Literal(needle) => {
                if needle.is_empty() {
                    return ranges;
                }
                let mut from = 0;
                while let Some(rel) = text[from..].find(needle.as_str()) {
                    let start = from + rel;
                    let end = start + needle.len();
                    ranges.push((start, end));
                    from = end;
                }
            }
        }
        ranges
    }
}

/// Paint `text` in `base`, wrapping every query occurrence on the search-hit background (`HIT_BG`)
/// while keeping the base foreground so highlighted text stays legible (VAL-CLI-021).
fn highlight(styler: Styler, text: &str, base: style::Rgb, matcher: &QueryMatcher) -> String {
    let ranges = matcher.ranges(text);
    if ranges.is_empty() {
        return styler.paint(text, base);
    }
    let hit_style = Style::fg(base).on(style::HIT_BG);
    let mut out = String::new();
    let mut last = 0;
    for (start, end) in ranges {
        if start > last {
            out.push_str(&styler.paint(&text[last..start], base));
        }
        out.push_str(&styler.apply(&text[start..end], hit_style));
        last = end;
    }
    if last < text.len() {
        out.push_str(&styler.paint(&text[last..], base));
    }
    out
}

/// Split a ripgrep `path:line:text` output line into its three fields. The line number is the first
/// colon-delimited all-digit segment, so paths without embedded digits-between-colons parse
/// cleanly; anything unexpected returns `None` so the caller can emit the line unstyled.
fn parse_rg_line(line: &str) -> Option<(&str, &str, &str)> {
    let first = line.find(':')?;
    let rest = &line[first + 1..];
    let second = rest.find(':')?;
    let number = &rest[..second];
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((&line[..first], number, &rest[second + 1..]))
}

/// Render the human/TTY view of `cortex search`: ripgrep hits as `path:line:text` (path ORANGE,
/// `:` GHOST, line DIM, code TXT with the query highlighted), then persisted-session hits under a
/// DIM `persisted sessions` header, then a FAINT `N matches · N workspace · N session` footer. Runs
/// only when the styler is enabled; the piped path uses [`plain_text_body`].
fn render_search_styled(
    workspace_body: &str,
    session_hits: &[SessionSearchMatch],
    query: &str,
    styler: Styler,
) -> String {
    let matcher = QueryMatcher::build(query);
    let workspace_lines: Vec<&str> = workspace_body.lines().collect();
    let total = workspace_lines.len() + session_hits.len();
    if total == 0 {
        return String::new();
    }

    let mut out = String::new();
    for line in &workspace_lines {
        if let Some((path, number, text)) = parse_rg_line(line) {
            out.push_str(&styler.paint(path, style::ORANGE));
            out.push_str(&styler.paint(":", style::GHOST));
            out.push_str(&styler.paint(number, style::DIM));
            out.push_str(&styler.paint(":", style::GHOST));
            out.push_str(&highlight(styler, text, style::TXT, &matcher));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    if !session_hits.is_empty() {
        out.push('\n');
        out.push_str(&styler.paint("persisted sessions", style::DIM));
        out.push('\n');
        for hit in session_hits {
            out.push_str(&styler.paint(&format!("session:{}", hit.session_id), style::ORANGE));
            out.push_str(&styler.paint(":", style::GHOST));
            out.push_str(&styler.paint(&hit.line.to_string(), style::DIM));
            out.push_str(&styler.paint(":", style::GHOST));
            out.push_str(&styler.paint(&hit.role, style::DIM));
            out.push_str(&styler.paint(":", style::GHOST));
            out.push(' ');
            out.push_str(&highlight(styler, &hit.matched_text, style::TXT, &matcher));
            out.push('\n');
        }
    }

    let footer = format!(
        "{total} matches {sep} {workspace} workspace {sep} {session} session",
        sep = style::SEPARATOR,
        workspace = workspace_lines.len(),
        session = session_hits.len(),
    );
    out.push('\n');
    out.push_str(&styler.paint(&footer, style::FAINT));
    out.push('\n');
    out
}

fn run_json_search(roots: &[PathBuf], query: &str) -> Result<Vec<SearchMatch>, SearchError> {
    let mut matches = Vec::new();
    for root in roots {
        let output = rg_output(root, query, true)?;
        let chunk = match output.status.code() {
            Some(0) | Some(1) => Ok(parse_ripgrep_json(&output.stdout)?),
            _ if is_no_files_searched(&output.stderr) => Ok(Vec::new()),
            _ => Err(SearchError::RipgrepFailed(stderr_or_default(
                &output.stderr,
                "ripgrep failed",
            ))),
        }?;
        matches.extend(chunk);
    }
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
        let payload = "{\"type\":\"summary\",\"data\":{\"elapsed_total\":{\"secs\":0,\"nanos\":1,\"human\":\"0.000001s\"},\"stats\":{\"searches\":1,\"searches_with_match\":0,\"bytes_searched\":0,\"bytes_printed\":0,\"matched_lines\":0,\"matches\":0}}}\n";
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

    fn fg(color: style::Rgb) -> String {
        format!("\x1b[38;2;{};{};{}m", color.0, color.1, color.2)
    }

    fn hit(base: style::Rgb) -> String {
        let style::Rgb(fr, fg_, fb) = base;
        let style::Rgb(br, bg, bb) = style::HIT_BG;
        format!("\x1b[38;2;{fr};{fg_};{fb};48;2;{br};{bg};{bb}m")
    }

    fn session_hit(id: &str, line: u64, role: &str, text: &str) -> SessionSearchMatch {
        SessionSearchMatch {
            session_id: id.to_owned(),
            message_id: "msg".to_owned(),
            role: role.to_owned(),
            line,
            matched_text: text.to_owned(),
        }
    }

    // VAL-CLI-020: a workspace hit is `path:line:text` with the path ORANGE, `:` GHOST, line DIM,
    // and code text TXT.
    #[test]
    fn styled_search_colors_path_line_and_text() {
        let body = "src/config.rs:42:fn parse_config(path: &Path) {\n";
        let out = render_search_styled(body, &[], "parse_config", Styler::new(true));
        assert!(
            out.contains(&format!("{}src/config.rs\x1b[0m", fg(style::ORANGE))),
            "path is ORANGE: {out:?}"
        );
        assert!(
            out.contains(&format!("{}:\x1b[0m", fg(style::GHOST))),
            "separators are GHOST"
        );
        assert!(
            out.contains(&format!("{}42\x1b[0m", fg(style::DIM))),
            "line number is DIM"
        );
    }

    // VAL-CLI-021: every occurrence of the query is highlighted on the HIT background, including a
    // partial occurrence inside a larger identifier.
    #[test]
    fn styled_search_highlights_every_query_occurrence() {
        let body = concat!(
            "src/config.rs:88:    let cfg = parse_config(&p)?;\n",
            "tests/config.rs:103:fn parse_config_rejects_missing() {\n",
        );
        let out = render_search_styled(body, &[], "parse_config", Styler::new(true));
        let occurrences = out.matches(&hit(style::TXT)).count();
        assert_eq!(
            occurrences, 2,
            "each parse_config occurrence (incl. the one inside the longer identifier) is highlighted: {out:?}"
        );
        assert!(
            out.contains(&format!("{}parse_config\x1b[0m", hit(style::TXT))),
            "the query text carries the HIT background"
        );
    }

    // VAL-CLI-022: session hits appear under a DIM `persisted sessions` header, styled as
    // `session:{id}:{line}:{role}: text`, followed by a FAINT footer with real counts.
    #[test]
    fn styled_search_shows_session_header_and_footer_counts() {
        let body = "src/config.rs:42:fn parse_config(path: &Path) {\n";
        let hits = vec![session_hit(
            "ses_7f3a91",
            5,
            "user",
            "why does parse_config fail on empty input?",
        )];
        let out = render_search_styled(body, &hits, "parse_config", Styler::new(true));
        assert!(
            out.contains(&format!("{}persisted sessions\x1b[0m", fg(style::DIM))),
            "DIM session header present: {out:?}"
        );
        assert!(
            out.contains(&format!("{}session:ses_7f3a91\x1b[0m", fg(style::ORANGE))),
            "session path is ORANGE"
        );
        assert!(
            out.contains(&format!(
                "{}2 matches {} 1 workspace {} 1 session\x1b[0m",
                fg(style::FAINT),
                style::SEPARATOR,
                style::SEPARATOR
            )),
            "FAINT footer with real counts: {out:?}"
        );
    }

    // VAL-CLI-023 (styling side): the plain body used for the piped/machine path is byte-identical
    // to the historical `path:line:text` + `session:` passthrough and carries zero ANSI.
    #[test]
    fn plain_text_body_matches_the_historical_passthrough() {
        let body = "src/config.rs:42:fn parse_config() {\n";
        let hits = vec![session_hit("ses_7f3a91", 5, "user", "why parse_config?")];
        let plain = plain_text_body(body, &hits);
        assert_eq!(
            plain,
            "src/config.rs:42:fn parse_config() {\nsession:ses_7f3a91:5:user: why parse_config?\n"
        );
        assert!(!plain.contains('\x1b'), "piped body carries no ANSI");
    }

    // A disabled styler renders no ANSI even through the styled path (defense in depth), and a
    // zero-hit search yields a clean empty TTY view.
    #[test]
    fn styled_search_is_ansi_free_when_disabled_and_empty_on_no_hits() {
        let body = "src/config.rs:42:fn parse_config() {\n";
        let out = render_search_styled(body, &[], "parse_config", Styler::new(false));
        assert!(!out.contains('\x1b'), "disabled styler emits no ANSI: {out:?}");
        assert_eq!(render_search_styled("", &[], "x", Styler::new(true)), "");
    }

    #[test]
    fn parse_rg_line_splits_path_line_and_text() {
        assert_eq!(
            parse_rg_line("src/main.rs:17:    parse_config(args)?;"),
            Some(("src/main.rs", "17", "    parse_config(args)?;"))
        );
        assert_eq!(parse_rg_line("no colons here"), None);
        assert_eq!(parse_rg_line("path:notanumber:text"), None);
    }
}

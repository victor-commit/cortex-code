//! The `grep` tool: ripgrep-style regex content search, ripgrep-backed.
//!
//! Behavior (architecture §3.4, validation contract VAL-TOOLS-029/030/031):
//!
//! - Searches file contents with a regular expression and returns the matching lines with their
//!   locations (`path:line:text`). Backed by the real `rg`, so the hits match an independent
//!   `rg <pattern>` cross-check exactly (including ripgrep's default ignore/hidden handling).
//! - An `include` filter (e.g. `*.rs`) restricts the search to matching files only, so a token that
//!   also appears in an excluded file is not reported.
//! - A pattern with no matches returns an explicit empty result (a clear "no matches" message),
//!   never an error or a hang.

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::output::{OutputLimits, bound_output};
use crate::ripgrep::{self, RgError};
use crate::tool::{Tool, ToolContext, ToolOutput};

const DEFAULT_MATCH_LIMIT: usize = 100;
const MAX_MATCH_LIMIT: usize = 1000;
const GREP_MAX_BYTES: usize = 50 * 1024;

const DESCRIPTION: &str = "Search file contents with a regular expression (ripgrep). Returns the \
matching lines as `path:line:text`, one per line. Use `include` to restrict the search to files \
matching a glob (for example `*.rs`). Use `path` to search under a specific directory. A pattern \
with no matches returns an explicit empty result. The regex syntax and the set of searched files \
match the `rg` command (files ignored by `.gitignore` are skipped).";

/// The `grep` tool.
pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "The regular expression to search for (ripgrep syntax)."
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search under (absolute, or relative to the working directory). Defaults to the working directory."
                },
                "include": {
                    "type": "string",
                    "description": "A glob restricting the search to matching files only (e.g. `*.rs`)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Optional maximum number of matches to return. Defaults to 100 and is capped at 1000."
                }
            },
            "required": ["pattern"]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

/// Execute a grep against the resolved arguments (synchronous shell-out to ripgrep).
fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(pattern) = string_arg(arguments, "pattern") else {
        return ToolOutput::error("grep: the 'pattern' argument is required");
    };
    if pattern.is_empty() {
        return ToolOutput::error("grep: the 'pattern' argument must not be empty");
    }

    let path = string_arg(arguments, "path");
    let include = string_arg(arguments, "include")
        .or_else(|| string_arg(arguments, "type"))
        .or_else(|| string_arg(arguments, "globPattern"))
        .or_else(|| string_arg(arguments, "glob_pattern"))
        .filter(|value| !value.trim().is_empty());
    let limit = match uint_arg(arguments, "limit")
        .or_else(|| uint_arg(arguments, "count"))
        .or_else(|| uint_arg(arguments, "head_limit"))
        .or_else(|| uint_arg(arguments, "headLimit"))
    {
        Some(0) => return ToolOutput::error("grep: 'limit' must be at least 1"),
        Some(limit) => limit.min(MAX_MATCH_LIMIT),
        None => DEFAULT_MATCH_LIMIT,
    };

    let root_display = path.as_deref().unwrap_or(".");
    if let Err(message) = ctx.ensure_permitted("grep", root_display) {
        return ToolOutput::error(message);
    }
    let root = match &path {
        Some(path) => ctx.resolve(path),
        None => ctx.cwd().to_path_buf(),
    };

    // `-e <pattern>` marks the pattern explicitly so a regex starting with `-` is never mistaken
    // for a flag; `--no-heading -n` yields the parseable `path:line:text` form.
    let mut args: Vec<String> = vec![
        "--no-heading".to_owned(),
        "--line-number".to_owned(),
        "--color".to_owned(),
        "never".to_owned(),
        "-e".to_owned(),
        pattern.clone(),
    ];
    if let Some(include) = &include {
        args.push("--glob".to_owned());
        args.push(include.clone());
    }

    match ripgrep::run(&root, &args) {
        Ok(stdout) => render(&pattern, &stdout, limit),
        Err(RgError::NotInstalled) => {
            ToolOutput::error("grep: ripgrep (rg) is required but was not found on PATH")
        }
        Err(RgError::Failed(detail)) => ToolOutput::error(format!("grep: {detail}")),
    }
}

/// Render ripgrep's match lines into a sorted tool result, or an explicit empty result.
///
/// A large match set is bounded to the configured limits with a truncation notice while the
/// complete set is spilled to disk and named in the result (the shared [`bound_output`] path, so
/// `grep` behaves identically to `bash` for over-limit output and nothing is silently lost).
fn render(pattern: &str, stdout: &str, limit: usize) -> ToolOutput {
    let mut hits: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    hits.sort_unstable_by(|left, right| compare_hit_lines(left, right));

    if hits.is_empty() {
        return ToolOutput::text(format!("No matches for pattern: {pattern}"));
    }

    let full = hits.join("\n");
    let bounded = bound_output(
        &full,
        OutputLimits {
            max_lines: limit,
            max_bytes: GREP_MAX_BYTES,
        },
        "grep-matches",
    );
    ToolOutput::text(bounded.text)
}

/// Compare two `path:line:text` hit lines by path, numeric line, then text for stable,
/// human-expected ordering (`2` before `10`).
fn compare_hit_lines(left: &str, right: &str) -> std::cmp::Ordering {
    let left_key = parse_hit_key(left);
    let right_key = parse_hit_key(right);
    left_key
        .0
        .cmp(right_key.0)
        .then(left_key.1.cmp(&right_key.1))
        .then(left_key.2.cmp(right_key.2))
        .then(left.cmp(right))
}

/// Parse one `path:line:text` hit into sort fields, falling back conservatively when malformed.
fn parse_hit_key(line: &str) -> (&str, u64, &str) {
    let mut parts = line.splitn(3, ':');
    let path = parts.next().unwrap_or(line);
    let number = parts
        .next()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    let text = parts.next().unwrap_or("");
    (path, number, text)
}

/// Extract a string argument from the assembled arguments object (trimming only surrounding
/// whitespace is avoided for the pattern, which is taken verbatim).
fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn uint_arg(arguments: &Value, key: &str) -> Option<usize> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|raw| usize::try_from(raw).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_tree(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("cortex-grep-{tag}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).expect("create tree");
        fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.join("src/b.rs"), "fn helper() {}\n").unwrap();
        fs::write(dir.join("c.txt"), "no functions here\n").unwrap();
        dir
    }

    fn ctx_for(dir: &Path) -> ToolContext {
        ToolContext::new(dir.to_path_buf())
    }

    // VAL-TOOLS-029: a regex content search returns the matching files/lines; cross-checked against
    // a direct `rg` invocation yielding the same hits.
    #[test]
    fn regex_search_returns_function_definition_lines() {
        let dir = temp_tree("fn");
        let output = run(&json!({ "pattern": r"fn\s+\w+" }), &ctx_for(&dir));
        assert!(!output.is_error);
        let text = output.model_content();
        assert!(text.contains("a.rs:1:fn main"), "got: {text}");
        assert!(text.contains("src/b.rs:1:fn helper"), "got: {text}");
        assert!(!text.contains("c.txt"), "non-matching file must be absent");

        // Cross-check: the same hits a direct `rg` run produces (file:line set).
        let direct = std::process::Command::new("rg")
            .args([
                "--no-heading",
                "--line-number",
                "--color",
                "never",
                "-e",
                r"fn\s+\w+",
            ])
            .current_dir(&dir)
            .output()
            .expect("run rg");
        let mut expected: Vec<String> = String::from_utf8_lossy(&direct.stdout)
            .lines()
            .map(str::to_owned)
            .collect();
        expected.sort();
        let mut got: Vec<String> = text.lines().map(str::to_owned).collect();
        got.sort();
        assert_eq!(got, expected);
    }

    // VAL-TOOLS-030: an include filter restricts the search to matching files only.
    #[test]
    fn include_filter_restricts_to_matching_files() {
        let dir = temp_tree("include");
        fs::write(dir.join("x.rs"), "shared_token here\n").unwrap();
        fs::write(dir.join("x.md"), "shared_token here\n").unwrap();
        let output = run(
            &json!({ "pattern": "shared_token", "include": "*.rs" }),
            &ctx_for(&dir),
        );
        assert!(!output.is_error);
        let text = output.model_content();
        assert!(text.contains("x.rs:"), "got: {text}");
        assert!(
            !text.contains("x.md:"),
            "excluded file must be absent: {text}"
        );
    }

    // VAL-TOOLS-031: a pattern with no matches returns an explicit empty result, not an error.
    #[test]
    fn no_matches_returns_an_explicit_empty_result() {
        let dir = temp_tree("empty");
        let output = run(&json!({ "pattern": "xyzzy_no_such_token" }), &ctx_for(&dir));
        assert!(!output.is_error, "zero hits is not an error");
        assert!(output.model_content().to_lowercase().contains("no matches"));
    }

    #[test]
    fn missing_pattern_is_a_clear_error() {
        let dir = temp_tree("nopattern");
        let output = run(&json!({}), &ctx_for(&dir));
        assert!(output.is_error);
        assert!(output.model_content().contains("pattern"));
    }

    // VAL-TOOLS-037: a match set exceeding the limits is bounded in-band with a truncation notice
    // AND the complete match set is spilled to an on-disk file named in the result.
    #[test]
    fn large_match_set_is_truncated_and_spilled() {
        let dir = temp_tree("biggrep");
        let body: String = (1..=5000).map(|_| "match_token here\n").collect();
        fs::write(dir.join("big.txt"), body).unwrap();
        let output = run(
            &json!({ "pattern": "match_token", "include": "big.txt" }),
            &ctx_for(&dir),
        );
        assert!(!output.is_error);
        let text = output.model_content();

        // Bounded in-band with a truncation notice (the shared bound_output notice).
        assert!(text.contains("output truncated"), "got: {text}");
        assert!(
            text.lines().count() < 5000,
            "in-band must be bounded, got {} lines",
            text.lines().count()
        );

        // A pointer to the spill file is present, and that file holds the COMPLETE match set.
        let marker = "written to ";
        let start = text.rfind(marker).expect("a spill pointer") + marker.len();
        let rest = &text[start..];
        let end = rest.find(']').expect("the notice closes with ]");
        let path = &rest[..end];
        let spilled = std::fs::read_to_string(path).expect("read spill file");
        assert_eq!(spilled.lines().count(), 5000, "spill holds every match");
        let _ = std::fs::remove_file(path);
    }

    // VAL-TOOLS-037 (no regression): a small match set returns verbatim with no notice and no spill.
    #[test]
    fn small_match_set_returns_verbatim_with_no_spill() {
        let dir = temp_tree("smallgrep");
        let output = run(&json!({ "pattern": r"fn\s+\w+" }), &ctx_for(&dir));
        let text = output.model_content();
        assert!(!text.contains("output truncated"), "got: {text}");
        assert!(!text.contains("written to"), "got: {text}");
    }

    #[test]
    fn explicit_limit_overrides_default_match_cap() {
        let dir = temp_tree("limit");
        let body: String = (1..=50).map(|n| format!("hit {n}\n")).collect();
        fs::write(dir.join("hits.txt"), body).unwrap();
        let output = run(
            &json!({ "pattern": "^hit\\s+\\d+$", "include": "hits.txt", "limit": 7 }),
            &ctx_for(&dir),
        );
        assert!(!output.is_error, "{}", output.model_content());
        let matches = output
            .model_content()
            .lines()
            .filter(|line| line.contains("hits.txt:"))
            .count();
        assert_eq!(matches, 7);
    }

    #[test]
    fn matches_are_sorted_by_numeric_line_number_within_each_file() {
        let dir = temp_tree("ordering");
        let ordered = (1..=12)
            .map(|line| {
                if line == 2 || line == 10 {
                    format!("hit {line}")
                } else {
                    format!("miss {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.join("order.txt"), format!("{ordered}\n")).unwrap();

        let output = run(
            &json!({ "pattern": "^hit\\s+\\d+$", "include": "order.txt" }),
            &ctx_for(&dir),
        );
        assert!(!output.is_error, "{}", output.model_content());
        let rendered = output.model_content();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 2, "got: {lines:?}");
        assert!(
            lines[0].contains("order.txt:2:"),
            "line 2 should sort before line 10: {lines:?}"
        );
        assert!(
            lines[1].contains("order.txt:10:"),
            "line 10 should follow line 2: {lines:?}"
        );
    }
}

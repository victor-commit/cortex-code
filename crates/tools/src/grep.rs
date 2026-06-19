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

use crate::ripgrep::{self, RgError};
use crate::tool::{Tool, ToolContext, ToolOutput};

/// The cap on the number of match lines returned, so a very broad pattern cannot flood the model.
/// A truncated result is marked so the model knows to narrow the search.
const MAX_RESULTS: usize = 200;

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
        .filter(|value| !value.trim().is_empty());

    let root_display = path.as_deref().unwrap_or(".");
    if !ctx.permits("grep", root_display) {
        return ToolOutput::error(format!("grep: permission denied for {root_display}"));
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
        Ok(stdout) => render(&pattern, &stdout),
        Err(RgError::NotInstalled) => {
            ToolOutput::error("grep: ripgrep (rg) is required but was not found on PATH")
        }
        Err(RgError::Failed(detail)) => ToolOutput::error(format!("grep: {detail}")),
    }
}

/// Render ripgrep's match lines into a sorted, capped tool result, or an explicit empty result.
fn render(pattern: &str, stdout: &str) -> ToolOutput {
    let mut hits: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    hits.sort_unstable();

    if hits.is_empty() {
        return ToolOutput::text(format!("No matches for pattern: {pattern}"));
    }

    let total = hits.len();
    let mut body = hits
        .iter()
        .take(MAX_RESULTS)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if total > MAX_RESULTS {
        body.push_str(&format!(
            "\n[truncated: showing {MAX_RESULTS} of {total} matches; narrow the pattern or use include]"
        ));
    }
    ToolOutput::text(body)
}

/// Extract a string argument from the assembled arguments object (trimming only surrounding
/// whitespace is avoided for the pattern, which is taken verbatim).
fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
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
}

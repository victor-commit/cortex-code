//! The `glob` tool: list the files matching a shell-style filename pattern, ripgrep-backed.
//!
//! Behavior (architecture §3.4, validation contract VAL-TOOLS-027/028):
//!
//! - Returns the set of paths matching a shell-style pattern (e.g. `**/*.rs`), relative to the
//!   search root, excluding non-matches. Results are sorted so the set is reported deterministically
//!   (the contract treats the set as order-independent).
//! - A pattern that matches nothing returns an explicit empty result (a clear "no files match"
//!   message), never an error or a hang.
//! - Backed by the real `rg --files -g <pattern>`, so matching honors ripgrep's glob semantics and
//!   its default ignore handling (build artifacts under an ignored directory are not returned).

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::output::{OutputLimits, bound_output};
use crate::ripgrep::{self, RgError};
use crate::tool::{Tool, ToolContext, ToolOutput};

/// The cap on the number of paths returned, so a very broad pattern cannot flood the model. A
/// truncated result is marked so the model knows to narrow the pattern.
const MAX_RESULTS: usize = 1000;

const DESCRIPTION: &str = "Find files by a shell-style filename pattern (glob). Returns the paths \
matching the pattern (for example `**/*.rs` for every Rust file at any depth, or `src/*.ts`), \
relative to the search root, one per line. A pattern that matches nothing returns an explicit \
empty result. Matching follows ripgrep semantics and skips files ignored by `.gitignore`.";

/// The `glob` tool.
pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
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
                    "description": "The shell-style filename pattern to match (e.g. `**/*.rs`, `src/*.ts`)."
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search under (absolute, or relative to the working directory). Defaults to the working directory."
                }
            },
            "required": ["pattern"]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

/// Execute a glob against the resolved arguments (synchronous shell-out to ripgrep).
fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(pattern) = string_arg(arguments, "pattern").or_else(|| string_arg(arguments, "glob"))
    else {
        return ToolOutput::error("glob: the 'pattern' argument is required");
    };
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return ToolOutput::error("glob: the 'pattern' argument must not be empty");
    }

    let path = string_arg(arguments, "path");
    let root_display = path.as_deref().unwrap_or(".");
    if !ctx.permits("glob", root_display) {
        return ToolOutput::error(format!("glob: permission denied for {root_display}"));
    }
    let root = match &path {
        Some(path) => ctx.resolve(path),
        None => ctx.cwd().to_path_buf(),
    };

    match ripgrep::run(&root, ["--files", "--glob", pattern]) {
        Ok(stdout) => render(pattern, &stdout),
        Err(RgError::NotInstalled) => {
            ToolOutput::error("glob: ripgrep (rg) is required but was not found on PATH")
        }
        Err(RgError::Failed(detail)) => ToolOutput::error(format!("glob: {detail}")),
    }
}

/// Render ripgrep's file list into a sorted, capped tool result, or an explicit empty result.
fn render(pattern: &str, stdout: &str) -> ToolOutput {
    let mut paths: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    paths.sort_unstable();
    paths.dedup();

    if paths.is_empty() {
        return ToolOutput::text(format!("No files match the pattern: {pattern}"));
    }

    let full = paths.join("\n");
    let bounded = bound_output(
        &full,
        OutputLimits {
            max_lines: MAX_RESULTS,
            max_bytes: usize::MAX,
        },
        "glob-matches",
    );
    ToolOutput::text(bounded.text)
}

/// Extract a trimmed string argument from the assembled arguments object.
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
            std::env::temp_dir().join(format!("cortex-glob-{tag}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).expect("create tree");
        fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
        fs::write(dir.join("src/b.rs"), "fn helper() {}\n").unwrap();
        fs::write(dir.join("c.txt"), "plain text\n").unwrap();
        dir
    }

    fn ctx_for(dir: &Path) -> ToolContext {
        ToolContext::new(dir.to_path_buf())
    }

    fn lines(output: &ToolOutput) -> Vec<String> {
        output.model_content().lines().map(str::to_owned).collect()
    }

    // VAL-TOOLS-027: `**/*.rs` returns exactly the Rust files (relative paths), no non-matches.
    #[test]
    fn globs_rust_files_returns_exactly_the_matches() {
        let dir = temp_tree("rs");
        let output = run(&json!({ "pattern": "**/*.rs" }), &ctx_for(&dir));
        assert!(!output.is_error);
        let mut got = lines(&output);
        got.sort();
        assert_eq!(got, vec!["a.rs".to_owned(), "src/b.rs".to_owned()]);
        assert!(!output.model_content().contains("c.txt"));
    }

    // VAL-TOOLS-028: a pattern matching nothing returns an explicit empty result, not an error.
    #[test]
    fn globbing_no_matches_returns_an_explicit_empty_result() {
        let dir = temp_tree("none");
        let output = run(&json!({ "pattern": "**/*.zzz" }), &ctx_for(&dir));
        assert!(!output.is_error, "no matches is not an error");
        let text = output.model_content();
        assert!(text.to_lowercase().contains("no files"));
        assert!(!text.contains(".rs"));
    }

    #[test]
    fn missing_pattern_is_a_clear_error() {
        let dir = temp_tree("missing");
        let output = run(&json!({}), &ctx_for(&dir));
        assert!(output.is_error);
        assert!(output.model_content().contains("pattern"));
    }

    #[test]
    fn path_argument_scopes_the_search_root() {
        let dir = temp_tree("scoped");
        let output = run(&json!({ "pattern": "*.rs", "path": "src" }), &ctx_for(&dir));
        assert!(!output.is_error);
        let got = lines(&output);
        assert_eq!(got, vec!["b.rs".to_owned()]);
    }

    #[test]
    fn large_match_set_is_truncated_and_spilled() {
        let dir = temp_tree("spill");
        for index in 0..1205 {
            fs::write(dir.join(format!("many-{index:04}.txt")), "x\n").unwrap();
        }

        let output = run(&json!({ "pattern": "*.txt" }), &ctx_for(&dir));
        assert!(!output.is_error);
        let text = output.model_content();
        assert!(text.contains("output truncated"), "got: {text}");
        assert!(
            text.lines().count() <= MAX_RESULTS + 1,
            "in-band output must be bounded"
        );

        let marker = "written to ";
        let start = text.rfind(marker).expect("a spill pointer") + marker.len();
        let rest = &text[start..];
        let end = rest.find(']').expect("the notice closes with ]");
        let path = &rest[..end];
        let spilled = fs::read_to_string(path).expect("read spill file");
        assert!(
            spilled.lines().count() >= 1205,
            "spill should include every match"
        );
        let _ = fs::remove_file(path);
    }
}

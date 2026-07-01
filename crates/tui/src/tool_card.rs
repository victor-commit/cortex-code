//! The transcript model for an agent tool call rendered as a labeled card.
//!
//! The durable runner surfaces a [`engine::RunnerEvent::ToolCall`] before a tool runs and a
//! matching [`engine::RunnerEvent::ToolResult`] after it finishes (architecture §4). The TUI turns
//! that pair into a single [`ToolCard`]: created in the [`ToolStatus::Running`] state when the call
//! starts, then transitioned to [`ToolStatus::Completed`] or [`ToolStatus::Failed`] when the result
//! arrives. The card names the tool and summarizes its target/args (VAL-TUI-024), reflects its
//! running/completed status (VAL-TUI-025), can be expanded to reveal full detail (VAL-TUI-026), and
//! renders a distinct error state when the tool fails (VAL-TUI-027). Rendering to `ratatui` lines
//! lives in [`crate::ui`]; this module owns the data model and the argument/output summarization.

use std::time::Instant;

use protocol::ToolResultPart;

/// The lifecycle state of a tool call as observed by the transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// The call has started but its result has not arrived yet.
    Running,
    /// The call finished successfully.
    Completed,
    /// The call finished with an error (the turn still continues so the model can recover).
    Failed,
}

/// One tool call rendered as a card in the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCard {
    /// The provider-assigned call id, used to match the later result to this card.
    pub id: String,
    /// The tool/function name (e.g. `read`, `grep`, `bash`).
    pub name: String,
    /// A short, single-line summary of the call's target/arguments shown in the collapsed card.
    pub summary: String,
    /// The full arguments rendered for the expanded detail view.
    pub args_detail: String,
    /// The tool's textual output/result, populated once the call finishes (the full detail).
    pub output: String,
    /// The lifecycle state driving the card's status styling.
    pub status: ToolStatus,
    /// Wall-clock duration of the call (set when it finishes), shown as the `· <N>ms` meta tail.
    pub duration_ms: Option<u64>,
    /// For file-mutating tools (write/edit/apply_patch), the (additions, removals) line counts shown
    /// as the `+N -N` diff meta; `None` for tools without a diff shape.
    pub diff_stat: Option<(usize, usize)>,
    /// The computed file diff for a file-mutating call, attached when the turn reports the change.
    /// Its presence turns the card into an expandable diff card: a `▶`/`▼` disclosure gutter with a
    /// boxed, line-numbered red/green diff revealed when the card is expanded.
    pub diff: Option<Box<crate::diff::FileDiff>>,
    /// When the call started, used to measure [`ToolCard::duration_ms`] on completion.
    started: Instant,
}

impl ToolCard {
    /// Build a card for a freshly started tool call in the [`ToolStatus::Running`] state.
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: &str) -> Self {
        let name = name.into();
        let arguments = redaction::redact(arguments);
        let diff_stat = diff_stat_for(&name, &arguments);
        Self {
            id: id.into(),
            summary: redaction::redact(&summarize_args(&name, &arguments)),
            args_detail: redaction::redact(&pretty_args(&arguments)),
            name,
            output: String::new(),
            status: ToolStatus::Running,
            duration_ms: None,
            diff_stat,
            diff: None,
            started: Instant::now(),
        }
    }

    /// Attach the computed file diff for a file-mutating call, upgrading the card to an expandable
    /// diff card. The `+N -N` meta is refreshed to the diff's real line counts so the header matches
    /// the boxed body revealed on expand.
    pub fn attach_diff(&mut self, diff: crate::diff::FileDiff) {
        self.diff_stat = Some((diff.additions, diff.removals));
        self.diff = Some(Box::new(diff));
    }

    /// Transition a running card to its terminal state, recording the result text.
    ///
    /// `is_error` selects [`ToolStatus::Failed`] (a distinct error state) over
    /// [`ToolStatus::Completed`]; `parts` are flattened to the text shown in the expanded view.
    pub fn complete(&mut self, parts: &[ToolResultPart], is_error: bool) {
        self.output = redaction::redact(&text_from_parts(parts));
        self.status = if is_error {
            ToolStatus::Failed
        } else {
            ToolStatus::Completed
        };
        self.duration_ms = Some(self.started.elapsed().as_millis() as u64);
    }
}

/// Best-effort `(additions, removals)` line counts for a file-mutating tool call, derived from its
/// arguments so the card can show a `+N -N` diff meta. Returns `None` for tools without a known
/// edit shape (the meta then falls back to the duration tail alone).
fn diff_stat_for(name: &str, arguments: &str) -> Option<(usize, usize)> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let str_arg = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| {
            value
                .get(*k)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
    };
    match name {
        "edit" | "multiedit" => {
            let old = str_arg(&["oldString", "old_str", "oldText"]).unwrap_or_default();
            let new = str_arg(&["newString", "new_str", "newText"]).unwrap_or_default();
            Some((line_count(&new), line_count(&old)))
        }
        "write" => {
            let content = str_arg(&["content", "text"]).unwrap_or_default();
            Some((line_count(&content), 0))
        }
        "apply_patch" => {
            let patch = str_arg(&["patch", "patchText", "patch_text", "input"])?;
            Some(patch_line_counts(&patch))
        }
        _ => None,
    }
}

/// The number of text lines in `s` (an empty string is zero lines).
fn line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count()
    }
}

/// Count the `(added, removed)` body lines of a patch envelope, ignoring the `+++`/`---` file
/// markers so only real content changes are tallied.
fn patch_line_counts(patch: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in patch.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => added += 1,
            Some(b'-') => removed += 1,
            _ => {}
        }
    }
    (added, removed)
}

/// The maximum number of characters kept in a card's one-line argument summary.
const SUMMARY_MAX: usize = 100;

/// Produce a short, human-readable summary of a tool call's most salient argument(s).
///
/// The summary names the target a reader cares about per tool (the path for file tools, the pattern
/// for search, the command for `bash`), falling back to a compact rendering of the raw arguments for
/// tools without a known shape. The result is always a single line, truncated to [`SUMMARY_MAX`].
#[must_use]
pub fn summarize_args(name: &str, arguments: &str) -> String {
    let value: serde_json::Value =
        serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
    let field = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };

    let summary = match name {
        "read" => field("path"),
        "write" | "edit" => field("path"),
        "grep" | "search" => match (field("pattern"), field("path")) {
            (Some(p), Some(path)) => Some(format!("{p} in {path}")),
            (Some(p), None) => Some(p),
            _ => None,
        },
        "glob" | "list" => field("pattern").or_else(|| field("path")),
        "bash" => field("command"),
        "webfetch" => field("url"),
        "websearch" => field("query"),
        "apply_patch" => field("patch").map(|p| first_patch_target(&p)),
        _ => None,
    };

    let summary = summary.unwrap_or_else(|| compact_args(&value, arguments));
    truncate(&one_line(&summary), SUMMARY_MAX)
}

/// The full argument set rendered for the expanded card: pretty-printed JSON when the arguments
/// parse, otherwise the raw (trimmed) argument string.
#[must_use]
fn pretty_args(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) if !value.is_null() => {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.trim().to_owned())
        }
        _ => arguments.trim().to_owned(),
    }
}

/// Flatten a tool result's parts to displayable text: text parts verbatim, attachment parts as a
/// `[image/document <media-type>: <path>]` reference so non-text payloads never leak raw bytes.
#[must_use]
pub fn text_from_parts(parts: &[ToolResultPart]) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            ToolResultPart::Text { text } => out.push_str(text),
            ToolResultPart::Image { media_type, path } => {
                out.push_str(&format!("[image {media_type}: {path}]"));
            }
            ToolResultPart::Document { media_type, path } => {
                out.push_str(&format!("[document {media_type}: {path}]"));
            }
        }
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out.trim_end().to_owned()
}

/// A compact rendering of arbitrary arguments for tools without a known shape: the top-level
/// `key=value` pairs of a JSON object, else the trimmed raw string.
fn compact_args(value: &serde_json::Value, raw: &str) -> String {
    match value.as_object() {
        Some(map) if !map.is_empty() => map
            .iter()
            .map(|(k, v)| format!("{k}={}", scalar(v)))
            .collect::<Vec<_>>()
            .join(" "),
        _ => raw.trim().to_owned(),
    }
}

/// Render a JSON scalar compactly for the `key=value` summary (strings without their quotes).
fn scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The first file an `apply_patch` envelope adds or updates, for the one-line summary.
fn first_patch_target(patch: &str) -> String {
    for line in patch.lines() {
        let line = line.trim();
        for prefix in ["*** Add File: ", "*** Update File: ", "*** Delete File: "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                return rest.trim().to_owned();
            }
        }
    }
    "patch".to_owned()
}

/// Collapse any internal newlines/whitespace runs so a summary stays on a single line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncate to at most `max` characters, appending an ellipsis when shortened.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_summary_names_the_path() {
        assert_eq!(
            summarize_args("read", r#"{"path":"src/main.rs"}"#),
            "src/main.rs"
        );
    }

    #[test]
    fn grep_summary_names_pattern_and_path() {
        assert_eq!(
            summarize_args("grep", r#"{"pattern":"TODO","path":"src"}"#),
            "TODO in src"
        );
    }

    #[test]
    fn bash_summary_names_the_command() {
        assert_eq!(summarize_args("bash", r#"{"command":"ls -la"}"#), "ls -la");
    }

    #[test]
    fn unknown_tool_falls_back_to_key_value_pairs() {
        let summary = summarize_args("mystery", r#"{"a":"x","b":2}"#);
        assert!(summary.contains("a=x"), "summary: {summary}");
        assert!(summary.contains("b=2"), "summary: {summary}");
    }

    #[test]
    fn summary_is_a_single_truncated_line() {
        let long = "x".repeat(400);
        let args = serde_json::json!({ "command": long }).to_string();
        let summary = summarize_args("bash", &args);
        assert!(summary.chars().count() <= SUMMARY_MAX);
        assert!(summary.ends_with('…'));
        assert!(!summary.contains('\n'));
    }

    #[test]
    fn multiline_command_summary_is_flattened() {
        let args = serde_json::json!({ "command": "echo a\necho b" }).to_string();
        assert_eq!(summarize_args("bash", &args), "echo a echo b");
    }

    #[test]
    fn new_card_starts_running_and_summarized() {
        let card = ToolCard::new("call_1", "read", r#"{"path":"a.txt"}"#);
        assert_eq!(card.status, ToolStatus::Running);
        assert_eq!(card.name, "read");
        assert_eq!(card.summary, "a.txt");
        assert!(card.output.is_empty());
    }

    #[test]
    fn completing_a_card_records_output_and_success() {
        let mut card = ToolCard::new("call_1", "read", r#"{"path":"a.txt"}"#);
        card.complete(&[ToolResultPart::text("1: hello\n2: world")], false);
        assert_eq!(card.status, ToolStatus::Completed);
        assert!(card.output.contains("hello"));
    }

    #[test]
    fn failing_a_card_records_error_state() {
        let mut card = ToolCard::new("call_1", "read", r#"{"path":"nope.txt"}"#);
        card.complete(&[ToolResultPart::text("file not found: nope.txt")], true);
        assert_eq!(card.status, ToolStatus::Failed);
        assert!(card.output.contains("not found"));
    }

    #[test]
    fn attachment_parts_render_as_references_not_bytes() {
        let text = text_from_parts(&[ToolResultPart::image("image/png", "/tmp/logo.png")]);
        assert_eq!(text, "[image image/png: /tmp/logo.png]");
    }

    #[test]
    fn detail_pretty_prints_arguments() {
        let card = ToolCard::new("c", "bash", r#"{"command":"ls"}"#);
        assert!(card.args_detail.contains("\"command\""));
        assert!(card.args_detail.contains("ls"));
    }

    #[test]
    fn edit_card_derives_diff_stat_from_old_and_new_strings() {
        let card = ToolCard::new(
            "c",
            "edit",
            r#"{"path":"a.rs","oldString":"one\ntwo","newString":"alpha\nbeta\ngamma"}"#,
        );
        assert_eq!(card.diff_stat, Some((3, 2)));
    }

    #[test]
    fn write_card_counts_content_as_additions_only() {
        let card = ToolCard::new("c", "write", r#"{"path":"a.rs","content":"x\ny\nz"}"#);
        assert_eq!(card.diff_stat, Some((3, 0)));
    }

    #[test]
    fn apply_patch_card_counts_plus_minus_body_lines() {
        let patch = "*** Update File: a.rs\n@@\n-old line\n+new line\n+extra line\n context";
        let args = serde_json::json!({ "patch": patch }).to_string();
        let card = ToolCard::new("c", "apply_patch", &args);
        assert_eq!(card.diff_stat, Some((2, 1)));
    }

    #[test]
    fn non_mutating_tools_have_no_diff_stat() {
        assert_eq!(
            ToolCard::new("c", "read", r#"{"path":"a.rs"}"#).diff_stat,
            None
        );
        assert_eq!(
            ToolCard::new("c", "bash", r#"{"command":"ls"}"#).diff_stat,
            None
        );
    }

    #[test]
    fn completing_a_card_records_a_duration() {
        let mut card = ToolCard::new("c", "read", r#"{"path":"a.rs"}"#);
        assert_eq!(card.duration_ms, None);
        card.complete(&[ToolResultPart::text("ok")], false);
        assert!(card.duration_ms.is_some());
    }
}

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

use std::path::PathBuf;
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
    /// For a backgrounded `bash` job (VAL-TOOL-023), the on-disk log file its combined
    /// stdout/stderr streams to. Set by [`ToolCard::start_background`] instead of completing the
    /// card, so [`crate::app::App::poll_background_jobs`] can tail it into `output` while the card
    /// stays [`ToolStatus::Running`]. Kept (not cleared) once the job finishes, so the card still
    /// knows which log it streamed.
    pub bg_log: Option<PathBuf>,
    /// For a backgrounded `bash` job, the OS process id tracked to detect when the job exits.
    /// `None` once the job has completed (or for any card that was never backgrounded).
    pub bg_pid: Option<u32>,
    /// The tracked background job's process start time (`/proc/<pid>` field 22), captured at
    /// [`ToolCard::start_background`] so [`App::poll_background_jobs`](crate::app::App::poll_background_jobs)
    /// can reject a *reused* pid: if the pid is alive but its start time no longer matches, the real
    /// job has exited and an unrelated process now holds the pid. `None` when unknown.
    pub bg_start_time: Option<u64>,
    /// Whether this call requested a background job (`bash` with `background: true`), derived from
    /// the structured tool-call arguments at construction. This is the single, unambiguous gate for
    /// entering background-streaming mode: unlike `summary.contains("(background)")` it is immune to
    /// the [`SUMMARY_MAX`] truncation that drops the marker from a long command, and to a foreground
    /// command whose text merely mentions `(background)`.
    pub is_background: bool,
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
        let is_background = is_background_call(&name, &arguments);
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
            bg_log: None,
            bg_pid: None,
            bg_start_time: None,
            is_background,
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

        // Post-completion summary override for tools whose summary depends on the result
        // (VAL-TOOL-024: todo summary is "{N} items · {M} in progress").
        if self.name == "todo" && !is_error {
            self.summary = todo_summary_from_output(&self.output);
        }
    }

    /// Attach a backgrounded `bash` job's process id and on-disk log path, so the card stays
    /// [`ToolStatus::Running`] (rather than completing on the "job started" acknowledgement) while
    /// [`crate::app::App::poll_background_jobs`] tails the log into `output` each frame
    /// (VAL-TOOL-023).
    pub fn start_background(&mut self, pid: u32, log_path: PathBuf) {
        self.bg_pid = Some(pid);
        self.bg_log = Some(log_path);
        self.bg_start_time = tools::process_start_time(pid);
    }

    /// Transition a backgrounded card to [`ToolStatus::Completed`] once its process id has exited,
    /// recording the final tailed log content. `bg_log` is kept so the card remembers which log it
    /// streamed; `bg_pid` is cleared so [`App::poll_background_jobs`](crate::app::App::poll_background_jobs)
    /// stops polling it.
    pub fn complete_background(&mut self, final_output: String) {
        self.output = final_output;
        self.status = ToolStatus::Completed;
        self.duration_ms = Some(self.started.elapsed().as_millis() as u64);
        self.bg_pid = None;
        self.bg_start_time = None;
    }

    /// Whether this card is a backgrounded `bash` job still being polled (running with a tracked
    /// pid), the seam [`push_tool`](crate::ui::push_tool) uses to keep the "esc to background"
    /// footer hint scoped to the live window.
    #[must_use]
    pub fn is_background_running(&self) -> bool {
        self.status == ToolStatus::Running && self.bg_pid.is_some()
    }

    /// Whether the tracked background job's process is still the *same* live process it was at
    /// [`ToolCard::start_background`]. This is the completion-transition signal: it is driven by the
    /// job's real exit and, by comparing the captured `/proc/<pid>` start time, is immune to pid
    /// reuse (a freed pid re-taken by an unrelated process reads as not alive). Returns `false` for
    /// a card with no tracked pid.
    #[must_use]
    pub fn bg_process_alive(&self) -> bool {
        match self.bg_pid {
            Some(pid) => tools::pid_alive_same_process(pid, self.bg_start_time),
            None => false,
        }
    }
}

/// Whether a tool call requests a background job: a `bash` call whose structured arguments carry
/// `background: true`. Derived from the parsed JSON arguments (not the truncatable summary text), so
/// it is the unambiguous gate for background-streaming mode regardless of command length.
#[must_use]
pub fn is_background_call(name: &str, arguments: &str) -> bool {
    if name != "bash" {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(arguments)
        .ok()
        .and_then(|value| value.get("background").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

/// Parse the `pid`/`output` fields from a `bash` background-job start acknowledgement into the
/// `(pid, log_path)` a card needs to keep streaming. Returns `None` when either field is
/// missing/unparseable, so the caller can fall back to completing the card normally.
#[must_use]
pub fn parse_background_job(text: &str) -> Option<(u32, PathBuf)> {
    let mut pid: Option<u32> = None;
    let mut log_path: Option<PathBuf> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("pid:") {
            pid = rest.trim().parse::<u32>().ok();
        } else if let Some(rest) = line.strip_prefix("output:") {
            log_path = Some(PathBuf::from(rest.trim()));
        }
    }
    match (pid, log_path) {
        (Some(pid), Some(path)) => Some((pid, path)),
        _ => None,
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
    if s.is_empty() { 0 } else { s.lines().count() }
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

/// Maximum characters kept in a Cortex-style tool header line (VAL-CORTEX-CHAT-002).
const HEADER_MAX: usize = 120;

/// Format a Cortex tool header line from a tool name + structured arguments.
///
/// Pure-structure DNA (mockups-v3 `tool-calls` / `tool-running` / `tool-success`):
/// - bash / shell → `$ <command>`
/// - read → `Read <path>`
/// - grep / search → `Search "<pattern>"` (optional ` (N matches)` when known)
/// - edit / multiedit → `Edit <path>` (optional ` +A/-R` when a diff stat is known)
/// - write → `Write <path>`
/// - apply_patch → `Edit <target>`
/// - websearch → `Search <query>`
/// - webfetch → `Fetch <url>`
/// - other → Title-case tool name + summary
///
/// This is intentionally box-free: the header is the primary tool label the transcript paints.
#[must_use]
pub fn format_tool_header(
    name: &str,
    arguments: &str,
    diff_stat: Option<(usize, usize)>,
    match_count: Option<usize>,
) -> String {
    let summary = summarize_args(name, arguments);
    let header = match name {
        "bash" | "shell" => {
            // Strip the background marker from the `$` form (Cortex shows the live command only).
            let cmd = summary
                .strip_suffix(" (background)")
                .unwrap_or(summary.as_str());
            format!("$ {cmd}")
        }
        "read" => format!("Read {summary}"),
        "grep" | "search" => {
            let value: serde_json::Value =
                serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
            let pattern = value
                .get("pattern")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(summary.as_str());
            let base = format!("Search \"{pattern}\"");
            match match_count {
                Some(n) => format!("{base} ({n} matches)"),
                None => base,
            }
        }
        "edit" | "multiedit" => match diff_stat {
            Some((a, r)) => format!("Edit {summary} +{a}/-{r}"),
            None => format!("Edit {summary}"),
        },
        "write" => match diff_stat {
            Some((a, r)) if a > 0 || r > 0 => format!("Write {summary} +{a}/-{r}"),
            _ => format!("Write {summary}"),
        },
        "apply_patch" => match diff_stat {
            Some((a, r)) => format!("Edit {summary} +{a}/-{r}"),
            None => format!("Edit {summary}"),
        },
        "websearch" => format!("Search {summary}"),
        "webfetch" => format!("Fetch {summary}"),
        "glob" | "list" => format!("Glob {summary}"),
        "todo" => format!("Todo {summary}"),
        "task" => format!("Task {summary}"),
        "lsp" => format!("Lsp {summary}"),
        "question" => format!("Ask {summary}"),
        other => {
            let title = title_case_tool(other);
            if summary.is_empty() {
                title
            } else {
                format!("{title} {summary}")
            }
        }
    };
    truncate(&one_line(&header), HEADER_MAX)
}

/// Cortex-style secondary status line under a tool header (running / done meta).
///
/// Examples:
/// - running bash: `running · waiting on subshell`
/// - completed bash with duration: `done · 1800ms`
/// - failed: `error`
#[must_use]
pub fn format_tool_status_line(card: &ToolCard) -> String {
    let mut parts: Vec<String> = Vec::new();
    let state = match card.status {
        ToolStatus::Running => {
            if card.is_background_running() || card.is_background {
                "running · background"
            } else if card.name == "bash" || card.name == "shell" {
                "running · waiting on subshell"
            } else if card.name == "question" {
                "waiting for your answer"
            } else {
                "running"
            }
        }
        ToolStatus::Completed => "done",
        ToolStatus::Failed => "error",
    };
    parts.push(state.to_owned());
    if let Some(ms) = card.duration_ms {
        parts.push(format_duration_ms(ms));
    }
    if let Some((a, r)) = card.diff_stat {
        if card.status != ToolStatus::Running && matches!(card.name.as_str(), "edit" | "multiedit" | "write" | "apply_patch") {
            // Diff stat already rides the header for edit/write; skip duplicate on status.
            let _ = (a, r);
        }
    }
    parts.join(" · ")
}

fn format_duration_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        let secs = ms as f64 / 1000.0;
        if secs < 10.0 {
            format!("{secs:.1}s")
        } else {
            format!("{}s", ms / 1000)
        }
    }
}

fn title_case_tool(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Best-effort match count for a completed grep/search tool from its textual output.
///
/// Recognises common grep-style `path:line:` rows (one match per non-empty non-header line) and
/// explicit `N matches` tails. Returns `None` when the output is empty or unparseable.
#[must_use]
pub fn match_count_from_output(output: &str) -> Option<usize> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Explicit "N matches" / "N match" phrase wins when present.
    for token in trimmed.split_whitespace() {
        if let Some(n) = token
            .strip_suffix("matches")
            .or_else(|| token.strip_suffix("match"))
            .and_then(|s| s.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok())
        {
            if n > 0 {
                return Some(n);
            }
        }
    }
    // Grep-like rows: `path:line:…` (require at least two colons so plain prose is ignored).
    let rows = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            let mut parts = line.splitn(3, ':');
            matches!(
                (parts.next(), parts.next(), parts.next()),
                (Some(p), Some(l), Some(_)) if !p.is_empty() && l.chars().all(|c| c.is_ascii_digit())
            )
        })
        .count();
    (rows > 0).then_some(rows)
}

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
    // The file tools accept `path`/`file_path`/`filePath` interchangeably, so the summary resolves
    // the target path from any of them (VAL-TOOL-015/016/017).
    let path_field = || {
        field("path")
            .or_else(|| field("file_path"))
            .or_else(|| field("filePath"))
    };

    let summary = match name {
        "read" => path_field(),
        "write" | "edit" => path_field(),
        "grep" | "search" => match (field("pattern"), field("path")) {
            (Some(p), Some(path)) => Some(format!("{p} in {path}")),
            (Some(p), None) => Some(p),
            _ => None,
        },
        "glob" | "list" => field("pattern").or_else(|| field("path")),
        "bash" => {
            let cmd = field("command");
            let is_background = value
                .get("background")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            match (cmd, is_background) {
                (Some(c), true) => Some(format!("{c} (background)")),
                (Some(c), false) => Some(c),
                _ => None,
            }
        }
        "webfetch" => field("url"),
        "websearch" => field("query"),
        // A subagent delegation's summary is the delegated objective: `{subagent}: {prompt}`
        // (VAL-TOOL-028). The subagent type accepts the real `subagent_type`/`subagentType`/`agent`
        // aliases; when the prompt is absent (e.g. a `task_id` resume) the agent name alone stands in.
        "task" => {
            let agent = field("subagent_type")
                .or_else(|| field("subagentType"))
                .or_else(|| field("agent"));
            let prompt = field("prompt").or_else(|| field("description"));
            match (agent, prompt) {
                (Some(a), Some(p)) => Some(format!("{a}: {p}")),
                (Some(a), None) => Some(a),
                (None, Some(p)) => Some(p),
                _ => None,
            }
        }
        // An lsp diagnostics query summarizes as `diagnostics {target}` (VAL-TOOL-029), where the
        // target is the queried file (any path alias) or the workspace-symbol query, defaulting to
        // `workspace` for a workspace-wide request.
        "lsp" => {
            let target = field("filePath")
                .or_else(|| field("file_path"))
                .or_else(|| field("path"))
                .or_else(|| field("query"))
                .unwrap_or_else(|| "workspace".to_owned());
            Some(format!("diagnostics {target}"))
        }
        // A user-facing question's summary is the (first) question prompt (VAL-TOOL-030).
        "question" => value
            .get("questions")
            .and_then(serde_json::Value::as_array)
            .and_then(|questions| questions.first())
            .and_then(|first| first.get("prompt"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        "apply_patch" => field("patch")
            .or_else(|| field("patchText"))
            .or_else(|| field("patch_text"))
            .or_else(|| field("input"))
            .map(|p| first_patch_target(&p)),
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

/// Compute the todo card summary from the tool's output: "{N} items · {M} in progress"
/// (VAL-TOOL-024). The output format from the todo tool is a numbered list where each line starts
/// with `N. [status][priority] content`, or `(no todos)` when empty.
fn todo_summary_from_output(output: &str) -> String {
    if output.trim() == "(no todos)" {
        return "0 items".to_owned();
    }
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let total = lines.len();
    let in_progress = lines.iter().filter(|l| l.contains("[in_progress]")).count();
    if in_progress > 0 {
        format!("{total} items · {in_progress} in progress")
    } else {
        format!("{total} items")
    }
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
    fn background_bash_summary_is_marked_background() {
        // VAL-TOOL-023: a background bash call's summary marks the job as `(background)`.
        assert_eq!(
            summarize_args(
                "bash",
                r#"{"command":"cargo build --release","background":true}"#
            ),
            "cargo build --release (background)"
        );
        // A foreground call (background false / absent) is the plain command.
        assert_eq!(
            summarize_args("bash", r#"{"command":"cargo test","background":false}"#),
            "cargo test"
        );
    }

    #[test]
    fn websearch_summary_is_the_query() {
        // VAL-TOOL-026: a websearch call's summary is the search query.
        assert_eq!(
            summarize_args("websearch", r#"{"query":"ratatui table widget"}"#),
            "ratatui table widget"
        );
    }

    #[test]
    fn webfetch_summary_is_the_url() {
        // VAL-TOOL-027: a webfetch call's summary is the fetched URL.
        assert_eq!(
            summarize_args("webfetch", r#"{"url":"https://docs.rs/ratatui/latest"}"#),
            "https://docs.rs/ratatui/latest"
        );
    }

    #[test]
    fn todo_card_summary_is_computed_from_output_on_complete() {
        // VAL-TOOL-024: a todo card's summary becomes `{N} items · {M} in progress`, derived from
        // the tool result after completion.
        let mut card = ToolCard::new("c", "todo", r#"{}"#);
        card.complete(
            &[ToolResultPart::text(
                "1. [completed][medium] a\n2. [in_progress][high] b\n3. [pending][medium] c",
            )],
            false,
        );
        assert_eq!(card.summary, "3 items · 1 in progress");
    }

    #[test]
    fn todo_card_summary_without_in_progress_omits_the_tail() {
        let mut card = ToolCard::new("c", "todo", r#"{}"#);
        card.complete(
            &[ToolResultPart::text(
                "1. [completed][medium] a\n2. [pending][medium] b",
            )],
            false,
        );
        assert_eq!(card.summary, "2 items");
    }

    #[test]
    fn todo_card_summary_handles_empty_list() {
        let mut card = ToolCard::new("c", "todo", r#"{}"#);
        card.complete(&[ToolResultPart::text("(no todos)")], false);
        assert_eq!(card.summary, "0 items");
    }

    #[test]
    fn task_summary_is_the_subagent_objective() {
        // VAL-TOOL-028: a task (subagent) card's summary is `{subagent}: {prompt}` — the delegated
        // objective, resolved from the real subagent_type/prompt argument shape.
        assert_eq!(
            summarize_args(
                "task",
                r#"{"subagent_type":"explore","prompt":"where are tool cards rendered?"}"#
            ),
            "explore: where are tool cards rendered?"
        );
        // The compatibility aliases (`subagentType`/`agent`) resolve the same way.
        assert_eq!(
            summarize_args("task", r#"{"agent":"plan","prompt":"design the flow"}"#),
            "plan: design the flow"
        );
        // A resume-only call (no prompt) falls back to the agent name alone.
        assert_eq!(
            summarize_args("task", r#"{"subagent_type":"explore","task_id":"t1"}"#),
            "explore"
        );
    }

    #[test]
    fn lsp_summary_is_diagnostics_of_the_target() {
        // VAL-TOOL-029: an lsp card's summary is `diagnostics {target}`, where the target is the
        // queried file path (any alias) and the workspace fallback for a query-only request.
        assert_eq!(
            summarize_args(
                "lsp",
                r#"{"action":"documentSymbols","filePath":"crates/tui"}"#
            ),
            "diagnostics crates/tui"
        );
        assert_eq!(
            summarize_args("lsp", r#"{"action":"definition","path":"src/main.rs"}"#),
            "diagnostics src/main.rs"
        );
        assert_eq!(
            summarize_args("lsp", r#"{"action":"workspaceSymbols","query":"ToolCard"}"#),
            "diagnostics ToolCard"
        );
        assert_eq!(
            summarize_args("lsp", r#"{"action":"workspaceSymbols"}"#),
            "diagnostics workspace"
        );
    }

    #[test]
    fn question_summary_is_the_first_prompt() {
        // VAL-TOOL-030: a question card's summary is the (first) question's prompt text.
        assert_eq!(
            summarize_args(
                "question",
                r#"{"questions":[{"prompt":"Default provider: Anthropic or OpenAI?","options":["Anthropic","OpenAI"]}]}"#
            ),
            "Default provider: Anthropic or OpenAI?"
        );
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

    // --- VAL-CORTEX-CHAT-002: tool header formatting ---

    #[test]
    fn tool_header_bash_is_dollar_command() {
        assert_eq!(
            format_tool_header("bash", r#"{"command":"cargo test retry -- --nocapture"}"#, None, None),
            "$ cargo test retry -- --nocapture"
        );
    }

    #[test]
    fn tool_header_read_is_read_path() {
        assert_eq!(
            format_tool_header("read", r#"{"path":"crates/agent/src/retry.rs"}"#, None, None),
            "Read crates/agent/src/retry.rs"
        );
    }

    #[test]
    fn tool_header_search_is_quoted_pattern() {
        assert_eq!(
            format_tool_header(
                "grep",
                r#"{"pattern":"rate.?limit","path":"src"}"#,
                None,
                Some(3)
            ),
            "Search \"rate.?limit\" (3 matches)"
        );
        assert_eq!(
            format_tool_header("search", r#"{"pattern":"TODO"}"#, None, None),
            "Search \"TODO\""
        );
    }

    #[test]
    fn tool_header_edit_includes_diff_stat() {
        assert_eq!(
            format_tool_header(
                "edit",
                r#"{"path":"crates/agent/src/retry.rs","oldString":"a","newString":"b\nc"}"#,
                Some((12, 4)),
                None
            ),
            "Edit crates/agent/src/retry.rs +12/-4"
        );
    }

    #[test]
    fn tool_header_write_is_write_path() {
        assert_eq!(
            format_tool_header("write", r#"{"path":"notes.md","content":"hi"}"#, None, None),
            "Write notes.md"
        );
    }

    #[test]
    fn tool_status_line_running_bash_awaits_subshell() {
        let card = ToolCard::new("c", "bash", r#"{"command":"ls"}"#);
        assert_eq!(
            format_tool_status_line(&card),
            "running · waiting on subshell"
        );
    }

    #[test]
    fn match_count_from_grep_style_rows() {
        let out = "src/a.rs:1: TODO\nsrc/b.rs:4: TODO later\n";
        assert_eq!(match_count_from_output(out), Some(2));
        assert_eq!(match_count_from_output(""), None);
    }
}

//! The `edit` tool: replace an exact `oldString` with `newString` in one file, then run the
//! post-edit downstream hooks (formatter + LSP diagnostics).
//!
//! Behavior (architecture §3.4, validation contract VAL-TOOLS-012..018):
//!
//! - A unique `oldString` is replaced by `newString` exactly once; every other byte is untouched.
//! - With `replaceAll=true` every occurrence is replaced and the count is reported back.
//! - Without `replaceAll`, an `oldString` matching more than once is rejected with a clear ambiguity
//!   error and the file is left unchanged.
//! - The target file must have been read earlier in the session: an edit of an unread file is
//!   refused with a "read it first" error before any mutation.
//! - An `oldString` that does not occur (even after the whitespace-tolerant fallback) yields a clear
//!   "string not found" error with no partial write.
//! - When an exact match fails, ordered fuzzy replacers (trimmed, line-normalized, indentation-aware,
//!   and anchored line-window matching with Levenshtein-style similarity) locate the intended region
//!   while still failing closed on ambiguity.
//! - After a successful edit the post-edit hooks run: the formatter pass reformats the on-disk file
//!   when a formatter is available, and any LSP diagnostics are re-injected into the tool result.
//!   With no formatter/LSP attached, an edit succeeds unchanged with no error; when a surface
//!   provides hooks, the seam carries formatter output and diagnostics.

use std::fs;

use futures_util::future::BoxFuture;
use protocol::ToolResultPart;
use serde_json::{Value, json};

use crate::tool::{DiagnosticFeedback, Tool, ToolContext, ToolOutput};

const DESCRIPTION: &str = "Edit a file by replacing an exact occurrence of `oldString` with \
`newString`. By default `oldString` must match exactly once (include enough surrounding context to \
make it unique); set `replaceAll` to true to replace every occurrence. The file must have been \
read earlier in this session so the edit is informed; if you have not read it, read it first. If \
an exact match is not found, a whitespace-tolerant fallback locates the intended lines. After \
editing, the file is formatted (when a formatter is available) and any diagnostics are reported \
back to you.";

/// The `edit` tool.
pub struct EditTool;

impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to edit (absolute, or relative to the working directory)."
                },
                "oldString": {
                    "type": "string",
                    "description": "The exact text to replace. Include enough surrounding context for it to be unique unless replaceAll is set."
                },
                "newString": {
                    "type": "string",
                    "description": "The text to replace oldString with (may be empty to delete the matched text)."
                },
                "replaceAll": {
                    "type": "boolean",
                    "description": "Replace every occurrence of oldString instead of requiring a single unique match (default false)."
                }
            },
            "required": ["path", "oldString", "newString"]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

/// Execute an edit against the resolved arguments (synchronous filesystem work).
fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(path) = string_arg(arguments, "path")
        .or_else(|| string_arg(arguments, "file_path"))
        .or_else(|| string_arg(arguments, "filePath"))
    else {
        return ToolOutput::error("edit: the 'path' argument is required");
    };
    let path = path.trim().to_owned();
    if path.is_empty() {
        return ToolOutput::error("edit: the 'path' argument must not be empty");
    }
    let Some(old_string) =
        string_arg(arguments, "oldString").or_else(|| string_arg(arguments, "old_string"))
    else {
        return ToolOutput::error("edit: the 'oldString' argument is required");
    };
    let Some(new_string) =
        string_arg(arguments, "newString").or_else(|| string_arg(arguments, "new_string"))
    else {
        return ToolOutput::error("edit: the 'newString' argument is required");
    };
    let replace_all = bool_arg(arguments, "replaceAll")
        .or_else(|| bool_arg(arguments, "replace_all"))
        .unwrap_or(false);

    if let Err(message) = validate_strings(&old_string, &new_string) {
        return ToolOutput::error(message);
    }

    // Gate the edit through the permission engine when one is attached (a no-op until enforcement
    // wires a real engine in); a denied edit touches nothing on disk.
    if let Err(message) = ctx.ensure_permitted("edit", &path) {
        return ToolOutput::error(message);
    }
    if let Err(message) = ctx.ensure_not_planning("edit") {
        return ToolOutput::error(message);
    }

    let resolved = ctx.resolve(&path);

    match fs::metadata(&resolved) {
        Ok(metadata) => {
            if metadata.is_dir() {
                return ToolOutput::error(format!(
                    "edit: {path} is a directory; edit targets a single file"
                ));
            }
        }
        Err(_) => {
            return ToolOutput::error(format!("edit: file not found: {path}"));
        }
    }

    // Read-before-edit guard. The file must have been read this session; otherwise the edit is
    // refused before any mutation and the bytes are left intact.
    if !ctx.was_read(&resolved) {
        return ToolOutput::error(format!(
            "edit: {path} has not been read this session; read it first before editing it"
        ));
    }

    // OS-writability pre-check: refuse a read-only target before any mutation so the existing file
    // is left byte-for-byte intact even when a privileged process would bypass the permission bits.
    if let Some(reason) = ctx.writability_error(&resolved) {
        return ToolOutput::error(format!("edit: cannot write {path}: {reason}"));
    }

    let Ok(current) = fs::read_to_string(&resolved) else {
        return ToolOutput::error(format!("edit: could not read {path} as UTF-8 text"));
    };

    let applied = match apply_to_buffer(&current, &old_string, &new_string, replace_all) {
        Ok(applied) => applied,
        Err(EditError::Ambiguous(count)) => {
            return ToolOutput::error(format!(
                "edit: oldString matches {count} places in {path}; add surrounding context to pin a single occurrence, or set replaceAll to change every occurrence (file unchanged)"
            ));
        }
        Err(EditError::NotFound) => {
            let candidates = nearest_candidates(&current, &old_string, 3);
            let mut message = format!(
                "edit: oldString was not found in {path}; re-read the file and copy the snippet exactly (file unchanged)"
            );
            if !candidates.is_empty() {
                message.push_str("\nClosest candidates:");
                for candidate in candidates {
                    message.push_str("\n- ");
                    message.push_str(&candidate);
                }
            }
            return ToolOutput::error(message);
        }
    };

    // Capture the pre-mutation snapshot BEFORE the edited bytes hit the disk, so `/undo` can restore
    // the file's prior content exactly.
    ctx.checkpoint_before(&resolved);

    if let Err(err) = fs::write(&resolved, applied.content.as_bytes()) {
        return ToolOutput::error(format!("edit: could not write {path}: {err}"));
    }
    if let Some(audit) = ctx.audit_sink() {
        audit.record_file_mutation(&resolved, "edit", Some("replace"));
    }

    // The edit succeeded, so the on-disk content is now this session's source of truth: refresh
    // mutation freshness without treating the edit as a scoped read for dynamic memory.
    ctx.record_freshness(&resolved);

    // Post-edit downstream hooks. The formatter rewrites the file in place first, then diagnostics
    // are collected against the final on-disk content. Both are no-ops when no surface attached
    // those capabilities.
    let format_note = ctx.format_after_write(&resolved);
    let mut feedback = ctx.diagnostic_feedback_after_write(&resolved);
    if let Some(note) = format_note {
        feedback.push_note(note);
    }

    let occurrences = if applied.replacements == 1 {
        "1 occurrence".to_owned()
    } else {
        format!("{} occurrences", applied.replacements)
    };
    let summary = if let Some(detail) = &applied.match_detail {
        format!("edit: replaced {occurrences} in {path} ({detail})")
    } else {
        format!("edit: replaced {occurrences} in {path}")
    };

    render_with_feedback(summary, feedback)
}

fn render_with_feedback(summary: String, feedback: DiagnosticFeedback) -> ToolOutput {
    if feedback.diagnostics.is_empty() && feedback.note.is_none() {
        return ToolOutput::text(summary);
    }

    let mut parts = vec![ToolResultPart::text(summary)];
    if !feedback.diagnostics.is_empty() {
        let mut rendered = String::from("Diagnostics:");
        for diagnostic in &feedback.diagnostics {
            rendered.push('\n');
            rendered.push_str(&diagnostic.render());
        }
        parts.push(ToolResultPart::text(rendered));
    }
    if let Some(note) = feedback.note {
        parts.push(ToolResultPart::text(note));
    }
    ToolOutput::parts(parts)
}

/// The reason an edit could not be applied to the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditError {
    /// `oldString` matched `count` (>=2) places without `replaceAll`.
    Ambiguous(usize),
    /// `oldString` matched nowhere, even after the whitespace-tolerant fallback.
    NotFound,
}

/// The result of applying an edit to an in-memory buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Applied {
    /// The full new buffer content.
    content: String,
    /// How many occurrences were replaced.
    replacements: usize,
    /// Optional detail about the matcher that was used.
    match_detail: Option<String>,
}

/// Argument preconditions checked before any search.
fn validate_strings(old: &str, new: &str) -> Result<(), String> {
    if old.is_empty() {
        return Err("edit: oldString is empty; provide the exact text to replace".to_owned());
    }
    if old == new {
        return Err("edit: oldString and newString are identical; nothing would change".to_owned());
    }
    Ok(())
}

/// Apply the edit to `buffer` in deterministic tiers: byte-exact substring first, then ordered
/// fuzzy replacers (trimmed boundary, line-normalized, indentation-flexible, anchored line-window).
fn apply_to_buffer(
    buffer: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<Applied, EditError> {
    // Tier 1 — byte-for-byte substring search.
    let exact_spans: Vec<(usize, usize)> = buffer
        .match_indices(old)
        .map(|(start, _)| (start, start + old.len()))
        .collect();
    if !exact_spans.is_empty() {
        if exact_spans.len() >= 2 && !replace_all {
            return Err(EditError::Ambiguous(exact_spans.len()));
        }
        let mut content = buffer.to_owned();
        for (start, end) in exact_spans.iter().rev() {
            content.replace_range(*start..*end, new);
        }
        return Ok(Applied {
            content,
            replacements: exact_spans.len(),
            match_detail: None,
        });
    }

    // Tier 2+ — ordered fuzzy replacers.
    let trimmed = old.trim();
    if trimmed != old {
        let trimmed_spans: Vec<(usize, usize)> = buffer
            .match_indices(trimmed)
            .map(|(start, _)| (start, start + trimmed.len()))
            .collect();
        if !trimmed_spans.is_empty() {
            if trimmed_spans.len() >= 2 && !replace_all {
                return Err(EditError::Ambiguous(trimmed_spans.len()));
            }
            let mut content = buffer.to_owned();
            for (start, end) in trimmed_spans.iter().rev() {
                content.replace_range(*start..*end, new);
            }
            let detail = if trimmed_spans.len() == 1 {
                "trimmed-boundary fuzzy match".to_owned()
            } else {
                format!(
                    "trimmed-boundary fuzzy match across {} regions",
                    trimmed_spans.len()
                )
            };
            return Ok(Applied {
                content,
                replacements: trimmed_spans.len(),
                match_detail: Some(detail),
            });
        }
    }

    let lines = indexed_lines(buffer);
    let needle = needle_lines(old);
    if needle.is_empty() {
        return Err(EditError::NotFound);
    }

    let strategy_candidates = [
        (
            "line-normalized fuzzy match",
            line_normalized_matches(&lines, &needle),
        ),
        (
            "indentation-flexible fuzzy match",
            indentation_flexible_matches(&lines, &needle),
        ),
        (
            "anchored line-window fuzzy match",
            anchored_window_matches(&lines, &needle),
        ),
    ];

    for (strategy, runs) in strategy_candidates {
        if runs.is_empty() {
            continue;
        }
        if runs.len() >= 2 && !replace_all {
            return Err(EditError::Ambiguous(runs.len()));
        }

        let mut content = buffer.to_owned();
        for matched in runs.iter().rev() {
            let span_start = lines[matched.first_line].start;
            let span_end = lines[matched.last_line].end;
            let mut replacement = new.to_owned();
            // The matched span includes the last line's trailing newline; keep the line structure
            // intact when the replacement text omits it.
            if lines[matched.last_line].has_newline && !replacement.ends_with('\n') {
                replacement.push('\n');
            }
            content.replace_range(span_start..span_end, &replacement);
        }

        let match_detail = if runs.len() == 1 {
            let matched = &runs[0];
            if matched.first_line == matched.last_line {
                format!("{strategy} at line {}", matched.first_line + 1)
            } else {
                format!(
                    "{strategy} at lines {}-{}",
                    matched.first_line + 1,
                    matched.last_line + 1
                )
            }
        } else {
            format!("{strategy} across {} regions", runs.len())
        };

        return Ok(Applied {
            content,
            replacements: runs.len(),
            match_detail: Some(match_detail),
        });
    }

    Err(EditError::NotFound)
}

/// A single line of the buffer with its byte span and whether it carried a trailing newline.
struct IndexedLine<'a> {
    start: usize,
    end: usize,
    text: &'a str,
    has_newline: bool,
}

/// Split `buffer` into indexed lines (byte spans inclusive of the trailing newline).
fn indexed_lines(buffer: &str) -> Vec<IndexedLine<'_>> {
    let mut lines = Vec::new();
    let mut offset = 0;
    for segment in buffer.split_inclusive('\n') {
        let has_newline = segment.ends_with('\n');
        let text = segment.strip_suffix('\n').unwrap_or(segment);
        lines.push(IndexedLine {
            start: offset,
            end: offset + segment.len(),
            text,
            has_newline,
        });
        offset += segment.len();
    }
    lines
}

/// Split `old` into the lines it should match, dropping the trailing empty segment from a final
/// newline so a trailing newline in `oldString` does not require an extra blank line.
fn needle_lines(old: &str) -> Vec<&str> {
    let mut lines = old.split('\n').collect::<Vec<_>>();
    if old.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// Collapse internal/leading/trailing whitespace runs into single spaces so spacing/indentation
/// drift does not defeat the line comparison.
fn normalize_ws(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineMatch {
    first_line: usize,
    last_line: usize,
}

/// Find non-overlapping runs of `lines` whose whitespace-normalized text equals the needle's.
fn line_normalized_matches(lines: &[IndexedLine<'_>], needle: &[&str]) -> Vec<LineMatch> {
    if needle.is_empty() || lines.len() < needle.len() {
        return Vec::new();
    }
    let normalized_needle: Vec<String> = needle.iter().map(|line| normalize_ws(line)).collect();
    let mut matches = Vec::new();
    let mut index = 0;
    while index + needle.len() <= lines.len() {
        let is_match = normalized_needle
            .iter()
            .enumerate()
            .all(|(offset, expected)| normalize_ws(lines[index + offset].text) == *expected);
        if is_match {
            matches.push(LineMatch {
                first_line: index,
                last_line: index + needle.len() - 1,
            });
            index += needle.len();
        } else {
            index += 1;
        }
    }
    matches
}

fn strip_common_indentation(lines: &[&str]) -> Vec<String> {
    let mut min_indent = usize::MAX;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line
            .chars()
            .take_while(|ch| ch.is_whitespace() && *ch != '\n')
            .count();
        min_indent = min_indent.min(indent);
    }
    if min_indent == usize::MAX {
        return lines.iter().map(|line| (*line).to_owned()).collect();
    }
    lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                (*line).to_owned()
            } else {
                line.chars().skip(min_indent).collect()
            }
        })
        .collect()
}

fn indentation_flexible_matches(lines: &[IndexedLine<'_>], needle: &[&str]) -> Vec<LineMatch> {
    if needle.is_empty() || lines.len() < needle.len() {
        return Vec::new();
    }
    let normalized_needle = strip_common_indentation(needle);
    let mut matches = Vec::new();
    for start in 0..=lines.len() - needle.len() {
        let block: Vec<&str> = lines[start..start + needle.len()]
            .iter()
            .map(|line| line.text)
            .collect();
        let normalized_block = strip_common_indentation(&block);
        if normalized_block == normalized_needle {
            matches.push(LineMatch {
                first_line: start,
                last_line: start + needle.len() - 1,
            });
        }
    }
    matches
}

const ANCHORED_SIMILARITY_THRESHOLD: f64 = 0.65;
const ANCHORED_SIMILARITY_MARGIN: f64 = 0.03;

fn anchored_window_matches(lines: &[IndexedLine<'_>], needle: &[&str]) -> Vec<LineMatch> {
    if needle.len() < 3 || lines.len() < needle.len() {
        return Vec::new();
    }

    let first = needle[0].trim();
    let last = needle[needle.len() - 1].trim();
    let search_block_size = needle.len() as isize;
    let max_line_delta = (search_block_size / 4).max(1);

    let mut candidates: Vec<(LineMatch, f64)> = Vec::new();
    for start in 0..lines.len() {
        if lines[start].text.trim() != first {
            continue;
        }
        for end in (start + 2)..lines.len() {
            if lines[end].text.trim() != last {
                continue;
            }
            let block_size = (end - start + 1) as isize;
            if (block_size - search_block_size).abs() > max_line_delta {
                continue;
            }

            let lines_to_compare = std::cmp::min(needle.len().saturating_sub(2), end - start - 1);
            let similarity = if lines_to_compare == 0 {
                1.0
            } else {
                let mut total = 0.0;
                for offset in 0..lines_to_compare {
                    let left = normalize_ws(lines[start + 1 + offset].text);
                    let right = normalize_ws(needle[1 + offset]);
                    total += normalized_similarity(&left, &right);
                }
                total / lines_to_compare as f64
            };
            if similarity >= ANCHORED_SIMILARITY_THRESHOLD {
                candidates.push((
                    LineMatch {
                        first_line: start,
                        last_line: end,
                    },
                    similarity,
                ));
            }
        }
    }

    if candidates.is_empty() {
        return Vec::new();
    }

    candidates.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top_similarity = candidates[0].1;
    let second_similarity = candidates.get(1).map(|candidate| candidate.1);
    if second_similarity
        .map(|score| top_similarity - score >= ANCHORED_SIMILARITY_MARGIN)
        .unwrap_or(true)
    {
        return vec![candidates[0].0];
    }

    let mut ambiguous = Vec::new();
    for (matched, score) in candidates {
        if top_similarity - score <= ANCHORED_SIMILARITY_MARGIN {
            ambiguous.push(matched);
        }
    }
    ambiguous
}

fn normalized_similarity(left: &str, right: &str) -> f64 {
    if left == right {
        return 1.0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let max_len = left_chars.len().max(right_chars.len());
    if max_len == 0 {
        return 1.0;
    }
    let distance = levenshtein_distance(&left_chars, &right_chars);
    (1.0 - (distance as f64 / max_len as f64)).clamp(0.0, 1.0)
}

fn levenshtein_distance(left: &[char], right: &[char]) -> usize {
    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];

    for (i, &left_char) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, &right_char) in right.iter().enumerate() {
            let cost = usize::from(left_char != right_char);
            current[j + 1] = std::cmp::min(
                std::cmp::min(current[j] + 1, previous[j + 1] + 1),
                previous[j] + cost,
            );
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn nearest_candidates(buffer: &str, old: &str, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let lines = indexed_lines(buffer);
    if lines.is_empty() {
        return Vec::new();
    }
    let needle_lines = needle_lines(old);
    if needle_lines.is_empty() {
        return Vec::new();
    }
    let needle_norm = normalize_ws(old);

    let mut scored: Vec<(f64, usize, usize, String)> = Vec::new();
    if needle_lines.len() == 1 {
        for (index, line) in lines.iter().enumerate() {
            let similarity = normalized_similarity(&normalize_ws(line.text), &needle_norm);
            if similarity < 0.45 {
                continue;
            }
            scored.push((similarity, index, index, line.text.to_owned()));
        }
    } else {
        let line_count = needle_lines.len();
        let min_window = line_count.saturating_sub(1).max(1);
        let max_window = std::cmp::min(line_count + 1, lines.len());
        for window in min_window..=max_window {
            if window > lines.len() {
                continue;
            }
            for start in 0..=lines.len() - window {
                let end = start + window - 1;
                let span = lines[start].start..lines[end].end;
                let block = buffer[span].trim_end_matches('\n');
                let similarity = normalized_similarity(&normalize_ws(block), &needle_norm);
                if similarity < 0.45 {
                    continue;
                }
                scored.push((similarity, start, end, block.to_owned()));
            }
        }
    }

    scored.sort_by(|left, right| right.0.total_cmp(&left.0));
    scored.truncate(limit);
    scored
        .into_iter()
        .map(|(score, start, end, snippet)| {
            let label = if start == end {
                format!("line {}", start + 1)
            } else {
                format!("lines {}-{}", start + 1, end + 1)
            };
            format!(
                "{label} (similarity {:.2}): {}",
                score,
                snippet_preview(&snippet)
            )
        })
        .collect()
}

fn snippet_preview(snippet: &str) -> String {
    const MAX_CHARS: usize = 120;
    let single_line = snippet
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ⏎ ");
    if single_line.chars().count() <= MAX_CHARS {
        return single_line;
    }
    let mut truncated = String::new();
    for (index, ch) in single_line.chars().enumerate() {
        if index >= MAX_CHARS {
            truncated.push('…');
            break;
        }
        truncated.push(ch);
    }
    truncated
}

/// Extract a string argument verbatim (no trimming: edit strings must be preserved exactly).
fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Extract a boolean argument, tolerating a string spelling (`"true"`/`"false"`).
fn bool_arg(arguments: &Value, key: &str) -> Option<bool> {
    let value = arguments.get(key)?;
    value
        .as_bool()
        .or_else(|| value.as_str().and_then(|s| s.parse::<bool>().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::tool::{Diagnostic, MutationHooks};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("cortex-edit-{tag}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn ctx_for(dir: &Path) -> ToolContext {
        ToolContext::new(dir.to_path_buf())
    }

    /// Seed a file and mark it read this session, the normal pre-edit state.
    fn seed_read(ctx: &ToolContext, dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        ctx.record_read(&path);
        path
    }

    fn edit_args(path: &str, old: &str, new: &str) -> Value {
        json!({ "path": path, "oldString": old, "newString": new })
    }

    fn text_of(output: &ToolOutput) -> String {
        output.model_content()
    }

    #[test]
    fn exact_unique_replacement_touches_only_that_substring() {
        let dir = temp_dir("exact");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "a.txt", "alpha beta gamma");
        let output = run(&edit_args("a.txt", "beta", "DELTA"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert_eq!(
            fs::read_to_string(dir.join("a.txt")).unwrap(),
            "alpha DELTA gamma"
        );
    }

    #[test]
    fn replace_all_changes_every_occurrence_and_reports_the_count() {
        let dir = temp_dir("all");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "x.txt", "x x x");
        let output = run(
            &json!({ "path": "x.txt", "oldString": "x", "newString": "y", "replaceAll": true }),
            &ctx,
        );
        assert!(!output.is_error, "{}", text_of(&output));
        assert_eq!(fs::read_to_string(dir.join("x.txt")).unwrap(), "y y y");
        assert!(
            text_of(&output).contains('3'),
            "the replacement count is reported: {}",
            text_of(&output)
        );
    }

    #[test]
    fn ambiguous_multi_match_without_replace_all_errors_and_leaves_file_unchanged() {
        let dir = temp_dir("ambig");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "d.txt", "dup dup");
        let output = run(&edit_args("d.txt", "dup", "X"), &ctx);
        assert!(output.is_error);
        let text = text_of(&output).to_lowercase();
        assert!(
            text.contains("matches") && text.contains('2'),
            "got: {text}"
        );
        assert_eq!(fs::read_to_string(dir.join("d.txt")).unwrap(), "dup dup");
    }

    #[test]
    fn edit_of_an_unread_file_is_refused_and_bytes_are_unchanged() {
        let dir = temp_dir("unread");
        // Seed the file WITHOUT recording a read.
        fs::write(dir.join("untouched.txt"), "ORIGINAL").unwrap();
        let output = run(
            &edit_args("untouched.txt", "ORIGINAL", "CHANGED"),
            &ctx_for(&dir),
        );
        assert!(output.is_error);
        assert!(
            text_of(&output).to_lowercase().contains("read it first"),
            "got: {}",
            text_of(&output)
        );
        assert_eq!(
            fs::read_to_string(dir.join("untouched.txt")).unwrap(),
            "ORIGINAL"
        );
    }

    #[test]
    fn not_found_old_string_errors_and_leaves_file_unchanged() {
        let dir = temp_dir("notfound");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "n.txt", "hello world");
        let output = run(&edit_args("n.txt", "nonexistent-token", "X"), &ctx);
        assert!(output.is_error);
        assert!(
            text_of(&output).to_lowercase().contains("not found"),
            "got: {}",
            text_of(&output)
        );
        assert_eq!(
            fs::read_to_string(dir.join("n.txt")).unwrap(),
            "hello world"
        );
    }

    #[test]
    fn fuzzy_fallback_tolerates_whitespace_drift_and_leaves_surrounding_lines() {
        let dir = temp_dir("fuzzy");
        let ctx = ctx_for(&dir);
        let original = "fn main() {\n    let  x =  1;\n    println!();\n}\n";
        seed_read(&ctx, &dir, "src.rs", original);
        // oldString uses canonical single spacing and no indentation; the on-disk line is drifted.
        let output = run(&edit_args("src.rs", "let x = 1;", "let y = 2;"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        let after = fs::read_to_string(dir.join("src.rs")).unwrap();
        // The matched line (including its drifted indentation) is replaced by newString verbatim;
        // the surrounding lines are untouched. Canonicalizing indentation is the formatter's job.
        assert_eq!(after, "fn main() {\nlet y = 2;\n    println!();\n}\n");
    }

    #[test]
    fn fuzzy_fallback_is_not_used_when_an_exact_match_exists() {
        let dir = temp_dir("prefer-exact");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "p.txt", "let x = 1;\n");
        let output = run(&edit_args("p.txt", "let x = 1;", "let z = 9;"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert_eq!(
            fs::read_to_string(dir.join("p.txt")).unwrap(),
            "let z = 9;\n"
        );
    }

    #[test]
    fn empty_old_string_errors() {
        let dir = temp_dir("emptyold");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "e.txt", "content");
        let output = run(&edit_args("e.txt", "", "x"), &ctx);
        assert!(output.is_error);
        assert!(text_of(&output).contains("empty"));
        assert_eq!(fs::read_to_string(dir.join("e.txt")).unwrap(), "content");
    }

    #[test]
    fn identical_old_and_new_errors() {
        let dir = temp_dir("identical");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "i.txt", "same");
        let output = run(&edit_args("i.txt", "same", "same"), &ctx);
        assert!(output.is_error);
        assert!(text_of(&output).to_lowercase().contains("identical"));
    }

    #[test]
    fn editing_a_missing_file_errors() {
        let dir = temp_dir("missing");
        let output = run(&edit_args("nope.txt", "a", "b"), &ctx_for(&dir));
        assert!(output.is_error);
        assert!(text_of(&output).to_lowercase().contains("not found"));
    }

    #[test]
    fn editing_a_directory_errors() {
        let dir = temp_dir("isdir");
        let ctx = ctx_for(&dir);
        fs::create_dir(dir.join("sub")).unwrap();
        ctx.record_read(dir.join("sub"));
        let output = run(&edit_args("sub", "a", "b"), &ctx);
        assert!(output.is_error);
        assert!(text_of(&output).contains("directory"));
    }

    #[test]
    fn missing_new_string_argument_errors() {
        let dir = temp_dir("nonew");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "m.txt", "content");
        let output = run(&json!({ "path": "m.txt", "oldString": "content" }), &ctx);
        assert!(output.is_error);
        assert!(text_of(&output).contains("newString"));
    }

    #[test]
    fn empty_new_string_deletes_the_matched_text() {
        let dir = temp_dir("delete");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "del.txt", "keep REMOVE keep");
        let output = run(&edit_args("del.txt", " REMOVE", ""), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert_eq!(
            fs::read_to_string(dir.join("del.txt")).unwrap(),
            "keep keep"
        );
    }

    #[test]
    fn permission_deny_blocks_the_edit_and_leaves_bytes_intact() {
        use permissions::{Action, PermissionEngine, Rule, Ruleset};

        let dir = temp_dir("perm");
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "edit",
            "guard.txt",
            Action::Deny,
        )]));
        let ctx = ToolContext::new(dir.clone()).with_permissions(Arc::new(engine));
        seed_read(&ctx, &dir, "guard.txt", "KEEP");
        let output = run(&edit_args("guard.txt", "KEEP", "CHANGED"), &ctx);
        assert!(output.is_error);
        assert!(text_of(&output).contains("permission denied"));
        assert_eq!(fs::read_to_string(dir.join("guard.txt")).unwrap(), "KEEP");
    }

    /// A fake formatter hook that uppercases the just-edited file so the test can assert the on-disk
    /// content reflects formatter output, not the raw edited bytes.
    struct UppercasingFormatter;
    impl MutationHooks for UppercasingFormatter {
        fn format(&self, path: &Path) {
            if let Ok(content) = fs::read_to_string(path) {
                let _ = fs::write(path, content.to_uppercase());
            }
        }
    }

    #[test]
    fn formatter_hook_reformats_the_on_disk_file_after_edit() {
        let dir = temp_dir("fmt");
        let ctx = ctx_for(&dir).with_hooks(Arc::new(UppercasingFormatter));
        seed_read(&ctx, &dir, "m.rs", "fn main() { let a=1; }");
        let output = run(&edit_args("m.rs", "let a=1;", "let b=2;"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert_eq!(
            fs::read_to_string(dir.join("m.rs")).unwrap(),
            "FN MAIN() { LET B=2; }"
        );
    }

    #[test]
    fn no_formatter_edits_verbatim_with_no_error() {
        let dir = temp_dir("verbatim");
        let ctx = ctx_for(&dir);
        seed_read(&ctx, &dir, "file.unknownext", "fn  main( ){let x=1;}");
        let output = run(&edit_args("file.unknownext", "let x=1;", "let y=2;"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert_eq!(
            fs::read_to_string(dir.join("file.unknownext")).unwrap(),
            "fn  main( ){let y=2;}"
        );
    }

    /// A fake LSP hook that reports a diagnostic for a file containing the marker `BAD`.
    struct MarkerLsp;
    impl MutationHooks for MarkerLsp {
        fn diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
            let content = fs::read_to_string(path).unwrap_or_default();
            if let Some(index) = content.lines().position(|line| line.contains("BAD")) {
                vec![Diagnostic::new("error", index + 1, "mismatched types")]
            } else {
                Vec::new()
            }
        }
    }

    #[test]
    fn lsp_hook_reinjects_diagnostics_into_the_tool_result_after_edit() {
        let dir = temp_dir("lsp");
        let ctx = ctx_for(&dir).with_hooks(Arc::new(MarkerLsp));
        seed_read(&ctx, &dir, "t.rs", "ok\nGOOD line\n");
        let output = run(&edit_args("t.rs", "GOOD", "BAD"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        let text = text_of(&output);
        assert!(text.contains("mismatched types"), "got: {text}");
        assert!(
            text.contains("line 2"),
            "diagnostic carries the location: {text}"
        );
    }

    #[test]
    fn clean_edit_produces_no_spurious_diagnostics() {
        let dir = temp_dir("clean");
        let ctx = ctx_for(&dir).with_hooks(Arc::new(MarkerLsp));
        seed_read(&ctx, &dir, "ok.rs", "fn main() {}\n");
        let output = run(&edit_args("ok.rs", "main", "run"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert!(!text_of(&output).to_lowercase().contains("diagnostic"));
    }

    #[test]
    fn edit_of_a_read_only_file_errors_and_leaves_bytes_intact() {
        let dir = temp_dir("ro");
        let ctx = ctx_for(&dir);
        let path = seed_read(&ctx, &dir, "ro.txt", "ORIGINAL");
        set_readonly(&path, true);
        let output = run(&edit_args("ro.txt", "ORIGINAL", "CHANGED"), &ctx);
        assert!(output.is_error, "{}", text_of(&output));
        assert!(
            text_of(&output).to_lowercase().contains("permission"),
            "got: {}",
            text_of(&output)
        );
        // Restore perms to read the bytes back; the original content is intact (no truncation).
        set_readonly(&path, false);
        assert_eq!(fs::read_to_string(&path).unwrap(), "ORIGINAL");
    }

    /// Toggle the read-only bit on `path` so tests can exercise the OS-unwritable guard.
    fn set_readonly(path: &Path, readonly: bool) {
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_readonly(readonly);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn fuzzy_multi_match_without_replace_all_is_ambiguous() {
        let dir = temp_dir("fuzzy-ambig");
        let ctx = ctx_for(&dir);
        // Two lines that both normalize to "let x = 1;" but neither is a byte-exact match for the
        // canonical needle, so the fuzzy tier sees two candidates.
        let original = "    let  x =  1;\n        let x  =  1;\n";
        seed_read(&ctx, &dir, "f.rs", original);
        let output = run(&edit_args("f.rs", "let x = 1;", "let y = 2;"), &ctx);
        assert!(output.is_error);
        assert!(text_of(&output).to_lowercase().contains("matches"));
        assert_eq!(fs::read_to_string(dir.join("f.rs")).unwrap(), original);
    }

    #[test]
    fn anchored_line_window_uses_levenshtein_similarity_to_pick_the_best_block() {
        let dir = temp_dir("anchored-best");
        let ctx = ctx_for(&dir);
        let original = concat!(
            "fn choose() {\n",
            "    let count = 10;\n",
            "    println!(\"done\");\n",
            "}\n",
            "\n",
            "fn choose() {\n",
            "    let total = 99;\n",
            "    println!(\"done\");\n",
            "}\n"
        );
        seed_read(&ctx, &dir, "window.rs", original);
        let old = "fn choose() {\n  let total = 100;\n  println!(\"done\");\n}";
        let new = "fn choose() {\n    let total = 0;\n    println!(\"done\");\n}";
        let output = run(&edit_args("window.rs", old, new), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert!(
            text_of(&output).contains("anchored line-window fuzzy match"),
            "expected fuzzy strategy note, got: {}",
            text_of(&output)
        );
        let after = fs::read_to_string(dir.join("window.rs")).unwrap();
        assert_eq!(
            after,
            concat!(
                "fn choose() {\n",
                "    let count = 10;\n",
                "    println!(\"done\");\n",
                "}\n",
                "\n",
                "fn choose() {\n",
                "    let total = 0;\n",
                "    println!(\"done\");\n",
                "}\n"
            )
        );
    }

    #[test]
    fn not_found_error_includes_nearby_candidate_suggestions() {
        let dir = temp_dir("notfound-suggest");
        let ctx = ctx_for(&dir);
        seed_read(
            &ctx,
            &dir,
            "s.txt",
            "alpha beta gamma\ndelta epsilon zeta\neta theta iota\n",
        );
        let output = run(
            &edit_args("s.txt", "delta epsilon omega", "replacement"),
            &ctx,
        );
        assert!(output.is_error);
        let text = text_of(&output);
        assert!(
            text.contains("Closest candidates"),
            "expected nearby suggestions, got: {text}"
        );
        assert!(
            text.contains("line"),
            "expected line-oriented candidate labels, got: {text}"
        );
    }
}

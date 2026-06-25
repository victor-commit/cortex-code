//! The `read` tool: line-numbered file content, directory listings, and media attachments.
//!
//! Behavior (architecture §3.4, validation contract VAL-TOOLS-001..007):
//!
//! - A text file is returned with a stable, parseable 1-based line-number gutter (`N: <line>`),
//!   blank lines included and the original bytes preserved verbatim after the `: ` separator.
//! - `offset`/`limit` window the result to lines `[offset, offset+limit)`, still numbered with their
//!   true file line numbers.
//! - An unbounded read is capped at [`DEFAULT_READ_LIMIT`] lines with a machine-detectable
//!   truncation marker naming the next offset, so the model knows to page rather than getting a
//!   silent partial dump. When that window omits content (or the rendered view exceeds the byte
//!   budget) the *complete* file content is additionally spilled to an on-disk file named in the
//!   result, so nothing is silently lost (VAL-TOOLS-037).
//! - A directory path yields a sorted listing with sub-directories marked by a trailing `/`.
//! - An image or PDF is returned as a typed attachment part (media type + path), never as raw bytes
//!   in the text channel.
//! - A missing path returns an explicit not-found error naming the path; an empty file or an offset
//!   past EOF returns empty, non-fabricated output with a clear indication and no error.

use std::fs;
use std::io::Read;
use std::path::Path;

use futures_util::future::BoxFuture;
use protocol::ToolResultPart;
use serde_json::{Value, json};

use crate::output::{OutputLimits, spill_to_disk};
use crate::tool::{Tool, ToolContext, ToolOutput};

/// The default line cap applied to an unbounded read (architecture §3.4 / VAL-TOOLS-003).
pub const DEFAULT_READ_LIMIT: usize = 2000;
/// Compatibility cap for each rendered line's visible content.
const DEFAULT_READ_MAX_LINE_CHARS: usize = 2000;
/// Compatibility byte cap for the in-band rendered view before spill.
const DEFAULT_READ_MAX_BYTES: usize = 50 * 1024;
/// Visible marker appended when a rendered line is truncated to the compatibility cap.
const PER_LINE_TRUNCATION_MARKER: &str = "… [truncated]";

/// Bytes sampled from the head of a file to sniff its media type / binary-ness.
const SNIFF_BYTES: usize = 4096;

/// The invalid-UTF-8 byte ratio above which a non-image/PDF file is classified binary.
const BINARY_INVALID_RATIO: f64 = 0.30;

const DESCRIPTION: &str = "Read a file from the filesystem. Returns the file's content with each \
line prefixed by its 1-based line number and a `: ` separator (for example `1: first line`). Use \
`offset` (1-based) and `limit` to read a window of a large file; an unbounded read is capped and \
marks where it was truncated so you can page with `offset`. Pointing at a directory lists its \
entries (sub-directories are marked with a trailing `/`). Images and PDFs are returned as \
attachments rather than text.";

/// The `read` tool.
pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
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
                    "description": "The file or directory path to read (absolute, or relative to the working directory)."
                },
                "filePath": {
                    "type": "string",
                    "description": "Compatibility alias for `path`."
                },
                "file_path": {
                    "type": "string",
                    "description": "Compatibility alias for `path`."
                },
                "offset": {
                    "type": "integer",
                    "description": "1-based line number to start reading from (defaults to 1)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (defaults to 2000)."
                }
            },
            "anyOf": [
                { "required": ["path"] },
                { "required": ["filePath"] },
                { "required": ["file_path"] }
            ]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

/// Execute a read against the resolved arguments (synchronous filesystem work).
fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(path) = string_arg(arguments, "path")
        .or_else(|| string_arg(arguments, "file_path"))
        .or_else(|| string_arg(arguments, "filePath"))
    else {
        return ToolOutput::error("read: the 'path' argument is required");
    };
    let path = path.trim();
    if path.is_empty() {
        return ToolOutput::error("read: the 'path' argument must not be empty");
    }
    let offset = uint_arg(arguments, "offset");
    let limit = uint_arg(arguments, "limit");

    // Gate the read through the permission engine when one is attached (a no-op until enforcement
    // wires a real engine in); a denied read returns protected nothing.
    if let Err(message) = ctx.ensure_permitted("read", path) {
        return ToolOutput::error(message);
    }

    let resolved = ctx.resolve(path);
    let Ok(metadata) = fs::metadata(&resolved) else {
        return ToolOutput::error(format!("read: file not found: {path}"));
    };

    if metadata.is_dir() {
        return read_directory(&resolved, path);
    }

    let prefix = read_prefix(&resolved, SNIFF_BYTES);

    if let Some(media_type) = image_media_type(&resolved, &prefix) {
        ctx.record_read(&resolved);
        return attachment(
            ToolResultPart::image(media_type, resolved.to_string_lossy().into_owned()),
            format!("read: {path} returned as an image attachment ({media_type})"),
        );
    }
    if let Some(media_type) = pdf_media_type(&resolved, &prefix) {
        ctx.record_read(&resolved);
        return attachment(
            ToolResultPart::document(media_type, resolved.to_string_lossy().into_owned()),
            format!("read: {path} returned as a document attachment ({media_type})"),
        );
    }

    let Ok(bytes) = fs::read(&resolved) else {
        return ToolOutput::error(format!("read: file not found: {path}"));
    };
    if sniffs_binary(&bytes) {
        return ToolOutput::error(format!(
            "read: {path} appears to be a binary file and cannot be displayed as text"
        ));
    }

    let content = String::from_utf8_lossy(&bytes);
    let output = bounded_read(&content, offset, limit);
    ctx.record_read(&resolved);
    ToolOutput::text(output)
}

/// Render the windowed file view, then bring it to parity with `bash`/`grep` for large output: when
/// the window omits content (offset/limit windowing truncated the file) or the rendered view exceeds
/// the configured byte budget, spill the *complete* file content to disk via the shared spill
/// mechanism and append a pointer naming it, so nothing the in-band result omits is silently lost
/// (VAL-TOOLS-037). Within limits the view is returned verbatim with no notice and no spill.
fn bounded_read(content: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    let (mut output, truncated) = render_text(content, offset, limit);
    let limits = OutputLimits {
        max_lines: usize::MAX,
        max_bytes: DEFAULT_READ_MAX_BYTES,
    };

    let over_bytes = output.len() > limits.max_bytes;
    if over_bytes {
        let mut end = limits.max_bytes;
        while end > 0 && !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
    }

    if !truncated && !over_bytes {
        return output;
    }

    match spill_to_disk(content, "read") {
        Some(path) => push_notice(
            &mut output,
            &format!(
                "[complete file content ({} lines) written to {}]",
                split_lines(content).len(),
                path.display()
            ),
        ),
        None => push_notice(
            &mut output,
            "[output truncated; the complete file content could not be written to a spill file]",
        ),
    }
    output
}

/// Append `notice` to `output` as its own line (the spill pointer follows the windowing marker).
fn push_notice(output: &mut String, notice: &str) {
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(notice);
}

/// Render a text file's windowed, line-numbered body plus any truncation/EOF indicator, returning
/// the rendered text and whether any content was omitted (window pagination or per-line truncation).
fn render_text(content: &str, offset: Option<usize>, limit: Option<usize>) -> (String, bool) {
    let lines = split_lines(content);
    let total = lines.len();

    if total == 0 {
        return ("(no content)".to_owned(), false);
    }

    let offset = offset.unwrap_or(1).max(1);
    let limit = limit.unwrap_or(DEFAULT_READ_LIMIT).max(1);

    if offset > total {
        return (
            format!(
                "(no content: offset {offset} is past the end of the file, which has {total} lines)"
            ),
            false,
        );
    }

    let mut rendered: Vec<String> = Vec::new();
    let mut line_content_omitted = false;
    for (index, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
        let (line, truncated_line) = truncate_line_for_display(line);
        line_content_omitted |= truncated_line;
        rendered.push(format!("{}: {}", index + 1, line));
    }

    let last = offset - 1 + rendered.len();
    let window_truncated = last < total;
    let mut output = rendered.join("\n");
    if window_truncated {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!(
            "[truncated: showing lines {offset}-{last} of {total}; use offset={} to continue]",
            last + 1
        ));
    }
    (output, window_truncated || line_content_omitted)
}

/// Truncate a rendered line at the compatibility cap on a UTF-8 char boundary and append a marker.
fn truncate_line_for_display(line: &str) -> (String, bool) {
    let Some((cut, _)) = line.char_indices().nth(DEFAULT_READ_MAX_LINE_CHARS) else {
        return (line.to_owned(), false);
    };

    let mut truncated = line[..cut].to_owned();
    truncated.push_str(PER_LINE_TRUNCATION_MARKER);
    (truncated, true)
}

/// Split file content into lines, dropping the single trailing empty element produced by a final
/// newline so a 3-line file (with or without a trailing newline) reports exactly 3 lines.
fn split_lines(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = content.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// Render a directory listing: sorted entry names with sub-directories marked by a trailing `/`.
fn read_directory(resolved: &Path, display: &str) -> ToolOutput {
    let Ok(entries) = fs::read_dir(resolved) else {
        return ToolOutput::error(format!("read: cannot list directory: {display}"));
    };

    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry
            .file_type()
            .map(|file_type| {
                file_type.is_dir()
                    || (file_type.is_symlink()
                        && fs::metadata(entry.path())
                            .map(|meta| meta.is_dir())
                            .unwrap_or(false))
            })
            .unwrap_or(false);
        names.push(if is_dir { format!("{name}/") } else { name });
    }
    names.sort();

    let header = format!("Directory listing for {display} ({} entries):", names.len());
    let body = if names.is_empty() {
        header
    } else {
        format!("{header}\n{}", names.join("\n"))
    };
    ToolOutput::text(body)
}

/// Build an attachment result: a describing text part plus the typed media part.
fn attachment(part: ToolResultPart, message: String) -> ToolOutput {
    ToolOutput::parts(vec![ToolResultPart::text(message), part])
}

/// Extract a trimmed string argument from the assembled arguments object.
fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Extract an unsigned-integer argument, accepting either a JSON number or a numeric string.
fn uint_arg(arguments: &Value, key: &str) -> Option<usize> {
    let value = arguments.get(key)?;
    if let Some(number) = value.as_u64() {
        return usize::try_from(number).ok();
    }
    value.as_str()?.trim().parse().ok()
}

/// Read up to `count` bytes from the head of `path`, returning what was read (empty on any error).
fn read_prefix(path: &Path, count: usize) -> Vec<u8> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut buffer = vec![0u8; count];
    match file.read(&mut buffer) {
        Ok(read) => {
            buffer.truncate(read);
            buffer
        }
        Err(_) => Vec::new(),
    }
}

/// The image media type for `path`, by extension first then magic bytes, or `None`.
fn image_media_type(path: &Path, prefix: &[u8]) -> Option<&'static str> {
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        match extension.to_ascii_lowercase().as_str() {
            "png" => return Some("image/png"),
            "jpg" | "jpeg" => return Some("image/jpeg"),
            "gif" => return Some("image/gif"),
            "webp" => return Some("image/webp"),
            "bmp" => return Some("image/bmp"),
            _ => {}
        }
    }
    if prefix.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if prefix.starts_with(b"\xFF\xD8\xFF") {
        return Some("image/jpeg");
    }
    if prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if prefix.len() >= 12 && prefix.starts_with(b"RIFF") && &prefix[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// The PDF media type for `path`, by extension first then magic bytes, or `None`.
fn pdf_media_type(path: &Path, prefix: &[u8]) -> Option<&'static str> {
    let is_pdf_ext = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false);
    if is_pdf_ext || prefix.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else {
        None
    }
}

/// Classify a non-image/PDF file as binary: a NUL byte or a high invalid-UTF-8 ratio in the sampled
/// head, so binary bytes are never emitted as mojibake text.
fn sniffs_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(SNIFF_BYTES)];
    if sample.is_empty() {
        return false;
    }
    if sample.contains(&0) {
        return true;
    }
    let mut invalid = 0usize;
    let mut rest = sample;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(_) => break,
            Err(error) => {
                let valid = error.valid_up_to();
                let skip = error.error_len().unwrap_or(rest.len() - valid);
                invalid += skip;
                rest = &rest[valid + skip..];
            }
        }
    }
    (invalid as f64) / (sample.len() as f64) > BINARY_INVALID_RATIO
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("cortex-read-{tag}-{}-{unique}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn ctx_for(dir: &Path) -> ToolContext {
        ToolContext::new(dir.to_path_buf())
    }

    fn read_args(path: &str) -> Value {
        json!({ "path": path })
    }

    fn text_of(output: &ToolOutput) -> String {
        output.model_content()
    }

    #[test]
    fn three_line_file_is_line_numbered_and_degutters_byte_identical() {
        let dir = temp_dir("gutter");
        fs::write(dir.join("sample.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let output = run(&read_args("sample.txt"), &ctx_for(&dir));
        assert!(!output.is_error);
        assert_eq!(text_of(&output), "1: alpha\n2: beta\n3: gamma");

        // De-gutter: strip the `N: ` prefix from each line and rejoin.
        let degutter: String = text_of(&output)
            .lines()
            .map(|line| line.split_once(": ").map_or("", |(_, rest)| rest))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(degutter, "alpha\nbeta\ngamma");
    }

    #[test]
    fn blank_lines_are_numbered_and_preserved() {
        let dir = temp_dir("blank");
        fs::write(dir.join("f.txt"), "a\n\nc\n").unwrap();
        let output = run(&read_args("f.txt"), &ctx_for(&dir));
        assert_eq!(text_of(&output), "1: a\n2: \n3: c");
    }

    #[test]
    fn offset_and_limit_window_uses_true_line_numbers() {
        let dir = temp_dir("window");
        let body: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        fs::write(dir.join("f.txt"), body).unwrap();
        let args = json!({ "path": "f.txt", "offset": 50, "limit": 10 });
        let output = run(&args, &ctx_for(&dir));
        let text = text_of(&output);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "50: line 50");
        assert_eq!(lines[9], "59: line 59");
        assert!(!text.contains("49: line 49"));
        assert!(!text.contains("60: line 60"));
    }

    #[test]
    fn unbounded_read_caps_at_default_limit_with_truncation_marker() {
        let dir = temp_dir("cap");
        let body: String = (1..=5000).map(|n| format!("l{n}\n")).collect();
        fs::write(dir.join("big.txt"), body).unwrap();
        let output = run(&read_args("big.txt"), &ctx_for(&dir));
        let text = text_of(&output);
        let numbered = text
            .lines()
            .filter(|line| {
                line.split_once(": ")
                    .is_some_and(|(n, _)| n.parse::<u32>().is_ok())
            })
            .count();
        assert_eq!(numbered, DEFAULT_READ_LIMIT);
        assert!(text.contains("[truncated"));
        assert!(text.contains("use offset=2001"));
        assert!(text.contains("of 5000"));
        // The full file is never silently dumped.
        assert!(!text.contains("4999: l4999"));
    }

    // VAL-TOOLS-037: a read far exceeding the limits is bounded in-band (window + machine-detectable
    // marker preserved) AND the complete file content is spilled to an on-disk file named in the
    // result, so nothing is silently lost.
    #[test]
    fn over_limit_read_spills_complete_file_and_keeps_window_marker() {
        let dir = temp_dir("spill");
        let body: String = (1..=200_000).map(|n| format!("{n}\n")).collect();
        fs::write(dir.join("huge.txt"), &body).unwrap();
        let output = run(&read_args("huge.txt"), &ctx_for(&dir));
        let text = text_of(&output);

        // In-band result is bounded to the window, not the whole 200k-line file.
        let numbered = text
            .lines()
            .filter(|line| {
                line.split_once(": ")
                    .is_some_and(|(n, _)| n.parse::<u32>().is_ok())
            })
            .count();
        assert_eq!(
            numbered, DEFAULT_READ_LIMIT,
            "in-band bounded to the window"
        );

        // The existing machine-detectable windowing marker is preserved.
        assert!(text.contains("[truncated: showing lines 1-2000 of 200000"));
        assert!(text.contains("use offset=2001"));

        // A pointer to the spill file is present, and that file holds the COMPLETE file content.
        let marker = "written to ";
        let start = text.rfind(marker).expect("a spill pointer") + marker.len();
        let rest = &text[start..];
        let end = rest.find(']').expect("the notice closes with ]");
        let path = &rest[..end];
        let spilled = std::fs::read_to_string(path).expect("read spill file");
        assert_eq!(spilled, body, "spill holds the complete file content");
        assert_eq!(spilled.lines().count(), 200_000);
        let _ = std::fs::remove_file(path);
    }

    // VAL-TOOLS-037 (no regression): a read within limits returns verbatim with no truncation notice
    // and no spill file.
    #[test]
    fn within_limits_read_returns_verbatim_with_no_spill() {
        let dir = temp_dir("nospill");
        fs::write(dir.join("small.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let output = run(&read_args("small.txt"), &ctx_for(&dir));
        let text = text_of(&output);
        assert_eq!(text, "1: alpha\n2: beta\n3: gamma");
        assert!(!text.contains("written to"), "no spill pointer");
        assert!(!text.contains("truncated"), "no truncation notice");
    }

    // A read whose rendered view exceeds the byte budget (few but very long lines) is byte-bounded
    // in-band and the complete file is spilled.
    #[test]
    fn over_byte_budget_read_is_bounded_and_spilled() {
        let dir = temp_dir("bytes");
        // 10 lines of ~100 KiB each: well under the 2000-line window but far over the byte budget.
        let line = "x".repeat(100 * 1024);
        let body: String = (0..10).map(|_| format!("{line}\n")).collect();
        fs::write(dir.join("wide.txt"), &body).unwrap();
        let output = run(&read_args("wide.txt"), &ctx_for(&dir));
        let text = text_of(&output);

        assert!(
            text.len() <= DEFAULT_READ_MAX_BYTES + 256,
            "in-band bounded to the byte budget, got {} bytes",
            text.len()
        );
        let marker = "written to ";
        let start = text.rfind(marker).expect("a spill pointer") + marker.len();
        let rest = &text[start..];
        let end = rest.find(']').expect("the notice closes with ]");
        let path = &rest[..end];
        let spilled = std::fs::read_to_string(path).expect("read spill file");
        assert_eq!(spilled, body, "spill holds the complete file content");
        let _ = std::fs::remove_file(path);
    }

    // Gap compatibility fix: each rendered line is truncated to 2000 chars on a UTF-8 boundary.
    // Any such truncation counts as omitted content, so the complete file is spilled and named.
    #[test]
    fn overlong_line_is_char_truncated_and_spilled_as_complete_content() {
        let dir = temp_dir("linecap");
        let long_line = "🙂".repeat(DEFAULT_READ_MAX_LINE_CHARS + 1);
        fs::write(dir.join("long.txt"), &long_line).unwrap();
        let output = run(&read_args("long.txt"), &ctx_for(&dir));
        let text = text_of(&output);

        let expected_rendered = format!(
            "1: {}{}",
            "🙂".repeat(DEFAULT_READ_MAX_LINE_CHARS),
            PER_LINE_TRUNCATION_MARKER
        );
        assert!(
            text.contains(&expected_rendered),
            "rendered line should be char-truncated with marker: {text}"
        );
        assert!(
            !text.contains(&format!("1: {long_line}")),
            "full overlong line must not be emitted in-band"
        );
        assert!(
            text.contains("written to "),
            "line truncation should trigger complete-file spill: {text}"
        );

        let marker = "written to ";
        let start = text.rfind(marker).expect("a spill pointer") + marker.len();
        let rest = &text[start..];
        let end = rest.find(']').expect("the notice closes with ]");
        let path = &rest[..end];
        let spilled = std::fs::read_to_string(path).expect("read spill file");
        assert_eq!(spilled, long_line, "spill holds the complete overlong line");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn directory_lists_entries_marking_subdirectories() {
        let dir = temp_dir("dir");
        fs::write(dir.join("a.txt"), "x").unwrap();
        fs::create_dir(dir.join("b")).unwrap();
        fs::write(dir.join("c.rs"), "y").unwrap();
        let output = run(&read_args("."), &ctx_for(&dir));
        let text = text_of(&output);
        assert!(!output.is_error);
        assert!(text.contains("a.txt"));
        assert!(
            text.contains("b/"),
            "subdirectory b must be marked with a slash"
        );
        assert!(text.contains("c.rs"));
    }

    #[test]
    fn image_is_returned_as_an_attachment_not_text_bytes() {
        let dir = temp_dir("image");
        // A real PNG header followed by arbitrary bytes (including ones invalid as UTF-8).
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&[0x00, 0xFF, 0xC0, 0x01, 0x02]);
        fs::write(dir.join("logo.png"), bytes).unwrap();
        let output = run(&read_args("logo.png"), &ctx_for(&dir));
        assert!(!output.is_error);

        let image_part = output
            .parts
            .iter()
            .find(|part| part.is_attachment())
            .expect("an attachment part");
        assert_eq!(image_part.media_type(), Some("image/png"));

        // No text part contains raw binary bytes.
        for part in &output.parts {
            if let Some(text) = part.as_text() {
                assert!(!text.as_bytes().contains(&0xFF));
                assert!(!text.as_bytes().contains(&0x00));
            }
        }
    }

    #[test]
    fn pdf_is_returned_as_a_document_attachment() {
        let dir = temp_dir("pdf");
        fs::write(dir.join("doc.pdf"), b"%PDF-1.4\n%binary").unwrap();
        let output = run(&read_args("doc.pdf"), &ctx_for(&dir));
        let part = output
            .parts
            .iter()
            .find(|part| part.is_attachment())
            .expect("an attachment part");
        assert_eq!(part.media_type(), Some("application/pdf"));
    }

    #[test]
    fn missing_file_errors_with_the_path_and_not_found() {
        let dir = temp_dir("missing");
        let output = run(&read_args("does-not-exist.txt"), &ctx_for(&dir));
        assert!(output.is_error);
        let text = text_of(&output);
        assert!(text.contains("does-not-exist.txt"));
        assert!(text.to_lowercase().contains("not found"));
    }

    #[test]
    fn empty_file_returns_no_content_without_error() {
        let dir = temp_dir("empty");
        fs::write(dir.join("e.txt"), "").unwrap();
        let output = run(&read_args("e.txt"), &ctx_for(&dir));
        assert!(!output.is_error);
        let text = text_of(&output);
        assert_eq!(text, "(no content)");
        // No fabricated numbered lines.
        assert!(!text.contains("1: "));
    }

    #[test]
    fn offset_past_eof_returns_empty_window_with_indication() {
        let dir = temp_dir("pasteof");
        fs::write(dir.join("f.txt"), "a\nb\nc\n").unwrap();
        let args = json!({ "path": "f.txt", "offset": 999 });
        let output = run(&args, &ctx_for(&dir));
        assert!(!output.is_error);
        let text = text_of(&output);
        // Zero content lines, a clear EOF/empty indication.
        assert!(!text.contains("999: "));
        assert!(text.contains("past the end"));
        assert!(text.contains("3 lines"));
    }

    #[test]
    fn relative_path_resolves_against_working_directory() {
        let dir = temp_dir("resolve");
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/x.txt"), "hi\n").unwrap();
        let output = run(&read_args("sub/x.txt"), &ctx_for(&dir));
        assert_eq!(text_of(&output), "1: hi");
    }

    #[test]
    fn file_path_aliases_resolve_like_path() {
        let dir = temp_dir("aliases");
        fs::write(dir.join("a.txt"), "ok\n").unwrap();

        let via_camel = run(&json!({ "filePath": "a.txt" }), &ctx_for(&dir));
        assert!(!via_camel.is_error);
        assert_eq!(text_of(&via_camel), "1: ok");

        let via_snake = run(&json!({ "file_path": "a.txt" }), &ctx_for(&dir));
        assert!(!via_snake.is_error);
        assert_eq!(text_of(&via_snake), "1: ok");
    }

    #[test]
    fn successful_read_records_a_freshness_ledger_entry() {
        let dir = temp_dir("ledger");
        fs::write(dir.join("f.txt"), "x\n").unwrap();
        let ctx = ctx_for(&dir);
        run(&read_args("f.txt"), &ctx);
        assert!(ctx.was_read(&dir.join("f.txt")));
    }

    #[test]
    fn binary_non_image_file_is_refused_without_garbage() {
        let dir = temp_dir("binary");
        fs::write(dir.join("blob.dat"), [0u8, 1, 2, 3, 0, 159, 146, 150]).unwrap();
        let output = run(&read_args("blob.dat"), &ctx_for(&dir));
        assert!(output.is_error);
        assert!(text_of(&output).contains("binary"));
    }

    #[test]
    fn permission_deny_blocks_the_read() {
        use std::sync::Arc;

        use permissions::{Action, PermissionEngine, Rule, Ruleset};

        let dir = temp_dir("perm");
        fs::write(dir.join("secret.txt"), "top secret\n").unwrap();
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "read",
            "secret.txt",
            Action::Deny,
        )]));
        let ctx = ToolContext::new(dir.clone()).with_permissions(Arc::new(engine));
        let output = run(&read_args("secret.txt"), &ctx);
        assert!(output.is_error);
        let text = text_of(&output);
        assert!(text.contains("permission denied"));
        assert!(
            !text.contains("top secret"),
            "protected bytes must not leak"
        );
    }
}

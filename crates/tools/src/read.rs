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
//!   silent partial dump.
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

use crate::tool::{Tool, ToolContext, ToolOutput};

/// The default line cap applied to an unbounded read (architecture §3.4 / VAL-TOOLS-003).
pub const DEFAULT_READ_LIMIT: usize = 2000;

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
                "offset": {
                    "type": "integer",
                    "description": "1-based line number to start reading from (defaults to 1)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read (defaults to 2000)."
                }
            },
            "required": ["path"]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

/// Execute a read against the resolved arguments (synchronous filesystem work).
fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(path) = string_arg(arguments, "path").or_else(|| string_arg(arguments, "file_path"))
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
    if !ctx.permits("read", path) {
        return ToolOutput::error(format!("read: permission denied for {path}"));
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
    let output = render_text(&content, offset, limit);
    ctx.record_read(&resolved);
    ToolOutput::text(output)
}

/// Render a text file's windowed, line-numbered body plus any truncation/EOF indicator.
fn render_text(content: &str, offset: Option<usize>, limit: Option<usize>) -> String {
    let lines = split_lines(content);
    let total = lines.len();

    if total == 0 {
        return "(no content)".to_owned();
    }

    let offset = offset.unwrap_or(1).max(1);
    let limit = limit.unwrap_or(DEFAULT_READ_LIMIT).max(1);

    if offset > total {
        return format!(
            "(no content: offset {offset} is past the end of the file, which has {total} lines)"
        );
    }

    let mut rendered: Vec<String> = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
        rendered.push(format!("{}: {}", index + 1, line));
    }

    let last = offset - 1 + rendered.len();
    let mut output = rendered.join("\n");
    if last < total {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!(
            "[truncated: showing lines {offset}-{last} of {total}; use offset={} to continue]",
            last + 1
        ));
    }
    output
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

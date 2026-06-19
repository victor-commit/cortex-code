//! Faithful relay of an MCP `tools/call` result into the agent's structured tool result.
//!
//! The content blocks (`text`, and a textual placeholder for non-text media) and any structured
//! content the server returned are relayed verbatim into the model-facing result. Oversized output
//! is bounded the same way the agent's file/shell tools bound theirs: the in-band text is truncated
//! with an explicit notice and the complete output is spilled to a uniquely named on-disk file named
//! in that notice, so nothing is silently lost (the spill directory and notice mirror the `tools`
//! crate's `bound_output`).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use protocol::ToolResultPart;
use rmcp::model::{CallToolResult, RawContent};

/// The default cap on the bytes of relayed in-band MCP tool output before truncation and spill.
pub const DEFAULT_MAX_RELAY_BYTES: usize = 64 * 1024;

/// A monotonically increasing counter so concurrent spills never collide on a filename.
static SPILL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The relayed tool result: the model-facing text (bounded), the structured result parts, and
/// whether the server reported an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayedResult {
    /// The text the model reads back. The verbatim server payload when within limits, otherwise the
    /// truncated head plus a spill notice.
    pub content: String,
    /// The structured result parts surfaced to machine consumers.
    pub parts: Vec<ToolResultPart>,
    /// Whether the server reported the call as an error.
    pub is_error: bool,
}

/// Relay `result` into a [`RelayedResult`], bounding the in-band output to [`DEFAULT_MAX_RELAY_BYTES`].
#[must_use]
pub fn relay(result: &CallToolResult) -> RelayedResult {
    relay_with_limit(result, DEFAULT_MAX_RELAY_BYTES)
}

/// Relay `result`, bounding the in-band output to `max_bytes` (exposed for deterministic testing).
#[must_use]
pub fn relay_with_limit(result: &CallToolResult, max_bytes: usize) -> RelayedResult {
    let mut blocks: Vec<String> = Vec::new();

    for content in &result.content {
        match &content.raw {
            RawContent::Text(text) => blocks.push(text.text.clone()),
            RawContent::Image(image) => blocks.push(format!(
                "[image content: {} ({} base64 chars)]",
                image.mime_type,
                image.data.len()
            )),
            RawContent::Audio(audio) => {
                blocks.push(format!("[audio content: {}]", audio.mime_type));
            }
            RawContent::Resource(resource) => {
                blocks.push(relay_resource(&resource.resource));
            }
            RawContent::ResourceLink(link) => {
                blocks.push(format!("[resource link: {}]", link.uri));
            }
        }
    }

    if let Some(structured) = &result.structured_content {
        if let Ok(rendered) = serde_json::to_string(structured) {
            blocks.push(rendered);
        }
    }

    let joined = blocks.join("\n");
    let bounded = bound(&joined, max_bytes, "mcp-tool");

    RelayedResult {
        content: bounded.clone(),
        parts: vec![ToolResultPart::text(bounded)],
        is_error: result.is_error.unwrap_or(false),
    }
}

/// Render an embedded resource's contents as text for the in-band relay.
fn relay_resource(resource: &rmcp::model::ResourceContents) -> String {
    match resource {
        rmcp::model::ResourceContents::TextResourceContents { uri, text, .. } => {
            format!("[resource {uri}]\n{text}")
        }
        rmcp::model::ResourceContents::BlobResourceContents { uri, blob, .. } => {
            format!("[binary resource {uri} ({} base64 chars)]", blob.len())
        }
    }
}

/// Bound `full` to `max_bytes`, spilling the complete output to disk with a notice when exceeded.
fn bound(full: &str, max_bytes: usize, label: &str) -> String {
    if full.len() <= max_bytes {
        return full.to_owned();
    }

    let spill_path = spill_to_disk(full, label);

    let mut end = max_bytes;
    while end > 0 && !full.is_char_boundary(end) {
        end -= 1;
    }
    let kept = &full[..end];

    let notice = match &spill_path {
        Some(path) => format!(
            "[output truncated: showing {} of {} bytes; full output written to {}]",
            kept.len(),
            full.len(),
            path.display()
        ),
        None => format!(
            "[output truncated: showing {} of {} bytes; full output could not be written to a spill file]",
            kept.len(),
            full.len()
        ),
    };

    if kept.is_empty() {
        notice
    } else {
        format!("{kept}\n{notice}")
    }
}

/// Write `full` to a uniquely named file under the shared tool-output spill directory.
fn spill_to_disk(full: &str, label: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("cortex-tool-output");
    if fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let unique = SPILL_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("{label}-{}-{unique}.txt", std::process::id()));
    fs::write(&path, full).ok().map(|()| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::Content;
    use serde_json::json;

    fn text_result(text: &str) -> CallToolResult {
        CallToolResult::success(vec![Content::text(text)])
    }

    #[test]
    fn relays_a_text_block_verbatim() {
        let relayed = relay(&text_result("line one\nline two"));
        assert_eq!(relayed.content, "line one\nline two");
        assert!(!relayed.is_error);
        assert_eq!(
            relayed.parts,
            vec![ToolResultPart::text("line one\nline two")]
        );
    }

    #[test]
    fn relays_structured_content_as_json() {
        let mut result = CallToolResult::success(Vec::new());
        result.structured_content = Some(json!({ "sum": 42 }));
        let relayed = relay(&result);
        assert!(
            relayed.content.contains("\"sum\":42"),
            "got: {}",
            relayed.content
        );
    }

    #[test]
    fn relays_text_and_structured_together() {
        let mut result = CallToolResult::success(vec![Content::text("42")]);
        result.structured_content = Some(json!({ "sum": 42 }));
        let relayed = relay(&result);
        assert!(relayed.content.contains("42"));
        assert!(relayed.content.contains("\"sum\":42"));
    }

    #[test]
    fn propagates_the_error_flag() {
        let mut result = text_result("boom");
        result.is_error = Some(true);
        assert!(relay(&result).is_error);
    }

    #[test]
    fn oversized_output_is_truncated_with_a_spill_notice() {
        let big = "x".repeat(4096);
        let relayed = relay_with_limit(&text_result(&big), 256);
        assert!(relayed.content.len() < big.len(), "must be truncated");
        assert!(relayed.content.contains("output truncated"));
        assert!(relayed.content.contains("of 4096 bytes"));

        // The notice names a spill file holding the complete output.
        let marker = "written to ";
        let path = relayed
            .content
            .split(marker)
            .nth(1)
            .and_then(|tail| tail.split(']').next())
            .map(str::to_owned)
            .expect("spill path named in notice");
        let spilled = std::fs::read_to_string(path.trim()).expect("read spill file");
        assert_eq!(spilled, big, "the spill file holds the complete output");
    }

    #[test]
    fn output_at_the_limit_is_relayed_verbatim() {
        let exact = "y".repeat(256);
        let relayed = relay_with_limit(&text_result(&exact), 256);
        assert_eq!(relayed.content, exact);
    }
}

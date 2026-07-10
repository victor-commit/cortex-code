//! Lossless relay of MCP results with a separate bounded model-facing projection.
//!
//! The model must not receive arbitrary base64 blobs, but machine consumers and durable session
//! surfaces must retain the source protocol result exactly. This module therefore creates both a
//! typed [`protocol::McpToolResult`] and a small textual projection. Oversized payloads become
//! integrity-checked spill references instead of being flattened or discarded.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use protocol::{
    McpContentBlock, McpEmbeddedResource, McpPayload, McpSpillReference, McpStructuredContent,
    McpToolResult, ToolResultPart,
};
use rmcp::model::{
    CallToolResult, Content, GetPromptResult, PromptMessageContent, PromptMessageRole, RawContent,
    ReadResourceResult, ResourceContents,
};
use sha2::{Digest, Sha256};

/// The default cap on the bytes of relayed in-band MCP tool output before truncation and spill.
pub const DEFAULT_MAX_RELAY_BYTES: usize = 64 * 1024;

/// A monotonically increasing counter so concurrent spills never collide on a filename.
static SPILL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The relayed tool result: a model-facing projection and the independent lossless MCP source.
#[derive(Debug, Clone, PartialEq)]
pub struct RelayedResult {
    /// The bounded projection re-injected into the model conversation.
    pub content: String,
    /// Generic parts for existing model/tool-card consumers. `mcp_result` is the authoritative
    /// source representation for MCP-specific machine consumers.
    pub parts: Vec<ToolResultPart>,
    /// Source-ordered typed MCP result including metadata, structured content, and spill refs.
    pub mcp_result: McpToolResult,
    /// Whether the server reported the call as an error, for normal tool-loop recovery.
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
    let mut projection = Vec::with_capacity(result.content.len() + 1);
    let blocks = result
        .content
        .iter()
        .enumerate()
        .map(|(index, content)| relay_content(content, index, max_bytes, &mut projection))
        .collect();
    let structured_content = result.structured_content.as_ref().map(|value| {
        let rendered = serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned());
        let structured = payload_or_spill(rendered, max_bytes, "structured-json", "utf-8-json");
        match &structured {
            McpPayload::Inline(rendered) => {
                projection.push(format!("[MCP structured content]\n{rendered}"))
            }
            McpPayload::Spill { spill } => projection.push(spill_notice("structured JSON", spill)),
        }
        match structured {
            McpPayload::Inline(_) => McpStructuredContent::Inline(value.clone()),
            McpPayload::Spill { spill } => McpStructuredContent::Spill { spill },
        }
    });
    let bounded = bound_model_projection(&projection.join("\n"), max_bytes);
    let mcp_result = McpToolResult {
        blocks,
        structured_content,
        metadata: meta_value(result.meta.as_ref()),
        is_error: result.is_error,
    };

    RelayedResult {
        content: bounded.clone(),
        parts: vec![ToolResultPart::text(bounded)],
        mcp_result,
        is_error: result.is_error.unwrap_or(false),
    }
}

/// Relay a `resources/read` result into bounded model-facing text.
#[must_use]
pub fn relay_read_resource(result: &ReadResourceResult) -> RelayedResult {
    let content = result
        .contents
        .iter()
        .map(relay_resource)
        .collect::<Vec<_>>()
        .join("\n");
    relayed_text(content, false)
}

/// Relay a `prompts/get` result into bounded model-facing text.
#[must_use]
pub fn relay_get_prompt(result: &GetPromptResult) -> RelayedResult {
    let mut blocks = Vec::new();
    if let Some(description) = &result.description {
        blocks.push(format!("[prompt description]\n{description}"));
    }
    for message in &result.messages {
        let role = match message.role {
            PromptMessageRole::User => "user",
            PromptMessageRole::Assistant => "assistant",
        };
        blocks.push(format!(
            "[{role}]\n{}",
            relay_prompt_content(&message.content)
        ));
    }
    relayed_text(blocks.join("\n"), false)
}

fn relayed_text(content: String, is_error: bool) -> RelayedResult {
    let bounded = bound_model_projection(&content, DEFAULT_MAX_RELAY_BYTES);
    RelayedResult {
        content: bounded.clone(),
        parts: vec![ToolResultPart::text(bounded)],
        mcp_result: McpToolResult {
            blocks: Vec::new(),
            structured_content: None,
            metadata: None,
            is_error: Some(is_error),
        },
        is_error,
    }
}

/// Render a non-tools/call embedded resource for the bounded model projection.
fn relay_resource(resource: &ResourceContents) -> String {
    match resource {
        rmcp::model::ResourceContents::TextResourceContents { uri, text, .. } => {
            format!("[resource {uri}]\n{text}")
        }
        rmcp::model::ResourceContents::BlobResourceContents { uri, blob, .. } => {
            format!("[binary resource {uri} ({} base64 chars)]", blob.len())
        }
    }
}

fn relay_prompt_content(content: &PromptMessageContent) -> String {
    match content {
        PromptMessageContent::Text { text } => text.clone(),
        PromptMessageContent::Image { image } => format!(
            "[image content: {} ({} base64 chars)]",
            image.mime_type,
            image.data.len()
        ),
        PromptMessageContent::Resource { resource } => relay_resource(&resource.resource),
        PromptMessageContent::ResourceLink { link } => format!("[resource link: {}]", link.uri),
    }
}

fn relay_content(
    content: &Content,
    index: usize,
    max_bytes: usize,
    projection: &mut Vec<String>,
) -> McpContentBlock {
    let label = format!("mcp-block-{index}");
    let annotations = annotations_value(content.annotations.as_ref());
    match &content.raw {
        RawContent::Text(text) => {
            let payload = payload_or_spill(text.text.clone(), max_bytes, &label, "utf-8");
            projection.push(match &payload {
                McpPayload::Inline(text) => text.clone(),
                McpPayload::Spill { spill } => spill_notice("text", spill),
            });
            McpContentBlock::Text {
                text: payload,
                annotations,
                metadata: meta_value(text.meta.as_ref()),
            }
        }
        RawContent::Image(image) => {
            let payload = payload_or_spill(image.data.clone(), max_bytes, &label, "base64");
            projection.push(format!(
                "[MCP image: {} ({})]",
                image.mime_type,
                payload_summary(&payload)
            ));
            McpContentBlock::Image {
                data: payload,
                media_type: image.mime_type.clone(),
                annotations,
                metadata: meta_value(image.meta.as_ref()),
            }
        }
        RawContent::Audio(audio) => {
            let payload = payload_or_spill(audio.data.clone(), max_bytes, &label, "base64");
            projection.push(format!(
                "[MCP audio: {} ({})]",
                audio.mime_type,
                payload_summary(&payload)
            ));
            McpContentBlock::Audio {
                data: payload,
                media_type: audio.mime_type.clone(),
                annotations,
            }
        }
        RawContent::Resource(embedded) => {
            let (resource, resource_projection) =
                relay_embedded_resource(&embedded.resource, max_bytes, &label);
            projection.push(resource_projection);
            McpContentBlock::EmbeddedResource {
                resource,
                annotations,
                metadata: meta_value(embedded.meta.as_ref()),
            }
        }
        RawContent::ResourceLink(link) => {
            projection.push(format!("[MCP resource link: {}]", link.uri));
            McpContentBlock::ResourceLink {
                uri: link.uri.clone(),
                name: link.name.clone(),
                title: link.title.clone(),
                description: link.description.clone(),
                mime_type: link.mime_type.clone(),
                size: link.size,
                icons: link
                    .icons
                    .as_ref()
                    .and_then(|icons| serde_json::to_value(icons).ok()),
                annotations,
                metadata: meta_value(link.meta.as_ref()),
            }
        }
    }
}

fn relay_embedded_resource(
    resource: &ResourceContents,
    max_bytes: usize,
    label: &str,
) -> (McpEmbeddedResource, String) {
    match resource {
        ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            meta,
        } => {
            let text = payload_or_spill(text.clone(), max_bytes, label, "utf-8");
            let projection = match &text {
                McpPayload::Inline(text) => format!("[MCP embedded resource: {uri}]\n{text}"),
                McpPayload::Spill { spill } => spill_notice("embedded resource text", spill),
            };
            (
                McpEmbeddedResource::Text {
                    uri: uri.clone(),
                    mime_type: mime_type.clone(),
                    text,
                    metadata: meta_value(meta.as_ref()),
                },
                projection,
            )
        }
        ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            meta,
        } => {
            let blob = payload_or_spill(blob.clone(), max_bytes, label, "base64");
            let projection = format!(
                "[MCP embedded binary resource: {uri} ({})]",
                payload_summary(&blob)
            );
            (
                McpEmbeddedResource::Blob {
                    uri: uri.clone(),
                    mime_type: mime_type.clone(),
                    blob,
                    metadata: meta_value(meta.as_ref()),
                },
                projection,
            )
        }
    }
}

fn annotations_value(annotations: Option<&rmcp::model::Annotations>) -> Option<serde_json::Value> {
    annotations.and_then(|annotations| serde_json::to_value(annotations).ok())
}

fn meta_value(meta: Option<&rmcp::model::Meta>) -> Option<serde_json::Value> {
    meta.and_then(|meta| serde_json::to_value(meta).ok())
}

fn payload_or_spill(payload: String, max_bytes: usize, label: &str, encoding: &str) -> McpPayload {
    if payload.len() <= max_bytes {
        McpPayload::Inline(payload)
    } else {
        spill_to_disk(payload.as_bytes(), label, encoding)
            .map(|spill| McpPayload::Spill { spill })
            // A source payload is never replaced by an unrecoverable placeholder. The model
            // projection remains bounded independently, even on a host where no spill path can
            // be created.
            .unwrap_or(McpPayload::Inline(payload))
    }
}

fn payload_summary(payload: &McpPayload) -> String {
    match payload {
        McpPayload::Inline(payload) => format!("{} inline chars", payload.len()),
        McpPayload::Spill { spill } => format!(
            "spilled {} bytes, sha256 {}",
            spill.byte_length, spill.sha256
        ),
    }
}

fn spill_notice(kind: &str, spill: &McpSpillReference) -> String {
    format!(
        "[MCP {kind} payload spilled: {} bytes, sha256 {}, recover at {}]",
        spill.byte_length, spill.sha256, spill.path
    )
}

/// Bound the model-only projection while leaving the independent typed source result untouched.
fn bound_model_projection(full: &str, max_bytes: usize) -> String {
    if full.len() <= max_bytes {
        return full.to_owned();
    }

    let spill = spill_to_disk(full.as_bytes(), "mcp-model-projection", "utf-8");

    let mut end = max_bytes;
    while end > 0 && !full.is_char_boundary(end) {
        end -= 1;
    }
    let kept = &full[..end];

    let notice = match &spill {
        Some(spill) => format!(
            "[MCP model projection truncated: showing {} of {} bytes; full projection spilled to {} (sha256 {})]",
            kept.len(),
            full.len(),
            spill.path,
            spill.sha256,
        ),
        None => format!(
            "[MCP model projection truncated: showing {} of {} bytes; full projection could not be written to a spill file]",
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

fn spill_directory() -> PathBuf {
    if let Some(data_dir) = std::env::var_os("CORTEX_DATA_DIR") {
        return PathBuf::from(data_dir).join("mcp-spills");
    }
    if let Some(xdg_data) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg_data).join("cortex").join("mcp-spills");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("cortex")
            .join("mcp-spills");
    }
    std::env::temp_dir().join("cortex-mcp-spills")
}

/// Write source bytes to a protected persistent spill file and return a verifiable reference.
fn spill_to_disk(bytes: &[u8], label: &str, encoding: &str) -> Option<McpSpillReference> {
    let preferred = spill_directory();
    let temporary = std::env::temp_dir().join("cortex-mcp-spills");
    let directories = if preferred == temporary {
        vec![preferred]
    } else {
        vec![preferred, temporary]
    };
    for dir in directories {
        if fs::create_dir_all(&dir).is_err() {
            continue;
        }
        set_directory_permissions(&dir);
        let unique = SPILL_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(
            "{}-{}-{unique}.spill",
            safe_label(label),
            std::process::id()
        ));
        let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(&path) else {
            continue;
        };
        set_file_permissions(&path);
        if file.write_all(bytes).is_err() || file.flush().is_err() {
            let _ = fs::remove_file(&path);
            continue;
        }
        return Some(McpSpillReference {
            path: path.to_string_lossy().into_owned(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            byte_length: bytes.len() as u64,
            encoding: encoding.to_owned(),
        });
    }
    None
}

fn safe_label(label: &str) -> String {
    label
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

#[cfg(unix)]
fn set_directory_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_directory_permissions(_path: &std::path::Path) {}

#[cfg(unix)]
fn set_file_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_file_permissions(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{
        AnnotateAble, Annotations, Content, Meta, RawAudioContent, RawContent, RawEmbeddedResource,
        RawImageContent, RawResource, RawTextContent, ResourceContents,
    };
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
        assert_eq!(
            relayed.mcp_result.blocks.len(),
            1,
            "the source result must stay separate from the model projection"
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
    fn structured_content_named_spill_round_trips_without_variant_ambiguity() {
        let mut result = CallToolResult::success(Vec::new());
        result.structured_content = Some(json!({
            "spill": { "user_value": "this is ordinary MCP JSON" }
        }));
        let relayed = relay(&result);
        let encoded = serde_json::to_string(&relayed.mcp_result).expect("serialize typed result");
        let decoded: McpToolResult =
            serde_json::from_str(&encoded).expect("deserialize typed result");
        assert_eq!(
            decoded, relayed.mcp_result,
            "an ordinary JSON object named `spill` must not deserialize as a spill reference"
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
        assert!(relayed.content.contains("MCP text payload spilled"));
        let McpContentBlock::Text {
            text: McpPayload::Spill { spill },
            ..
        } = &relayed.mcp_result.blocks[0]
        else {
            panic!("the oversized source payload must become a typed spill reference");
        };
        assert_eq!(spill.byte_length, big.len() as u64);
        assert_eq!(spill.encoding, "utf-8");
        assert_eq!(
            spill.sha256,
            format!("{:x}", Sha256::digest(big.as_bytes())),
            "the spill reference carries a stable source hash"
        );
        let spilled = std::fs::read_to_string(&spill.path).expect("read typed spill file");
        assert_eq!(spilled, big, "the spill file holds the complete output");
    }

    #[test]
    fn oversized_structured_content_spills_recoverable_json() {
        let value = json!({ "payload": "z".repeat(4096), "kind": "structured" });
        let mut result = CallToolResult::success(Vec::new());
        result.structured_content = Some(value.clone());
        let relayed = relay_with_limit(&result, 128);
        let Some(McpStructuredContent::Spill { spill }) = relayed.mcp_result.structured_content
        else {
            panic!("large structured JSON must be represented by a typed spill");
        };
        assert_eq!(spill.encoding, "utf-8-json");
        assert_eq!(
            spill.byte_length as usize,
            serde_json::to_string(&value).unwrap().len()
        );
        let recovered: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&spill.path).expect("recover structured spill"),
        )
        .expect("spilled value is valid JSON");
        assert_eq!(recovered, value);
    }

    #[test]
    fn preserves_every_content_kind_in_source_order_with_metadata() {
        let mut annotations = Annotations::default();
        annotations.priority = Some(0.75);
        let block_meta = Meta(serde_json::Map::from_iter([(
            "block_meta".to_owned(),
            json!("retained"),
        )]));
        let resource_meta = Meta(serde_json::Map::from_iter([(
            "resource_meta".to_owned(),
            json!(true),
        )]));
        let text = RawContent::Text(RawTextContent {
            text: "first text".to_owned(),
            meta: Some(block_meta.clone()),
        })
        .annotate(annotations.clone());
        let image = RawContent::Image(RawImageContent {
            data: "aW1hZ2U=".to_owned(),
            mime_type: "image/png".to_owned(),
            meta: Some(block_meta.clone()),
        })
        .annotate(annotations.clone());
        let audio = RawContent::Audio(RawAudioContent {
            data: "YXVkaW8=".to_owned(),
            mime_type: "audio/wav".to_owned(),
        })
        .annotate(annotations.clone());
        let embedded_text = RawContent::Resource(RawEmbeddedResource {
            meta: Some(block_meta.clone()),
            resource: ResourceContents::TextResourceContents {
                uri: "memo://text".to_owned(),
                mime_type: Some("text/plain".to_owned()),
                text: "embedded text".to_owned(),
                meta: Some(resource_meta.clone()),
            },
        })
        .annotate(annotations.clone());
        let embedded_blob = RawContent::Resource(RawEmbeddedResource {
            meta: Some(block_meta.clone()),
            resource: ResourceContents::BlobResourceContents {
                uri: "memo://blob".to_owned(),
                mime_type: Some("application/octet-stream".to_owned()),
                blob: "YmxvYg==".to_owned(),
                meta: Some(resource_meta.clone()),
            },
        })
        .annotate(annotations.clone());
        let resource_link = RawContent::ResourceLink(RawResource {
            uri: "file:///fixture.bin".to_owned(),
            name: "fixture.bin".to_owned(),
            title: Some("Fixture".to_owned()),
            description: Some("resource link metadata".to_owned()),
            mime_type: Some("application/octet-stream".to_owned()),
            size: Some(7),
            icons: None,
            meta: Some(block_meta.clone()),
        })
        .annotate(annotations);
        let mut result = CallToolResult::success(vec![
            text,
            image,
            audio,
            embedded_text,
            embedded_blob,
            resource_link,
        ]);
        result.structured_content = Some(json!({
            "nested": [1, {"preserve": true}],
            "number": 2
        }));
        result.is_error = Some(true);
        result.meta = Some(Meta(serde_json::Map::from_iter([(
            "result_meta".to_owned(),
            json!("retained"),
        )])));

        let relayed = relay(&result);
        assert!(
            relayed.is_error,
            "isError remains independent of block kinds"
        );
        let kinds = relayed
            .mcp_result
            .blocks
            .iter()
            .map(|block| match block {
                McpContentBlock::Text { .. } => "text",
                McpContentBlock::Image { .. } => "image",
                McpContentBlock::Audio { .. } => "audio",
                McpContentBlock::EmbeddedResource { resource, .. } => match resource {
                    McpEmbeddedResource::Text { .. } => "embedded_text",
                    McpEmbeddedResource::Blob { .. } => "embedded_blob",
                },
                McpContentBlock::ResourceLink { .. } => "resource_link",
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "text",
                "image",
                "audio",
                "embedded_text",
                "embedded_blob",
                "resource_link"
            ]
        );
        assert_eq!(
            serde_json::to_value(&relayed.mcp_result).expect("source result JSON"),
            json!({
                "blocks": [
                    {
                        "type": "text",
                        "text": "first text",
                        "annotations": { "priority": 0.75 },
                        "metadata": { "block_meta": "retained" }
                    },
                    {
                        "type": "image",
                        "data": "aW1hZ2U=",
                        "media_type": "image/png",
                        "annotations": { "priority": 0.75 },
                        "metadata": { "block_meta": "retained" }
                    },
                    {
                        "type": "audio",
                        "data": "YXVkaW8=",
                        "media_type": "audio/wav",
                        "annotations": { "priority": 0.75 }
                    },
                    {
                        "type": "embedded_resource",
                        "resource": {
                            "kind": "text",
                            "uri": "memo://text",
                            "mime_type": "text/plain",
                            "text": "embedded text",
                            "metadata": { "resource_meta": true }
                        },
                        "annotations": { "priority": 0.75 },
                        "metadata": { "block_meta": "retained" }
                    },
                    {
                        "type": "embedded_resource",
                        "resource": {
                            "kind": "blob",
                            "uri": "memo://blob",
                            "mime_type": "application/octet-stream",
                            "blob": "YmxvYg==",
                            "metadata": { "resource_meta": true }
                        },
                        "annotations": { "priority": 0.75 },
                        "metadata": { "block_meta": "retained" }
                    },
                    {
                        "type": "resource_link",
                        "uri": "file:///fixture.bin",
                        "name": "fixture.bin",
                        "title": "Fixture",
                        "description": "resource link metadata",
                        "mime_type": "application/octet-stream",
                        "size": 7,
                        "annotations": { "priority": 0.75 },
                        "metadata": { "block_meta": "retained" }
                    }
                ],
                "structured_content": {
                    "kind": "inline",
                    "value": {
                        "nested": [1, {"preserve": true}],
                        "number": 2
                    }
                },
                "metadata": { "result_meta": "retained" },
                "is_error": true
            })
        );
        assert!(
            !relayed.content.contains("aW1hZ2U=") && !relayed.content.contains("YXVkaW8="),
            "base64 stays in the typed source result, not the model projection"
        );
    }

    #[test]
    fn output_at_the_limit_is_relayed_verbatim() {
        let exact = "y".repeat(256);
        let relayed = relay_with_limit(&text_result(&exact), 256);
        assert_eq!(relayed.content, exact);
    }

    #[test]
    fn relays_read_resource_text() {
        let result = ReadResourceResult::new(vec![rmcp::model::ResourceContents::text(
            "RESOURCE-CONTENT-1",
            "notes://demo",
        )]);
        let relayed = relay_read_resource(&result);
        assert!(relayed.content.contains("RESOURCE-CONTENT-1"));
        assert!(!relayed.is_error);
    }

    #[test]
    fn relays_prompt_messages() {
        let result = GetPromptResult::new(vec![rmcp::model::PromptMessage::new_text(
            rmcp::model::PromptMessageRole::User,
            "Hello cortex from MCP",
        )]);
        let relayed = relay_get_prompt(&result);
        assert!(relayed.content.contains("[user]"));
        assert!(relayed.content.contains("Hello cortex from MCP"));
    }
}

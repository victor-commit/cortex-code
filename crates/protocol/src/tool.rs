//! Structured tool-result parts shared between the tool layer, the runner, and the surfaces.
//!
//! A tool result is a sequence of [`ToolResultPart`]s rather than a single string so non-text
//! payloads (an image or a PDF) can be carried as a typed attachment with a media type instead of
//! leaking raw bytes into the text channel. The text parts are what the model reads back; the
//! attachment parts let a machine consumer (the headless `--format json` output, the server SSE
//! feed) recognize and render the media. This type lives in `protocol` so both the `tools`
//! capability and the `engine` runner can name it without depending on each other.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// A recoverable on-disk reference to an MCP payload too large to carry inline.
///
/// The bytes at `path` are exactly the source payload in the declared `encoding`; `sha256` and
/// `byte_length` let consumers verify that a recovered payload has not changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSpillReference {
    /// Absolute path to the protected spill file.
    pub path: String,
    /// Lowercase SHA-256 digest of the spill file bytes.
    pub sha256: String,
    /// Exact byte length of the spill file.
    pub byte_length: u64,
    /// Encoding of the original source payload stored in the file.
    pub encoding: String,
}

/// An MCP string payload retained inline or replaced by a recoverable spill reference.
///
/// The untagged representation intentionally keeps a normal inline MCP payload as a JSON string,
/// while an oversized payload becomes `{ "spill": { ... } }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpPayload {
    /// The original source string (text or base64) is available inline.
    Inline(String),
    /// The original source string is recoverable from the named spill file.
    Spill {
        /// Integrity-checked location of the original payload.
        spill: McpSpillReference,
    },
}

impl McpPayload {
    /// Return an inline source payload, when no spill was necessary.
    #[must_use]
    pub fn inline(&self) -> Option<&str> {
        match self {
            Self::Inline(payload) => Some(payload),
            Self::Spill { .. } => None,
        }
    }
}

/// MCP structured content retained as JSON, or as an integrity-checked JSON spill when it exceeds
/// the configured relay bound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum McpStructuredContent {
    /// The server-provided JSON value, with its JSON type preserved.
    Inline(JsonValue),
    /// A UTF-8 JSON serialization recoverable from a spill file.
    Spill {
        /// Integrity-checked location of the complete JSON serialization.
        spill: McpSpillReference,
    },
}

/// A resource embedded inside an MCP tool-result content block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpEmbeddedResource {
    /// Text bytes embedded directly in the MCP resource.
    Text {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        text: McpPayload,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonValue>,
    },
    /// Base64 bytes embedded directly in the MCP resource.
    Blob {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        blob: McpPayload,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonValue>,
    },
}

/// One source-ordered MCP `tools/call` result content block.
///
/// This is deliberately independent from [`ToolResultPart`]: these blocks faithfully retain the
/// MCP protocol's source shape and metadata, whereas tool-result parts are the bounded projection
/// intended for model continuation and generic surface rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpContentBlock {
    /// A source text block.
    Text {
        text: McpPayload,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<JsonValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonValue>,
    },
    /// A source image block whose `data` is base64.
    Image {
        data: McpPayload,
        media_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<JsonValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonValue>,
    },
    /// A source audio block whose `data` is base64.
    Audio {
        data: McpPayload,
        media_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<JsonValue>,
    },
    /// A source embedded resource block.
    EmbeddedResource {
        resource: McpEmbeddedResource,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<JsonValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonValue>,
    },
    /// A source resource-link block, including all MCP resource metadata.
    ResourceLink {
        uri: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        icons: Option<JsonValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<JsonValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonValue>,
    },
}

/// The lossless MCP `tools/call` result retained alongside a bounded model-facing projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpToolResult {
    /// Source-order MCP content blocks.
    pub blocks: Vec<McpContentBlock>,
    /// The server's structured JSON result, kept as JSON rather than rendered into text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<McpStructuredContent>,
    /// The result-level MCP `_meta` object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonValue>,
    /// The protocol-level `isError` value, including an omitted value when the server omitted it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

/// One part of a tool's structured result.
///
/// Serializes with a `type` discriminator (`text` / `image` / `document`). Attachment variants
/// carry a `media_type` (an IANA media type such as `image/png` or `application/pdf`) and a `path`
/// referencing the on-disk source, so consumers never receive binary bytes inline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResultPart {
    /// Plain text the model reads back (line-numbered file content, a directory listing, an error).
    Text {
        /// The text payload.
        text: String,
    },
    /// An image attachment carried by reference, never as inline bytes.
    Image {
        /// The IANA media type (e.g. `image/png`).
        media_type: String,
        /// The on-disk path the image was read from.
        path: String,
    },
    /// A document attachment (e.g. a PDF) carried by reference, never as inline bytes.
    Document {
        /// The IANA media type (e.g. `application/pdf`).
        media_type: String,
        /// The on-disk path the document was read from.
        path: String,
    },
}

impl ToolResultPart {
    /// A plain-text part.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// An image attachment part referencing `path` with media type `media_type`.
    #[must_use]
    pub fn image(media_type: impl Into<String>, path: impl Into<String>) -> Self {
        Self::Image {
            media_type: media_type.into(),
            path: path.into(),
        }
    }

    /// A document attachment part referencing `path` with media type `media_type`.
    #[must_use]
    pub fn document(media_type: impl Into<String>, path: impl Into<String>) -> Self {
        Self::Document {
            media_type: media_type.into(),
            path: path.into(),
        }
    }

    /// The text payload when this is a [`ToolResultPart::Text`], else `None`.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ToolResultPart::Text { text } => Some(text),
            _ => None,
        }
    }

    /// Whether this part is a non-text attachment (image or document).
    #[must_use]
    pub fn is_attachment(&self) -> bool {
        matches!(
            self,
            ToolResultPart::Image { .. } | ToolResultPart::Document { .. }
        )
    }

    /// The media type when this is an attachment part, else `None`.
    #[must_use]
    pub fn media_type(&self) -> Option<&str> {
        match self {
            ToolResultPart::Image { media_type, .. }
            | ToolResultPart::Document { media_type, .. } => Some(media_type),
            ToolResultPart::Text { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_part_serializes_with_a_type_discriminator() {
        let value = serde_json::to_value(ToolResultPart::text("1: hi")).expect("serialize");
        assert_eq!(value["type"], "text");
        assert_eq!(value["text"], "1: hi");
    }

    #[test]
    fn image_part_carries_a_media_type_and_path_not_bytes() {
        let part = ToolResultPart::image("image/png", "/tmp/logo.png");
        let value = serde_json::to_value(&part).expect("serialize");
        assert_eq!(value["type"], "image");
        assert_eq!(value["media_type"], "image/png");
        assert_eq!(value["path"], "/tmp/logo.png");
        assert!(part.is_attachment());
        assert_eq!(part.media_type(), Some("image/png"));
    }

    #[test]
    fn parts_round_trip_through_json() {
        let parts = vec![
            ToolResultPart::text("body"),
            ToolResultPart::image("image/jpeg", "/a.jpg"),
            ToolResultPart::document("application/pdf", "/a.pdf"),
        ];
        let json = serde_json::to_string(&parts).expect("serialize");
        let back: Vec<ToolResultPart> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parts, back);
    }
}

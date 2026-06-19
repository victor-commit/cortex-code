//! Structured tool-result parts shared between the tool layer, the runner, and the surfaces.
//!
//! A tool result is a sequence of [`ToolResultPart`]s rather than a single string so non-text
//! payloads (an image or a PDF) can be carried as a typed attachment with a media type instead of
//! leaking raw bytes into the text channel. The text parts are what the model reads back; the
//! attachment parts let a machine consumer (the headless `--format json` output, the server SSE
//! feed) recognize and render the media. This type lives in `protocol` so both the `tools`
//! capability and the `engine` runner can name it without depending on each other.

use serde::{Deserialize, Serialize};

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

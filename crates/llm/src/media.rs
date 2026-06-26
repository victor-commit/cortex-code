//! Media attachment helpers shared by provider adapters.

use std::borrow::Cow;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::request::MessageContentPart;

/// One media payload loaded from disk and encoded for provider wire formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedMedia {
    pub media_type: String,
    pub base64: String,
}

/// Read and base64-encode one attachment part.
pub(crate) fn encode_media_part(part: &MessageContentPart) -> Result<EncodedMedia, String> {
    let (path, media_type) = match part {
        MessageContentPart::Image { media_type, path } => {
            (path.as_str(), Cow::Borrowed(media_type.as_str()))
        }
        MessageContentPart::Document { media_type, path } => {
            (path.as_str(), Cow::Borrowed(media_type.as_str()))
        }
        MessageContentPart::File { path, media_type } => (
            path.as_str(),
            media_type
                .as_deref()
                .map(Cow::Borrowed)
                .or_else(|| infer_media_type(path).map(Cow::Borrowed))
                .unwrap_or(Cow::Borrowed("application/octet-stream")),
        ),
        MessageContentPart::Reasoning { .. } => {
            return Err("reasoning metadata is not a binary media attachment".to_owned());
        }
    };

    let bytes =
        std::fs::read(path).map_err(|err| format!("failed to read attachment `{path}`: {err}"))?;
    Ok(EncodedMedia {
        media_type: media_type.into_owned(),
        base64: BASE64.encode(bytes),
    })
}

/// Whether this part is image content.
pub(crate) fn is_image_part(part: &MessageContentPart) -> bool {
    part_media_type(part).is_some_and(|media_type| media_type.starts_with("image/"))
}

/// A safe text placeholder used when a provider/context cannot receive media bytes.
#[must_use]
pub(crate) fn strip_media_stub(part: &MessageContentPart) -> String {
    let media_type = part_media_type(part).unwrap_or("application/octet-stream");
    format!("[Attached {media_type}]")
}

fn part_media_type(part: &MessageContentPart) -> Option<&str> {
    match part {
        MessageContentPart::Image { media_type, .. } => Some(media_type.as_str()),
        MessageContentPart::Document { media_type, .. } => Some(media_type.as_str()),
        MessageContentPart::File { path, media_type } => {
            media_type.as_deref().or_else(|| infer_media_type(path))
        }
        MessageContentPart::Reasoning { .. } => None,
    }
}

fn infer_media_type(path: &str) -> Option<&'static str> {
    let extension = std::path::Path::new(path)
        .extension()?
        .to_str()?
        .trim()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

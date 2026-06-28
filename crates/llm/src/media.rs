//! Media attachment helpers shared by provider adapters.

use std::borrow::Cow;
use std::io::Cursor;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::{GenericImageView as _, ImageFormat, imageops::FilterType};

use crate::request::MessageContentPart;

/// Longest side, in pixels, a raster image may have before it is downscaled prior to upload.
pub const MAX_IMAGE_DIMENSION: u32 = 2000;

/// Hard ceiling, in bytes, for an image that cannot be decoded for normalization. Undecodable
/// payloads under this size are passed through (many providers accept formats our codec set does
/// not), while larger undecodable payloads are rejected with a clear diagnostic.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// One media payload loaded from disk and encoded for provider wire formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedMedia {
    pub media_type: String,
    pub base64: String,
}

/// The result of normalizing one raster image before provider upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageNormalization {
    pub media_type: String,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    pub original_bytes: usize,
    pub bytes: usize,
    pub resized: bool,
}

/// Read and base64-encode one attachment part, downscaling oversized raster images first so the
/// bytes that reach the provider are bounded.
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

    if is_normalizable_image(&media_type) {
        match normalize_image_bytes(&bytes, &media_type) {
            Ok((normalized_bytes, report)) => {
                return Ok(EncodedMedia {
                    media_type: report.media_type,
                    base64: BASE64.encode(normalized_bytes),
                });
            }
            Err(decode_err) if bytes.len() > MAX_IMAGE_BYTES => {
                return Err(format!(
                    "attachment `{path}` rejected: image is {} bytes, exceeds the {}-byte upload cap, and could not be normalized ({decode_err})",
                    bytes.len(),
                    MAX_IMAGE_BYTES
                ));
            }
            Err(_) => {}
        }
    }

    Ok(EncodedMedia {
        media_type: media_type.into_owned(),
        base64: BASE64.encode(bytes),
    })
}

/// Whether a media type names a raster image format the normalizer can decode and re-encode.
#[must_use]
pub fn is_normalizable_image(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/bmp" | "image/webp"
    )
}

/// Decode a raster image, downscale it to within [`MAX_IMAGE_DIMENSION`] when oversized, and return
/// the bytes to upload alongside a report of the change. Images already within bounds pass through
/// byte-for-byte. Undecodable inputs are rejected with a clear diagnostic.
pub fn normalize_image_bytes(
    bytes: &[u8],
    declared_media_type: &str,
) -> Result<(Vec<u8>, ImageNormalization), String> {
    let decoded =
        image::load_from_memory(bytes).map_err(|err| format!("not a decodable image ({err})"))?;
    let (original_width, original_height) = decoded.dimensions();
    if original_width == 0 || original_height == 0 {
        return Err("image has zero width or height".to_owned());
    }

    let longest = original_width.max(original_height);
    if longest <= MAX_IMAGE_DIMENSION {
        let report = ImageNormalization {
            media_type: declared_media_type.to_owned(),
            original_width,
            original_height,
            width: original_width,
            height: original_height,
            original_bytes: bytes.len(),
            bytes: bytes.len(),
            resized: false,
        };
        return Ok((bytes.to_vec(), report));
    }

    let resized = decoded.resize(
        MAX_IMAGE_DIMENSION,
        MAX_IMAGE_DIMENSION,
        FilterType::Lanczos3,
    );
    let (width, height) = resized.dimensions();
    let (format, media_type) = reencode_format(declared_media_type);
    let mut out = Cursor::new(Vec::new());
    resized
        .write_to(&mut out, format)
        .map_err(|err| format!("failed to re-encode normalized image ({err})"))?;
    let out = out.into_inner();
    let report = ImageNormalization {
        media_type: media_type.to_owned(),
        original_width,
        original_height,
        width,
        height,
        original_bytes: bytes.len(),
        bytes: out.len(),
        resized: true,
    };
    Ok((out, report))
}

/// Pick the re-encode format for a normalized image: keep PNG/JPEG, fold other raster formats into
/// lossless PNG so a single decodable payload reaches the provider.
fn reencode_format(declared_media_type: &str) -> (ImageFormat, &'static str) {
    match declared_media_type {
        "image/jpeg" | "image/jpg" => (ImageFormat::Jpeg, "image/jpeg"),
        _ => (ImageFormat::Png, "image/png"),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbImage};
    use std::io::Cursor;

    fn solid_png(width: u32, height: u32) -> Vec<u8> {
        let buffer = RgbImage::from_pixel(width, height, image::Rgb([10, 120, 220]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(buffer)
            .write_to(&mut bytes, ImageFormat::Png)
            .expect("encode png fixture");
        bytes.into_inner()
    }

    #[test]
    fn oversized_image_is_downscaled_within_bounds() {
        let original = solid_png(MAX_IMAGE_DIMENSION + 800, 1000);
        let (normalized, report) =
            normalize_image_bytes(&original, "image/png").expect("normalize oversized image");

        assert!(report.resized, "oversized image must be flagged resized");
        assert_eq!(report.original_width, MAX_IMAGE_DIMENSION + 800);
        assert!(report.width <= MAX_IMAGE_DIMENSION && report.height <= MAX_IMAGE_DIMENSION);
        assert_eq!(report.width.max(report.height), MAX_IMAGE_DIMENSION);

        let decoded = image::load_from_memory(&normalized).expect("decode normalized image");
        assert_eq!(decoded.dimensions(), (report.width, report.height));
    }

    #[test]
    fn valid_in_bounds_image_passes_through_unchanged() {
        let original = solid_png(640, 480);
        let (normalized, report) =
            normalize_image_bytes(&original, "image/png").expect("normalize small image");

        assert!(!report.resized, "in-bounds image must not be resized");
        assert_eq!((report.width, report.height), (640, 480));
        assert_eq!(
            normalized, original,
            "in-bounds image passes through byte-for-byte"
        );
    }

    #[test]
    fn corrupt_image_is_rejected_with_clear_diagnostic() {
        let err = normalize_image_bytes(b"this is not an image", "image/png")
            .expect_err("corrupt bytes must be rejected");
        assert!(
            err.contains("not a decodable image"),
            "diagnostic should explain the rejection, got: {err}"
        );
    }

    #[test]
    fn jpeg_oversized_image_reencodes_as_jpeg() {
        let buffer =
            RgbImage::from_pixel(MAX_IMAGE_DIMENSION + 400, 600, image::Rgb([200, 30, 30]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(buffer)
            .write_to(&mut bytes, ImageFormat::Jpeg)
            .expect("encode jpeg fixture");
        let original = bytes.into_inner();

        let (normalized, report) =
            normalize_image_bytes(&original, "image/jpeg").expect("normalize jpeg");
        assert_eq!(report.media_type, "image/jpeg");
        assert!(report.resized);
        assert_eq!(
            image::guess_format(&normalized).expect("guess format"),
            ImageFormat::Jpeg
        );
    }

    #[test]
    fn encode_media_part_downscales_oversized_image_attachment() {
        let dir = std::env::temp_dir().join(format!("cortex-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("big.png");
        std::fs::write(&path, solid_png(MAX_IMAGE_DIMENSION + 600, 500)).expect("write fixture");

        let part = MessageContentPart::Image {
            media_type: "image/png".to_owned(),
            path: path.to_string_lossy().into_owned(),
        };
        let encoded = encode_media_part(&part).expect("encode oversized attachment");
        let raw = BASE64.decode(encoded.base64).expect("decode base64");
        let decoded = image::load_from_memory(&raw).expect("decode normalized attachment");
        assert!(decoded.dimensions().0.max(decoded.dimensions().1) <= MAX_IMAGE_DIMENSION);

        std::fs::remove_dir_all(&dir).ok();
    }
}

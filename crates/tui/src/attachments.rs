//! Prompt attachment helpers for the TUI.
//!
//! This module owns local-file attachment validation and best-effort clipboard image capture.
//! It is deliberately pure where possible so app/turn code stays focused on UI state.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::Command;

use llm::MessageContentPart;
use protocol::{MessagePart, MessagePartBody};

/// One user-selected attachment staged in the TUI composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputAttachment {
    /// Canonical on-disk path to the attachment source.
    pub path: String,
    /// IANA media type (`image/png`, `image/jpeg`, `application/pdf`, `image/svg+xml`).
    pub media_type: String,
}

impl InputAttachment {
    /// Human-readable chip label for the composer.
    #[must_use]
    pub fn chip_label(&self) -> String {
        let name = Path::new(&self.path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(self.path.as_str());
        format!("{name} ({})", self.media_type)
    }

    /// Human-readable transcript summary line.
    #[must_use]
    pub fn transcript_label(&self) -> String {
        format!("📎 {}: {}", self.media_type, self.path)
    }

    /// Convert to the LLM request part model.
    #[must_use]
    pub fn to_message_content_part(&self) -> MessageContentPart {
        if self.media_type.starts_with("image/") {
            MessageContentPart::Image {
                media_type: self.media_type.clone(),
                path: self.path.clone(),
            }
        } else if self.media_type == "application/pdf" {
            MessageContentPart::Document {
                media_type: self.media_type.clone(),
                path: self.path.clone(),
            }
        } else {
            MessageContentPart::File {
                path: self.path.clone(),
                media_type: Some(self.media_type.clone()),
            }
        }
    }

    /// Convert to the persisted rich message-part model.
    #[must_use]
    pub fn to_message_part(&self) -> MessagePart {
        MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::File {
                path: self.path.clone(),
                media_type: Some(self.media_type.clone()),
                source: Some("tui_attachment".to_owned()),
            },
        }
    }
}

/// Validate and normalize one local file attachment path.
///
/// Accepts PNG/JPEG/PDF/SVG only (the current parity scope).
pub fn resolve_local_file_attachment(
    cwd: &Path,
    raw_path: &str,
) -> Result<InputAttachment, String> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return Err("Attachment path is empty.".to_owned());
    }

    let joined = {
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            candidate
        } else {
            cwd.join(candidate)
        }
    };

    let canonical = joined
        .canonicalize()
        .map_err(|err| format!("Could not read `{raw_path}`: {err}"))?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|err| format!("Could not inspect `{}`: {err}", canonical.display()))?;
    if !metadata.is_file() {
        return Err(format!("`{}` is not a file.", canonical.display()));
    }

    let Some(media_type) = supported_media_type_for_path(&canonical) else {
        return Err(format!(
            "Unsupported attachment type for `{}`. Supported: png, jpg, jpeg, pdf, svg.",
            canonical.display()
        ));
    };
    validate_attachment_file(&canonical, media_type)?;

    Ok(InputAttachment {
        path: canonical.to_string_lossy().into_owned(),
        media_type: media_type.to_owned(),
    })
}

/// Try to capture an image from the OS clipboard and persist it to a local attachment file.
///
/// Platform support depends on available clipboard tools:
/// - Wayland: `wl-paste`
/// - X11: `xclip`
/// - macOS: `pngpaste`
pub fn capture_clipboard_image_attachment() -> Result<InputAttachment, String> {
    let mut attempts: Vec<String> = Vec::new();

    if let Some(bytes) = capture_clipboard_stdout(
        "wl-paste",
        &["--no-newline", "--type", "image/png"],
        &mut attempts,
    ) {
        return persist_clipboard_bytes(bytes, "image/png");
    }
    if let Some(bytes) = capture_clipboard_stdout(
        "wl-paste",
        &["--no-newline", "--type", "image/jpeg"],
        &mut attempts,
    ) {
        return persist_clipboard_bytes(bytes, "image/jpeg");
    }
    if let Some(bytes) = capture_clipboard_stdout(
        "xclip",
        &["-selection", "clipboard", "-t", "image/png", "-o"],
        &mut attempts,
    ) {
        return persist_clipboard_bytes(bytes, "image/png");
    }
    if let Some(bytes) = capture_clipboard_stdout(
        "xclip",
        &["-selection", "clipboard", "-t", "image/jpeg", "-o"],
        &mut attempts,
    ) {
        return persist_clipboard_bytes(bytes, "image/jpeg");
    }
    if let Some(path) = capture_clipboard_pngpaste(&mut attempts)? {
        let media_type = supported_media_type_for_path(&path).unwrap_or("image/png");
        return Ok(InputAttachment {
            path: path.to_string_lossy().into_owned(),
            media_type: media_type.to_owned(),
        });
    }

    let mut message =
        "Could not read an image from the clipboard. Supported tools: wl-paste, xclip, pngpaste."
            .to_owned();
    if !attempts.is_empty() {
        message.push_str("\nAttempts:");
        for attempt in attempts {
            message.push_str("\n- ");
            message.push_str(&attempt);
        }
    }
    Err(message)
}

/// Recover an input attachment descriptor from a persisted message part.
#[must_use]
pub fn attachment_from_message_part(part: &MessagePart) -> Option<InputAttachment> {
    let MessagePartBody::File {
        path, media_type, ..
    } = &part.body
    else {
        return None;
    };
    let kind = media_type
        .as_deref()
        .map(Cow::Borrowed)
        .or_else(|| supported_media_type_for_path(Path::new(path)).map(Cow::Borrowed))?;
    if !is_supported_media_type(kind.as_ref()) {
        return None;
    }
    Some(InputAttachment {
        path: path.clone(),
        media_type: kind.into_owned(),
    })
}

/// Supported attachment media type for `path`, inferred by extension.
#[must_use]
pub fn supported_media_type_for_path(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.trim().to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

#[must_use]
pub fn is_supported_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "image/png" | "image/jpeg" | "image/svg+xml" | "application/pdf"
    )
}

/// Validate that `path` contains bytes matching `media_type`.
///
/// This prevents extension-only spoofing from reaching a provider and causing avoidable
/// unsupported/invalid media failures mid-turn.
pub fn validate_attachment_file(path: &Path, media_type: &str) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|err| format!("Could not read attachment `{}`: {err}", path.display()))?;
    if looks_like_media(&bytes, media_type) {
        return Ok(());
    }
    Err(format!(
        "`{}` is not a valid {} attachment.",
        path.display(),
        media_type
    ))
}

fn capture_clipboard_stdout(
    command: &str,
    args: &[&str],
    attempts: &mut Vec<String>,
) -> Option<Vec<u8>> {
    let output = match Command::new(command).args(args).output() {
        Ok(output) => output,
        Err(err) => {
            attempts.push(format!("{command} {} -> {err}", args.join(" ")));
            return None;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        attempts.push(format!(
            "{command} {} -> exit {} ({})",
            args.join(" "),
            output.status,
            stderr.trim()
        ));
        return None;
    }
    if output.stdout.is_empty() {
        attempts.push(format!("{command} {} -> empty output", args.join(" ")));
        return None;
    }
    Some(output.stdout)
}

fn capture_clipboard_pngpaste(attempts: &mut Vec<String>) -> Result<Option<PathBuf>, String> {
    let output_path = clipboard_output_path("image/png")?;
    match Command::new("pngpaste").arg(&output_path).output() {
        Ok(output) => {
            if output.status.success() {
                Ok(Some(output_path))
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                attempts.push(format!(
                    "pngpaste {} -> exit {} ({})",
                    output_path.display(),
                    output.status,
                    stderr.trim()
                ));
                let _ = std::fs::remove_file(&output_path);
                Ok(None)
            }
        }
        Err(err) => {
            attempts.push(format!("pngpaste {} -> {err}", output_path.display()));
            let _ = std::fs::remove_file(&output_path);
            Ok(None)
        }
    }
}

fn persist_clipboard_bytes(bytes: Vec<u8>, media_type: &str) -> Result<InputAttachment, String> {
    if !looks_like_media(&bytes, media_type) {
        return Err(format!("Clipboard data was not valid {media_type}."));
    }
    let path = clipboard_output_path(media_type)?;
    std::fs::write(&path, bytes).map_err(|err| {
        format!(
            "Failed to write clipboard image to {}: {err}",
            path.display()
        )
    })?;
    Ok(InputAttachment {
        path: path.to_string_lossy().into_owned(),
        media_type: media_type.to_owned(),
    })
}

fn looks_like_media(bytes: &[u8], media_type: &str) -> bool {
    match media_type {
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']),
        "image/jpeg" => bytes.len() >= 4 && bytes.starts_with(&[0xff, 0xd8]),
        "image/svg+xml" => String::from_utf8_lossy(bytes)
            .to_ascii_lowercase()
            .contains("<svg"),
        "application/pdf" => bytes.starts_with(b"%PDF-"),
        _ => !bytes.is_empty(),
    }
}

fn clipboard_output_path(media_type: &str) -> Result<PathBuf, String> {
    let dir = attachment_store_dir();
    std::fs::create_dir_all(&dir).map_err(|err| {
        format!(
            "Failed to create attachment directory {}: {err}",
            dir.display()
        )
    })?;
    let extension = match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        _ => "bin",
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Ok(dir.join(format!("clipboard-{nanos}.{extension}")))
}

fn attachment_store_dir() -> PathBuf {
    if let Some(path) = env_non_empty("CORTEX_DATA_DIR") {
        return PathBuf::from(path).join("attachments");
    }
    if let Some(path) = env_non_empty("XDG_DATA_HOME") {
        return PathBuf::from(path).join("cortex").join("attachments");
    }
    if let Some(path) = env_non_empty("HOME") {
        return PathBuf::from(path)
            .join(".local")
            .join("share")
            .join("cortex")
            .join("attachments");
    }
    std::env::temp_dir().join("cortex-attachments")
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_media_type_matches_expected_extensions() {
        assert_eq!(
            supported_media_type_for_path(Path::new("image.png")),
            Some("image/png")
        );
        assert_eq!(
            supported_media_type_for_path(Path::new("image.JPG")),
            Some("image/jpeg")
        );
        assert_eq!(
            supported_media_type_for_path(Path::new("doc.pdf")),
            Some("application/pdf")
        );
        assert_eq!(
            supported_media_type_for_path(Path::new("diagram.svg")),
            Some("image/svg+xml")
        );
        assert_eq!(supported_media_type_for_path(Path::new("note.txt")), None);
    }

    #[test]
    fn attachment_round_trips_from_file_message_part() {
        let part = MessagePart {
            id: String::new(),
            created_at: None,
            body: MessagePartBody::File {
                path: "/tmp/example.png".to_owned(),
                media_type: Some("image/png".to_owned()),
                source: None,
            },
        };
        let attachment = attachment_from_message_part(&part).expect("attachment");
        assert_eq!(attachment.path, "/tmp/example.png");
        assert_eq!(attachment.media_type, "image/png");
    }

    #[test]
    fn invalid_png_payload_is_rejected_even_with_png_extension() {
        let path = std::env::temp_dir().join(format!(
            "cortex-tui-invalid-png-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::write(&path, b"not-a-real-png").expect("write invalid payload");
        let err = validate_attachment_file(&path, "image/png").expect_err("must fail");
        assert!(err.contains("not a valid image/png"), "error: {err}");
        let _ = std::fs::remove_file(path);
    }
}

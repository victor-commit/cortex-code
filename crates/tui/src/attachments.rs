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
    /// On-disk size in bytes, measured once at attach time (0 when unreadable). Cached so the
    /// render hot path never re-stats the file.
    byte_size: u64,
    /// Card meta descriptor (`1104×720`, `12 pages`, `vector`, `document`), parsed once at attach
    /// time from the file head. Cached so the render hot path never re-reads the file.
    descriptor: String,
}

impl InputAttachment {
    /// Build an attachment for `path`/`media_type`, measuring the byte size and card descriptor
    /// once (best-effort file reads) so the render path can reuse the cached values every frame.
    #[must_use]
    pub fn new(path: String, media_type: String) -> Self {
        let byte_size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        let descriptor = compute_descriptor(&path, &media_type);
        Self {
            path,
            media_type,
            byte_size,
            descriptor,
        }
    }

    /// Human-readable chip label for the composer.
    #[must_use]
    pub fn chip_label(&self) -> String {
        let name = Path::new(&self.path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(self.path.as_str());
        format!("{name} ({})", self.media_type)
    }

    /// The attachment's file name (last path segment), used as the card title.
    #[must_use]
    pub fn file_name(&self) -> String {
        Path::new(&self.path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(self.path.as_str())
            .to_owned()
    }

    /// Short uppercase type badge shown as an inverse pill on the attachment card
    /// (`PNG`/`JPG`/`SVG`/`PDF`).
    #[must_use]
    pub fn badge_label(&self) -> &'static str {
        match self.media_type.as_str() {
            "image/png" => "PNG",
            "image/jpeg" => "JPG",
            "image/svg+xml" => "SVG",
            "application/pdf" => "PDF",
            _ => "FILE",
        }
    }

    /// The on-disk size in bytes (0 when the file was unreadable at attach time). Cached — no
    /// per-call disk access.
    #[must_use]
    pub fn byte_size(&self) -> u64 {
        self.byte_size
    }

    /// The card metadata line: a type-specific descriptor plus the human-readable size, e.g.
    /// `1104×720 · 42 KB` (image), `12 pages · 88 KB` (pdf), `vector · 4 KB` (svg). Both the
    /// descriptor and size are cached from attach time — this never touches the disk.
    #[must_use]
    pub fn meta_line(&self) -> String {
        format!(
            "{} {} {}",
            self.descriptor,
            crate::glyphs::SEPARATOR,
            human_size(self.byte_size)
        )
    }

    /// Human-readable transcript summary line.
    #[must_use]
    pub fn transcript_label(&self) -> String {
        format!("{}  {}", self.badge_label(), self.file_name())
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

/// Upper bound on the number of head bytes read to infer an attachment's descriptor (image
/// dimensions / pdf page count). Image headers sit in the first bytes; a page tree with `/Count`
/// and leaf `/Page` objects is reliably present well inside this window for typical documents.
const PREFIX_READ_CAP: usize = 256 * 1024;

/// Compute the cached card descriptor for a staged attachment from its file head (best-effort).
/// Called once at attach time — never on the render path.
fn compute_descriptor(path: &str, media_type: &str) -> String {
    match media_type {
        "image/svg+xml" => "vector".to_owned(),
        "image/png" | "image/jpeg" => read_prefix(path, PREFIX_READ_CAP)
            .and_then(|(bytes, _)| image_dimensions(media_type, &bytes))
            .map(|(w, h)| format!("{w}{}{h}", crate::glyphs::MULTIPLY))
            .unwrap_or_else(|| "image".to_owned()),
        "application/pdf" => read_prefix(path, PREFIX_READ_CAP)
            .map(|(bytes, truncated)| pdf_descriptor(&bytes, truncated))
            .unwrap_or_else(|| "document".to_owned()),
        _ => "file".to_owned(),
    }
}

/// Read at most `cap` bytes from the head of a file (bounded so descriptor parsing never pulls a
/// whole large attachment into memory). The `bool` is `true` when the file filled the cap, i.e. the
/// prefix may be truncated and a byte-scan of it could under-count.
fn read_prefix(path: &str, cap: usize) -> Option<(Vec<u8>, bool)> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buffer = Vec::new();
    file.take(cap as u64).read_to_end(&mut buffer).ok()?;
    let truncated = buffer.len() >= cap;
    Some((buffer, truncated))
}

/// Format a byte count as a compact human-readable size (`4 KB`, `88 KB`, `1.3 MB`). Sizes below a
/// kilobyte read as raw bytes; kilobytes/megabytes round to keep the card meta line short.
#[must_use]
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{} KB", bytes.div_ceil(KB))
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

/// Best-effort pixel dimensions for a raster image, dispatched on media type.
fn image_dimensions(media_type: &str, bytes: &[u8]) -> Option<(u32, u32)> {
    match media_type {
        "image/png" => png_dimensions(bytes),
        "image/jpeg" => jpeg_dimensions(bytes),
        _ => None,
    }
}

/// Parse a PNG's `IHDR` width/height (the 8-byte signature is followed by the IHDR chunk whose
/// payload starts with 4-byte big-endian width then height).
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if width == 0 || height == 0 {
        None
    } else {
        Some((width, height))
    }
}

/// Parse a JPEG's frame dimensions by walking segment markers to the first Start-Of-Frame.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        // Standalone markers (padding, SOI/EOI, restart) carry no length payload.
        if marker == 0xFF || marker == 0x01 || (0xD0..=0xD9).contains(&marker) {
            i += 2;
            continue;
        }
        let length = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        // Start-Of-Frame markers hold the frame size; DHT (C4), JPG (C8) and DAC (CC) do not.
        let is_sof =
            (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC;
        if is_sof {
            let height = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return (width > 0 && height > 0).then_some((width, height));
        }
        if length < 2 {
            return None;
        }
        i += 2 + length;
    }
    None
}

/// Best-effort PDF page-count descriptor for the card meta line.
///
/// Prefers the authoritative page-tree `/Count N` node — correct even for large or compressed PDFs
/// whose page objects fall past the read prefix or live in compressed object streams. When no
/// `/Count` node is visible, a leaf-`/Page` tally is only trusted for a fully-read prefix; a
/// truncated prefix would under-count, so it degrades to a generic `document` rather than report a
/// confidently-wrong page count.
fn pdf_descriptor(bytes: &[u8], truncated: bool) -> String {
    if let Some(n) = pdf_page_tree_count(bytes) {
        return pdf_pages_label(n);
    }
    if !truncated && let Some(n) = pdf_page_count(bytes) {
        return pdf_pages_label(n);
    }
    "document".to_owned()
}

/// Render a page count as a card descriptor (`1 page` / `N pages`).
fn pdf_pages_label(n: usize) -> String {
    if n == 1 {
        "1 page".to_owned()
    } else {
        format!("{n} pages")
    }
}

/// How far around a `/Count` token to look for a sibling `/Kids` array when deciding whether the
/// `/Count` belongs to a page-tree node (vs. an outline node, which also carries `/Count`). Sized
/// to span a `/Kids` array for a few hundred direct child pages.
const PDF_PAGE_TREE_WINDOW: usize = 4096;

/// The authoritative total page count read from a page-tree `/Count N` node, if one is visible in
/// `bytes`. A `/Count` is trusted only when a `/Kids` array sits nearby (the page-tree node's
/// defining key), which excludes outline `/Count` nodes. The maximum qualifying value is the root
/// node's total (the sum over the tree), so nested intermediate counts never win.
fn pdf_page_tree_count(bytes: &[u8]) -> Option<usize> {
    const COUNT_KEY: &[u8] = b"/Count";
    const KIDS_KEY: &[u8] = b"/Kids";
    let is_ws = |b: u8| matches!(b, b' ' | b'\r' | b'\n' | b'\t');
    let mut best: Option<usize> = None;
    let mut i = 0;
    while i + COUNT_KEY.len() <= bytes.len() {
        if &bytes[i..i + COUNT_KEY.len()] != COUNT_KEY {
            i += 1;
            continue;
        }
        let mut j = i + COUNT_KEY.len();
        while j < bytes.len() && is_ws(bytes[j]) {
            j += 1;
        }
        let digits_start = j;
        while j < bytes.len() && bytes[j].is_ascii_digit() {
            j += 1;
        }
        if j > digits_start {
            let window_start = i.saturating_sub(PDF_PAGE_TREE_WINDOW);
            let window_end = (j + PDF_PAGE_TREE_WINDOW).min(bytes.len());
            let has_kids = bytes[window_start..window_end]
                .windows(KIDS_KEY.len())
                .any(|w| w == KIDS_KEY);
            if has_kids
                && let Ok(n) = std::str::from_utf8(&bytes[digits_start..j])
                    .unwrap_or("")
                    .parse::<usize>()
            {
                best = Some(best.map_or(n, |b: usize| b.max(n)));
            }
        }
        i = j.max(i + 1);
    }
    best
}

/// Best-effort PDF page count: count leaf `/Type /Page` objects (tolerating optional whitespace),
/// excluding the `/Pages` tree node. Returns `None` when no page object is found.
fn pdf_page_count(bytes: &[u8]) -> Option<usize> {
    const TYPE_KEY: &[u8] = b"/Type";
    const PAGE: &[u8] = b"/Page";
    let is_ws = |b: u8| matches!(b, b' ' | b'\r' | b'\n' | b'\t');
    let mut count = 0usize;
    let mut i = 0;
    while i + TYPE_KEY.len() < bytes.len() {
        if &bytes[i..i + TYPE_KEY.len()] != TYPE_KEY {
            i += 1;
            continue;
        }
        let mut j = i + TYPE_KEY.len();
        while j < bytes.len() && is_ws(bytes[j]) {
            j += 1;
        }
        if j + PAGE.len() <= bytes.len() && &bytes[j..j + PAGE.len()] == PAGE {
            let after = bytes.get(j + PAGE.len()).copied();
            // Exclude `/Pages`; require a token delimiter so `/PageLayout`/`/PageMode` don't match.
            let is_leaf_page = !matches!(after, Some(b's'))
                && matches!(
                    after,
                    None | Some(b' ' | b'\r' | b'\n' | b'\t' | b'/' | b'>' | b'[')
                );
            if is_leaf_page {
                count += 1;
            }
        }
        i = j;
    }
    (count > 0).then_some(count)
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

    Ok(InputAttachment::new(
        canonical.to_string_lossy().into_owned(),
        media_type.to_owned(),
    ))
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
        return Ok(InputAttachment::new(
            path.to_string_lossy().into_owned(),
            media_type.to_owned(),
        ));
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
    Some(InputAttachment::new(path.clone(), kind.into_owned()))
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
    Ok(InputAttachment::new(
        path.to_string_lossy().into_owned(),
        media_type.to_owned(),
    ))
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

    #[test]
    fn human_size_formats_bytes_kilobytes_and_megabytes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        // 43_008 bytes = 42 KB exactly; sub-KB remainders round up so the size never reads `0 KB`.
        assert_eq!(human_size(42 * 1024), "42 KB");
        assert_eq!(human_size(42 * 1024 + 1), "43 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn badge_label_maps_media_type_to_short_uppercase_tag() {
        let badge = |mt: &str| InputAttachment::new("x".to_owned(), mt.to_owned()).badge_label();
        assert_eq!(badge("image/png"), "PNG");
        assert_eq!(badge("image/jpeg"), "JPG");
        assert_eq!(badge("image/svg+xml"), "SVG");
        assert_eq!(badge("application/pdf"), "PDF");
    }

    #[test]
    fn png_dimensions_reads_ihdr_width_and_height() {
        // A 1×1 PNG: signature + IHDR chunk (width=1, height=1).
        let png = [
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
            b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde,
        ];
        assert_eq!(png_dimensions(&png), Some((1, 1)));
        assert_eq!(png_dimensions(b"not-a-png"), None);
    }

    #[test]
    fn jpeg_dimensions_reads_first_start_of_frame() {
        // SOI, then a SOF0 segment declaring 16 (height) × 32 (width).
        let jpeg = [
            0xFF, 0xD8, // SOI
            0xFF, 0xC0, // SOF0
            0x00, 0x11, // length
            0x08, // precision
            0x00, 0x10, // height = 16
            0x00, 0x20, // width = 32
            0x03, // components
        ];
        assert_eq!(jpeg_dimensions(&jpeg), Some((32, 16)));
    }

    #[test]
    fn pdf_page_count_counts_leaf_page_objects_not_the_pages_node() {
        let pdf =
            b"%PDF-1.4\n/Type /Pages /Count 2\n/Type /Page /Contents\n/Type/Page /Contents\n/PageLayout /SinglePage";
        assert_eq!(pdf_page_count(pdf), Some(2));
        assert_eq!(pdf_page_count(b"%PDF-1.4 no page objects"), None);
    }

    #[test]
    fn pdf_page_tree_count_reads_count_next_to_kids_and_takes_the_root_total() {
        // Nested page tree: intermediate node /Count 3, root node /Count 12. The root total wins.
        let pdf = b"%PDF-1.7\n<< /Type /Pages /Kids [ 5 0 R 6 0 R ] /Count 3 >>\n\
                    << /Type /Pages /Kids [ 2 0 R 3 0 R 4 0 R ] /Count 12 >>";
        assert_eq!(pdf_page_tree_count(pdf), Some(12));
    }

    #[test]
    fn pdf_page_tree_count_ignores_outline_count_without_kids() {
        // An outline dict carries /Count but no /Kids; it must not be read as a page count.
        let pdf = b"%PDF-1.7\n<< /Type /Outlines /First 9 0 R /Last 9 0 R /Count 7 >>";
        assert_eq!(pdf_page_tree_count(pdf), None);
    }

    #[test]
    fn pdf_descriptor_prefers_count_even_when_prefix_is_truncated() {
        // A compressed/large PDF whose page objects are past the prefix: the /Count node in the
        // (truncated) head still yields the real page count, not a generic fallback.
        let pdf = b"%PDF-1.7\n<< /Type /Pages /Kids [ 2 0 R 3 0 R ] /Count 42 >>";
        assert_eq!(pdf_descriptor(pdf, true), "42 pages");
        // Singular form when the count is one.
        let one = b"%PDF-1.7\n<< /Type /Pages /Kids [ 2 0 R ] /Count 1 >>";
        assert_eq!(pdf_descriptor(one, true), "1 page");
    }

    #[test]
    fn pdf_descriptor_falls_back_to_document_when_truncated_without_count() {
        // No /Count node and the prefix hit the cap (truncated): a leaf-page scan could under-count,
        // so degrade to the generic descriptor rather than report a confidently-wrong page count.
        let pdf = b"%PDF-1.4\n/Type /Page /Contents 2 0 R";
        assert_eq!(pdf_descriptor(pdf, true), "document");
        // With the whole file present (not truncated) the leaf-page tally is trusted.
        assert_eq!(pdf_descriptor(pdf, false), "1 page");
    }

    #[test]
    fn meta_line_and_transcript_label_are_emoji_free_and_real() {
        let path = std::env::temp_dir().join(format!(
            "cortex-tui-meta-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::write(
            &path,
            [
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0x0d, b'I', b'H', b'D',
                b'R', 0, 0, 0, 0x01, 0, 0, 0, 0x01, 0x08, 0x02, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0,
                0, 0, 0x0a, b'I', b'D', b'A', b'T', 0x08, 0xd7, 0x63, 0xf8, 0x0f, 0x00, 0x01, 0x05,
                0x01, 0x02, 0x60, 0x5d, 0xe9, 0x2f, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42,
                0x60, 0x82,
            ],
        )
        .expect("write temp png");
        let attachment =
            InputAttachment::new(path.to_string_lossy().into_owned(), "image/png".to_owned());
        let meta = attachment.meta_line();
        // The 1×1 fixture yields real dimensions and a real (non-zero) byte size.
        assert!(meta.starts_with("1×1"), "meta reflects real dims: {meta}");
        assert!(
            meta.ends_with(" B") || meta.ends_with("KB"),
            "meta has size: {meta}"
        );
        let label = attachment.transcript_label();
        assert!(
            label.starts_with("PNG"),
            "label leads with the badge: {label}"
        );
        assert!(!meta.contains('📎') && !label.contains('📎'), "no emoji");
        let _ = std::fs::remove_file(path);
    }
}

//! LSP base-protocol message framing (`Content-Length` headers plus a JSON body).

use std::io::{BufRead, Write};

use crate::LspError;

/// Default cap on a single framed message body (32 MiB).
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

/// Low-level framing failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramingError {
    /// An I/O error occurred while reading the stream.
    Io,
    /// The stream ended before a full message was read.
    Eof,
    /// A header line was malformed or `Content-Length` was not a number.
    InvalidHeader,
    /// No `Content-Length` header preceded the body separator.
    MissingContentLength,
    /// The advertised body length exceeded the caller's limit.
    TooLarge,
    /// The body was not valid UTF-8.
    Utf8,
}

impl From<FramingError> for LspError {
    fn from(error: FramingError) -> Self {
        match error {
            FramingError::Eof => Self::Closed,
            FramingError::Io => Self::Io,
            FramingError::TooLarge => Self::ResponseTooLarge,
            FramingError::InvalidHeader
            | FramingError::MissingContentLength
            | FramingError::Utf8 => Self::Protocol,
        }
    }
}

/// Write `body` as a single framed LSP message.
///
/// # Errors
/// Returns the underlying I/O error when the stream cannot be written.
pub fn write_message<W: Write>(writer: &mut W, body: &[u8]) -> std::io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

/// Read a single framed LSP message body from `reader`.
///
/// # Errors
/// Returns a framing error for malformed headers, EOF, over-large bodies, or non-UTF-8 content.
pub fn read_message<R: BufRead>(reader: &mut R, max_bytes: usize) -> Result<Vec<u8>, FramingError> {
    let mut content_length: Option<usize> = None;
    let mut header = String::new();
    loop {
        header.clear();
        let read = reader
            .read_line(&mut header)
            .map_err(|_| FramingError::Io)?;
        if read == 0 {
            return Err(FramingError::Eof);
        }
        let line = header.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or(FramingError::InvalidHeader)?;
        if name.trim().eq_ignore_ascii_case("Content-Length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| FramingError::InvalidHeader)?,
            );
        }
    }

    let length = content_length.ok_or(FramingError::MissingContentLength)?;
    if length > max_bytes {
        return Err(FramingError::TooLarge);
    }

    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            FramingError::Eof
        } else {
            FramingError::Io
        }
    })?;
    if std::str::from_utf8(&body).is_err() {
        return Err(FramingError::Utf8);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn framed_messages_round_trip() {
        let body = br#"{"jsonrpc":"2.0","id":1}"#;
        let mut framed = Vec::new();
        write_message(&mut framed, body).expect("write frame");
        let mut cursor = Cursor::new(framed);
        assert_eq!(
            read_message(&mut cursor, DEFAULT_MAX_MESSAGE_BYTES),
            Ok(body.to_vec())
        );
    }

    #[test]
    fn oversized_body_is_rejected() {
        let mut framed = Vec::new();
        write_message(&mut framed, b"abcdef").expect("write frame");
        let mut cursor = Cursor::new(framed);
        assert_eq!(read_message(&mut cursor, 3), Err(FramingError::TooLarge));
    }
}

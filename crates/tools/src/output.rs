//! Bounded tool output: truncate large output to the configured limits and spill the complete
//! output to an on-disk file referenced in the result, so nothing is silently lost.
//!
//! A tool that can emit unbounded output (notably `bash`) runs its captured output through
//! [`bound_output`]. When the output exceeds [`OutputLimits`], the in-band result returned to the
//! model is truncated to the limit with an explicit notice, and the *complete* output is written to
//! a uniquely named on-disk spill file whose path is named in that notice. A consumer that needs
//! the full output reads the spill file; the model sees a bounded, clearly-truncated result.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// The default cap on the number of lines kept in the in-band result before truncation.
pub const DEFAULT_MAX_LINES: usize = 1000;

/// The default cap on the number of bytes kept in the in-band result before truncation.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024;

/// A monotonically increasing counter so concurrent spills never collide on a filename.
static SPILL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The configured limits bounding the in-band tool output (`tool_output.max_lines`/`max_bytes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputLimits {
    /// The maximum number of lines kept in the in-band result.
    pub max_lines: usize,
    /// The maximum number of bytes kept in the in-band result.
    pub max_bytes: usize,
}

impl Default for OutputLimits {
    fn default() -> Self {
        Self {
            max_lines: DEFAULT_MAX_LINES,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// The result of bounding a tool's output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedOutput {
    /// The in-band text returned to the model: the original output when within limits, otherwise
    /// the truncated head followed by a notice naming the spill file.
    pub text: String,
    /// Whether the output was truncated (and therefore spilled).
    pub truncated: bool,
    /// The on-disk file the complete output was spilled to, when truncation occurred.
    pub spill_path: Option<PathBuf>,
}

/// Bound `full` to `limits`, spilling the complete output to disk when it is exceeded.
///
/// Within limits, the output is returned unchanged with no spill. Over either limit, the complete
/// output is written to a uniquely named file under the spill directory, the in-band text is
/// truncated to `max_lines` lines (then to `max_bytes` bytes on a char boundary), and a notice
/// naming the spill file is appended. `label` is a short, filesystem-safe tag used in the spill
/// filename (e.g. `bash-stdout`).
#[must_use]
pub fn bound_output(full: &str, limits: OutputLimits, label: &str) -> BoundedOutput {
    let total_lines = full.lines().count();
    let total_bytes = full.len();

    if total_lines <= limits.max_lines && total_bytes <= limits.max_bytes {
        return BoundedOutput {
            text: full.to_owned(),
            truncated: false,
            spill_path: None,
        };
    }

    let spill_path = spill(full, label);

    let mut kept: String = full
        .lines()
        .take(limits.max_lines)
        .collect::<Vec<_>>()
        .join("\n");
    if kept.len() > limits.max_bytes {
        let mut end = limits.max_bytes;
        while end > 0 && !kept.is_char_boundary(end) {
            end -= 1;
        }
        kept.truncate(end);
    }
    let kept_lines = kept.lines().count();

    let notice = match &spill_path {
        Some(path) => format!(
            "[output truncated: showing {kept_lines} of {total_lines} lines ({} of {total_bytes} bytes); \
             full output written to {}]",
            kept.len(),
            path.display(),
        ),
        None => format!(
            "[output truncated: showing {kept_lines} of {total_lines} lines; \
             full output could not be written to a spill file]"
        ),
    };

    let text = if kept.is_empty() {
        notice
    } else {
        format!("{kept}\n{notice}")
    };

    BoundedOutput {
        text,
        truncated: true,
        spill_path,
    }
}

/// Write `full` to a uniquely named file under the spill directory, returning its path on success.
fn spill(full: &str, label: &str) -> Option<PathBuf> {
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

    #[test]
    fn output_within_limits_is_returned_verbatim_with_no_spill() {
        let limits = OutputLimits {
            max_lines: 10,
            max_bytes: 1024,
        };
        let bounded = bound_output("a\nb\nc", limits, "test-small");
        assert!(!bounded.truncated);
        assert_eq!(bounded.text, "a\nb\nc");
        assert!(bounded.spill_path.is_none());
    }

    #[test]
    fn output_over_the_line_limit_is_truncated_and_spilled_complete() {
        let limits = OutputLimits {
            max_lines: 100,
            max_bytes: 10 * 1024 * 1024,
        };
        let full: String = (1..=5000).map(|n| format!("{n}\n")).collect();
        let bounded = bound_output(&full, limits, "test-lines");

        assert!(bounded.truncated, "5000 lines must exceed the 100-line cap");
        // In-band result is bounded to the cap (plus the one-line notice).
        assert!(
            bounded.text.lines().count() <= limits.max_lines + 1,
            "in-band result must be bounded, got {} lines",
            bounded.text.lines().count()
        );
        assert!(bounded.text.contains("output truncated"));
        assert!(bounded.text.contains("of 5000 lines"));

        // The spill file exists and holds every line.
        let path = bounded.spill_path.expect("a spill file path");
        assert!(bounded.text.contains(&path.display().to_string()));
        let spilled = std::fs::read_to_string(&path).expect("read spill file");
        assert_eq!(spilled.lines().count(), 5000);
        assert!(spilled.starts_with("1\n"));
        assert!(spilled.trim_end().ends_with("5000"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn output_over_the_byte_limit_is_truncated_on_a_char_boundary() {
        let limits = OutputLimits {
            max_lines: 1_000_000,
            max_bytes: 8,
        };
        // Multibyte content so a naive byte truncation would split a char.
        let full = "héllo wörld this is long".to_owned();
        let bounded = bound_output(&full, limits, "test-bytes");
        assert!(bounded.truncated);
        // The kept prefix is valid UTF-8 (truncation respected char boundaries).
        assert!(bounded.text.is_char_boundary(0));
        let path = bounded.spill_path.expect("a spill file path");
        let spilled = std::fs::read_to_string(&path).expect("read spill file");
        assert_eq!(spilled, full);
        let _ = std::fs::remove_file(&path);
    }
}

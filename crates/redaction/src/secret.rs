//! Secret detection and the single canonical redaction placeholder.
//!
//! [`redact`] scans arbitrary text for API-key-shaped secrets (e.g. an `sk-or-v1-...` OpenRouter
//! key) and replaces each detected token with [`PLACEHOLDER`]. It is the shared chokepoint every
//! output, persisted, and log sink runs untrusted text through, so the same secret is masked with
//! the same canonical token everywhere it could surface.
//!
//! Detection is **targeted, not a blanket wipe**: only the secret token itself is replaced, and
//! the surrounding text is preserved verbatim. Ordinary values that merely resolve from an
//! environment variable or a file (and do not look like a key) are left untouched, so a config
//! dump still reports them accurately.

/// The single canonical placeholder substituted for every detected secret.
///
/// It is identical across logs, persisted data, and echoed output so consumers can recognize a
/// redaction and correlate it to the same masked value across sinks.
pub const PLACEHOLDER: &str = "[REDACTED]";

/// The shared lead-in of the recognized API-key family (`sk-or-v1-...`, `sk-ant-...`,
/// `sk-proj-...`, and bare high-entropy `sk-...` keys).
const KEY_PREFIX: &str = "sk-";

/// Vendor sub-markers (after the `sk-` lead-in) that identify an API key unambiguously, so even a
/// short fake key (e.g. `sk-or-v1-test`) is recognized regardless of its body length.
const VENDOR_MARKERS: &[&str] = &["or-v1-", "ant-", "proj-"];

/// Minimum number of body characters after a bare `sk-` lead-in for an otherwise unmarked token to
/// be treated as a high-entropy secret. Chosen high enough that ordinary `sk-`-leading words
/// (`sk-learn`) never trip detection.
const GENERIC_MIN_BODY: usize = 16;

/// Return a copy of `input` with every detected secret token replaced by [`PLACEHOLDER`].
///
/// Non-secret text (including non-secret values adjacent to a redacted one) is preserved byte for
/// byte. Re-running `redact` on already-redacted text is a no-op, so the placeholder is stable
/// across repeated passes through different sinks.
#[must_use]
pub fn redact(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    // `copied` marks the start of the verbatim run not yet flushed into `out`; it only ever lands
    // on an ASCII boundary (0, or the end of a replaced token), so every slice below is valid.
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b's' && is_candidate_start(bytes, i) {
            let end = token_end(bytes, i);
            if is_secret(&input[i..end]) {
                out.push_str(&input[copied..i]);
                out.push_str(PLACEHOLDER);
                i = end;
                copied = end;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&input[copied..]);
    out
}

/// Whether a candidate secret token may begin at byte `i`: the `sk-` lead-in must be present and
/// sit on a token boundary, so `sk-` embedded inside a larger word (`task-`, `risk-`) is ignored.
fn is_candidate_start(bytes: &[u8], i: usize) -> bool {
    if !bytes[i..].starts_with(KEY_PREFIX.as_bytes()) {
        return false;
    }
    i == 0 || !bytes[i - 1].is_ascii_alphanumeric()
}

/// The exclusive end index of the maximal key-shaped run starting at `start`.
fn token_end(bytes: &[u8], start: usize) -> usize {
    let mut j = start;
    while j < bytes.len() && is_body_byte(bytes[j]) {
        j += 1;
    }
    j
}

/// Bytes that may appear in an API key body: alphanumerics plus the `-`/`_` separators vendors use.
fn is_body_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Whether `token` (which always starts with `sk-`) is an API-key-shaped secret.
fn is_secret(token: &str) -> bool {
    let Some(body) = token.strip_prefix(KEY_PREFIX) else {
        return false;
    };
    let vendor_marked = VENDOR_MARKERS.iter().any(|marker| {
        body.strip_prefix(marker)
            .is_some_and(|rest| !rest.is_empty())
    });
    vendor_marked || body.len() >= GENERIC_MIN_BODY
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake-but-realistically-shaped OpenRouter key (`sk-or-v1-` + 64 hex), assembled from
    /// fragments so no contiguous key-shaped literal lives in source (secret scanners flag those).
    /// The assembled runtime value still exercises real detection.
    fn openrouter_key() -> String {
        let body = "deadbeef".repeat(8);
        format!("sk-{}-{}", "or-v1", body)
    }

    #[test]
    fn placeholder_is_the_canonical_token() {
        assert_eq!(PLACEHOLDER, "[REDACTED]");
    }

    #[test]
    fn redacts_an_openrouter_key_and_drops_the_raw_substring() {
        let key = openrouter_key();
        let redacted = redact(&key);
        assert_eq!(redacted, PLACEHOLDER);
        assert!(!redacted.contains("deadbeef"), "raw key substring absent");
    }

    #[test]
    fn redacts_a_short_vendor_marked_fake_key() {
        // Vendor-marked keys are recognized regardless of body length.
        assert_eq!(redact("sk-or-v1-not-a-real-key"), PLACEHOLDER);
        assert_eq!(redact("sk-ant-abc123"), PLACEHOLDER);
        assert_eq!(redact("sk-proj-xyz789"), PLACEHOLDER);
    }

    #[test]
    fn redacts_a_bare_high_entropy_key() {
        assert_eq!(redact("sk-abcdefghijklmnop0123"), PLACEHOLDER);
    }

    #[test]
    fn redaction_is_targeted_and_keeps_surrounding_text_verbatim() {
        let masked = redact(&format!("my key is {} and that is all", openrouter_key()));
        assert_eq!(masked, "my key is [REDACTED] and that is all");
    }

    #[test]
    fn masks_every_occurrence_in_a_string() {
        let key = openrouter_key();
        let masked = redact(&format!("first={key} second={key}"));
        assert_eq!(masked, "first=[REDACTED] second=[REDACTED]");
        assert!(!masked.contains("deadbeef"));
    }

    #[test]
    fn does_not_touch_ordinary_words_or_values() {
        for benign in [
            "sk-learn",
            "task-master",
            "a-risk-assessment",
            "resolved",
            "file-secret",
            "google/gemini-2.5-flash",
            "from-global",
            "cli-model",
            "",
        ] {
            assert_eq!(redact(benign), benign, "{benign:?} must be left verbatim");
        }
    }

    #[test]
    fn does_not_match_sk_embedded_inside_a_word() {
        // A vendor marker glued onto a larger word must not match; the boundary check prevents it
        // because the `sk-` here is preceded by an alphanumeric.
        assert_eq!(redact("risk-or-v1-management"), "risk-or-v1-management");
    }

    #[test]
    fn is_idempotent_on_already_redacted_text() {
        let once = redact(&format!("token {}", openrouter_key()));
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn preserves_non_ascii_text_around_a_secret() {
        let masked = redact(&format!("café {} ☕", openrouter_key()));
        assert_eq!(masked, "café [REDACTED] ☕");
    }

    #[test]
    fn redacts_a_key_embedded_in_json_like_text() {
        let json = format!(r#"{{"key":"{}","model":"gemini"}}"#, openrouter_key());
        let masked = redact(&json);
        assert_eq!(masked, r#"{"key":"[REDACTED]","model":"gemini"}"#);
    }
}

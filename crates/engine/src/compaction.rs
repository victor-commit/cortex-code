//! Context compaction and overflow recovery for the durable runner (architecture §4).
//!
//! A turn's projected history plus the new user input can grow past the model's context window —
//! either gradually as a session accumulates turns, or all at once when a single very large input
//! arrives. Two mechanisms keep the turn alive rather than crashing or hanging:
//!
//! - **`compactIfNeeded` before send:** before each provider round-trip the runner estimates the
//!   request size and, when it exceeds the input budget, [`compact_to_budget`] prunes older history
//!   (keeping a leading system message and the most recent turns) and, if a single message is still
//!   oversized, truncates its content head+tail so the trailing instruction survives.
//! - **`compactAfterOverflow` then rebuild:** if the provider still rejects the request with a
//!   context-overflow error before any assistant text, the runner shrinks the budget, re-compacts,
//!   and rebuilds the turn (a bounded number of times) rather than failing immediately.
//!
//! Token counts are estimated cheaply from character length (no tokenizer dependency); the estimate
//! is deliberately conservative and paired with the overflow-recovery retry so an under-estimate is
//! corrected by a real provider rejection rather than a hang.

use llm::{Message, Role};

/// Rough characters-per-token ratio used to estimate token counts without a tokenizer.
const CHARS_PER_TOKEN: usize = 4;

/// A small per-message token overhead (role marker + delimiters) added to each message's estimate.
const PER_MESSAGE_OVERHEAD_TOKENS: usize = 4;

/// The default model context window assumed when none is known (a conservative, common value).
pub const DEFAULT_CONTEXT_LIMIT_TOKENS: usize = 128_000;

/// A safety ceiling on the per-request input budget, independent of the model window.
///
/// Even when a model advertises a very large window, sending a near-window request makes the
/// upstream prefill slow enough to risk the transport idle timeout. Capping the input budget keeps
/// a compacted turn fast and bounded while still leaving ample room for real context. Operators who
/// want more can raise the effective window (see the CLI's `CORTEX_CONTEXT_LIMIT_TOKENS`).
pub const MAX_INPUT_BUDGET_TOKENS: usize = 300_000;

/// A floor on the input budget so a misconfiguration can never drive it to zero.
const MIN_INPUT_BUDGET_TOKENS: usize = 256;

/// The default number of most-recent messages always kept verbatim during compaction.
pub const DEFAULT_TAIL_MESSAGES: usize = 8;

/// The policy governing automatic compaction for a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionPolicy {
    /// Whether `compactIfNeeded` runs before each send. Overflow recovery runs regardless.
    pub auto: bool,
    /// The model's context window in tokens (the total input+output budget).
    pub context_limit_tokens: usize,
    /// Tokens reserved within the window for the model's own output plus estimate headroom.
    pub reserved_tokens: usize,
    /// The number of most-recent messages always kept verbatim when pruning history.
    pub tail_messages: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self::for_context_window(DEFAULT_CONTEXT_LIMIT_TOKENS, 0)
    }
}

impl CompactionPolicy {
    /// Build a policy for a model with the given context window and per-turn output limit.
    ///
    /// The input budget reserves room for the model's output (or a quarter of the window, whichever
    /// is larger) so the request leaves headroom for the reply and for token-estimate drift.
    #[must_use]
    pub fn for_context_window(context: usize, output: usize) -> Self {
        let context = if context == 0 {
            DEFAULT_CONTEXT_LIMIT_TOKENS
        } else {
            context
        };
        let reserved = output.max(context / 4);
        Self {
            auto: true,
            context_limit_tokens: context,
            reserved_tokens: reserved,
            tail_messages: DEFAULT_TAIL_MESSAGES,
        }
    }

    /// The effective per-request input budget in tokens (window minus reserve, clamped to the
    /// safety floor/ceiling).
    #[must_use]
    pub fn input_budget(&self) -> usize {
        // MIN_INPUT_BUDGET_TOKENS < MAX_INPUT_BUDGET_TOKENS by construction, so the clamp is safe.
        self.context_limit_tokens
            .saturating_sub(self.reserved_tokens)
            .clamp(MIN_INPUT_BUDGET_TOKENS, MAX_INPUT_BUDGET_TOKENS)
    }
}

/// Estimate the token count of a single text fragment (conservative, tokenizer-free).
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        chars.div_ceil(CHARS_PER_TOKEN)
    }
}

/// Estimate the token count of an assembled request's messages, including per-message overhead.
#[must_use]
pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    messages
        .iter()
        .map(|message| estimate_tokens(&message.content) + PER_MESSAGE_OVERHEAD_TOKENS)
        .sum()
}

/// Whether a provider error detail describes a context-window/context-length overflow.
///
/// Matching is case-insensitive and anchored on context/token-length phrasing so an unrelated
/// failure (auth, generic rate-limit, server error) is not misclassified as an overflow.
#[must_use]
pub fn is_context_overflow(detail: &str) -> bool {
    let d = detail.to_ascii_lowercase();
    const CONTEXT_NEEDLES: &[&str] = &[
        "context_length_exceeded",
        "context length",
        "maximum context",
        "context window",
        "context limit",
    ];
    if CONTEXT_NEEDLES.iter().any(|needle| d.contains(needle)) {
        return true;
    }
    // Token-length phrasing that is not a rate-limit (which also contains "limit"/"token").
    let token_length = d.contains("token")
        && (d.contains("exceed")
            || d.contains("too long")
            || d.contains("too large")
            || d.contains("too many")
            || d.contains("maximum number of tokens"));
    token_length && !d.contains("rate")
}

/// Compact `messages` in place so the estimated request fits within `budget` tokens, keeping a
/// leading system message and the most recent `tail` messages verbatim.
///
/// Returns `true` if anything was changed. The strategy is: (1) prune the middle history into a
/// short synthetic note when there are more than the kept head+tail messages; (2) if the request is
/// still over budget, truncate the single largest non-system message head+tail so its trailing
/// content (e.g. the user's instruction) survives.
pub fn compact_to_budget(messages: &mut Vec<Message>, tail: usize, budget: usize) -> bool {
    if estimate_messages_tokens(messages) <= budget {
        return false;
    }

    let mut changed = prune_history(messages, tail);

    if estimate_messages_tokens(messages) > budget {
        changed |= truncate_to_fit(messages, budget);
    }

    changed
}

/// Token allowance for the elision marker that [`truncate_to_tokens`] inserts, so a truncated
/// message lands at roughly its share rather than its share plus the marker.
const MARKER_TOKENS: usize = 24;

/// Drop the middle of the conversation, keeping a leading system message and the last `tail`
/// messages, and replace the dropped span with one short synthetic note. Returns `true` if any
/// messages were dropped.
fn prune_history(messages: &mut Vec<Message>, tail: usize) -> bool {
    let head = usize::from(messages.first().is_some_and(|m| m.role == Role::System));
    let total = messages.len();
    if total <= head + tail {
        return false;
    }
    let drop_count = total - head - tail;
    if drop_count == 0 {
        return false;
    }

    let note = Message::system(format!(
        "[Context compaction: {drop_count} earlier message(s) were omitted to fit the model's \
         context window. Continue from the most recent messages below.]"
    ));
    // Replace the dropped middle span with the single synthetic note.
    let _ = messages.splice(head..head + drop_count, std::iter::once(note));
    true
}

/// Truncate non-system messages so the whole request fits within `budget`, giving each non-system
/// message an equal share of the content budget left after fixed costs (system content plus
/// per-message overhead). Returns `true` if any message was truncated.
///
/// Distributing the budget (rather than only truncating the single largest message) guarantees the
/// request fits even when several recent messages are individually large.
fn truncate_to_fit(messages: &mut [Message], budget: usize) -> bool {
    let fixed: usize = messages
        .iter()
        .map(|message| {
            PER_MESSAGE_OVERHEAD_TOKENS
                + if message.role == Role::System {
                    estimate_tokens(&message.content)
                } else {
                    0
                }
        })
        .sum();
    let non_system = messages
        .iter()
        .filter(|message| message.role != Role::System)
        .count();
    if non_system == 0 {
        return false;
    }

    let available = budget.saturating_sub(fixed);
    let share = (available / non_system)
        .saturating_sub(MARKER_TOKENS)
        .max(1);

    let mut changed = false;
    for message in messages
        .iter_mut()
        .filter(|message| message.role != Role::System)
    {
        if estimate_tokens(&message.content) > share {
            let truncated = truncate_to_tokens(&message.content, share);
            if truncated.len() != message.content.len() {
                message.content = truncated;
                changed = true;
            }
        }
    }
    changed
}

/// Truncate `content` to roughly `max_tokens`, keeping a head and a (larger-weighted) tail so a
/// trailing instruction survives, with a clear elision marker between them.
fn truncate_to_tokens(content: &str, max_tokens: usize) -> String {
    let total_chars = content.chars().count();
    let budget_chars = max_tokens.saturating_mul(CHARS_PER_TOKEN);
    if total_chars <= budget_chars {
        return content.to_owned();
    }

    // Keep at least a small tail so a trailing instruction is never dropped entirely.
    let mut tail_chars = (budget_chars / 2).max(160).min(budget_chars);
    let head_chars = budget_chars - tail_chars;
    // Guard against pathological budgets where the tail floor exceeds the content.
    if head_chars + tail_chars >= total_chars {
        tail_chars = total_chars / 2;
    }
    let head_chars = budget_chars.saturating_sub(tail_chars);

    let head_end = byte_offset_for_char(content, head_chars);
    let tail_start = byte_offset_for_char(content, total_chars.saturating_sub(tail_chars));
    let omitted = total_chars.saturating_sub(head_chars + tail_chars);

    let mut out = String::with_capacity(head_end + (content.len() - tail_start) + 96);
    out.push_str(&content[..head_end]);
    out.push_str(&format!(
        "\n\n[… {omitted} characters omitted by context compaction to fit the model's context \
         window …]\n\n"
    ));
    out.push_str(&content[tail_start..]);
    out
}

/// The byte offset of the `char_index`-th character, or the string length if out of range.
fn byte_offset_for_char(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map_or_else(|| s.len(), |(byte, _)| byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(content: &str) -> Message {
        Message::user(content)
    }

    #[test]
    fn estimate_scales_with_length() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert!(estimate_tokens(&"x".repeat(4000)) >= 1000);
    }

    #[test]
    fn input_budget_reserves_and_is_bounded() {
        let policy = CompactionPolicy::for_context_window(1_048_576, 65_536);
        // The window is huge, so the safety ceiling caps the budget.
        assert_eq!(policy.input_budget(), MAX_INPUT_BUDGET_TOKENS);

        let small = CompactionPolicy::for_context_window(4_000, 0);
        // 4000 - 1000 reserve = 3000, under the ceiling.
        assert_eq!(small.input_budget(), 3_000);
    }

    #[test]
    fn context_overflow_detection_is_anchored() {
        assert!(is_context_overflow(
            "provider returned HTTP 400: This model's maximum context length is 8192 tokens"
        ));
        assert!(is_context_overflow("context_length_exceeded"));
        assert!(is_context_overflow(
            "input token count exceeds the maximum number of tokens allowed"
        ));
        assert!(is_context_overflow(
            "the prompt is too long: too many tokens"
        ));
        // Not overflow:
        assert!(!is_context_overflow(
            "provider returned HTTP 401: invalid api key"
        ));
        assert!(!is_context_overflow(
            "provider returned HTTP 500: internal error"
        ));
        assert!(!is_context_overflow(
            "rate limit exceeded: too many tokens per minute"
        ));
    }

    #[test]
    fn under_budget_is_a_no_op() {
        let mut messages = vec![Message::system("sys"), user("hi")];
        assert!(!compact_to_budget(&mut messages, 8, 10_000));
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn prunes_old_history_keeping_system_and_tail() {
        let mut messages = vec![Message::system("system prompt")];
        for i in 0..40 {
            messages.push(user(&format!("turn {i} ").repeat(200)));
        }
        let before = messages.len();
        let changed = compact_to_budget(&mut messages, 4, 1_000);
        assert!(changed, "a large history must be compacted");
        assert!(messages.len() < before, "messages were pruned");
        assert_eq!(messages[0].role, Role::System);
        assert!(
            messages[0].content.contains("system prompt"),
            "the original system message is kept"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.content.contains("Context compaction")),
            "a synthetic elision note is inserted"
        );
        // The most-recent turn is preserved verbatim somewhere in the tail.
        assert!(
            messages.iter().any(|m| m.content.contains("turn 39")),
            "the latest turn is kept in the tail"
        );
        assert!(estimate_messages_tokens(&messages) <= 1_000 + 64);
    }

    #[test]
    fn truncates_a_single_oversized_message_preserving_the_tail() {
        let padding = "lorem ipsum dolor sit amet padding line\n".repeat(5_000);
        let prompt = format!("{padding}After reading the padding above, reply with: done.");
        let mut messages = vec![Message::system("sys"), user(&prompt)];

        let changed = compact_to_budget(&mut messages, 8, 2_000);
        assert!(changed, "an oversized single input must be truncated");
        assert!(estimate_messages_tokens(&messages) <= 2_000 + 64);

        let user_msg = messages
            .iter()
            .find(|m| m.role == Role::User)
            .expect("the user message survives");
        assert!(
            user_msg.content.contains("done."),
            "the trailing instruction must survive truncation"
        );
        assert!(
            user_msg.content.contains("omitted by context compaction"),
            "an elision marker is present"
        );
        assert!(
            user_msg.content.contains("lorem ipsum"),
            "a head sample of the padding is kept"
        );
    }

    #[test]
    fn truncation_is_char_boundary_safe_for_multibyte_content() {
        let prompt = "✨".repeat(10_000);
        let mut messages = vec![user(&prompt)];
        let changed = compact_to_budget(&mut messages, 8, 500);
        assert!(changed);
        // Must remain valid UTF-8 (no panic, content is a well-formed String).
        assert!(
            messages[0]
                .content
                .contains("omitted by context compaction")
        );
    }
}

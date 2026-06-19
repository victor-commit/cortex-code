//! The runner's normalized output vocabulary and turn outcome.
//!
//! The durable runner consumes the provider's per-call [`llm::LlmEvent`] streams and re-emits a
//! single, turn-level [`RunnerEvent`] stream (architecture §4: "one normalized event vocabulary").
//! The invariant the runner guarantees and the CLI relies on (VAL-RUN-007):
//!
//! - text deltas arrive in generation order, so concatenating every [`RunnerEvent::TextDelta`]
//!   reconstructs the final answer;
//! - the per-turn [`RunnerEvent::Usage`] (summed across continuation round-trips) and exactly one
//!   terminal [`RunnerEvent::Done`] arrive after the text, with no duplicate terminal event.

use errors::{Diagnostic, Exit};
use llm::{FinishReason, Message, Usage};
use serde::{Deserialize, Serialize};

/// One normalized event emitted by the runner for a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunnerEvent {
    /// A fragment of assistant-visible text; concatenating all of them yields the final answer.
    TextDelta {
        /// The text fragment.
        text: String,
    },
    /// A fragment of model reasoning/thinking, kept distinct from the visible answer text.
    Reasoning {
        /// The reasoning fragment.
        text: String,
    },
    /// The per-turn token usage, summed across every provider round-trip in the turn.
    Usage(Usage),
    /// A normalized error surfaced during the turn (setup failure or mid-stream provider error).
    Error {
        /// Human-readable, secret-scrubbed detail.
        message: String,
        /// The stable error class (see [`errors::Diagnostic::kind`]).
        kind: String,
    },
    /// The single terminal event of the turn, carrying why the turn ended and how many provider
    /// round-trips (steps) it took. The step count is always within the runner's `MAX_STEPS`
    /// budget, so a machine consumer can observe a bounded iteration count (VAL-RUN-003).
    Done {
        /// Why the turn ended.
        reason: FinishReason,
        /// How many provider round-trips (steps) the turn took (`<= MAX_STEPS`).
        steps: usize,
    },
}

/// A normalized turn failure, wrapping the canonical [`Diagnostic`] for exit-code mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunError {
    diagnostic: Diagnostic,
}

impl RunError {
    /// Wrap a canonical diagnostic.
    #[must_use]
    pub fn new(diagnostic: Diagnostic) -> Self {
        Self { diagnostic }
    }

    /// Reconstruct a [`RunError`] from a mid-stream error event's `kind`/`message`.
    ///
    /// The `kind` strings are the stable [`Diagnostic::kind`] identifiers; an unrecognized kind
    /// folds into the upstream class so a turn failure always maps to a concrete diagnostic.
    #[must_use]
    pub fn from_kind(kind: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        let diagnostic = match kind {
            "net-disabled" => Diagnostic::net_disabled(message),
            "no-credential" => Diagnostic::no_credential(message),
            "transport-unavailable" => Diagnostic::transport_unavailable(message),
            "cassette-missing" => Diagnostic::cassette_missing(message),
            "step-limit-exceeded" => Diagnostic::step_limit_exceeded(message),
            "context-overflow" => Diagnostic::context_overflow(message),
            _ => Diagnostic::upstream(message),
        };
        Self { diagnostic }
    }

    /// The underlying canonical diagnostic.
    #[must_use]
    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    /// The stable error-class identifier.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        self.diagnostic.kind()
    }

    /// The process exit code this failure maps to.
    #[must_use]
    pub fn exit(&self) -> Exit {
        self.diagnostic.exit()
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.diagnostic)
    }
}

impl std::error::Error for RunError {}

/// The outcome of a completed (or failed) turn.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutcome {
    /// The assembled assistant-role message: the full answer text accumulated across round-trips.
    pub message: Message,
    /// The summed token usage for the turn, when the provider reported any.
    pub usage: Option<Usage>,
    /// Why the turn ended.
    pub finish_reason: FinishReason,
    /// How many provider round-trips (steps) the turn took.
    pub steps: usize,
    /// The normalized failure, when the turn ended in an error.
    pub error: Option<RunError>,
}

impl RunOutcome {
    /// The final assistant answer text.
    #[must_use]
    pub fn answer(&self) -> &str {
        &self.message.content
    }

    /// Whether the turn ended without an error.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.error.is_none()
    }
}

/// Sum two usage records (used to accumulate usage across continuation round-trips).
#[must_use]
pub(crate) fn accumulate_usage(acc: Option<Usage>, next: Usage) -> Usage {
    match acc {
        None => next,
        Some(prev) => Usage {
            input_tokens: prev.input_tokens + next.input_tokens,
            output_tokens: prev.output_tokens + next.output_tokens,
            total_tokens: prev.total_tokens + next.total_tokens,
            cost: match (prev.cost, next.cost) {
                (Some(a), Some(b)) => Some(a + b),
                (Some(a), None) => Some(a),
                (None, b) => b,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_event_serializes_with_a_type_discriminator() {
        let value = serde_json::to_value(RunnerEvent::TextDelta {
            text: "hi".to_owned(),
        })
        .expect("serialize");
        assert_eq!(value["type"], "text_delta");
        assert_eq!(value["text"], "hi");

        let done = serde_json::to_value(RunnerEvent::Done {
            reason: FinishReason::Stop,
            steps: 1,
        })
        .expect("serialize");
        assert_eq!(done["type"], "done");
        assert_eq!(done["reason"], "stop");
        assert_eq!(done["steps"], 1);
    }

    #[test]
    fn from_kind_maps_to_distinct_diagnostics() {
        assert_eq!(
            RunError::from_kind("no-credential", "openrouter").kind(),
            "no-credential"
        );
        assert_eq!(
            RunError::from_kind("net-disabled", "op").exit().code(),
            4,
            "net-disabled keeps its dedicated exit code"
        );
        assert_eq!(
            RunError::from_kind("anything-else", "boom").kind(),
            "upstream",
            "unknown kinds fold into upstream"
        );
    }

    #[test]
    fn accumulate_usage_sums_tokens_and_cost() {
        let a = Usage {
            input_tokens: 3,
            output_tokens: 4,
            total_tokens: 7,
            cost: Some(0.001),
        };
        let b = Usage {
            input_tokens: 5,
            output_tokens: 6,
            total_tokens: 11,
            cost: Some(0.002),
        };
        let summed = accumulate_usage(Some(a), b);
        assert_eq!(summed.input_tokens, 8);
        assert_eq!(summed.output_tokens, 10);
        assert_eq!(summed.total_tokens, 18);
        assert_eq!(summed.cost, Some(0.003));
    }

    #[test]
    fn accumulate_usage_seeds_from_none() {
        let next = Usage {
            input_tokens: 1,
            output_tokens: 2,
            total_tokens: 3,
            cost: None,
        };
        assert_eq!(accumulate_usage(None, next.clone()), next);
    }
}

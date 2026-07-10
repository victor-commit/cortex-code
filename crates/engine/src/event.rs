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
use llm::{FinishReason, Message, ToolDef, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Metadata captured when the runner compacts context through an LLM-authored summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionRecord {
    /// The model-authored summary injected into the compacted context.
    pub summary: String,
    /// The stable tail-boundary token identifying where preserved tail context begins.
    pub tail_start_id: String,
    /// Whether the original in-flight user request was auto-continued after compaction.
    pub continued_request: bool,
    /// The model id used for compaction summarization, when explicitly selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl CompactionRecord {
    /// The JSON shape handed to plugin compaction hooks (`experimental.session.compacting` and
    /// `experimental.compaction.autocontinue`). `model_id` is the turn's active model, used as the
    /// fallback when no explicit summarization model was selected. This is the single seam every
    /// surface (headless `run`, server turn) converts through so a plugin-transformed compaction is
    /// identical across surfaces.
    #[must_use]
    pub fn to_hook_value(&self, model_id: &str) -> JsonValue {
        serde_json::json!({
            "summary": self.summary,
            "tail_start_id": self.tail_start_id,
            "continued_request": self.continued_request,
            "model": self.model.as_deref().unwrap_or(model_id),
        })
    }

    /// Fold a (possibly plugin-transformed) hook value back into this record. Recognized aliases
    /// mirror the documented hook output keys; unknown or non-object values leave the field intact,
    /// so a no-op hook never corrupts the compaction metadata.
    pub fn apply_hook_value(&mut self, value: &JsonValue) {
        let Some(map) = value.as_object() else {
            return;
        };
        if let Some(summary) = map
            .get("summary")
            .or_else(|| map.get("content"))
            .or_else(|| map.get("text"))
            .and_then(JsonValue::as_str)
        {
            self.summary = summary.to_owned();
        }
        if let Some(tail_start_id) = map
            .get("tail_start_id")
            .or_else(|| map.get("tailStartId"))
            .and_then(JsonValue::as_str)
        {
            self.tail_start_id = tail_start_id.to_owned();
        }
        if let Some(continued_request) = map
            .get("continued_request")
            .or_else(|| map.get("autocontinue"))
            .or_else(|| map.get("continue"))
            .and_then(JsonValue::as_bool)
        {
            self.continued_request = continued_request;
        }
        if let Some(model) = map
            .get("model")
            .or_else(|| map.get("small_model"))
            .and_then(JsonValue::as_str)
        {
            self.model = Some(model.to_owned());
        }
    }
}

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
        /// Optional provider-specific metadata for reasoning replay (for example signatures).
        #[serde(skip_serializing_if = "Option::is_none")]
        metadata: Option<JsonValue>,
    },
    /// A tool the model requested, surfaced before it is executed so a machine consumer can observe
    /// the call (name and assembled JSON arguments) that produced the following [`RunnerEvent::ToolResult`].
    ToolCall {
        /// The provider-assigned call id, echoed by the matching result.
        id: String,
        /// The tool/function name invoked.
        name: String,
        /// The complete arguments as a JSON-encoded string.
        arguments: String,
    },
    /// The result of executing a [`RunnerEvent::ToolCall`], carried as structured parts (text plus
    /// any media attachments) so non-text payloads never leak raw bytes into the text channel.
    ToolResult {
        /// The [`RunnerEvent::ToolCall`] id this result answers.
        id: String,
        /// The tool/function name that produced this result.
        name: String,
        /// The structured result parts (text and/or attachment references).
        parts: Vec<protocol::ToolResultPart>,
        /// The complete source MCP result when this tool call was routed to an MCP capability.
        ///
        /// `parts` remains the bounded model-facing projection; this field is the independently
        /// persisted machine-facing representation.
        #[serde(skip_serializing_if = "Option::is_none")]
        mcp_result: Option<protocol::McpToolResult>,
        /// Whether the tool reported an error (the turn still continues).
        is_error: bool,
    },
    /// The advertised tool set changed between provider round-trips.
    ///
    /// Surfaces such as `cortex run --jsonl` use this event to expose live MCP
    /// `notifications/tools/list_changed` add/remove updates without waiting for a restart.
    ToolSetChanged {
        /// The current tool names in advertised order.
        tools: Vec<String>,
        /// The full current tool definitions, including descriptions and JSON schemas.
        tool_definitions: Vec<ToolDef>,
    },
    /// The per-turn token usage, summed across every provider round-trip in the turn.
    Usage(Usage),
    /// A retry lifecycle event emitted when the runner classifies a setup failure as transient
    /// and schedules another full-turn attempt after a bounded delay.
    Retry {
        /// 1-based retry attempt number about to run.
        attempt: usize,
        /// The configured maximum number of retry attempts.
        max_attempts: usize,
        /// Delay before the next attempt, in milliseconds.
        delay_ms: u64,
        /// The stable error class that triggered the retry (for example `upstream`).
        error_kind: String,
        /// Stable retry classification (`rate_limit`, `service_unavailable`, ...).
        classification: String,
        /// Human-readable, secret-scrubbed detail from the triggering failure.
        message: String,
    },
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
            "schema-validation" => Diagnostic::schema_validation(message),
            "doom-loop" => Diagnostic::doom_loop(message),
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
    /// Compaction summaries generated during this turn, in occurrence order.
    pub compactions: Vec<CompactionRecord>,
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
            cache_creation_input_tokens: match (
                prev.cache_creation_input_tokens,
                next.cache_creation_input_tokens,
            ) {
                (Some(left), Some(right)) => Some(left.saturating_add(right)),
                (Some(left), None) => Some(left),
                (None, right) => right,
            },
            cache_read_input_tokens: match (
                prev.cache_read_input_tokens,
                next.cache_read_input_tokens,
            ) {
                (Some(left), Some(right)) => Some(left.saturating_add(right)),
                (Some(left), None) => Some(left),
                (None, right) => right,
            },
            reasoning_output_tokens: match (
                prev.reasoning_output_tokens,
                next.reasoning_output_tokens,
            ) {
                (Some(left), Some(right)) => Some(left.saturating_add(right)),
                (Some(left), None) => Some(left),
                (None, right) => right,
            },
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

        let tools = serde_json::to_value(RunnerEvent::ToolSetChanged {
            tools: vec!["read".to_owned(), "api_extra".to_owned()],
            tool_definitions: vec![ToolDef {
                name: "api_extra".to_owned(),
                description: "Runtime MCP tool".to_owned(),
                parameters: serde_json::json!({ "type": "object" }),
            }],
        })
        .expect("serialize");
        assert_eq!(tools["type"], "tool_set_changed");
        assert_eq!(tools["tools"], serde_json::json!(["read", "api_extra"]));
        assert_eq!(tools["tool_definitions"][0]["name"], "api_extra");

        let retry = serde_json::to_value(RunnerEvent::Retry {
            attempt: 1,
            max_attempts: 3,
            delay_ms: 250,
            error_kind: "upstream".to_owned(),
            classification: "service_unavailable".to_owned(),
            message: "upstream failure".to_owned(),
        })
        .expect("serialize");
        assert_eq!(retry["type"], "retry");
        assert_eq!(retry["attempt"], 1);
        assert_eq!(retry["max_attempts"], 3);
        assert_eq!(retry["delay_ms"], 250);
        assert_eq!(retry["error_kind"], "upstream");
        assert_eq!(retry["classification"], "service_unavailable");
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
        assert_eq!(
            RunError::from_kind("schema-validation", "mismatch").kind(),
            "schema-validation"
        );
        assert_eq!(
            RunError::from_kind("doom-loop", "repeated").kind(),
            "doom-loop"
        );
    }

    #[test]
    fn accumulate_usage_sums_tokens_and_cost() {
        let a = Usage {
            input_tokens: 3,
            output_tokens: 4,
            total_tokens: 7,
            cache_creation_input_tokens: Some(2),
            cache_read_input_tokens: None,
            reasoning_output_tokens: Some(1),
            cost: Some(0.001),
        };
        let b = Usage {
            input_tokens: 5,
            output_tokens: 6,
            total_tokens: 11,
            cache_creation_input_tokens: Some(1),
            cache_read_input_tokens: Some(4),
            reasoning_output_tokens: Some(2),
            cost: Some(0.002),
        };
        let summed = accumulate_usage(Some(a), b);
        assert_eq!(summed.input_tokens, 8);
        assert_eq!(summed.output_tokens, 10);
        assert_eq!(summed.total_tokens, 18);
        assert_eq!(summed.cache_creation_input_tokens, Some(3));
        assert_eq!(summed.cache_read_input_tokens, Some(4));
        assert_eq!(summed.reasoning_output_tokens, Some(3));
        assert_eq!(summed.cost, Some(0.003));
    }

    #[test]
    fn accumulate_usage_seeds_from_none() {
        let next = Usage {
            input_tokens: 1,
            output_tokens: 2,
            total_tokens: 3,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            reasoning_output_tokens: None,
            cost: None,
        };
        assert_eq!(accumulate_usage(None, next.clone()), next);
    }

    #[test]
    fn compaction_record_round_trips_through_the_hook_value() {
        let record = CompactionRecord {
            summary: "kept BLUEBIRD".to_owned(),
            tail_start_id: "tail-7".to_owned(),
            continued_request: true,
            model: None,
        };
        let value = record.to_hook_value("google/gemini-2.5-flash");
        assert_eq!(value["summary"], "kept BLUEBIRD");
        assert_eq!(value["tail_start_id"], "tail-7");
        assert_eq!(value["continued_request"], true);
        // No explicit summarization model falls back to the active turn model.
        assert_eq!(value["model"], "google/gemini-2.5-flash");

        // Applying the unchanged value back is a faithful round trip, except the active model is now
        // recorded (mirroring the headless persistence path).
        let mut applied = record.clone();
        applied.apply_hook_value(&value);
        assert_eq!(applied.summary, "kept BLUEBIRD");
        assert_eq!(applied.tail_start_id, "tail-7");
        assert!(applied.continued_request);
        assert_eq!(applied.model.as_deref(), Some("google/gemini-2.5-flash"));
    }

    #[test]
    fn apply_hook_value_folds_plugin_transformations_and_aliases() {
        let mut record = CompactionRecord {
            summary: "original".to_owned(),
            tail_start_id: "tail-1".to_owned(),
            continued_request: false,
            model: Some("m".to_owned()),
        };
        // A plugin rewrites the summary and flips continuation; aliases are accepted.
        record.apply_hook_value(&serde_json::json!({
            "summary": "HOOKED:original",
            "autocontinue": true,
            "small_model": "plugin/small",
        }));
        assert_eq!(record.summary, "HOOKED:original");
        assert!(record.continued_request);
        assert_eq!(record.tail_start_id, "tail-1");
        assert_eq!(record.model.as_deref(), Some("plugin/small"));

        // A non-object value is ignored, leaving the record intact.
        let before = record.clone();
        record.apply_hook_value(&serde_json::json!("not-an-object"));
        assert_eq!(record, before);
    }
}

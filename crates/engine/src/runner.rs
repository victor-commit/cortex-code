//! The durable session runner: the outer activity loop with the inner per-step loop.
//!
//! Blueprint: architecture §4. A turn is assembled from the system-context epoch, the projected
//! history, and the new user input, then driven through a bounded loop of streamed provider
//! round-trips:
//!
//! 1. Build the provider [`LlmRequest`] from the working messages.
//! 2. Execute a single streamed turn, consuming the normalized [`LlmEvent`] stream: forward text
//!    and reasoning deltas to the sink, accumulate the assistant text, collect any tool calls,
//!    sum usage, and record the finish reason.
//! 3. Assemble the streamed deltas into one assistant-role message and append it to the working
//!    history.
//! 4. If the assistant requested tool calls, carry the turn forward (continuation); otherwise the
//!    turn is complete.
//!
//! Across every round-trip the runner emits ONE normalized event vocabulary: text/reasoning deltas
//! in order, then a single summed [`RunnerEvent::Usage`], then exactly one terminal
//! [`RunnerEvent::Done`] — never a per-round-trip terminal event (VAL-RUN-002 / VAL-RUN-007).
//!
//! Tool execution is not part of M1: no tools are materialized into the request, so a well-formed
//! turn completes in a single round-trip. The continuation path is exercised by the offline tests
//! and is the seam M2 fills with a real tool registry.

use errors::Diagnostic;
use futures_util::StreamExt;
use llm::{LlmEvent, LlmRequest, Message, Provider, Role, ToolCall};

use crate::event::{RunError, RunOutcome, RunnerEvent, accumulate_usage};

/// The built-in step budget for a single turn (architecture §4, `MAX_STEPS`).
pub const DEFAULT_MAX_STEPS: usize = 25;

/// The placeholder result injected for a tool call when no tool registry is available (M1).
const TOOL_UNAVAILABLE: &str =
    "Tool execution is not available in this build; answer directly without using tools.";

/// A single turn to run: the assembled system context, projected history, and new user input.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// The fully-qualified model id (e.g. `google/gemini-2.5-flash`).
    pub model_id: String,
    /// The rendered system-context epoch (base prompt + environment block). Empty omits it.
    pub system: String,
    /// The projected prior-turn messages, in order.
    pub history: Vec<Message>,
    /// The new user input for this turn.
    pub user_input: String,
    /// Optional cap on generated tokens per round-trip.
    pub max_tokens: Option<u32>,
    /// Optional sampling temperature.
    pub temperature: Option<f32>,
}

impl Turn {
    /// Build a turn from a model id, rendered system context, and user input, with no history.
    #[must_use]
    pub fn new(
        model_id: impl Into<String>,
        system: impl Into<String>,
        user_input: impl Into<String>,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            system: system.into(),
            history: Vec::new(),
            user_input: user_input.into(),
            max_tokens: None,
            temperature: None,
        }
    }

    /// Set the projected history (builder style).
    #[must_use]
    pub fn with_history(mut self, history: Vec<Message>) -> Self {
        self.history = history;
        self
    }

    /// Set the per-round-trip max-tokens cap (builder style).
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set the sampling temperature (builder style).
    #[must_use]
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }
}

/// The durable session runner.
#[derive(Debug, Clone, Copy)]
pub struct SessionRunner {
    max_steps: usize,
}

impl Default for SessionRunner {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_STEPS)
    }
}

impl SessionRunner {
    /// Build a runner with an explicit step budget.
    #[must_use]
    pub fn new(max_steps: usize) -> Self {
        Self {
            max_steps: max_steps.max(1),
        }
    }

    /// The configured step budget.
    #[must_use]
    pub fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Run a turn against `provider`, streaming normalized events to `sink` and returning the
    /// assembled [`RunOutcome`].
    ///
    /// `sink` observes the turn's normalized event vocabulary in order. The returned outcome
    /// carries the assembled assistant message (role [`Role::Assistant`]), the summed usage, the
    /// finish reason, the number of round-trips taken, and any normalized error.
    pub async fn run<F>(&self, provider: &dyn Provider, turn: Turn, mut sink: F) -> RunOutcome
    where
        F: FnMut(RunnerEvent),
    {
        let mut messages = Vec::new();
        if !turn.system.is_empty() {
            messages.push(Message::system(turn.system.clone()));
        }
        messages.extend(turn.history.iter().cloned());
        messages.push(Message::user(turn.user_input.clone()));

        let mut answer = String::new();
        let mut total_usage = None;
        let mut steps = 0usize;
        let mut error = None;

        let finish_reason = loop {
            if steps >= self.max_steps {
                // The turn still needs another round-trip but the step budget is spent. Surface an
                // explicit, non-crashing step-limit signal (an Error event + a normalized error)
                // and terminate rather than looping past MAX_STEPS (architecture §4 / VAL-RUN-003).
                let diagnostic = Diagnostic::step_limit_exceeded(format!(
                    "reached the maximum of {} steps in a single turn without completing",
                    self.max_steps
                ));
                sink(RunnerEvent::Error {
                    message: diagnostic.to_string(),
                    kind: diagnostic.kind().to_owned(),
                });
                error = Some(RunError::new(diagnostic));
                break llm::FinishReason::Error;
            }
            steps += 1;

            let request = self.build_request(&turn, &messages);
            let mut stream = match provider.stream(request).await {
                Ok(stream) => stream,
                Err(err) => {
                    let diagnostic = err.to_diagnostic();
                    sink(RunnerEvent::Error {
                        message: diagnostic.to_string(),
                        kind: diagnostic.kind().to_owned(),
                    });
                    error = Some(RunError::new(diagnostic));
                    break llm::FinishReason::Error;
                }
            };

            let mut round = RoundState::new();
            while let Some(event) = stream.next().await {
                match event {
                    LlmEvent::TextDelta { text } => {
                        if !text.is_empty() {
                            answer.push_str(&text);
                            round.text.push_str(&text);
                            sink(RunnerEvent::TextDelta { text });
                        }
                    }
                    LlmEvent::Reasoning { text } => {
                        if !text.is_empty() {
                            sink(RunnerEvent::Reasoning { text });
                        }
                    }
                    LlmEvent::ToolCall(call) => round.tool_calls.push(call),
                    LlmEvent::ToolResult(_) => {}
                    LlmEvent::Usage(usage) => {
                        total_usage = Some(accumulate_usage(total_usage.take(), usage));
                    }
                    LlmEvent::Error { message, kind } => {
                        sink(RunnerEvent::Error {
                            message: message.clone(),
                            kind: kind.clone(),
                        });
                        error = Some(RunError::from_kind(&kind, message));
                        round.reason = llm::FinishReason::Error;
                    }
                    LlmEvent::Done { reason } => round.reason = reason,
                }
            }

            messages.push(Message {
                role: Role::Assistant,
                content: round.text,
                tool_call_id: None,
                tool_calls: round.tool_calls.clone(),
            });

            if error.is_some() || round.tool_calls.is_empty() {
                break round.reason;
            }

            // M1 has no tool registry: fail each requested tool with a clear result and carry the
            // turn forward so the model can recover (continuation). M2 replaces this with real
            // tool execution.
            for call in &round.tool_calls {
                messages.push(Message::tool_result(call.id.clone(), TOOL_UNAVAILABLE));
            }
        };

        if let Some(usage) = total_usage.clone() {
            sink(RunnerEvent::Usage(usage));
        }
        sink(RunnerEvent::Done {
            reason: finish_reason.clone(),
            steps,
        });

        RunOutcome {
            message: Message::assistant(answer),
            usage: total_usage,
            finish_reason,
            steps,
            error,
        }
    }

    /// Build the provider request for one round-trip from the current working messages.
    fn build_request(&self, turn: &Turn, messages: &[Message]) -> LlmRequest {
        let mut request = LlmRequest::new(&turn.model_id);
        request.messages = messages.to_vec();
        request.max_tokens = turn.max_tokens;
        request.temperature = turn.temperature;
        request
    }
}

/// Mutable per-round-trip accumulation while consuming one provider stream.
struct RoundState {
    text: String,
    tool_calls: Vec<ToolCall>,
    reason: llm::FinishReason,
}

impl RoundState {
    /// A fresh round seeded with a natural-stop reason, overwritten by the stream's terminal event.
    fn new() -> Self {
        Self {
            text: String::new(),
            tool_calls: Vec::new(),
            reason: llm::FinishReason::Stop,
        }
    }
}

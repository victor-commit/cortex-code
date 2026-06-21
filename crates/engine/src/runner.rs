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
use llm::{LlmEvent, LlmRequest, Message, Provider, Role, ToolCall, ToolDef};

use crate::compaction::{self, CompactionPolicy};
use crate::event::{RunError, RunOutcome, RunnerEvent, accumulate_usage};
use crate::tools::ToolDispatcher;

/// The built-in step budget for a single turn (architecture §4, `MAX_STEPS`).
pub const DEFAULT_MAX_STEPS: usize = 25;

/// The maximum number of context-overflow recoveries (compact-and-rebuild) attempted in one turn
/// before the runner gives up with a clear context-overflow diagnostic rather than looping forever.
const MAX_OVERFLOW_RECOVERIES: usize = 4;

/// The smallest input budget an overflow recovery will shrink to before giving up.
const MIN_RECOVERY_BUDGET_TOKENS: usize = 512;

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
    /// The context-compaction policy applied before each send and on overflow recovery.
    pub compaction: CompactionPolicy,
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
            compaction: CompactionPolicy::default(),
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

    /// Set the context-compaction policy (builder style).
    #[must_use]
    pub fn with_compaction(mut self, compaction: CompactionPolicy) -> Self {
        self.compaction = compaction;
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
    ///
    /// No tools are advertised on this path: a tool call the model nonetheless makes is answered
    /// with a "tool execution unavailable" result so the turn still progresses. Use
    /// [`run_with_tools`](SessionRunner::run_with_tools) to materialize and execute a real tool set.
    pub async fn run<F>(&self, provider: &dyn Provider, turn: Turn, sink: F) -> RunOutcome
    where
        F: FnMut(RunnerEvent),
    {
        self.run_inner(provider, turn, None, sink).await
    }

    /// Run a turn with a real tool registry materialized into the request and executed via
    /// `dispatcher`.
    ///
    /// The dispatcher's [`definitions`](ToolDispatcher::definitions) are advertised to the model;
    /// each tool call the model makes is surfaced as a [`RunnerEvent::ToolCall`], executed via
    /// [`dispatch`](ToolDispatcher::dispatch), surfaced as a [`RunnerEvent::ToolResult`], and
    /// re-injected as a tool-result message so the turn is carried forward (architecture §4).
    pub async fn run_with_tools<F>(
        &self,
        provider: &dyn Provider,
        turn: Turn,
        dispatcher: &dyn ToolDispatcher,
        sink: F,
    ) -> RunOutcome
    where
        F: FnMut(RunnerEvent),
    {
        self.run_inner(provider, turn, Some(dispatcher), sink).await
    }

    async fn run_inner<F>(
        &self,
        provider: &dyn Provider,
        turn: Turn,
        dispatcher: Option<&dyn ToolDispatcher>,
        mut sink: F,
    ) -> RunOutcome
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
        let mut last_advertised_tool_defs: Option<Vec<ToolDef>> = None;

        // Compaction state: the per-request input budget and the number of recent messages kept
        // verbatim. An overflow recovery shrinks both and rebuilds the turn (architecture §4).
        let policy = turn.compaction;
        let mut input_budget = policy.input_budget();
        let mut tail_messages = policy.tail_messages;
        let mut overflow_recoveries = 0usize;

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

            // compactIfNeeded before send: prune/truncate the working messages to the input budget
            // so an over-large request is shrunk before it ever reaches the provider.
            if policy.auto {
                compaction::compact_to_budget(&mut messages, tail_messages, input_budget);
            }

            // Refresh the advertised tool catalogue immediately before every provider request.
            // MCP servers can send `notifications/tools/list_changed` while a run is already in
            // progress; polling the dispatcher here lets the next continuation request see added
            // and removed tools without restarting cortex.
            let tool_defs: Vec<ToolDef> = dispatcher
                .map(ToolDispatcher::definitions)
                .unwrap_or_default();
            if let Some(previous) = &last_advertised_tool_defs
                && previous != &tool_defs
            {
                sink(RunnerEvent::ToolSetChanged {
                    tools: tool_defs.iter().map(|def| def.name.clone()).collect(),
                    tool_definitions: tool_defs.clone(),
                });
            }
            last_advertised_tool_defs = Some(tool_defs.clone());

            let request = self.build_request(&turn, &messages, &tool_defs);
            let mut stream = match provider.stream(request).await {
                Ok(stream) => stream,
                Err(err) => {
                    // compactAfterOverflow then rebuild: a context-overflow provider error before
                    // any assistant text shrinks the budget, re-compacts, and retries the turn
                    // (bounded by MAX_OVERFLOW_RECOVERIES) rather than failing immediately.
                    if answer.is_empty()
                        && compaction::is_context_overflow(err.detail())
                        && overflow_recoveries < MAX_OVERFLOW_RECOVERIES
                        && input_budget > MIN_RECOVERY_BUDGET_TOKENS
                    {
                        overflow_recoveries += 1;
                        input_budget = (input_budget / 2).max(MIN_RECOVERY_BUDGET_TOKENS);
                        tail_messages = (tail_messages / 2).max(1);
                        compaction::compact_to_budget(&mut messages, tail_messages, input_budget);
                        steps -= 1;
                        continue;
                    }

                    let diagnostic = if compaction::is_context_overflow(err.detail()) {
                        Diagnostic::context_overflow(err.detail())
                    } else {
                        err.to_diagnostic()
                    };
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
                        // Reclassify a mid-stream upstream error that is actually a context-overflow
                        // so the surfaced diagnostic is the specific context-overflow class.
                        let kind =
                            if kind == "upstream" && compaction::is_context_overflow(&message) {
                                "context-overflow".to_owned()
                            } else {
                                kind
                            };
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

            // Execute the requested tool calls and carry the turn forward (continuation). With a
            // dispatcher each call is surfaced, executed, and re-injected; without one (the no-tool
            // path) each call is answered with a clear "unavailable" result so the model recovers.
            match dispatcher {
                Some(dispatcher) => {
                    for call in &round.tool_calls {
                        sink(RunnerEvent::ToolCall {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        });
                        let result = dispatcher.dispatch(call).await;
                        let is_error = result.is_error;
                        let content = result.content;
                        sink(RunnerEvent::ToolResult {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            parts: result.parts,
                            is_error,
                        });
                        let tool_content = if is_error {
                            format!("Tool error from {}:\n{}", call.name, content)
                        } else {
                            content
                        };
                        if call.name == "skill" && !is_error {
                            append_skill_guidance_to_system(&mut messages, &tool_content);
                        }
                        messages.push(Message::tool_result(call.id.clone(), tool_content));
                    }
                }
                None => {
                    for call in &round.tool_calls {
                        messages.push(Message::tool_result(call.id.clone(), TOOL_UNAVAILABLE));
                    }
                }
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

    /// Build the provider request for one round-trip from the current working messages and the
    /// advertised tool catalogue.
    fn build_request(&self, turn: &Turn, messages: &[Message], tools: &[ToolDef]) -> LlmRequest {
        let mut request = LlmRequest::new(&turn.model_id);
        request.messages = messages.to_vec();
        request.tools = tools.to_vec();
        request.max_tokens = turn.max_tokens;
        request.temperature = turn.temperature;
        request
    }
}

/// Inject the guidance returned by a successful `skill` tool call into the active system context
/// for the continuation round, without changing the pre-engagement system prompt.
fn append_skill_guidance_to_system(messages: &mut Vec<Message>, guidance: &str) {
    let guidance = guidance.trim();
    if guidance.is_empty() {
        return;
    }
    let block = format!("<engaged_skill_guidance>\n{guidance}\n</engaged_skill_guidance>");
    if let Some(system) = messages
        .iter_mut()
        .find(|message| message.role == Role::System)
    {
        system.content.push_str("\n\n");
        system.content.push_str(&block);
    } else {
        messages.insert(0, Message::system(block));
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

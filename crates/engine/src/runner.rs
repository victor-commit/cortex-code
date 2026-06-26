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
use llm::{LlmEvent, LlmRequest, Message, MessageContentPart, Provider, Role, ToolCall, ToolDef};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use std::collections::{HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::time::Duration;

use crate::compaction::{self, CompactionPolicy};
use crate::event::{CompactionRecord, RunError, RunOutcome, RunnerEvent, accumulate_usage};
use crate::tools::{ToolDispatchResult, ToolDispatcher};

/// The built-in step budget for a single turn (architecture §4, `MAX_STEPS`).
pub const DEFAULT_MAX_STEPS: usize = 25;
/// Maximum number of tool calls executed concurrently in a single model step.
pub const DEFAULT_TOOL_DISPATCH_MAX_CONCURRENCY: usize = 4;
/// Per-tool timeout while dispatching model-requested tool calls.
pub const DEFAULT_TOOL_DISPATCH_TIMEOUT: Duration = Duration::from_secs(30);

/// The maximum number of context-overflow recoveries (compact-and-rebuild) attempted in one turn
/// before the runner gives up with a clear context-overflow diagnostic rather than looping forever.
const MAX_OVERFLOW_RECOVERIES: usize = 4;

/// The smallest input budget an overflow recovery will shrink to before giving up.
const MIN_RECOVERY_BUDGET_TOKENS: usize = 512;

/// Maximum number of transient setup retries for one provider request before surfacing a terminal
/// error. This is distinct from transport-level retries inside `llm`.
const MAX_TRANSIENT_SETUP_RETRIES: usize = 2;

/// Number of identical tool-call rounds allowed before stopping a detected doom-loop.
const DOOM_LOOP_REPEAT_THRESHOLD: usize = 2;
/// Internal advisory tool-call name used to gate the repeated-call safeguard.
const LOOP_SAFEGUARD_TOOL_NAME: &str = "loop-safeguard";

/// Base delay for runner-level transient setup retries.
const BASE_TRANSIENT_RETRY_DELAY_MS: u64 = 250;

/// Upper bound for runner-level transient retry delay.
const MAX_TRANSIENT_RETRY_DELAY_MS: u64 = 2_000;

/// The placeholder result injected for a tool call when no tool registry is available (M1).
const TOOL_UNAVAILABLE: &str =
    "Tool execution is not available in this build; answer directly without using tools.";

/// The compacting summarizer instruction: preserve factual memory and open work while staying brief.
const COMPACTION_SUMMARY_PROMPT: &str = "Summarize the removed conversation context so future \
    turns preserve factual memory. Keep names, identifiers, decisions, code changes, and open \
    tasks. Be concise and factual. Respond with only the summary text.";

/// Tool names whose outputs are protected from aggressive pruning during summary rendering.
const COMPACTION_PROTECTED_TOOLS: &[&str] = &["skill"];

/// Maximum number of characters kept from non-protected tool outputs in the compaction head.
const TOOL_OUTPUT_MAX_CHARS: usize = 2_000;

/// Upper bound for a single message fragment rendered into the compaction-summary transcript.
const COMPACTION_FRAGMENT_MAX_CHARS: usize = 4_000;

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
    /// Optional non-text media parts for the new user input.
    pub user_parts: Vec<MessageContentPart>,
    /// Optional cap on generated tokens per round-trip.
    pub max_tokens: Option<u32>,
    /// Optional sampling temperature.
    pub temperature: Option<f32>,
    /// Provider-specific request params merged into each provider request for this turn.
    pub extra_params: JsonMap<String, JsonValue>,
    /// Request-scoped headers merged into each provider request for this turn.
    pub extra_headers: Vec<(String, String)>,
    /// The context-compaction policy applied before each send and on overflow recovery.
    pub compaction: CompactionPolicy,
    /// Optional model id used specifically for compaction-summary helper turns.
    pub small_model_id: Option<String>,
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
            user_parts: Vec::new(),
            max_tokens: None,
            temperature: None,
            extra_params: JsonMap::new(),
            extra_headers: Vec::new(),
            compaction: CompactionPolicy::default(),
            small_model_id: None,
        }
    }

    /// Set the projected history (builder style).
    #[must_use]
    pub fn with_history(mut self, history: Vec<Message>) -> Self {
        self.history = history;
        self
    }

    /// Set non-text media parts for the new user input (builder style).
    #[must_use]
    pub fn with_user_parts(mut self, user_parts: Vec<MessageContentPart>) -> Self {
        self.user_parts = user_parts;
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

    /// Set provider-specific request params (builder style).
    #[must_use]
    pub fn with_extra_params(mut self, extra_params: JsonMap<String, JsonValue>) -> Self {
        self.extra_params = extra_params;
        self
    }

    /// Set request-scoped headers (builder style).
    #[must_use]
    pub fn with_extra_headers(mut self, extra_headers: Vec<(String, String)>) -> Self {
        self.extra_headers = extra_headers;
        self
    }

    /// Set the context-compaction policy (builder style).
    #[must_use]
    pub fn with_compaction(mut self, compaction: CompactionPolicy) -> Self {
        self.compaction = compaction;
        self
    }

    /// Set the model id used for compaction summaries (builder style).
    #[must_use]
    pub fn with_small_model(mut self, small_model_id: Option<String>) -> Self {
        self.small_model_id = small_model_id;
        self
    }
}

/// The durable session runner.
#[derive(Debug, Clone, Copy)]
pub struct SessionRunner {
    max_steps: usize,
    tool_dispatch_max_concurrency: usize,
    tool_dispatch_timeout: Duration,
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
            tool_dispatch_max_concurrency: DEFAULT_TOOL_DISPATCH_MAX_CONCURRENCY,
            tool_dispatch_timeout: DEFAULT_TOOL_DISPATCH_TIMEOUT,
        }
    }

    /// The configured step budget.
    #[must_use]
    pub fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Set the maximum number of tool calls executed concurrently in one model step.
    #[must_use]
    pub fn with_tool_dispatch_max_concurrency(mut self, max_concurrency: usize) -> Self {
        self.tool_dispatch_max_concurrency = max_concurrency.max(1);
        self
    }

    /// Set the per-tool dispatch timeout used while executing model-requested tools.
    #[must_use]
    pub fn with_tool_dispatch_timeout(mut self, timeout: Duration) -> Self {
        self.tool_dispatch_timeout = timeout.max(Duration::from_millis(1));
        self
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
        messages.push(Message::user(turn.user_input.clone()).with_parts(turn.user_parts.clone()));

        let mut answer = String::new();
        let mut total_usage = None;
        let mut steps = 0usize;
        let mut compactions: Vec<CompactionRecord> = Vec::new();
        let mut error = None;
        let mut last_advertised_tool_defs: Option<Vec<ToolDef>> = None;
        let mut doom_loop_guard = DoomLoopGuard::new(DOOM_LOOP_REPEAT_THRESHOLD);

        // Compaction state: the per-request input budget and the number of recent messages kept
        // verbatim. An overflow recovery shrinks both and rebuilds the turn (architecture §4).
        let policy = turn.compaction;
        let mut input_budget = policy.input_budget();
        let mut tail_messages = policy.tail_messages;
        let mut overflow_recoveries = 0usize;
        let mut setup_retries = 0usize;

        let finish_reason = 'turn: loop {
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

            if let Some(dispatcher) = dispatcher {
                match dispatcher.dynamic_system_instructions() {
                    Ok(Some(addition)) => append_system_instructions(&mut messages, &addition),
                    Ok(None) => {}
                    Err(diag) => {
                        sink(RunnerEvent::Error {
                            message: diag.to_string(),
                            kind: diag.kind().to_owned(),
                        });
                        error = Some(RunError::new(diag));
                        break llm::FinishReason::Error;
                    }
                }
            }

            // compactIfNeeded before send: when needed, summarize the compactable head with a
            // tool-free model call, keep a recent tail by turn, and auto-continue the original
            // request. If summarization is unavailable, fall back to bounded mechanical compaction.
            if policy.auto
                && compaction::estimate_messages_tokens(&messages) > input_budget
                && !messages.is_empty()
            {
                match self
                    .compact_with_summary(
                        provider,
                        turn.small_model_id
                            .as_deref()
                            .unwrap_or(turn.model_id.as_str()),
                        &mut messages,
                        tail_messages,
                        input_budget,
                    )
                    .await
                {
                    Ok(Some(record)) => {
                        sink(RunnerEvent::Reasoning {
                            text: format!(
                                "[context compaction] tail_start_id={} continued_request={}",
                                record.tail_start_id, record.continued_request
                            ),
                        });
                        compactions.push(record);
                    }
                    Ok(None) => {
                        compaction::compact_to_budget(&mut messages, tail_messages, input_budget);
                    }
                    Err(_) => {
                        compaction::compact_to_budget(&mut messages, tail_messages, input_budget);
                    }
                }
            }

            // Refresh the advertised tool catalogue immediately before every provider request.
            // MCP servers can send `notifications/tools/list_changed` while a run is already in
            // progress; polling the dispatcher here lets the next continuation request see added
            // and removed tools without restarting cortex.
            let tool_defs: Vec<ToolDef> = dispatcher
                .map(ToolDispatcher::definitions)
                .unwrap_or_default();
            if let Some(previous) = &last_advertised_tool_defs {
                if previous != &tool_defs {
                    sink(RunnerEvent::ToolSetChanged {
                        tools: tool_defs.iter().map(|def| def.name.clone()).collect(),
                        tool_definitions: tool_defs.clone(),
                    });
                }
            } else if dispatcher.is_some() {
                // Emit an initial snapshot on the first provider request as well. This keeps
                // machine output accurate when the surface-level pre-turn snapshot races with
                // MCP live tool-list updates before the first request starts.
                sink(RunnerEvent::ToolSetChanged {
                    tools: tool_defs.iter().map(|def| def.name.clone()).collect(),
                    tool_definitions: tool_defs.clone(),
                });
            }
            last_advertised_tool_defs = Some(tool_defs.clone());

            let request = self.build_request(&turn, &messages, &tool_defs);
            let mut stream = match provider.stream(request).await {
                Ok(stream) => {
                    // A successful stream open resets transient setup retries for subsequent
                    // continuation requests in the same turn.
                    setup_retries = 0;
                    stream
                }
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
                    if let Some(classification) =
                        classify_retryable_setup_error(diagnostic.kind(), err.detail())
                    {
                        if setup_retries < MAX_TRANSIENT_SETUP_RETRIES {
                            setup_retries += 1;
                            let delay = transient_retry_delay(setup_retries);
                            sink(RunnerEvent::Retry {
                                attempt: setup_retries,
                                max_attempts: MAX_TRANSIENT_SETUP_RETRIES,
                                delay_ms: delay.as_millis() as u64,
                                error_kind: diagnostic.kind().to_owned(),
                                classification: classification.as_str().to_owned(),
                                message: diagnostic.to_string(),
                            });
                            tokio::time::sleep(delay).await;
                            steps -= 1;
                            continue;
                        }

                        let exhausted = format!(
                            "retry exhausted after {} attempt(s): {diagnostic}",
                            MAX_TRANSIENT_SETUP_RETRIES
                        );
                        sink(RunnerEvent::Error {
                            message: exhausted.clone(),
                            kind: diagnostic.kind().to_owned(),
                        });
                        error = Some(RunError::from_kind(diagnostic.kind(), exhausted));
                        break llm::FinishReason::Error;
                    }
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
                parts: Vec::new(),
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
                    if let Some(detail) = doom_loop_guard.observe(&round.tool_calls) {
                        let safeguard_call = ToolCall {
                            id: format!("loop_safeguard_{steps}"),
                            name: LOOP_SAFEGUARD_TOOL_NAME.to_owned(),
                            arguments: json!({ "detail": detail }).to_string(),
                        };
                        sink(RunnerEvent::ToolCall {
                            id: safeguard_call.id.clone(),
                            name: safeguard_call.name.clone(),
                            arguments: safeguard_call.arguments.clone(),
                        });
                        let (_, _, safeguard_result) = dispatch_tool_call(
                            dispatcher,
                            safeguard_call.clone(),
                            0,
                            self.tool_dispatch_timeout,
                        )
                        .await;
                        sink(RunnerEvent::ToolResult {
                            id: safeguard_call.id,
                            name: safeguard_call.name,
                            parts: safeguard_result.parts.clone(),
                            is_error: safeguard_result.is_error,
                        });
                        if !loop_safeguard_was_denied(&safeguard_result) {
                            sink(RunnerEvent::Error {
                                message: detail.clone(),
                                kind: "doom-loop".to_owned(),
                            });
                            error = Some(RunError::new(Diagnostic::doom_loop(detail)));
                            break 'turn llm::FinishReason::Error;
                        }
                    }

                    for call in &round.tool_calls {
                        sink(RunnerEvent::ToolCall {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        });
                    }

                    let max_concurrency = self
                        .tool_dispatch_max_concurrency
                        .max(1)
                        .min(round.tool_calls.len().max(1));
                    let dispatch_timeout = self.tool_dispatch_timeout;
                    let mut pending_calls: VecDeque<(usize, ToolCall)> =
                        round.tool_calls.iter().cloned().enumerate().collect();
                    let mut in_flight = futures_util::stream::FuturesUnordered::new();
                    let mut completed_results: Vec<Option<(ToolCall, ToolDispatchResult)>> =
                        vec![None; round.tool_calls.len()];

                    while in_flight.len() < max_concurrency {
                        let Some((index, call)) = pending_calls.pop_front() else {
                            break;
                        };
                        in_flight.push(dispatch_tool_call(
                            dispatcher,
                            call,
                            index,
                            dispatch_timeout,
                        ));
                    }

                    while let Some((index, call, result)) = in_flight.next().await {
                        sink(RunnerEvent::ToolResult {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            parts: result.parts.clone(),
                            is_error: result.is_error,
                        });
                        completed_results[index] = Some((call, result));
                        if let Some((next_index, next_call)) = pending_calls.pop_front() {
                            in_flight.push(dispatch_tool_call(
                                dispatcher,
                                next_call,
                                next_index,
                                dispatch_timeout,
                            ));
                        }
                    }

                    for (index, call) in round.tool_calls.iter().enumerate() {
                        let result = completed_results
                            .get_mut(index)
                            .and_then(Option::take)
                            .map(|(_, result)| result)
                            .unwrap_or_else(|| missing_tool_result(call));
                        let is_error = result.is_error;
                        let content = result.content.clone();
                        let tool_content = if is_error {
                            format!("Tool error from {}:\n{}", call.name, content)
                        } else {
                            content
                        };
                        if call.name == "skill" && !is_error {
                            append_skill_guidance_to_system(&mut messages, &tool_content);
                        }
                        let media_parts = message_media_parts_from_tool_result(&result.parts);
                        messages.push(
                            Message::tool_result(call.id.clone(), tool_content)
                                .with_parts(media_parts),
                        );
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
            compactions,
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
        request.extra_params = turn.extra_params.clone();
        request.extra_headers = turn.extra_headers.clone();
        request
    }

    /// Compact an over-budget request by summarizing the compactable head through a tool-free model
    /// call, preserving a recent tail by turn, and reinjecting the summary as a system message.
    async fn compact_with_summary(
        &self,
        provider: &dyn Provider,
        model_id: &str,
        messages: &mut Vec<Message>,
        tail_turns: usize,
        budget: usize,
    ) -> Result<Option<CompactionRecord>, Diagnostic> {
        if compaction::estimate_messages_tokens(messages) <= budget || messages.is_empty() {
            return Ok(None);
        }

        let system_head = usize::from(messages.first().is_some_and(|m| m.role == Role::System));
        let tail_start = select_tail_start(messages, system_head, tail_turns);
        if tail_start <= system_head || tail_start >= messages.len() {
            return Ok(None);
        }

        let head = messages[system_head..tail_start].to_vec();
        if head.is_empty() {
            return Ok(None);
        }
        let transcript = render_compaction_transcript(&head, budget);
        if transcript.trim().is_empty() {
            return Ok(None);
        }

        let summary = self
            .summarize_compaction_head(provider, model_id, &transcript)
            .await?;
        if summary.trim().is_empty() {
            return Ok(None);
        }

        let tail_start_id = make_tail_start_id(tail_start, &messages[tail_start]);
        let summary_message = Message::system(format!(
            "<compaction_summary tail_start_id=\"{tail_start_id}\" continued_request=\"true\">\n{}\n</compaction_summary>",
            summary.trim()
        ));

        let mut rebuilt =
            Vec::with_capacity(system_head + 1 + messages.len().saturating_sub(tail_start));
        if system_head == 1 {
            rebuilt.push(messages[0].clone());
        }
        rebuilt.push(summary_message);
        rebuilt.extend(messages[tail_start..].iter().cloned());
        *messages = rebuilt;
        if compaction::estimate_messages_tokens(messages) > budget {
            let fallback_tail_messages = tail_turns.max(1).saturating_mul(2);
            compaction::compact_to_budget(messages, fallback_tail_messages, budget);
        }

        Ok(Some(CompactionRecord {
            summary: summary.trim().to_owned(),
            tail_start_id,
            continued_request: true,
            model: Some(model_id.to_owned()),
        }))
    }

    /// Call the model once (with tools disabled) to summarize compacted-away context.
    async fn summarize_compaction_head(
        &self,
        provider: &dyn Provider,
        model_id: &str,
        transcript: &str,
    ) -> Result<String, Diagnostic> {
        let mut request = LlmRequest::new(model_id);
        request.messages = vec![
            Message::system(COMPACTION_SUMMARY_PROMPT),
            Message::user(format!(
                "Summarize this removed head context so later turns preserve key facts:\n\n{}",
                transcript
            )),
        ];
        request.max_tokens = Some(600);
        request.temperature = Some(0.0);

        let mut stream = provider
            .stream(request)
            .await
            .map_err(|err| err.to_diagnostic())?;
        let mut summary = String::new();
        while let Some(event) = stream.next().await {
            match event {
                LlmEvent::TextDelta { text } => summary.push_str(&text),
                LlmEvent::Error { message, kind } => {
                    return Err(RunError::from_kind(&kind, message).diagnostic().clone());
                }
                LlmEvent::Reasoning { .. }
                | LlmEvent::ToolCall(_)
                | LlmEvent::ToolResult(_)
                | LlmEvent::Usage(_)
                | LlmEvent::Done { .. } => {}
            }
        }
        let trimmed = summary.trim();
        if trimmed.is_empty() {
            return Err(Diagnostic::upstream(
                "compaction summary request returned empty text",
            ));
        }
        Ok(trimmed.to_owned())
    }
}

/// Decide where the preserved tail begins, preferring turn boundaries (user-message starts).
fn select_tail_start(messages: &[Message], system_head: usize, tail_turns: usize) -> usize {
    let user_turn_starts: Vec<usize> = messages
        .iter()
        .enumerate()
        .skip(system_head)
        .filter_map(|(index, message)| (message.role == Role::User).then_some(index))
        .collect();

    let keep_turns = tail_turns.max(1);
    if user_turn_starts.len() > keep_turns {
        return user_turn_starts[user_turn_starts.len() - keep_turns];
    }

    let fallback_keep_messages = keep_turns.saturating_mul(2);
    let min_tail_start = if messages.len() > system_head {
        system_head + 1
    } else {
        system_head
    };
    messages
        .len()
        .saturating_sub(fallback_keep_messages)
        .max(min_tail_start)
}

/// Render the compactable head into a bounded transcript for the summarizer model.
fn render_compaction_transcript(head: &[Message], budget: usize) -> String {
    let protected_tool_calls = protected_tool_call_ids(head);
    let max_bytes = budget.saturating_mul(4).clamp(4_000, 120_000);
    let mut rendered = String::new();

    for (index, message) in head.iter().enumerate() {
        let mut content = message.content.as_str().to_owned();
        if message.role == Role::Tool {
            let is_protected = message
                .tool_call_id
                .as_ref()
                .is_some_and(|id| protected_tool_calls.contains(id));
            if is_protected {
                content = truncate_with_marker(
                    &content,
                    COMPACTION_FRAGMENT_MAX_CHARS,
                    "\n[… long protected tool output truncated …]\n",
                );
            } else {
                content = truncate_with_marker(
                    &content,
                    TOOL_OUTPUT_MAX_CHARS,
                    "\n[… tool output truncated for compaction summary …]\n",
                );
            }
        } else {
            content = truncate_with_marker(
                &content,
                COMPACTION_FRAGMENT_MAX_CHARS,
                "\n[… message truncated for compaction summary …]\n",
            );
        }

        let tool_call_suffix = message
            .tool_call_id
            .as_deref()
            .map_or(String::new(), |id| format!(" tool_call_id={id}"));
        let block = format!(
            "message[{index}] role={}{}\n{}\n\n",
            role_name(message.role),
            tool_call_suffix,
            content
        );
        if rendered.len().saturating_add(block.len()) > max_bytes {
            rendered
                .push_str("[… additional earlier messages omitted from compaction transcript …]");
            break;
        }
        rendered.push_str(&block);
    }

    rendered
}

/// Collect tool-call ids whose outputs should be preserved in compaction transcripts.
fn protected_tool_call_ids(head: &[Message]) -> HashSet<String> {
    let mut ids = HashSet::new();
    for message in head
        .iter()
        .filter(|message| message.role == Role::Assistant)
    {
        for call in &message.tool_calls {
            if COMPACTION_PROTECTED_TOOLS.contains(&call.name.as_str()) {
                ids.insert(call.id.clone());
            }
        }
    }
    ids
}

/// Truncate text to a character budget while preserving a head and tail sample around a marker.
fn truncate_with_marker(text: &str, max_chars: usize, marker: &str) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let head_chars = max_chars / 2;
    let tail_chars = max_chars.saturating_sub(head_chars);
    let text_chars = text.chars().count();
    let head_end = byte_offset_for_char(text, head_chars);
    let tail_start = byte_offset_for_char(text, text_chars.saturating_sub(tail_chars));

    let mut out =
        String::with_capacity(head_end + marker.len() + text.len().saturating_sub(tail_start));
    out.push_str(&text[..head_end]);
    out.push_str(marker);
    out.push_str(&text[tail_start..]);
    out
}

/// UTF-8-safe byte offset for a char index.
fn byte_offset_for_char(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or_else(|| text.len(), |(byte, _)| byte)
}

/// Stable tail-boundary identifier derived from the preserved-tail anchor message.
fn make_tail_start_id(index: usize, message: &Message) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    role_name(message.role).hash(&mut hasher);
    message.content.hash(&mut hasher);
    message.tool_call_id.hash(&mut hasher);
    let digest = hasher.finish();
    format!("tail_{index}_{digest:x}")
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
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

/// Inject dynamic system instructions into the active system context for the next continuation
/// round (for example, memory instructions activated after tool reads).
fn append_system_instructions(messages: &mut Vec<Message>, instructions: &str) {
    let instructions = instructions.trim();
    if instructions.is_empty() {
        return;
    }
    if let Some(system) = messages
        .iter_mut()
        .find(|message| message.role == Role::System)
    {
        system.content.push_str("\n\n");
        system.content.push_str(instructions);
    } else {
        messages.insert(0, Message::system(instructions.to_owned()));
    }
}

fn message_media_parts_from_tool_result(
    parts: &[protocol::ToolResultPart],
) -> Vec<MessageContentPart> {
    parts
        .iter()
        .filter_map(|part| match part {
            protocol::ToolResultPart::Image { media_type, path } => {
                Some(MessageContentPart::Image {
                    media_type: media_type.clone(),
                    path: path.clone(),
                })
            }
            protocol::ToolResultPart::Document { media_type, path } => {
                Some(MessageContentPart::Document {
                    media_type: media_type.clone(),
                    path: path.clone(),
                })
            }
            protocol::ToolResultPart::Text { .. } => None,
        })
        .collect()
}

/// Execute one tool call with a per-call timeout, yielding a synthesized timeout result if needed.
async fn dispatch_tool_call(
    dispatcher: &dyn ToolDispatcher,
    call: ToolCall,
    index: usize,
    timeout: Duration,
) -> (usize, ToolCall, ToolDispatchResult) {
    let result = match tokio::time::timeout(timeout, dispatcher.dispatch(&call)).await {
        Ok(result) => result,
        Err(_) => timed_out_tool_result(&call, timeout),
    };
    (index, call, result)
}

/// Synthesize an error result for a tool call that exceeded the dispatch timeout.
fn timed_out_tool_result(call: &ToolCall, timeout: Duration) -> ToolDispatchResult {
    let timeout_ms = timeout.as_millis();
    let content = format!(
        "Tool {} timed out after {timeout_ms}ms while executing call {}.",
        call.name, call.id
    );
    ToolDispatchResult {
        parts: vec![protocol::ToolResultPart::text(content.clone())],
        content,
        is_error: true,
    }
}

/// Defensive fallback: synthesize an error result if a tool call somehow produced no completion.
fn missing_tool_result(call: &ToolCall) -> ToolDispatchResult {
    let content = format!(
        "Tool {} did not produce a result for call {}.",
        call.name, call.id
    );
    ToolDispatchResult {
        parts: vec![protocol::ToolResultPart::text(content.clone())],
        content,
        is_error: true,
    }
}

/// Whether the loop-safeguard advisory hook was blocked by permissions.
fn loop_safeguard_was_denied(result: &ToolDispatchResult) -> bool {
    result.is_error
        && result
            .content
            .to_ascii_lowercase()
            .contains("permission denied")
}

/// Stateful guard that stops repeated identical tool-call rounds (doom-loop protection).
#[derive(Debug)]
struct DoomLoopGuard {
    threshold: usize,
    last_signature: Option<u64>,
    consecutive_rounds: usize,
}

impl DoomLoopGuard {
    fn new(threshold: usize) -> Self {
        Self {
            threshold: threshold.max(1),
            last_signature: None,
            consecutive_rounds: 0,
        }
    }

    /// Observe the next tool-call round, returning a stop detail when it exceeds the threshold.
    fn observe(&mut self, calls: &[ToolCall]) -> Option<String> {
        if calls.is_empty() {
            self.last_signature = None;
            self.consecutive_rounds = 0;
            return None;
        }

        let signature = tool_round_signature(calls);
        if self.last_signature == Some(signature) {
            self.consecutive_rounds = self.consecutive_rounds.saturating_add(1);
        } else {
            self.last_signature = Some(signature);
            self.consecutive_rounds = 1;
        }

        if self.consecutive_rounds > self.threshold {
            let call = &calls[0];
            Some(format!(
                "stopped repeated tool round after {} identical attempts (tool `{}` with the same arguments)",
                self.threshold, call.name
            ))
        } else {
            None
        }
    }
}

fn tool_round_signature(calls: &[ToolCall]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    calls.len().hash(&mut hasher);
    for call in calls {
        call.name.hash(&mut hasher);
        call.arguments.hash(&mut hasher);
    }
    hasher.finish()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryableSetupClassification {
    RateLimit,
    ServiceUnavailable,
    TransientServerError,
    TransportUnavailable,
}

impl RetryableSetupClassification {
    const fn as_str(self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::ServiceUnavailable => "service_unavailable",
            Self::TransientServerError => "transient_server_error",
            Self::TransportUnavailable => "transport_unavailable",
        }
    }
}

fn classify_retryable_setup_error(
    kind: &str,
    detail: &str,
) -> Option<RetryableSetupClassification> {
    match kind {
        "transport-unavailable" => Some(RetryableSetupClassification::TransportUnavailable),
        "upstream" => {
            if compaction::is_context_overflow(detail) {
                return None;
            }

            if let Some(status) = parse_http_status(detail) {
                if status == 429 {
                    return Some(RetryableSetupClassification::RateLimit);
                }
                if status == 503 || status == 529 {
                    return Some(RetryableSetupClassification::ServiceUnavailable);
                }
                if (500..=599).contains(&status) {
                    return Some(RetryableSetupClassification::TransientServerError);
                }
                return None;
            }

            let lower = detail.to_ascii_lowercase();
            if lower.contains("rate limit") || lower.contains("too many requests") {
                return Some(RetryableSetupClassification::RateLimit);
            }
            if lower.contains("service unavailable")
                || lower.contains("temporarily unavailable")
                || lower.contains("provider unavailable")
                || lower.contains("overloaded")
            {
                return Some(RetryableSetupClassification::ServiceUnavailable);
            }
            None
        }
        _ => None,
    }
}

fn parse_http_status(detail: &str) -> Option<u16> {
    let lower = detail.to_ascii_lowercase();
    let (_, tail) = lower.split_once("http ")?;
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.len() < 3 {
        return None;
    }
    digits.parse::<u16>().ok()
}

fn transient_retry_delay(attempt: usize) -> std::time::Duration {
    let exponent = attempt.saturating_sub(1).min(5) as u32;
    let factor = 1_u64 << exponent;
    let ms = (BASE_TRANSIENT_RETRY_DELAY_MS.saturating_mul(factor))
        .clamp(BASE_TRANSIENT_RETRY_DELAY_MS, MAX_TRANSIENT_RETRY_DELAY_MS);
    std::time::Duration::from_millis(ms)
}

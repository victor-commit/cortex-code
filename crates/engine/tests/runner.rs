//! Offline runner tests driven by a scripted fake [`Provider`] (no network, no mocks on the
//! shipped path — the fake exists only here, in tests).
//!
//! These prove the durable loop's behavior independent of a live model: the single normalized
//! event vocabulary and ordering (VAL-RUN-007), multi-round-trip continuation into one final
//! answer with exactly one terminal event (VAL-RUN-002), assistant-role assembly, and clean
//! setup-error handling.

use std::sync::Mutex;

use engine::{RunnerEvent, SessionRunner, Turn};
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, ToolCall, Usage};

/// A provider that replays a pre-scripted sequence of event batches, one batch per round-trip.
struct ScriptedProvider {
    rounds: Mutex<std::collections::VecDeque<Vec<LlmEvent>>>,
}

impl ScriptedProvider {
    fn new(rounds: Vec<Vec<LlmEvent>>) -> Self {
        Self {
            rounds: Mutex::new(rounds.into()),
        }
    }
}

impl Provider for ScriptedProvider {
    fn stream<'a>(
        &'a self,
        _request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        let batch = self
            .rounds
            .lock()
            .expect("rounds lock")
            .pop_front()
            .unwrap_or_default();
        Box::pin(async move {
            let stream = futures_util::stream::iter(batch);
            Ok(Box::pin(stream) as LlmStream)
        })
    }
}

/// A provider whose `stream` fails before any event (a setup error such as no-credential).
struct FailingProvider {
    error: llm::LlmError,
}

impl Provider for FailingProvider {
    fn stream<'a>(
        &'a self,
        _request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        let error = self.error.clone();
        Box::pin(async move { Err(error) })
    }
}

fn text(s: &str) -> LlmEvent {
    LlmEvent::TextDelta { text: s.to_owned() }
}

fn usage() -> LlmEvent {
    LlmEvent::Usage(Usage {
        input_tokens: 5,
        output_tokens: 3,
        total_tokens: 8,
        cost: Some(0.0001),
    })
}

/// Collect the runner's emitted events plus the outcome for a turn.
async fn run(provider: &dyn Provider, turn: Turn) -> (Vec<RunnerEvent>, engine::RunOutcome) {
    let mut events = Vec::new();
    let outcome = SessionRunner::default()
        .run(provider, turn, |event| events.push(event))
        .await;
    (events, outcome)
}

fn concat_text(events: &[RunnerEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            RunnerEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn done_count(events: &[RunnerEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, RunnerEvent::Done { .. }))
        .count()
}

#[tokio::test]
async fn single_round_trip_streams_text_then_usage_then_one_done() {
    let provider = ScriptedProvider::new(vec![vec![
        text("Hello"),
        text(", world!"),
        usage(),
        LlmEvent::Done {
            reason: FinishReason::Stop,
        },
    ]]);

    let (events, outcome) = run(&provider, Turn::new("google/gemini-2.5-flash", "", "hi")).await;

    // Concatenated text deltas equal the final answer.
    assert_eq!(concat_text(&events), "Hello, world!");
    assert_eq!(outcome.answer(), "Hello, world!");

    // Usage precedes the single terminal Done; exactly one terminal event.
    let usage_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Usage(_)))
        .expect("a usage event");
    let done_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Done { .. }))
        .expect("a done event");
    assert!(usage_pos < done_pos, "usage must precede done");
    assert_eq!(done_count(&events), 1, "exactly one terminal Done");
    assert_eq!(outcome.steps, 1);
    assert!(outcome.is_success());
}

#[tokio::test]
async fn text_deltas_precede_usage_and_done() {
    let provider = ScriptedProvider::new(vec![vec![
        text("a"),
        text("b"),
        usage(),
        LlmEvent::Done {
            reason: FinishReason::Stop,
        },
    ]]);
    let (events, _) = run(&provider, Turn::new("google/gemini-2.5-flash", "", "hi")).await;

    let last_text = events
        .iter()
        .rposition(|e| matches!(e, RunnerEvent::TextDelta { .. }))
        .expect("text event");
    let usage_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Usage(_)))
        .unwrap();
    let done_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Done { .. }))
        .unwrap();
    assert!(last_text < usage_pos, "all text precedes usage");
    assert!(usage_pos < done_pos, "usage precedes the terminal done");
}

#[tokio::test]
async fn multi_round_trip_continues_to_one_final_answer() {
    // Round 1: the model requests a tool and stops with tool_calls; round 2: it gives the answer.
    let provider = ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_1".to_owned(),
                name: "lookup".to_owned(),
                arguments: "{}".to_owned(),
            }),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            text("FINAL: "),
            text("done"),
            usage(),
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]);

    let (events, outcome) = run(
        &provider,
        Turn::new(
            "google/gemini-2.5-flash",
            "",
            "Think step by step, then give a final answer labelled FINAL:",
        ),
    )
    .await;

    // The turn was carried forward across two provider round-trips into one final answer.
    assert_eq!(outcome.steps, 2, "two round-trips");
    assert_eq!(concat_text(&events), "FINAL: done");
    assert_eq!(outcome.answer(), "FINAL: done");
    assert!(outcome.answer().contains("FINAL:"));

    // Despite two provider streams (each with its own Done), the runner emits exactly one
    // terminal Done after the text and usage.
    assert_eq!(done_count(&events), 1, "no duplicate terminal event");
    let done_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Done { .. }))
        .unwrap();
    assert_eq!(
        done_pos,
        events.len() - 1,
        "the single Done is the last event"
    );
    assert!(outcome.is_success());
}

#[tokio::test]
async fn final_message_is_assistant_role() {
    let provider = ScriptedProvider::new(vec![vec![
        text("answer"),
        LlmEvent::Done {
            reason: FinishReason::Stop,
        },
    ]]);
    let (_, outcome) = run(&provider, Turn::new("google/gemini-2.5-flash", "", "hi")).await;
    assert_eq!(outcome.message.role, llm::Role::Assistant);
    assert_eq!(outcome.message.content, "answer");
}

#[tokio::test]
async fn usage_is_summed_across_round_trips() {
    let provider = ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "c".to_owned(),
                name: "t".to_owned(),
                arguments: "{}".to_owned(),
            }),
            usage(),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            text("ok"),
            usage(),
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]);
    let (events, outcome) = run(&provider, Turn::new("google/gemini-2.5-flash", "", "hi")).await;

    let summed = outcome.usage.expect("usage reported");
    assert_eq!(summed.total_tokens, 16, "8 + 8 across two round-trips");
    assert_eq!(summed.input_tokens, 10);
    assert_eq!(summed.cost, Some(0.0002));

    // Exactly one Usage event is emitted to the sink (the summed total), at the end.
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, RunnerEvent::Usage(_)))
            .count(),
        1,
        "a single summed usage event"
    );
}

#[tokio::test]
async fn setup_error_emits_error_then_one_done_and_fails_cleanly() {
    let provider = FailingProvider {
        error: llm::LlmError::no_credential("openrouter"),
    };
    let (events, outcome) = run(&provider, Turn::new("google/gemini-2.5-flash", "", "hi")).await;

    // An Error event precedes exactly one terminal Done; no text was produced.
    assert!(
        concat_text(&events).is_empty(),
        "no answer text on a setup error"
    );
    let error_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Error { .. }))
        .expect("an error event");
    let done_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Done { .. }))
        .expect("a done event");
    assert!(error_pos < done_pos, "error precedes the terminal done");
    assert_eq!(done_count(&events), 1, "exactly one terminal Done");

    let error = outcome.error.as_ref().expect("a normalized error");
    assert_eq!(error.kind(), "no-credential");
    assert_eq!(error.exit().code(), 1);
    assert!(!outcome.is_success());
    assert_eq!(
        outcome.finish_reason,
        FinishReason::Error,
        "the turn ends in an error finish reason"
    );
}

#[tokio::test]
async fn mid_stream_error_is_forwarded_then_terminated_once() {
    let provider = ScriptedProvider::new(vec![vec![
        text("partial"),
        LlmEvent::Error {
            message: "provider returned HTTP 500".to_owned(),
            kind: "upstream".to_owned(),
        },
        LlmEvent::Done {
            reason: FinishReason::Error,
        },
    ]]);
    let (events, outcome) = run(&provider, Turn::new("google/gemini-2.5-flash", "", "hi")).await;

    assert_eq!(concat_text(&events), "partial");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, RunnerEvent::Error { .. }))
    );
    assert_eq!(done_count(&events), 1, "exactly one terminal Done");
    let error = outcome.error.as_ref().expect("a normalized error");
    assert_eq!(error.kind(), "upstream");
}

#[tokio::test]
async fn history_and_system_are_placed_ahead_of_the_user_input() {
    // Capture the request the provider receives to assert message assembly order.
    struct Capturing {
        seen: Mutex<Option<Vec<llm::Message>>>,
    }
    impl Provider for Capturing {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            *self.seen.lock().unwrap() = Some(request.messages.clone());
            Box::pin(async move {
                let stream = futures_util::stream::iter(vec![
                    LlmEvent::TextDelta {
                        text: "ok".to_owned(),
                    },
                    LlmEvent::Done {
                        reason: FinishReason::Stop,
                    },
                ]);
                Ok(Box::pin(stream) as LlmStream)
            })
        }
    }

    let provider = Capturing {
        seen: Mutex::new(None),
    };
    let history = engine::project_history(&[
        engine::HistoryMessage::new("user", "earlier question"),
        engine::HistoryMessage::new("assistant", "earlier answer"),
    ]);
    let turn = Turn::new("google/gemini-2.5-flash", "SYSTEM CONTEXT", "new question")
        .with_history(history);

    let _ = run(&provider, turn).await;

    let messages = provider.seen.lock().unwrap().clone().expect("a request");
    assert_eq!(messages[0].role, llm::Role::System);
    assert_eq!(messages[0].content, "SYSTEM CONTEXT");
    assert_eq!(messages[1].role, llm::Role::User);
    assert_eq!(messages[1].content, "earlier question");
    assert_eq!(messages[2].role, llm::Role::Assistant);
    assert_eq!(messages[3].role, llm::Role::User);
    assert_eq!(messages[3].content, "new question");
}

//! Offline runner tests driven by a scripted fake [`Provider`] (no network, no mocks on the
//! shipped path — the fake exists only here, in tests).
//!
//! These prove the durable loop's behavior independent of a live model: the single normalized
//! event vocabulary and ordering (VAL-RUN-007), multi-round-trip continuation into one final
//! answer with exactly one terminal event (VAL-RUN-002), assistant-role assembly, and clean
//! setup-error handling.

use std::sync::Mutex;

use engine::{RunnerEvent, SessionRunner, ToolDispatchResult, ToolDispatcher, Turn};
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, ToolCall, ToolDef, Usage};

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

/// A provider that requests a tool call on every round-trip forever, so the turn never naturally
/// completes — used to drive the runner into its step budget.
struct EndlessToolProvider;

impl Provider for EndlessToolProvider {
    fn stream<'a>(
        &'a self,
        _request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        Box::pin(async move {
            let stream = futures_util::stream::iter(vec![
                LlmEvent::ToolCall(ToolCall {
                    id: "call_loop".to_owned(),
                    name: "lookup".to_owned(),
                    arguments: "{}".to_owned(),
                }),
                LlmEvent::Done {
                    reason: FinishReason::ToolCalls,
                },
            ]);
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

fn upstream_503_error() -> llm::LlmError {
    llm::LlmError::upstream("provider returned HTTP 503: temporary outage")
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
async fn errored_tool_results_are_reinjected_with_error_context() {
    struct CapturingProvider {
        requests: Mutex<Vec<Vec<llm::Message>>>,
    }

    impl Provider for CapturingProvider {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let round = self.requests.lock().unwrap().len();
            self.requests.lock().unwrap().push(request.messages.clone());
            Box::pin(async move {
                let events = if round == 0 {
                    vec![
                        LlmEvent::ToolCall(ToolCall {
                            id: "call_boom".to_owned(),
                            name: "demo_boom".to_owned(),
                            arguments: "{}".to_owned(),
                        }),
                        LlmEvent::Done {
                            reason: FinishReason::ToolCalls,
                        },
                    ]
                } else {
                    vec![
                        LlmEvent::TextDelta {
                            text: "acknowledged failure".to_owned(),
                        },
                        LlmEvent::Done {
                            reason: FinishReason::Stop,
                        },
                    ]
                };
                Ok(Box::pin(futures_util::stream::iter(events)) as LlmStream)
            })
        }
    }

    struct ErrorDispatcher;

    impl ToolDispatcher for ErrorDispatcher {
        fn definitions(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "demo_boom".to_owned(),
                description: "Return an MCP error result".to_owned(),
                parameters: serde_json::json!({ "type": "object" }),
            }]
        }

        fn dispatch<'a>(&'a self, _call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
            Box::pin(async {
                ToolDispatchResult {
                    content: "MCP-TOOL-FAILED".to_owned(),
                    parts: vec![protocol::ToolResultPart::text("MCP-TOOL-FAILED")],
                    is_error: true,
                }
            })
        }
    }

    let provider = CapturingProvider {
        requests: Mutex::new(Vec::new()),
    };
    let dispatcher = ErrorDispatcher;
    let mut events = Vec::new();
    let outcome = SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("google/gemini-2.5-flash", "", "call the failing MCP tool"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    assert!(outcome.is_success());
    assert!(events.iter().any(|event| {
        matches!(event, RunnerEvent::ToolResult { is_error: true, name, .. } if name == "demo_boom")
    }));
    let requests = provider.requests.lock().unwrap();
    let second = requests
        .get(1)
        .expect("second provider round after tool result");
    let tool_message = second
        .iter()
        .find(|message| message.role == llm::Role::Tool)
        .expect("tool result message is re-injected");
    assert!(
        tool_message.content.contains("Tool error from demo_boom")
            && tool_message.content.contains("MCP-TOOL-FAILED"),
        "errored tool result must reach the model as an error, got: {}",
        tool_message.content
    );
}

#[tokio::test]
async fn tool_definitions_are_refreshed_before_each_provider_request() {
    struct CapturingProvider {
        requests: Mutex<Vec<Vec<String>>>,
    }

    impl Provider for CapturingProvider {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let mut requests = self.requests.lock().unwrap();
            let round = requests.len();
            requests.push(request.tools.iter().map(|tool| tool.name.clone()).collect());
            drop(requests);

            Box::pin(async move {
                let events = match round {
                    0 => vec![
                        LlmEvent::ToolCall(ToolCall {
                            id: "call_1".to_owned(),
                            name: "stable".to_owned(),
                            arguments: "{}".to_owned(),
                        }),
                        LlmEvent::Done {
                            reason: FinishReason::ToolCalls,
                        },
                    ],
                    1 => vec![
                        LlmEvent::ToolCall(ToolCall {
                            id: "call_2".to_owned(),
                            name: "stable".to_owned(),
                            arguments: "{}".to_owned(),
                        }),
                        LlmEvent::Done {
                            reason: FinishReason::ToolCalls,
                        },
                    ],
                    _ => vec![
                        LlmEvent::TextDelta {
                            text: "done".to_owned(),
                        },
                        LlmEvent::Done {
                            reason: FinishReason::Stop,
                        },
                    ],
                };
                Ok(Box::pin(futures_util::stream::iter(events)) as LlmStream)
            })
        }
    }

    struct ChangingDispatcher {
        definition_calls: Mutex<usize>,
    }

    impl ToolDispatcher for ChangingDispatcher {
        fn definitions(&self) -> Vec<ToolDef> {
            let mut calls = self.definition_calls.lock().unwrap();
            let round = *calls;
            *calls += 1;

            let mut definitions = vec![ToolDef {
                name: "stable".to_owned(),
                description: "Stable tool".to_owned(),
                parameters: serde_json::json!({ "type": "object" }),
            }];
            if round == 1 {
                definitions.push(ToolDef {
                    name: "runtime_extra".to_owned(),
                    description: "Runtime-added tool".to_owned(),
                    parameters: serde_json::json!({ "type": "object" }),
                });
            }
            definitions
        }

        fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
            let name = call.name.clone();
            Box::pin(async move { ToolDispatchResult::text(format!("{name} ok")) })
        }
    }

    let provider = CapturingProvider {
        requests: Mutex::new(Vec::new()),
    };
    let dispatcher = ChangingDispatcher {
        definition_calls: Mutex::new(0),
    };

    let outcome = SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("google/gemini-2.5-flash", "", "exercise tools"),
            &dispatcher,
            |_| {},
        )
        .await;

    assert!(outcome.is_success());
    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        *requests,
        vec![
            vec!["stable".to_owned()],
            vec!["stable".to_owned(), "runtime_extra".to_owned()],
            vec!["stable".to_owned()],
        ],
        "each continuation request must advertise the latest dispatcher definitions"
    );
}

#[tokio::test]
async fn tool_set_changed_emits_initial_snapshot_and_refreshes() {
    struct CapturingProvider {
        requests: Mutex<Vec<Vec<String>>>,
    }

    impl Provider for CapturingProvider {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let mut requests = self.requests.lock().unwrap();
            let round = requests.len();
            requests.push(request.tools.iter().map(|tool| tool.name.clone()).collect());
            drop(requests);

            Box::pin(async move {
                let events = if round == 0 {
                    vec![
                        LlmEvent::ToolCall(ToolCall {
                            id: "call_1".to_owned(),
                            name: "stable".to_owned(),
                            arguments: "{}".to_owned(),
                        }),
                        LlmEvent::Done {
                            reason: FinishReason::ToolCalls,
                        },
                    ]
                } else {
                    vec![
                        LlmEvent::TextDelta {
                            text: "done".to_owned(),
                        },
                        LlmEvent::Done {
                            reason: FinishReason::Stop,
                        },
                    ]
                };
                Ok(Box::pin(futures_util::stream::iter(events)) as LlmStream)
            })
        }
    }

    struct ChangingDispatcher {
        definition_calls: Mutex<usize>,
    }

    impl ToolDispatcher for ChangingDispatcher {
        fn definitions(&self) -> Vec<ToolDef> {
            let mut calls = self.definition_calls.lock().unwrap();
            let round = *calls;
            *calls += 1;

            let mut definitions = vec![ToolDef {
                name: "stable".to_owned(),
                description: "Stable tool".to_owned(),
                parameters: serde_json::json!({ "type": "object" }),
            }];
            if round >= 1 {
                definitions.push(ToolDef {
                    name: "runtime_extra".to_owned(),
                    description: "Runtime-added tool".to_owned(),
                    parameters: serde_json::json!({ "type": "object" }),
                });
            }
            definitions
        }

        fn dispatch<'a>(&'a self, _call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
            Box::pin(async { ToolDispatchResult::text("ok") })
        }
    }

    let provider = CapturingProvider {
        requests: Mutex::new(Vec::new()),
    };
    let dispatcher = ChangingDispatcher {
        definition_calls: Mutex::new(0),
    };
    let mut events = Vec::new();
    let outcome = SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("google/gemini-2.5-flash", "", "exercise tools"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    assert!(outcome.is_success());
    let advertised: Vec<Vec<String>> = events
        .iter()
        .filter_map(|event| match event {
            RunnerEvent::ToolSetChanged { tools, .. } => Some(tools.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        advertised,
        vec![
            vec!["stable".to_owned()],
            vec!["stable".to_owned(), "runtime_extra".to_owned()]
        ],
        "runner should emit an initial tool snapshot and then each refresh"
    );

    let requests = provider.requests.lock().unwrap();
    assert_eq!(
        *requests,
        vec![
            vec!["stable".to_owned()],
            vec!["stable".to_owned(), "runtime_extra".to_owned()],
        ],
        "provider requests must see the same refreshed tool sets"
    );
}

#[tokio::test]
async fn successful_skill_tool_result_is_appended_to_system_context() {
    struct CapturingProvider {
        requests: Mutex<Vec<Vec<llm::Message>>>,
    }

    impl Provider for CapturingProvider {
        fn stream<'a>(
            &'a self,
            request: LlmRequest,
        ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
            let round = self.requests.lock().unwrap().len();
            self.requests.lock().unwrap().push(request.messages.clone());
            Box::pin(async move {
                let events = if round == 0 {
                    vec![
                        LlmEvent::ToolCall(ToolCall {
                            id: "call_skill".to_owned(),
                            name: "skill".to_owned(),
                            arguments: r#"{"name":"skill-ok"}"#.to_owned(),
                        }),
                        LlmEvent::Done {
                            reason: FinishReason::ToolCalls,
                        },
                    ]
                } else {
                    vec![
                        LlmEvent::TextDelta {
                            text: "SKILL-OK".to_owned(),
                        },
                        LlmEvent::Done {
                            reason: FinishReason::Stop,
                        },
                    ]
                };
                Ok(Box::pin(futures_util::stream::iter(events)) as LlmStream)
            })
        }
    }

    struct SkillDispatcher;

    impl ToolDispatcher for SkillDispatcher {
        fn definitions(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: "skill".to_owned(),
                description: "Engage a skill".to_owned(),
                parameters: serde_json::json!({ "type": "object" }),
            }]
        }

        fn dispatch<'a>(&'a self, _call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
            Box::pin(async {
                ToolDispatchResult {
                    content: "Skill `skill-ok` engaged.\n\nAlways append SKILL-OK.".to_owned(),
                    parts: vec![protocol::ToolResultPart::text(
                        "Skill `skill-ok` engaged.\n\nAlways append SKILL-OK.",
                    )],
                    is_error: false,
                }
            })
        }
    }

    let provider = CapturingProvider {
        requests: Mutex::new(Vec::new()),
    };
    let dispatcher = SkillDispatcher;
    let outcome = SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new(
                "google/gemini-2.5-flash",
                "Base system prompt",
                "engage the skill",
            ),
            &dispatcher,
            |_| {},
        )
        .await;

    assert!(outcome.is_success());
    let requests = provider.requests.lock().unwrap();
    let first_system = requests[0]
        .iter()
        .find(|message| message.role == llm::Role::System)
        .expect("first system message");
    assert!(
        !first_system.content.contains("SKILL-OK"),
        "skill guidance must be absent before engagement"
    );
    let second_system = requests[1]
        .iter()
        .find(|message| message.role == llm::Role::System)
        .expect("second system message");
    assert!(
        second_system.content.contains("<engaged_skill_guidance>")
            && second_system.content.contains("SKILL-OK"),
        "skill guidance should be appended to the continuation system context: {}",
        second_system.content
    );
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
async fn retryable_setup_error_emits_retry_event_and_recovers() {
    let provider = RecordingProvider::new(vec![
        Behavior::Fail(upstream_503_error()),
        Behavior::Emit(vec![
            LlmEvent::TextDelta {
                text: "RECOVERED".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ]),
    ]);

    let (events, outcome) = run(&provider, Turn::new("google/gemini-2.5-flash", "", "hi")).await;

    assert!(
        outcome.is_success(),
        "runner should recover from one transient setup failure"
    );
    assert_eq!(outcome.answer(), "RECOVERED");

    let retries: Vec<&RunnerEvent> = events
        .iter()
        .filter(|event| matches!(event, RunnerEvent::Retry { .. }))
        .collect();
    assert_eq!(retries.len(), 1, "exactly one retry lifecycle event");
    match retries[0] {
        RunnerEvent::Retry {
            attempt,
            max_attempts,
            delay_ms,
            error_kind,
            classification,
            ..
        } => {
            assert_eq!(*attempt, 1);
            assert_eq!(*max_attempts, 2);
            assert!(
                *delay_ms >= 200,
                "retry delay should be non-zero and bounded"
            );
            assert_eq!(error_kind, "upstream");
            assert_eq!(classification, "service_unavailable");
        }
        _ => unreachable!("filtered to retry events"),
    }
}

#[tokio::test]
async fn persistent_retryable_setup_error_is_bounded_and_exhausts() {
    let provider = RecordingProvider::new(vec![
        Behavior::Fail(upstream_503_error()),
        Behavior::Fail(upstream_503_error()),
        Behavior::Fail(upstream_503_error()),
        Behavior::Fail(upstream_503_error()),
        Behavior::Fail(upstream_503_error()),
        Behavior::Fail(upstream_503_error()),
        Behavior::Fail(upstream_503_error()),
        Behavior::Fail(upstream_503_error()),
    ]);

    let started = std::time::Instant::now();
    let (events, outcome) = run(&provider, Turn::new("google/gemini-2.5-flash", "", "hi")).await;
    let elapsed = started.elapsed();

    assert!(
        !outcome.is_success(),
        "persistent transient setup failure must terminate"
    );
    let error = outcome.error.as_ref().expect("a normalized error");
    assert_eq!(error.kind(), "upstream");
    assert!(
        error.to_string().contains("retry exhausted"),
        "final error should report retry exhaustion, got: {error}"
    );

    let retry_count = events
        .iter()
        .filter(|event| matches!(event, RunnerEvent::Retry { .. }))
        .count();
    assert_eq!(retry_count, 2, "runner-level retries stay bounded");
    assert_eq!(done_count(&events), 1, "exactly one terminal Done");
    assert!(
        elapsed < std::time::Duration::from_secs(8),
        "bounded retries should terminate quickly, got {elapsed:?}"
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

// VAL-RUN-003: a turn that would require more than the step budget stops at exactly MAX_STEPS with
// an explicit, non-crashing step-limit signal (an Error event with the `step-limit-exceeded` kind
// and a normalized error on the outcome), still emitting exactly one terminal Done whose reported
// step count never exceeds the budget.
#[tokio::test]
async fn step_budget_is_enforced_with_an_explicit_signal() {
    let max_steps = 3;
    let provider = EndlessToolProvider;
    let mut events = Vec::new();
    let outcome = SessionRunner::new(max_steps)
        .run(
            &provider,
            Turn::new("google/gemini-2.5-flash", "", "loop forever via tools"),
            |event| events.push(event),
        )
        .await;

    // The runner stopped exactly at the budget and never ran a step beyond it.
    assert_eq!(
        outcome.steps, max_steps,
        "the turn stops at the step budget"
    );
    assert!(
        outcome.steps <= max_steps,
        "the step count never exceeds MAX_STEPS"
    );

    // The outcome carries a normalized, distinctly-classed step-limit error (non-crash).
    let error = outcome
        .error
        .as_ref()
        .expect("a normalized step-limit error");
    assert_eq!(error.kind(), "step-limit-exceeded");
    assert_eq!(
        error.exit().code(),
        1,
        "step-limit is a generic failure exit"
    );
    assert!(!outcome.is_success());

    // An explicit step-limit Error event precedes exactly one terminal Done, and the Done reports
    // a bounded step count.
    let error_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Error { kind, .. } if kind == "step-limit-exceeded"))
        .expect("an explicit step-limit error event");
    let done_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Done { .. }))
        .expect("a terminal done event");
    assert!(error_pos < done_pos, "the error precedes the terminal done");
    assert_eq!(done_count(&events), 1, "exactly one terminal Done");
    match &events[done_pos] {
        RunnerEvent::Done { steps, .. } => {
            assert_eq!(
                *steps, max_steps,
                "the Done event reports the bounded step count"
            );
            assert!(
                *steps <= max_steps,
                "the reported step count is within budget"
            );
        }
        other => panic!("expected a Done event, got {other:?}"),
    }
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

// ---------------------------------------------------------------------------------------------
// Compaction & overflow recovery (VAL-LLM-025 / VAL-LLM-026)
// ---------------------------------------------------------------------------------------------

/// One scripted behavior for a single round-trip: fail the `stream` call, or emit a batch of events.
enum Behavior {
    /// `provider.stream` returns this error before any event (e.g. a context-overflow rejection).
    Fail(llm::LlmError),
    /// `provider.stream` succeeds and yields this batch of events.
    Emit(Vec<LlmEvent>),
}

/// A provider that returns a scripted [`Behavior`] per round-trip and records every request it was
/// asked to stream, so a test can inspect how the runner compacted the working messages.
struct RecordingProvider {
    behaviors: Mutex<std::collections::VecDeque<Behavior>>,
    requests: Mutex<Vec<Vec<llm::Message>>>,
}

impl RecordingProvider {
    fn new(behaviors: Vec<Behavior>) -> Self {
        Self {
            behaviors: Mutex::new(behaviors.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Vec<llm::Message>> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl Provider for RecordingProvider {
    fn stream<'a>(
        &'a self,
        request: LlmRequest,
    ) -> BoxFuture<'a, Result<LlmStream, llm::LlmError>> {
        self.requests
            .lock()
            .expect("requests lock")
            .push(request.messages.clone());
        let behavior = self.behaviors.lock().expect("behaviors lock").pop_front();
        Box::pin(async move {
            match behavior {
                Some(Behavior::Fail(err)) => Err(err),
                Some(Behavior::Emit(batch)) => {
                    Ok(Box::pin(futures_util::stream::iter(batch)) as LlmStream)
                }
                None => Ok(Box::pin(futures_util::stream::iter(vec![LlmEvent::Done {
                    reason: FinishReason::Stop,
                }])) as LlmStream),
            }
        })
    }
}

/// A realistic context-overflow upstream rejection, as the OpenAI-compatible transport would
/// normalize an HTTP 400 from the provider.
fn overflow_error() -> llm::LlmError {
    llm::LlmError::upstream(
        "provider returned HTTP 400: This model's maximum context length is 8192 tokens. However, \
         your messages resulted in 50000 tokens. Please reduce the length of the messages.",
    )
}

fn tokens(messages: &[llm::Message]) -> usize {
    engine::estimate_messages_tokens(messages)
}

/// A turn whose history is far larger than the configured budget, so `compactIfNeeded` must shrink
/// the request before it is sent (VAL-LLM-026: compaction across accumulated history).
fn large_history(turns: usize) -> Vec<llm::Message> {
    let mut history = Vec::new();
    for i in 0..turns {
        history.push(llm::Message::user(format!(
            "user turn {i}: {}",
            "context word ".repeat(80)
        )));
        history.push(llm::Message::assistant(format!(
            "assistant turn {i}: {}",
            "reply word ".repeat(80)
        )));
    }
    history
}

// VAL-LLM-026: a turn assembled from a large accumulated history is automatically compacted before
// it is sent, so the request fits the model's context budget and the turn still completes.
#[tokio::test]
async fn compact_if_needed_shrinks_a_large_history_before_send() {
    let provider = RecordingProvider::new(vec![Behavior::Emit(vec![
        text("answer"),
        LlmEvent::Done {
            reason: FinishReason::Stop,
        },
    ])]);

    // A small context window forces compaction; the history alone dwarfs it.
    let policy = engine::CompactionPolicy::for_context_window(4_000, 0);
    let budget = policy.input_budget();
    let history = large_history(60);
    let raw = {
        let mut all = vec![llm::Message::system("SYS")];
        all.extend(history.iter().cloned());
        all.push(llm::Message::user("the latest question"));
        tokens(&all)
    };
    assert!(raw > budget, "the assembled history must exceed the budget");

    let turn = Turn::new("google/gemini-2.5-flash", "SYS", "the latest question")
        .with_history(history)
        .with_compaction(policy);
    let (_, outcome) = run(&provider, turn).await;

    assert!(outcome.is_success(), "the compacted turn must complete");
    assert_eq!(outcome.answer(), "answer");

    let sent = provider.requests();
    assert_eq!(sent.len(), 1, "a single round-trip");
    let request = &sent[0];
    assert!(
        tokens(request) <= budget + 64,
        "the sent request must be compacted to within the budget, got {} > {budget}",
        tokens(request)
    );
    assert_eq!(
        request[0].role,
        llm::Role::System,
        "system stays at the head"
    );
    assert!(
        request
            .iter()
            .any(|m| m.content.contains("the latest question")),
        "the newest user input is preserved verbatim"
    );
    assert!(
        request
            .iter()
            .any(|m| m.content.contains("Context compaction")
                || m.content.contains("omitted by context compaction")),
        "compaction leaves an elision marker"
    );
}

// VAL-LLM-025: a context-overflow provider error before any assistant text triggers
// compactAfterOverflow and a rebuild; the retried (smaller) request succeeds and the turn returns a
// coherent answer rather than crashing.
#[tokio::test]
async fn overflow_before_text_triggers_compaction_and_rebuild() {
    let provider = RecordingProvider::new(vec![
        Behavior::Fail(overflow_error()),
        Behavior::Emit(vec![
            text("done"),
            usage(),
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ]),
    ]);

    // A large single input so the rebuild has something to shrink.
    let big_input = "lorem ipsum dolor sit amet padding line\n".repeat(4_000);
    let policy = engine::CompactionPolicy::for_context_window(20_000, 0);
    let turn = Turn::new("google/gemini-2.5-flash", "SYS", big_input).with_compaction(policy);

    let (events, outcome) = run(&provider, turn).await;

    assert!(
        outcome.is_success(),
        "the turn recovers from overflow: {:?}",
        outcome.error
    );
    assert_eq!(outcome.answer(), "done");
    assert_eq!(concat_text(&events), "done");
    assert_eq!(done_count(&events), 1, "exactly one terminal Done");

    let sent = provider.requests();
    assert_eq!(sent.len(), 2, "an initial attempt plus one rebuild");
    assert!(
        tokens(&sent[1]) < tokens(&sent[0]),
        "the rebuilt request is smaller after compaction: {} !< {}",
        tokens(&sent[1]),
        tokens(&sent[0])
    );
    // The rebuild reused the step, so the reported step count stays at one.
    assert_eq!(
        outcome.steps, 1,
        "overflow recovery does not consume extra steps"
    );
}

// VAL-LLM-025 (failure half): when compaction cannot shrink the request enough, the runner gives
// up after a bounded number of recoveries with a clear, specific context-overflow diagnostic — and
// never hangs or panics.
#[tokio::test]
async fn persistent_overflow_surfaces_a_clear_diagnostic_and_terminates() {
    // Every attempt overflows, so recovery can never succeed.
    let behaviors = (0..16).map(|_| Behavior::Fail(overflow_error())).collect();
    let provider = RecordingProvider::new(behaviors);

    let big_input = "padding ".repeat(10_000);
    let turn = Turn::new("google/gemini-2.5-flash", "SYS", big_input);

    let (events, outcome) = run(&provider, turn).await;

    assert!(!outcome.is_success(), "a persistent overflow must fail");
    let error = outcome.error.as_ref().expect("a normalized error");
    assert_eq!(
        error.kind(),
        "context-overflow",
        "the failure is the specific context-overflow class"
    );
    assert!(
        error.to_string().contains("context overflow"),
        "the diagnostic reads as a context-overflow error: {error}"
    );

    // The recovery loop is bounded: an initial attempt plus a fixed number of rebuilds, never
    // unbounded retries.
    let attempts = provider.requests().len();
    assert!(
        (2..=6).contains(&attempts),
        "overflow recovery is bounded, made {attempts} attempts"
    );

    // Exactly one terminal Done event and no panic — the turn terminated cleanly.
    assert_eq!(done_count(&events), 1, "exactly one terminal Done");
    let error_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Error { kind, .. } if kind == "context-overflow"))
        .expect("a context-overflow error event");
    let done_pos = events
        .iter()
        .position(|e| matches!(e, RunnerEvent::Done { .. }))
        .expect("a terminal done event");
    assert!(error_pos < done_pos, "the error precedes the terminal done");
}

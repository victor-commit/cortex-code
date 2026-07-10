//! End-to-end (offline) proof that the `edit` tool is reachable through the durable runner's
//! tool-dispatch seam: a scripted provider drives a `read` then an `edit` call, the runner
//! materializes the advertised tool catalogue, dispatches the calls to the real
//! [`tools::ToolRegistry`], and the file is mutated on disk by exactly the requested substitution.
//!
//! This exercises the same path the live CLI uses (`SessionRunner::run_with_tools` + a registry
//! dispatcher), differing only in the provider behind the [`llm::Provider`] trait.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use engine::{RunnerEvent, SessionRunner, ToolDispatchResult, ToolDispatcher, Turn};
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, ToolCall};
use tools::{ToolContext, ToolRegistry};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-edit-it-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// A provider that replays one scripted batch of events per round-trip.
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

/// A dispatcher backed by the real tool registry (mirrors the CLI's bridge).
struct RegistryDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
}

impl ToolDispatcher for RegistryDispatcher {
    fn definitions(&self) -> Vec<llm::ToolDef> {
        self.registry.definitions()
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
        Box::pin(async move {
            let output = self
                .registry
                .execute(&call.name, &call.arguments, &self.context)
                .await;
            ToolDispatchResult {
                content: output.model_content(),
                trusted_skill_guidance: None,
                parts: output.parts,
                mcp_result: None,
                is_error: output.is_error,
            }
        })
    }
}

fn tool_call(id: &str, name: &str, arguments: &str) -> LlmEvent {
    LlmEvent::ToolCall(ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        arguments: arguments.to_owned(),
    })
}

fn result_for(events: &[RunnerEvent], name: &str) -> (Vec<protocol::ToolResultPart>, bool) {
    events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult {
                name: n,
                parts,
                is_error,
                ..
            } if n == name => Some((parts.clone(), *is_error)),
            _ => None,
        })
        .expect("a tool result event")
}

fn result_text(parts: &[protocol::ToolResultPart]) -> String {
    parts
        .iter()
        .filter_map(protocol::ToolResultPart::as_text)
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase()
}

#[tokio::test]
async fn runner_dispatches_read_then_edit_and_applies_exact_substitution() {
    let dir = temp_dir("exact");
    fs::write(dir.join("a.txt"), "alpha beta gamma").unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    // Round 1: read the file (satisfies the prior-read guard). Round 2: edit it. Round 3: answer.
    let provider = ScriptedProvider::new(vec![
        vec![
            tool_call("c1", "read", r#"{"path":"a.txt"}"#),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            tool_call(
                "c2",
                "edit",
                r#"{"path":"a.txt","oldString":"beta","newString":"DELTA"}"#,
            ),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "done".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]);

    let mut events = Vec::new();
    let outcome = SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "edit a.txt"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(
        fs::read_to_string(dir.join("a.txt")).unwrap(),
        "alpha DELTA gamma"
    );

    let (_, is_error) = result_for(&events, "edit");
    assert!(!is_error, "the edit succeeded");
    assert!(events.iter().any(|event| matches!(
        event,
        RunnerEvent::ToolCall { name, .. } if name == "edit"
    )));
}

#[tokio::test]
async fn runner_refuses_edit_of_an_unread_file_and_keeps_original_bytes() {
    let dir = temp_dir("unread");
    fs::write(dir.join("untouched.txt"), "ORIGINAL").unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    // The model edits without reading first; the guard must fire and the bytes must be intact.
    let provider = ScriptedProvider::new(vec![
        vec![
            tool_call(
                "c1",
                "edit",
                r#"{"path":"untouched.txt","oldString":"ORIGINAL","newString":"CHANGED"}"#,
            ),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "ok".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]);

    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "edit untouched.txt"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    let (parts, is_error) = result_for(&events, "edit");
    assert!(is_error, "the unread-edit guard must fire");
    assert!(result_text(&parts).contains("read it first"));
    assert_eq!(
        fs::read_to_string(dir.join("untouched.txt")).unwrap(),
        "ORIGINAL"
    );
}

#[tokio::test]
async fn runner_reports_ambiguous_edit_and_leaves_file_unchanged() {
    let dir = temp_dir("ambig");
    fs::write(dir.join("d.txt"), "dup dup").unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    let provider = ScriptedProvider::new(vec![
        vec![
            tool_call("c1", "read", r#"{"path":"d.txt"}"#),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            tool_call(
                "c2",
                "edit",
                r#"{"path":"d.txt","oldString":"dup","newString":"X"}"#,
            ),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "ok".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]);

    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "edit d.txt"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    let (parts, is_error) = result_for(&events, "edit");
    assert!(is_error, "the ambiguity guard must fire");
    assert!(result_text(&parts).contains("matches"));
    assert_eq!(fs::read_to_string(dir.join("d.txt")).unwrap(), "dup dup");
}

#[tokio::test]
async fn runner_applies_multiline_near_match_and_reports_fuzzy_strategy() {
    let dir = temp_dir("fuzzy-near");
    fs::write(
        dir.join("main.rs"),
        concat!(
            "fn main() {\n",
            "        if  ready  {\n",
            "            do_work();\n",
            "            finalize();\n",
            "        }\n",
            "}\n"
        ),
    )
    .unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    let provider = ScriptedProvider::new(vec![
        vec![
            tool_call("c1", "read", r#"{"path":"main.rs"}"#),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            tool_call(
                "c2",
                "edit",
                r#"{"path":"main.rs","oldString":"if ready {\n  do_work();\n  finalize();\n}","newString":"if ready {\n  do_more_work();\n  finalize();\n}"}"#,
            ),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "done".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]);

    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "edit main.rs"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    let (parts, is_error) = result_for(&events, "edit");
    assert!(!is_error, "fuzzy replacement should succeed");
    let output_text = result_text(&parts);
    assert!(
        output_text.contains("fuzzy match"),
        "tool result should explain fuzzy strategy: {output_text}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("main.rs")).unwrap(),
        concat!(
            "fn main() {\n",
            "if ready {\n",
            "  do_more_work();\n",
            "  finalize();\n",
            "}\n",
            "}\n"
        )
    );
}

#[tokio::test]
async fn runner_uses_context_to_edit_the_best_matching_repeated_block() {
    let dir = temp_dir("context-window");
    fs::write(
        dir.join("window.rs"),
        concat!(
            "fn choose() {\n",
            "    let count = 10;\n",
            "    println!(\"done\");\n",
            "}\n",
            "\n",
            "fn choose() {\n",
            "    let total = 99;\n",
            "    println!(\"done\");\n",
            "}\n"
        ),
    )
    .unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    let provider = ScriptedProvider::new(vec![
        vec![
            tool_call("c1", "read", r#"{"path":"window.rs"}"#),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            tool_call(
                "c2",
                "edit",
                r#"{"path":"window.rs","oldString":"fn choose() {\n  let total = 100;\n  println!(\"done\");\n}","newString":"fn choose() {\n    let total = 0;\n    println!(\"done\");\n}"}"#,
            ),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "done".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]);

    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "edit window.rs"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    let (parts, is_error) = result_for(&events, "edit");
    assert!(!is_error, "context-aware fuzzy edit should succeed");
    let output_text = result_text(&parts);
    assert!(
        output_text.contains("anchored line-window fuzzy match"),
        "expected anchored line-window detail, got: {output_text}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("window.rs")).unwrap(),
        concat!(
            "fn choose() {\n",
            "    let count = 10;\n",
            "    println!(\"done\");\n",
            "}\n",
            "\n",
            "fn choose() {\n",
            "    let total = 0;\n",
            "    println!(\"done\");\n",
            "}\n"
        )
    );
}

#[tokio::test]
async fn runner_not_found_error_includes_nearby_candidates_and_preserves_bytes() {
    let dir = temp_dir("notfound-candidates");
    fs::write(
        dir.join("main.txt"),
        "alpha beta gamma\ndelta epsilon zeta\neta theta iota\n",
    )
    .unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    let provider = ScriptedProvider::new(vec![
        vec![
            tool_call("c1", "read", r#"{"path":"main.txt"}"#),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            tool_call(
                "c2",
                "edit",
                r#"{"path":"main.txt","oldString":"delta epsilon omega","newString":"replacement"}"#,
            ),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: "done".to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]);

    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "edit main.txt"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    let (parts, is_error) = result_for(&events, "edit");
    assert!(
        is_error,
        "edit should fail when no safe fuzzy target exists"
    );
    let output_text = result_text(&parts);
    assert!(
        output_text.contains("closest candidates"),
        "expected actionable nearby snippets, got: {output_text}"
    );
    assert_eq!(
        fs::read_to_string(dir.join("main.txt")).unwrap(),
        "alpha beta gamma\ndelta epsilon zeta\neta theta iota\n"
    );
}

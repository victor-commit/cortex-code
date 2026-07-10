//! End-to-end (offline) proof that the `write` tool is reachable through the durable runner's
//! tool-dispatch seam: a scripted provider requests a `write` call, the runner materializes the
//! advertised tool catalogue, dispatches the call to the real [`tools::ToolRegistry`], the file is
//! created on disk with byte-identical content, and a structured tool result is re-injected so the
//! turn completes.
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
        "cortex-write-it-{tag}-{}-{unique}",
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
                parts: output.parts,
                mcp_result: None,
                is_error: output.is_error,
            }
        })
    }
}

#[tokio::test]
async fn runner_dispatches_write_and_creates_the_file_byte_identical() {
    let dir = temp_dir("create");

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    // Round 1: the model calls `write`. Round 2: it answers and stops.
    let provider = ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_write".to_owned(),
                name: "write".to_owned(),
                arguments: r#"{"path":"new.txt","content":"hello\nworld\n"}"#.to_owned(),
            }),
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
            Turn::new("m", "", "write new.txt"),
            &dispatcher,
            |event| {
                events.push(event);
            },
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.answer(), "done");
    assert_eq!(outcome.steps, 2, "the turn continued after the tool call");

    // The file exists on disk with byte-identical content.
    let written = fs::read(dir.join("new.txt")).unwrap();
    assert_eq!(written, b"hello\nworld\n");

    // The runner surfaced the write tool call and a non-error structured result.
    assert!(events.iter().any(|event| matches!(
        event,
        RunnerEvent::ToolCall { name, .. } if name == "write"
    )));
    let is_error = events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult { name, is_error, .. } if name == "write" => Some(*is_error),
            _ => None,
        })
        .expect("a write tool result event");
    assert!(!is_error, "the write succeeded");
}

#[tokio::test]
async fn runner_refuses_overwrite_of_an_unread_file_and_keeps_original_bytes() {
    let dir = temp_dir("unread");
    fs::write(dir.join("cfg2.txt"), "ORIGINAL").unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    let provider = ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_write".to_owned(),
                name: "write".to_owned(),
                arguments: r#"{"path":"cfg2.txt","content":"NEW"}"#.to_owned(),
            }),
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
            Turn::new("m", "", "write cfg2.txt"),
            &dispatcher,
            |event| {
                events.push(event);
            },
        )
        .await;

    // The guard fired: the result is an error and the original bytes are intact.
    let (parts, is_error) = events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult {
                name,
                parts,
                is_error,
                ..
            } if name == "write" => Some((parts.clone(), *is_error)),
            _ => None,
        })
        .expect("a write tool result event");
    assert!(is_error, "the unread-overwrite guard must fire");
    let text: String = parts
        .iter()
        .filter_map(protocol::ToolResultPart::as_text)
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(text.contains("read it first"), "got: {text}");
    assert_eq!(
        fs::read_to_string(dir.join("cfg2.txt")).unwrap(),
        "ORIGINAL"
    );
}

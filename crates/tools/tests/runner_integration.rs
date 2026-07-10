//! End-to-end (offline) proof that the `read` tool is reachable through the durable runner's
//! tool-dispatch seam: a scripted provider requests a `read` call, the runner materializes the
//! advertised tool catalogue, dispatches the call to the real [`tools::ToolRegistry`], surfaces a
//! structured [`engine::RunnerEvent::ToolResult`], and re-injects the result so the turn completes.
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
        "cortex-read-it-{tag}-{}-{unique}",
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
async fn runner_dispatches_read_and_reinjects_line_numbered_content() {
    let dir = temp_dir("dispatch");
    fs::write(dir.join("sample.txt"), "alpha\nbeta\ngamma\n").unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    // Round 1: the model calls `read`. Round 2: it answers and stops.
    let provider = ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_read".to_owned(),
                name: "read".to_owned(),
                arguments: r#"{"path":"sample.txt"}"#.to_owned(),
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
            Turn::new("m", "", "read sample.txt"),
            &dispatcher,
            |event| {
                events.push(event);
            },
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.answer(), "done");
    assert_eq!(outcome.steps, 2, "the turn continued after the tool call");

    // The runner surfaced the tool call and its structured result.
    assert!(events.iter().any(|event| matches!(
        event,
        RunnerEvent::ToolCall { name, .. } if name == "read"
    )));
    let tool_result = events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult {
                name,
                parts,
                is_error,
                ..
            } if name == "read" => Some((parts.clone(), *is_error)),
            _ => None,
        })
        .expect("a read tool result event");
    assert!(!tool_result.1, "the read succeeded");
    let text: String = tool_result
        .0
        .iter()
        .filter_map(protocol::ToolResultPart::as_text)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(text, "1: alpha\n2: beta\n3: gamma");
}

#[tokio::test]
async fn runner_surfaces_image_attachment_part_not_bytes() {
    let dir = temp_dir("image");
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0x00, 0xFF, 0x10]);
    fs::write(dir.join("logo.png"), bytes).unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    let provider = ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_img".to_owned(),
                name: "read".to_owned(),
                arguments: r#"{"path":"logo.png"}"#.to_owned(),
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
            Turn::new("m", "", "read logo.png"),
            &dispatcher,
            |event| {
                events.push(event);
            },
        )
        .await;

    let parts = events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult { name, parts, .. } if name == "read" => Some(parts.clone()),
            _ => None,
        })
        .expect("a read tool result event");
    let attachment = parts
        .iter()
        .find(|part| part.is_attachment())
        .expect("an image attachment part");
    assert_eq!(attachment.media_type(), Some("image/png"));
    for part in &parts {
        if let Some(text) = part.as_text() {
            assert!(!text.as_bytes().contains(&0xFF));
        }
    }
}

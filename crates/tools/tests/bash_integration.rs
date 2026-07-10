//! End-to-end (offline) proof that the `bash` tool is reachable through the durable runner's
//! tool-dispatch seam: a scripted provider requests a `bash` call, the runner materializes the
//! advertised tool catalogue, dispatches the call to the real [`tools::ToolRegistry`], surfaces a
//! structured [`engine::RunnerEvent::ToolResult`], and re-injects the result so the turn completes.
//!
//! This exercises the same path the live CLI uses (`SessionRunner::run_with_tools` + a registry
//! dispatcher), differing only in the provider behind the [`llm::Provider`] trait. The `bash`
//! mechanics themselves (capture, gating, timeout, background, truncation) are unit-tested in the
//! `tools` crate; here we prove the tool is wired into the runner and produces real effects.

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
        "cortex-bash-it-{tag}-{}-{unique}",
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

/// Build the two rounds for a single tool call: round 1 calls `bash`, round 2 answers and stops.
fn one_bash_call(arguments: &str) -> Vec<Vec<LlmEvent>> {
    vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_bash".to_owned(),
                name: "bash".to_owned(),
                arguments: arguments.to_owned(),
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
    ]
}

fn bash_result(events: &[RunnerEvent]) -> (String, bool) {
    events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult {
                name,
                parts,
                is_error,
                ..
            } if name == "bash" => {
                let text = parts
                    .iter()
                    .filter_map(protocol::ToolResultPart::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                Some((text, *is_error))
            }
            _ => None,
        })
        .expect("a bash tool result event")
}

// VAL-TOOLS-032/042: the runner dispatches a `bash` call; stdout + exit are captured and the
// command's real on-disk effect lands in the working directory.
#[tokio::test]
async fn runner_dispatches_bash_and_captures_output_and_effect() {
    let dir = temp_dir("dispatch");

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };
    let provider = ScriptedProvider::new(one_bash_call(
        r#"{"command":"echo cortex-ok && touch made.txt"}"#,
    ));

    let mut events = Vec::new();
    let outcome = SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "run echo"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.answer(), "done");

    let (text, is_error) = bash_result(&events);
    assert!(!is_error, "the command succeeded: {text}");
    assert!(text.contains("cortex-ok"), "stdout captured: {text}");
    assert!(
        text.contains("[exit code: 0]"),
        "zero exit reported: {text}"
    );
    assert!(
        dir.join("made.txt").exists(),
        "the real side effect landed in cwd"
    );
}

// VAL-TOOLS-033: a failing command surfaces a non-zero exit and stderr through the runner.
#[tokio::test]
async fn runner_surfaces_a_failing_command() {
    let dir = temp_dir("fail");
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };
    let provider = ScriptedProvider::new(one_bash_call(r#"{"command":"ls /no/such/dir"}"#));

    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "list a missing dir"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    let (text, is_error) = bash_result(&events);
    assert!(is_error, "a failing command is marked an error");
    assert!(!text.contains("[exit code: 0]"), "non-zero exit: {text}");
    assert!(
        text.contains("/no/such/dir"),
        "stderr names the missing path: {text}"
    );
}

// VAL-TOOLS-037: large output is truncated in-band and spilled complete to disk, through the runner.
#[tokio::test]
async fn runner_truncates_and_spills_large_output() {
    let dir = temp_dir("spill");
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };
    let provider = ScriptedProvider::new(one_bash_call(r#"{"command":"seq 1 200000"}"#));

    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", "print many lines"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    let (text, _is_error) = bash_result(&events);
    assert!(
        text.contains("output truncated"),
        "truncation notice present: {text}"
    );
    assert!(text.lines().count() < 200000, "in-band result is bounded");

    let marker = "full output written to ";
    let start = text.find(marker).expect("a spill pointer") + marker.len();
    let rest = &text[start..];
    let end = rest.find(']').expect("notice closes with ]");
    let spilled = fs::read_to_string(&rest[..end]).expect("read spill file");
    assert_eq!(
        spilled.lines().count(),
        200000,
        "spill file holds every line"
    );
    let _ = fs::remove_file(&rest[..end]);
}

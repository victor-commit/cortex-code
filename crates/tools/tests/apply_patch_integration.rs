//! End-to-end (offline) proof that `apply_patch` is reachable through the durable runner's
//! tool-dispatch seam, and that the GPT-family tool set is materialized in place of `edit`/`write`.
//!
//! A scripted provider requests an `apply_patch` call; the runner materializes the GPT tool
//! catalogue, dispatches the call to the real [`tools::ToolRegistry`], and the files are mutated on
//! disk atomically. This exercises the same path the live CLI uses
//! (`SessionRunner::run_with_tools` + a registry dispatcher), differing only in the provider behind
//! the [`llm::Provider`] trait.

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
        "cortex-apply-patch-it-{tag}-{}-{unique}",
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

/// A dispatcher backed by the real GPT-family tool registry (mirrors the CLI's bridge).
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

#[test]
fn gpt_dispatcher_advertises_apply_patch_and_not_edit_or_write() {
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::gpt(),
        context: ToolContext::new(std::env::temp_dir()),
    };
    let names: Vec<String> = dispatcher
        .definitions()
        .into_iter()
        .map(|def| def.name)
        .collect();
    assert!(names.contains(&"apply_patch".to_owned()), "got: {names:?}");
    assert!(!names.contains(&"edit".to_owned()), "got: {names:?}");
    assert!(!names.contains(&"write".to_owned()), "got: {names:?}");
}

#[tokio::test]
async fn runner_dispatches_apply_patch_and_applies_a_multi_file_envelope() {
    let dir = temp_dir("multi");
    fs::write(dir.join("update.txt"), "old line\n").unwrap();
    fs::write(dir.join("delete.txt"), "remove me\n").unwrap();

    // The Update/Delete targets must have been read this session (consistent with write/edit).
    let context = ToolContext::new(dir.clone());
    context.record_read(dir.join("update.txt"));
    context.record_read(dir.join("delete.txt"));
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::gpt(),
        context,
    };

    let patch = "*** Begin Patch\n\
*** Add File: add.txt\n\
+created\n\
*** Update File: update.txt\n\
@@\n\
-old line\n\
+new line\n\
*** Delete File: delete.txt\n\
*** End Patch";
    let arguments = serde_json::json!({ "patch": patch }).to_string();

    let provider = ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_patch".to_owned(),
                name: "apply_patch".to_owned(),
                arguments,
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
            Turn::new("openai/gpt-4o-mini", "", "apply the patch"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    assert!(outcome.is_success());
    assert_eq!(outcome.answer(), "done");

    // Every section was applied on disk.
    assert_eq!(
        fs::read_to_string(dir.join("add.txt")).unwrap(),
        "created\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("update.txt")).unwrap(),
        "new line\n"
    );
    assert!(!dir.join("delete.txt").exists());

    // The runner surfaced a non-error structured apply_patch result.
    let is_error = events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult { name, is_error, .. } if name == "apply_patch" => {
                Some(*is_error)
            }
            _ => None,
        })
        .expect("an apply_patch tool result event");
    assert!(!is_error, "the patch applied successfully");
}

#[tokio::test]
async fn runner_apply_patch_broken_envelope_leaves_files_unchanged() {
    let dir = temp_dir("atomic");
    fs::write(dir.join("update.txt"), "old line\n").unwrap();
    fs::write(dir.join("delete.txt"), "remove me\n").unwrap();

    // Reads are recorded so the read-guard passes and the failure is genuinely the broken hunk.
    let context = ToolContext::new(dir.clone());
    context.record_read(dir.join("update.txt"));
    context.record_read(dir.join("delete.txt"));
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::gpt(),
        context,
    };

    // The update section's context cannot be found, so the whole envelope must fail atomically.
    let patch = "*** Begin Patch\n\
*** Add File: add.txt\n\
+created\n\
*** Update File: update.txt\n\
@@\n\
-NOT PRESENT\n\
+new line\n\
*** Delete File: delete.txt\n\
*** End Patch";
    let arguments = serde_json::json!({ "patch": patch }).to_string();

    let provider = ScriptedProvider::new(vec![
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: "call_patch".to_owned(),
                name: "apply_patch".to_owned(),
                arguments,
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
            Turn::new("openai/gpt-4o-mini", "", "apply the patch"),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    // No section was applied: nothing added, nothing updated, nothing deleted.
    assert!(!dir.join("add.txt").exists(), "no file was added");
    assert_eq!(
        fs::read_to_string(dir.join("update.txt")).unwrap(),
        "old line\n",
        "the update target is unchanged"
    );
    assert!(
        dir.join("delete.txt").exists(),
        "the delete target survives"
    );

    let is_error = events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult { name, is_error, .. } if name == "apply_patch" => {
                Some(*is_error)
            }
            _ => None,
        })
        .expect("an apply_patch tool result event");
    assert!(is_error, "the broken envelope is reported as an error");
}

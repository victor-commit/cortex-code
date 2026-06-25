//! End-to-end (offline) proof that the non-mutating `glob`, `grep`, and `todo` tools are reachable
//! through the durable runner's tool-dispatch seam: a scripted provider requests each call, the
//! runner materializes the advertised catalogue, dispatches to the real [`tools::ToolRegistry`],
//! and surfaces a structured [`engine::RunnerEvent::ToolResult`]. This mirrors the live CLI path
//! (`SessionRunner::run_with_tools` + a registry dispatcher), differing only in the provider behind
//! the [`llm::Provider`] trait and using an in-memory [`tools::TodoStore`] in place of SQLite.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use engine::{RunnerEvent, SessionRunner, ToolDispatchResult, ToolDispatcher, Turn};
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, ToolCall};
use protocol::{TodoItem, ToolResultPart};
use tools::{TodoStore, ToolContext, ToolRegistry};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_tree(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-search-it-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).expect("create tree");
    fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.join("src/b.rs"), "fn helper() {}\n").unwrap();
    fs::write(dir.join("c.txt"), "plain text, no functions\n").unwrap();
    dir
}

/// An in-memory [`TodoStore`] double keyed by session id.
#[derive(Default)]
struct MemoryTodoStore {
    sessions: Mutex<HashMap<String, Vec<TodoItem>>>,
}

impl TodoStore for MemoryTodoStore {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.to_owned(), items.to_vec());
        Ok(())
    }
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

/// A dispatcher backed by the real tool registry and a shared context (mirrors the CLI's bridge).
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
                is_error: output.is_error,
            }
        })
    }
}

/// One tool round (the call) followed by a terminating round (the answer), so the runner dispatches
/// the call and then completes.
fn call_then_done(call: ToolCall, answer: &str) -> Vec<Vec<LlmEvent>> {
    vec![
        vec![
            LlmEvent::ToolCall(call),
            LlmEvent::Done {
                reason: FinishReason::ToolCalls,
            },
        ],
        vec![
            LlmEvent::TextDelta {
                text: answer.to_owned(),
            },
            LlmEvent::Done {
                reason: FinishReason::Stop,
            },
        ],
    ]
}

/// Drive a single scripted tool call through the runner and return the matching tool result's text.
async fn dispatch_tool_text(
    dir: &std::path::Path,
    context: ToolContext,
    tool: &str,
    arguments: &str,
) -> (String, bool) {
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context,
    };
    let provider = ScriptedProvider::new(call_then_done(
        ToolCall {
            id: format!("call_{tool}"),
            name: tool.to_owned(),
            arguments: arguments.to_owned(),
        },
        "ok",
    ));

    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new("m", "", format!("use {tool} in {}", dir.display())),
            &dispatcher,
            |event| events.push(event),
        )
        .await;

    let (parts, is_error) = events
        .iter()
        .find_map(|event| match event {
            RunnerEvent::ToolResult {
                name,
                parts,
                is_error,
                ..
            } if name == tool => Some((parts.clone(), *is_error)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a {tool} tool result event"));
    let text = parts
        .iter()
        .filter_map(ToolResultPart::as_text)
        .collect::<Vec<_>>()
        .join("\n");
    (text, is_error)
}

#[tokio::test]
async fn runner_dispatches_glob_and_returns_matching_paths() {
    let dir = temp_tree("glob");
    let (text, is_error) = dispatch_tool_text(
        &dir,
        ToolContext::new(dir.clone()),
        "glob",
        r#"{"pattern":"**/*.rs"}"#,
    )
    .await;
    assert!(!is_error, "glob succeeded: {text}");
    assert!(text.contains("a.rs"), "got: {text}");
    assert!(text.contains("src/b.rs"), "got: {text}");
    assert!(!text.contains("c.txt"), "non-matches excluded: {text}");
}

#[tokio::test]
async fn runner_dispatches_grep_and_returns_matching_lines() {
    let dir = temp_tree("grep");
    let (text, is_error) = dispatch_tool_text(
        &dir,
        ToolContext::new(dir.clone()),
        "grep",
        r#"{"pattern":"fn\\s+\\w+"}"#,
    )
    .await;
    assert!(!is_error, "grep succeeded: {text}");
    assert!(text.contains("a.rs:1:fn main"), "got: {text}");
    assert!(text.contains("src/b.rs:1:fn helper"), "got: {text}");
    assert!(!text.contains("c.txt"), "non-matching file absent: {text}");
}

// VAL-TOOLS-037: a `read` of a file far exceeding the limits, dispatched through the runner, returns
// a bounded in-band result naming a spill file that holds the complete file content.
#[tokio::test]
async fn runner_dispatches_read_of_huge_file_spills_complete_content() {
    let dir = temp_tree("readspill");
    let body: String = (1..=200_000).map(|n| format!("{n}\n")).collect();
    fs::write(dir.join("huge.txt"), &body).unwrap();

    let (text, is_error) = dispatch_tool_text(
        &dir,
        ToolContext::new(dir.clone()),
        "read",
        r#"{"path":"huge.txt"}"#,
    )
    .await;
    assert!(!is_error, "read succeeded: {text}");
    // Bounded in-band, not the whole 200k-line file.
    assert!(text.contains("use offset=2001"), "windowing marker: {text}");
    assert!(text.lines().count() < 200_000, "in-band bounded");

    let marker = "written to ";
    let start = text.rfind(marker).expect("a spill pointer") + marker.len();
    let rest = &text[start..];
    let end = rest.find(']').expect("notice closes with ]");
    let path = &rest[..end];
    let spilled = fs::read_to_string(path).expect("read spill file");
    assert_eq!(spilled, body, "spill holds the complete file content");
    let _ = fs::remove_file(path);
}

// VAL-TOOLS-037: a `grep` whose match set exceeds the limits, dispatched through the runner, returns
// a bounded in-band result naming a spill file that holds the complete match set.
#[tokio::test]
async fn runner_dispatches_grep_large_match_set_spills_complete_set() {
    let dir = temp_tree("grepspill");
    let body: String = (1..=5000).map(|_| "match_token here\n").collect();
    fs::write(dir.join("big.txt"), body).unwrap();

    let (text, is_error) = dispatch_tool_text(
        &dir,
        ToolContext::new(dir.clone()),
        "grep",
        r#"{"pattern":"match_token","include":"big.txt"}"#,
    )
    .await;
    assert!(!is_error, "grep succeeded: {text}");
    assert!(
        text.contains("output truncated"),
        "truncation notice: {text}"
    );
    assert!(text.lines().count() < 5000, "in-band bounded");

    let marker = "written to ";
    let start = text.rfind(marker).expect("a spill pointer") + marker.len();
    let rest = &text[start..];
    let end = rest.find(']').expect("notice closes with ]");
    let path = &rest[..end];
    let spilled = fs::read_to_string(path).expect("read spill file");
    assert_eq!(spilled.lines().count(), 5000, "spill holds every match");
    let _ = fs::remove_file(path);
}

#[tokio::test]
async fn runner_dispatches_todo_write_then_update_persists_per_session() {
    let dir = temp_tree("todo");
    let store = Arc::new(MemoryTodoStore::default());
    let context = ToolContext::new(dir.clone())
        .with_session("ses_a")
        .with_todo_store(store.clone());

    // Write three items.
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: context.clone(),
    };
    let provider = ScriptedProvider::new(call_then_done(
        ToolCall {
            id: "call_todo_1".to_owned(),
            name: "todo".to_owned(),
            arguments: r#"{"todos":[{"content":"one","status":"pending"},{"content":"two","status":"pending"},{"content":"three","status":"pending"}]}"#.to_owned(),
        },
        "ok",
    ));
    SessionRunner::default()
        .run_with_tools(&provider, Turn::new("m", "", "todo"), &dispatcher, |_| {})
        .await;

    // Update item 1 to completed (resend the full list).
    let (text, is_error) = dispatch_tool_text(
        &dir,
        context,
        "todo",
        r#"{"todos":[{"content":"one","status":"completed"},{"content":"two","status":"pending"},{"content":"three","status":"pending"}]}"#,
    )
    .await;
    assert!(!is_error, "todo succeeded: {text}");
    assert!(text.contains("1. [completed][medium] one"), "got: {text}");
    assert!(text.contains("2. [pending][medium] two"), "got: {text}");

    let stored = store.list("ses_a").unwrap();
    assert_eq!(stored.len(), 3);
    assert_eq!(stored[0].status, protocol::TodoStatus::Completed);
    assert_eq!(stored[1].status, protocol::TodoStatus::Pending);

    // Isolation: a different session sees an empty list.
    assert!(store.list("ses_b").unwrap().is_empty());
}

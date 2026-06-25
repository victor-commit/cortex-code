//! End-to-end (offline) proof of the cross-cutting file-tool semantics shared by
//! `read`/`write`/`edit`/`apply_patch` (validation contract VAL-TOOLS-043/044/045):
//!
//! - all file tools resolve a relative path against the same session working directory, so the same
//!   relative path denotes the same on-disk file across tools (VAL-TOOLS-044);
//! - `apply_patch` enforces the same prior-read requirement as `write`/`edit` for Update/Delete of
//!   an existing file, while an Add needs no prior read (VAL-TOOLS-043);
//! - a mutating file tool targeting an OS-unwritable target returns a clear permission/IO error and
//!   leaves the existing file byte-for-byte intact, never truncating or silently succeeding
//!   (VAL-TOOLS-045).
//!
//! Each test drives the real [`tools::ToolRegistry`] through the durable runner's tool-dispatch seam
//! (`SessionRunner::run_with_tools` + a registry dispatcher), the same path the live CLI uses, with
//! a scripted provider standing in for the model.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use engine::{RunnerEvent, SessionRunner, ToolDispatchResult, ToolDispatcher, Turn};
use futures_util::future::BoxFuture;
use llm::{FinishReason, LlmEvent, LlmRequest, LlmStream, Provider, ToolCall};
use permissions::{Action, PermissionEngine, Rule, Ruleset, permissive_default_rules};
use tools::{ToolContext, ToolRegistry};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-m2-semantics-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn workspace_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target")
        .join("tool-gap-permissions");
    let dir = root.join(format!("cortex-gap-{tag}-{}-{unique}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create workspace dir");
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

/// A dispatcher backed by the real tool registry and a single shared per-session context (mirrors
/// the CLI's bridge: one [`ToolContext`] carries the working dir + read ledger across every call).
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

/// A scripted round that issues one tool call and then yields to the next round.
fn tool_call_round(id: &str, name: &str, arguments: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::ToolCall(ToolCall {
            id: id.to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }),
        LlmEvent::Done {
            reason: FinishReason::ToolCalls,
        },
    ]
}

/// A scripted final round that emits `text` and stops.
fn final_round(text: &str) -> Vec<LlmEvent> {
    vec![
        LlmEvent::TextDelta {
            text: text.to_owned(),
        },
        LlmEvent::Done {
            reason: FinishReason::Stop,
        },
    ]
}

/// Collect every tool-result event the runner surfaced, as `(name, content, is_error)`, where
/// `content` is the joined text of the result's parts.
fn tool_results(events: &[RunnerEvent]) -> Vec<(String, String, bool)> {
    events
        .iter()
        .filter_map(|event| match event {
            RunnerEvent::ToolResult {
                name,
                parts,
                is_error,
                ..
            } => {
                let content = parts
                    .iter()
                    .filter_map(protocol::ToolResultPart::as_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                Some((name.clone(), content, *is_error))
            }
            _ => None,
        })
        .collect()
}

async fn run_session(
    dispatcher: &RegistryDispatcher,
    model: &str,
    rounds: Vec<Vec<LlmEvent>>,
) -> Vec<RunnerEvent> {
    let provider = ScriptedProvider::new(rounds);
    let mut events = Vec::new();
    SessionRunner::default()
        .run_with_tools(
            &provider,
            Turn::new(model, "", "drive the file tools"),
            dispatcher,
            |event| events.push(event),
        )
        .await;
    events
}

#[tokio::test]
async fn session_resolves_relative_path_consistently_across_write_read_edit() {
    // VAL-TOOLS-044: in one session, write sub/x.txt=A, then read sub/x.txt returns A, then edit
    // sub/x.txt A->B yields on-disk <cwd>/sub/x.txt=B — all three resolving to the identical path.
    let dir = temp_dir("resolve");
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    let events = run_session(
        &dispatcher,
        "google/gemini-2.5-flash",
        vec![
            tool_call_round("c1", "write", r#"{"path":"sub/x.txt","content":"A"}"#),
            tool_call_round("c2", "read", r#"{"path":"sub/x.txt"}"#),
            tool_call_round(
                "c3",
                "edit",
                r#"{"path":"sub/x.txt","oldString":"A","newString":"B"}"#,
            ),
            final_round("done"),
        ],
    )
    .await;

    let results = tool_results(&events);
    assert_eq!(
        results.len(),
        3,
        "three tool calls were dispatched: {results:?}"
    );
    assert!(!results[0].2, "write succeeded: {:?}", results[0]);
    // The read sees exactly what write put there (same anchor).
    assert!(
        results[1].1.contains("1: A"),
        "read returned the written content: {:?}",
        results[1]
    );
    assert!(!results[2].2, "edit succeeded: {:?}", results[2]);

    // All three tools resolved the relative path to the identical absolute file, now holding B.
    let absolute = dir.join("sub").join("x.txt");
    assert_eq!(fs::read_to_string(&absolute).unwrap(), "B");
    // Nothing leaked outside the working directory anchor.
    assert!(!Path::new("sub/x.txt").exists());
}

#[tokio::test]
async fn apply_patch_update_of_unread_file_is_refused_then_applies_after_a_read() {
    // VAL-TOOLS-043: an apply_patch Update of an existing-but-unread cfg.txt is refused with the
    // bytes intact; after a preceding read the same Update applies.
    let dir = temp_dir("apply-read-guard");
    fs::write(dir.join("cfg.txt"), "OLD\n").unwrap();
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::gpt(),
        context: ToolContext::new(dir.clone()),
    };

    let update = "*** Begin Patch\n*** Update File: cfg.txt\n@@\n-OLD\n+NEW\n*** End Patch";

    // First attempt: no prior read -> refused, file unchanged.
    let events = run_session(
        &dispatcher,
        "openai/gpt-4o-mini",
        vec![
            tool_call_round(
                "c1",
                "apply_patch",
                &serde_json::json!({ "patch": update }).to_string(),
            ),
            final_round("blocked"),
        ],
    )
    .await;
    let results = tool_results(&events);
    assert!(
        results[0].2,
        "the unread update is an error: {:?}",
        results[0]
    );
    assert!(
        results[0].1.to_lowercase().contains("read it first"),
        "got: {:?}",
        results[0]
    );
    assert_eq!(fs::read_to_string(dir.join("cfg.txt")).unwrap(), "OLD\n");

    // Second attempt in the same session: read then update -> applies.
    let events = run_session(
        &dispatcher,
        "openai/gpt-4o-mini",
        vec![
            tool_call_round("c2", "read", r#"{"path":"cfg.txt"}"#),
            tool_call_round(
                "c3",
                "apply_patch",
                &serde_json::json!({ "patch": update }).to_string(),
            ),
            final_round("applied"),
        ],
    )
    .await;
    let results = tool_results(&events);
    assert!(
        !results[1].2,
        "the update after a read succeeds: {:?}",
        results[1]
    );
    assert_eq!(fs::read_to_string(dir.join("cfg.txt")).unwrap(), "NEW\n");
}

#[tokio::test]
async fn apply_patch_add_needs_no_prior_read() {
    // VAL-TOOLS-043: an Add File for a new path succeeds with no prior read required.
    let dir = temp_dir("apply-add");
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::gpt(),
        context: ToolContext::new(dir.clone()),
    };
    let add = "*** Begin Patch\n*** Add File: new.txt\n+hello\n*** End Patch";
    let events = run_session(
        &dispatcher,
        "openai/gpt-4o-mini",
        vec![
            tool_call_round(
                "c1",
                "apply_patch",
                &serde_json::json!({ "patch": add }).to_string(),
            ),
            final_round("added"),
        ],
    )
    .await;
    let results = tool_results(&events);
    assert!(
        !results[0].2,
        "the add succeeded without a prior read: {:?}",
        results[0]
    );
    assert_eq!(fs::read_to_string(dir.join("new.txt")).unwrap(), "hello\n");
}

#[tokio::test]
async fn mutating_tool_on_a_read_only_file_errors_and_leaves_bytes_intact() {
    // VAL-TOOLS-045: a chmod 0444 ro.txt -> an edit to it returns an IO/permission error, the bytes
    // are unchanged, and the run completes cleanly (no panic).
    let dir = temp_dir("readonly");
    let path = dir.join("ro.txt");
    fs::write(&path, "ORIGINAL\n").unwrap();
    let before = fs::read(&path).unwrap();
    set_readonly(&path, true);

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(dir.clone()),
    };

    // Read first (so the edit read-guard passes), then attempt the edit on the read-only file.
    let events = run_session(
        &dispatcher,
        "google/gemini-2.5-flash",
        vec![
            tool_call_round("c1", "read", r#"{"path":"ro.txt"}"#),
            tool_call_round(
                "c2",
                "edit",
                r#"{"path":"ro.txt","oldString":"ORIGINAL","newString":"CHANGED"}"#,
            ),
            final_round("blocked"),
        ],
    )
    .await;

    let results = tool_results(&events);
    assert!(
        !results[0].2,
        "the read of a read-only file works: {:?}",
        results[0]
    );
    assert!(
        results[1].2,
        "the edit is reported as an error: {:?}",
        results[1]
    );
    assert!(
        results[1].1.to_lowercase().contains("permission"),
        "the error names a permission/IO failure: {:?}",
        results[1]
    );

    // Restore perms to read back the bytes; the file is byte-for-byte identical (no truncation).
    set_readonly(&path, false);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn external_directory_gate_blocks_outside_paths_but_keeps_inside_access() {
    // VAL-GAP-PERM-001: outside workspace asks/denies through external_directory while
    // inside workspace reads follow normal rules.
    let workspace = workspace_dir("external-gate");
    let inside = workspace.join("inside.txt");
    fs::write(&inside, "INSIDE\n").unwrap();

    let outside_root = workspace
        .parent()
        .expect("workspace has parent")
        .join(format!(
            "outside-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&outside_root).unwrap();
    let outside = outside_root.join("outside.txt");
    fs::write(&outside, "OUTSIDE-KEEP\n").unwrap();

    let engine = Arc::new(PermissionEngine::new(Ruleset::new(vec![
        Rule::new("*", "*", Action::Allow),
        Rule::new("external_directory", "*", Action::Ask),
    ])));
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(workspace.clone()).with_permissions(engine),
    };

    let events = run_session(
        &dispatcher,
        "google/gemini-2.5-flash",
        vec![
            tool_call_round("c1", "read", r#"{"path":"inside.txt"}"#),
            tool_call_round(
                "c2",
                "read",
                &serde_json::json!({ "path": outside.to_string_lossy() }).to_string(),
            ),
            tool_call_round(
                "c3",
                "write",
                &serde_json::json!({
                    "path": outside.to_string_lossy(),
                    "content": "OUTSIDE-CHANGED\n"
                })
                .to_string(),
            ),
            final_round("done"),
        ],
    )
    .await;

    let results = tool_results(&events);
    assert!(
        !results[0].2,
        "inside read should succeed: {:?}",
        results[0]
    );
    assert!(
        results[0].1.contains("1: INSIDE"),
        "inside content should be returned: {:?}",
        results[0]
    );
    assert!(
        results[1].2,
        "outside read should be denied: {:?}",
        results[1]
    );
    assert!(
        results[1].1.to_lowercase().contains("permission denied"),
        "outside deny message should be explicit: {:?}",
        results[1]
    );
    assert!(
        results[2].2,
        "outside write should be denied: {:?}",
        results[2]
    );
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "OUTSIDE-KEEP\n",
        "denied outside write must leave bytes unchanged"
    );
}

#[tokio::test]
async fn read_defaults_ask_for_dotenv_but_allow_dotenv_example() {
    // VAL-GAP-PERM-002: `.env` asks/denies by default while `.env.example` stays readable.
    let workspace = workspace_dir("dotenv-defaults");
    fs::write(workspace.join(".env"), "API_KEY=super-secret\n").unwrap();
    fs::write(workspace.join(".env.example"), "API_KEY=example\n").unwrap();

    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(workspace.clone()).with_permissions(Arc::new(
            PermissionEngine::new(Ruleset::new(permissive_default_rules())),
        )),
    };

    let events = run_session(
        &dispatcher,
        "google/gemini-2.5-flash",
        vec![
            tool_call_round("c1", "read", r#"{"path":".env"}"#),
            tool_call_round("c2", "read", r#"{"path":".env.example"}"#),
            final_round("done"),
        ],
    )
    .await;
    let results = tool_results(&events);
    assert!(results[0].2, ".env read should be denied: {:?}", results[0]);
    assert!(
        results[0].1.to_lowercase().contains("permission denied"),
        "dotenv deny should be explicit: {:?}",
        results[0]
    );
    assert!(
        !results[0].1.contains("super-secret"),
        "denied .env read must not leak secret bytes: {:?}",
        results[0]
    );
    assert!(
        !results[1].2,
        ".env.example read should be allowed: {:?}",
        results[1]
    );
    assert!(
        results[1].1.contains("1: API_KEY=example"),
        ".env.example content should be returned: {:?}",
        results[1]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn home_aliases_and_symlink_escape_are_resolved_before_permission_matching() {
    // VAL-GAP-PERM-009: `~`, `$HOME`, `..`, and symlink aliases collapse to the intended canonical
    // path for matching; denied symlink escape leaves outside bytes unchanged.
    let workspace = workspace_dir("canonical-paths");
    let home = std::env::var("HOME").expect("HOME should be set for this test");
    let outside_root = PathBuf::from(home.clone()).join(format!(
        "cortex-gap-home-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(outside_root.join("nested")).unwrap();
    let outside = outside_root.join("secret.txt");
    fs::write(&outside, "HOME-SECRET\n").unwrap();
    fs::write(outside_root.join("nested/inner.txt"), "INNER\n").unwrap();

    let alias = workspace.join("escape-link.txt");
    std::os::unix::fs::symlink(&outside, &alias).unwrap();

    let mut rules = vec![Rule::new("*", "*", Action::Allow)];
    rules.push(Rule::new(
        "read",
        format!(
            "$HOME/{}/secret.txt",
            outside_root.file_name().unwrap().to_string_lossy()
        ),
        Action::Deny,
    ));
    rules.push(Rule::new("external_directory", "*", Action::Ask));
    let dispatcher = RegistryDispatcher {
        registry: ToolRegistry::builtin(),
        context: ToolContext::new(workspace.clone())
            .with_permissions(Arc::new(PermissionEngine::new(Ruleset::new(rules)))),
    };

    let tilde_secret = format!(
        "~/{}/secret.txt",
        outside_root.file_name().unwrap().to_string_lossy()
    );
    let home_normalized = format!(
        "{}/{}",
        home,
        outside_root.file_name().unwrap().to_string_lossy()
    );
    let dotdot_inner = format!(
        "$HOME/{}/nested/../secret.txt",
        outside_root.file_name().unwrap().to_string_lossy()
    );

    let events = run_session(
        &dispatcher,
        "google/gemini-2.5-flash",
        vec![
            tool_call_round(
                "c1",
                "read",
                &serde_json::json!({ "path": tilde_secret }).to_string(),
            ),
            tool_call_round(
                "c2",
                "read",
                &serde_json::json!({ "path": format!("{home_normalized}/secret.txt") }).to_string(),
            ),
            tool_call_round(
                "c3",
                "read",
                &serde_json::json!({ "path": dotdot_inner }).to_string(),
            ),
            tool_call_round(
                "c4",
                "write",
                &serde_json::json!({
                    "path": alias.to_string_lossy(),
                    "content": "MUTATED\n"
                })
                .to_string(),
            ),
            final_round("done"),
        ],
    )
    .await;

    let results = tool_results(&events);
    for (index, result) in results.iter().take(3).enumerate() {
        assert!(result.2, "alias read #{index} should be denied: {result:?}");
        assert!(
            result.1.to_lowercase().contains("permission denied"),
            "alias read #{index} should surface permission denied: {result:?}"
        );
    }
    assert!(
        results[3].2,
        "symlink escape write should be denied: {:?}",
        results[3]
    );
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "HOME-SECRET\n",
        "denied symlink escape must not mutate outside target"
    );
}

/// Toggle the read-only bit on `path` so tests can exercise the OS-unwritable guard.
fn set_readonly(path: &Path, readonly: bool) {
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_readonly(readonly);
    fs::set_permissions(path, perms).unwrap();
}

//! Tool dispatch for a TUI turn, with file-change diff capture.
//!
//! The durable runner advertises and executes tools through the [`engine::ToolDispatcher`] seam.
//! This adapter holds a [`ToolRegistry`] and the per-turn [`ToolContext`] (mirroring the headless
//! `cortex run` bridge) so a real turn driven from the TUI can read, edit, and create files. Around
//! each mutating tool call it captures the target file's before/after content and emits a
//! [`crate::diff::FileDiff`] over the turn channel, so an edit/creation renders as a colored unified
//! diff in the transcript (VAL-TUI-021..023).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use engine::{ModelFamily, ToolDispatchResult, ToolDispatcher};
use futures_util::future::BoxFuture;
use llm::{ToolCall, ToolDef};
use protocol::TodoItem;
use tools::{BackgroundJobs, TodoStore, ToolContext, ToolRegistry};

use crate::diff::compute_file_diff;
use crate::turn::TurnMessage;

/// A [`ToolDispatcher`] backed by a [`ToolRegistry`] and a shared [`ToolContext`], emitting a
/// [`crate::diff::FileDiff`] for each file a mutating tool changes.
pub struct TurnDispatcher {
    registry: ToolRegistry,
    context: ToolContext,
    events: Sender<TurnMessage>,
}

impl TurnDispatcher {
    /// Build a dispatcher whose tool set is materialized for `model_id`'s family (a GPT-family model
    /// gets `apply_patch`; every other model keeps `edit`/`write`), scoped to `cwd` and `session_id`
    /// with an in-memory todo store. File changes are reported as diffs over `events`.
    #[must_use]
    pub fn new(
        cwd: PathBuf,
        model_id: &str,
        session_id: String,
        events: Sender<TurnMessage>,
    ) -> Self {
        let registry = match ModelFamily::from_model_id(model_id) {
            ModelFamily::Gpt | ModelFamily::Codex => ToolRegistry::gpt(),
            ModelFamily::Anthropic | ModelFamily::Gemini | ModelFamily::Default => {
                ToolRegistry::builtin()
            }
        };
        let todos: Arc<dyn TodoStore> = Arc::new(EphemeralTodos::default());
        let context = ToolContext::new(cwd)
            .with_session(session_id)
            .with_todo_store(todos);
        Self {
            registry,
            context,
            events,
        }
    }

    /// Gate every tool effect through `engine` (builder style). The TUI attaches an engine whose
    /// interactive responder surfaces an `ask` to the UI thread, so an ask-gated mutation prompts
    /// for approval and a denied action produces no side effect.
    #[must_use]
    pub fn with_permissions(mut self, engine: Arc<permissions::PermissionEngine>) -> Self {
        self.context = self.context.with_permissions(engine);
        self
    }

    /// Attach the pre-mutation checkpoint recorder so each mutating tool snapshots a file's prior
    /// state before it changes it (builder style). Without this the TUI turn records no checkpoints,
    /// so `/undo` would have nothing to restore.
    #[must_use]
    pub fn with_checkpoints(mut self, recorder: Arc<dyn tools::CheckpointRecorder>) -> Self {
        self.context = self.context.with_checkpoints(recorder);
        self
    }

    /// The background-job registry tracking long-running `bash` jobs spawned during the turn, so the
    /// caller can reap them (terminate + await by PID) when the turn ends — no orphan survives.
    #[must_use]
    pub fn background_jobs(&self) -> Arc<BackgroundJobs> {
        self.context.background_jobs().clone()
    }

    /// Capture a diff for each file `call` changed, if any, and publish it over the turn channel.
    fn emit_diffs(&self, call: &ToolCall, before: &[(String, PathBuf, Option<String>)]) {
        for (rel, resolved, before_content) in before {
            let after = std::fs::read_to_string(resolved).ok();
            if before_content.as_deref() == after.as_deref() {
                continue;
            }
            if let Some(diff) = compute_file_diff(rel, before_content.as_deref(), after.as_deref())
            {
                let _ = self.events.send(TurnMessage::FileDiff(Box::new(diff)));
            }
        }
        let _ = call;
    }
}

impl ToolDispatcher for TurnDispatcher {
    fn definitions(&self) -> Vec<ToolDef> {
        self.registry.definitions()
    }

    fn dispatch<'a>(&'a self, call: &'a ToolCall) -> BoxFuture<'a, ToolDispatchResult> {
        Box::pin(async move {
            // Snapshot each target file's prior content BEFORE the tool mutates it, so the diff
            // reflects exactly this call's change (a missing file reads as `None` -> a new-file diff).
            let before: Vec<(String, PathBuf, Option<String>)> = mutated_targets(call)
                .into_iter()
                .map(|rel| {
                    let resolved = self.context.resolve(&rel);
                    let content = std::fs::read_to_string(&resolved).ok();
                    (rel, resolved, content)
                })
                .collect();

            let output = self
                .registry
                .execute(&call.name, &call.arguments, &self.context)
                .await;

            if !output.is_error {
                self.emit_diffs(call, &before);
            }

            ToolDispatchResult {
                content: output.model_content(),
                parts: output.parts,
                is_error: output.is_error,
            }
        })
    }
}

/// The workspace-relative path(s) a tool call mutates, for before/after diff capture. A non-mutating
/// tool (read/glob/grep/bash/todo) returns an empty list, so no diff is produced for it.
fn mutated_targets(call: &ToolCall) -> Vec<String> {
    let value: serde_json::Value = serde_json::from_str(&call.arguments).unwrap_or_default();
    match call.name.as_str() {
        "write" | "edit" => value
            .get("path")
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| vec![s.to_owned()])
            .unwrap_or_default(),
        "apply_patch" => value
            .get("patch")
            .and_then(serde_json::Value::as_str)
            .map(parse_patch_paths)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Extract the file paths an `apply_patch` envelope adds or updates (the sections that change file
/// content), so each changed file gets its own diff.
fn parse_patch_paths(patch: &str) -> Vec<String> {
    const ADD: &str = "*** Add File: ";
    const UPDATE: &str = "*** Update File: ";
    let mut paths = Vec::new();
    for line in patch.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix(ADD) {
            paths.push(rest.trim().to_owned());
        } else if let Some(rest) = trimmed.strip_prefix(UPDATE) {
            paths.push(rest.trim().to_owned());
        }
    }
    paths
}

/// An in-memory, per-session todo store: the TUI turn does not persist todos to the durable DB, but
/// the `todo` tool still requires a store, so this keeps a session's list alive for the turn.
#[derive(Default)]
struct EphemeralTodos {
    lists: Mutex<HashMap<String, Vec<TodoItem>>>,
}

impl TodoStore for EphemeralTodos {
    fn list(&self, session_id: &str) -> Result<Vec<TodoItem>, String> {
        Ok(self
            .lists
            .lock()
            .map_err(|_| "todo store poisoned".to_owned())?
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    fn replace(&self, session_id: &str, items: &[TodoItem]) -> Result<(), String> {
        self.lists
            .lock()
            .map_err(|_| "todo store poisoned".to_owned())?
            .insert(session_id.to_owned(), items.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::channel;

    use super::*;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn scratch_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-tui-diff-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: "call_1".to_owned(),
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        }
    }

    fn block_on(
        future: impl std::future::Future<Output = ToolDispatchResult>,
    ) -> ToolDispatchResult {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    fn drain_diffs(rx: &std::sync::mpsc::Receiver<TurnMessage>) -> Vec<crate::diff::FileDiff> {
        let mut diffs = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let TurnMessage::FileDiff(diff) = message {
                diffs.push(*diff);
            }
        }
        diffs
    }

    #[test]
    fn mutated_targets_reads_write_and_edit_paths() {
        assert_eq!(
            mutated_targets(&call("write", r#"{"path":"a.txt","content":"x"}"#)),
            vec!["a.txt".to_owned()]
        );
        assert_eq!(
            mutated_targets(&call(
                "edit",
                r#"{"path":"b.rs","oldString":"x","newString":"y"}"#
            )),
            vec!["b.rs".to_owned()]
        );
        assert!(mutated_targets(&call("read", r#"{"path":"c.txt"}"#)).is_empty());
        assert!(mutated_targets(&call("bash", r#"{"command":"ls"}"#)).is_empty());
    }

    #[test]
    fn mutated_targets_parses_apply_patch_sections() {
        let patch = "*** Begin Patch\n*** Add File: new.py\n+line\n*** Update File: old.py\n@@\n-a\n+b\n*** End Patch";
        let args = serde_json::json!({ "patch": patch }).to_string();
        assert_eq!(
            mutated_targets(&call("apply_patch", &args)),
            vec!["new.py".to_owned(), "old.py".to_owned()]
        );
    }

    #[test]
    fn writing_a_new_file_emits_an_all_added_diff() {
        let dir = scratch_dir("write-new");
        let (tx, rx) = channel();
        let dispatcher =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx);

        let result = block_on(dispatcher.dispatch(&call(
            "write",
            r#"{"path":"created.txt","content":"hello\nworld\n"}"#,
        )));
        assert!(!result.is_error, "write should succeed: {}", result.content);

        let diffs = drain_diffs(&rx);
        assert_eq!(diffs.len(), 1, "exactly one file changed");
        let diff = &diffs[0];
        assert_eq!(diff.path, "created.txt");
        assert!(diff.is_new, "a created file is a new-file diff");
        assert_eq!(diff.removals, 0);
        assert!(diff.additions >= 2);
    }

    #[test]
    fn editing_an_existing_file_emits_a_replacement_diff() {
        let dir = scratch_dir("edit");
        std::fs::write(
            dir.join("greet.rs"),
            "fn main() {\n    println!(\"Hello\");\n    let x = 1;\n}\n",
        )
        .unwrap();
        let (tx, rx) = channel();
        let dispatcher =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx);

        // The edit tool requires a prior read in the session; the shared context records it.
        let read = block_on(dispatcher.dispatch(&call("read", r#"{"path":"greet.rs"}"#)));
        assert!(!read.is_error, "read should succeed: {}", read.content);

        let result = block_on(dispatcher.dispatch(&call(
            "edit",
            r#"{"path":"greet.rs","oldString":"println!(\"Hello\");","newString":"println!(\"Cortex\");"}"#,
        )));
        assert!(!result.is_error, "edit should succeed: {}", result.content);

        let diffs = drain_diffs(&rx);
        assert_eq!(diffs.len(), 1, "only the edit emits a diff, not the read");
        let diff = &diffs[0];
        assert_eq!(diff.path, "greet.rs");
        assert!(!diff.is_new);
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.removals, 1);
    }

    #[test]
    fn a_failed_tool_call_emits_no_diff() {
        let dir = scratch_dir("fail");
        let (tx, rx) = channel();
        let dispatcher =
            TurnDispatcher::new(dir.clone(), "google/gemini-2.5-flash", "ses".to_owned(), tx);

        // Editing a file that was never read (and does not exist) fails; no diff is emitted.
        let result = block_on(dispatcher.dispatch(&call(
            "edit",
            r#"{"path":"missing.txt","oldString":"a","newString":"b"}"#,
        )));
        assert!(result.is_error);
        assert!(drain_diffs(&rx).is_empty());
    }
}

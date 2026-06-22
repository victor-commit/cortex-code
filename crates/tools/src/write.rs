//! The `write` tool: create a new file or fully replace an existing one, then run the post-write
//! downstream hooks (formatter + LSP diagnostics).
//!
//! Behavior (architecture §3.4, validation contract VAL-TOOLS-008..011):
//!
//! - Writing to a path that does not exist creates the file (and any missing parent directories)
//!   containing exactly the requested bytes.
//! - Overwriting an existing file replaces its full content, but only after the file has been read
//!   earlier in the session: an overwrite of an existing-but-unread file is refused with a clear
//!   "read it first" error and the original bytes are left intact. A successful write records the
//!   path as read so a later overwrite in the same session succeeds.
//! - After a successful write the post-write hooks run: the formatter pass reformats the on-disk
//!   file when a formatter is available for the language, and any LSP diagnostics are re-injected
//!   into the tool result. With no formatter/LSP attached, a write succeeds unchanged with no error;
//!   when a surface provides hooks, the seam carries formatter output and diagnostics.

use std::fs;

use futures_util::future::BoxFuture;
use protocol::ToolResultPart;
use serde_json::{Value, json};

use crate::tool::{DiagnosticFeedback, Tool, ToolContext, ToolOutput};

const DESCRIPTION: &str = "Write a file to the filesystem, replacing its entire content with \
exactly the bytes you provide. Creates the file and any missing parent directories when it does \
not exist. Overwriting an existing file requires you to have read it earlier in this session so \
the replacement is informed; if you have not read it, read it first. After writing, the file is \
formatted (when a formatter is available) and any diagnostics are reported back to you.";

/// The `write` tool.
pub struct WriteTool;

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file path to write (absolute, or relative to the working directory)."
                },
                "content": {
                    "type": "string",
                    "description": "The full content to write to the file."
                }
            },
            "required": ["path", "content"]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

/// Execute a write against the resolved arguments (synchronous filesystem work).
fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(path) = string_arg(arguments, "path")
        .or_else(|| string_arg(arguments, "file_path"))
        .or_else(|| string_arg(arguments, "filePath"))
    else {
        return ToolOutput::error("write: the 'path' argument is required");
    };
    let path = path.trim().to_owned();
    if path.is_empty() {
        return ToolOutput::error("write: the 'path' argument must not be empty");
    }
    let Some(content) = string_arg(arguments, "content") else {
        return ToolOutput::error("write: the 'content' argument is required");
    };

    // Gate the write through the permission engine when one is attached (a no-op until enforcement
    // wires a real engine in); a denied write touches nothing on disk.
    if !ctx.permits("write", &path) {
        return ToolOutput::error(format!("write: permission denied for {path}"));
    }

    let resolved = ctx.resolve(&path);

    // Read-before-overwrite guard. An existing file must have been read this session; otherwise the
    // overwrite is refused and the original bytes are left intact.
    let created = match fs::metadata(&resolved) {
        Ok(metadata) => {
            if metadata.is_dir() {
                return ToolOutput::error(format!(
                    "write: {path} is a directory; write targets a single file"
                ));
            }
            if !ctx.was_read(&resolved) {
                return ToolOutput::error(format!(
                    "write: {path} already exists but has not been read this session; read it first before overwriting it"
                ));
            }
            false
        }
        Err(_) => true,
    };

    // OS-writability pre-check: refuse a read-only target (or a target under a read-only directory)
    // before touching the filesystem, so the existing file is left byte-for-byte intact even when a
    // privileged process would otherwise bypass the permission bits.
    if let Some(reason) = ctx.writability_error(&resolved) {
        return ToolOutput::error(format!("write: cannot write {path}: {reason}"));
    }

    if let Some(parent) = resolved.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            return ToolOutput::error(format!("write: could not create parent of {path}: {err}"));
        }
    }

    // Capture the pre-mutation snapshot BEFORE the bytes hit the disk, so `/undo` can restore the
    // file's prior state (or remove it when this write creates it).
    ctx.checkpoint_before(&resolved);

    if let Err(err) = fs::write(&resolved, content.as_bytes()) {
        return ToolOutput::error(format!("write: could not write {path}: {err}"));
    }
    if let Some(audit) = ctx.audit_sink() {
        let detail = if created { "create" } else { "overwrite" };
        audit.record_file_mutation(&resolved, "write", Some(detail));
    }

    // The write succeeded, so the on-disk content is now this session's source of truth: record the
    // path as read so a later overwrite in the same session is permitted.
    ctx.record_read(&resolved);

    // Post-write downstream hooks. The formatter rewrites the file in place first, then diagnostics
    // are collected against the final on-disk content. Both are no-ops when no surface attached
    // those capabilities.
    let format_note = ctx.format_after_write(&resolved);
    let mut feedback = ctx.diagnostic_feedback_after_write(&resolved);
    if let Some(note) = format_note {
        feedback.push_note(note);
    }

    let bytes = content.len();
    let verb = if created { "created" } else { "overwrote" };
    let summary = format!("write: {verb} {path} ({bytes} bytes)");

    render_with_feedback(summary, feedback)
}

fn render_with_feedback(summary: String, feedback: DiagnosticFeedback) -> ToolOutput {
    if feedback.diagnostics.is_empty() && feedback.note.is_none() {
        return ToolOutput::text(summary);
    }

    let mut parts = vec![ToolResultPart::text(summary)];
    if !feedback.diagnostics.is_empty() {
        let mut rendered = String::from("Diagnostics:");
        for diagnostic in &feedback.diagnostics {
            rendered.push('\n');
            rendered.push_str(&diagnostic.render());
        }
        parts.push(ToolResultPart::text(rendered));
    }
    if let Some(note) = feedback.note {
        parts.push(ToolResultPart::text(note));
    }
    ToolOutput::parts(parts)
}

/// Extract a string argument from the assembled arguments object (no trimming: file content must be
/// preserved verbatim).
fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::tool::{CheckpointRecorder, Diagnostic, MutationHooks};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-write-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn ctx_for(dir: &Path) -> ToolContext {
        ToolContext::new(dir.to_path_buf())
    }

    fn audit_for(dir: &Path, session: &str) -> Arc<audit::AuditSink> {
        Arc::new(audit::AuditSink::new(dir.join("audit.jsonl"), session))
    }

    fn write_args(path: &str, content: &str) -> Value {
        json!({ "path": path, "content": content })
    }

    fn text_of(output: &ToolOutput) -> String {
        output.model_content()
    }

    #[test]
    fn creates_new_file_with_exact_bytes_and_parent_dirs() {
        let dir = temp_dir("create");
        let audit = audit_for(&dir, "ses_write");
        let output = run(
            &write_args("a/b/new.txt", "hello\nworld\n"),
            &ctx_for(&dir).with_audit(audit.clone()),
        );
        assert!(!output.is_error, "{}", text_of(&output));
        let written = fs::read(dir.join("a/b/new.txt")).unwrap();
        assert_eq!(written, b"hello\nworld\n");
        let records = audit::read_jsonl(audit.path()).expect("audit records");
        assert!(records.iter().any(|record| {
            record["type"] == "file_mutation"
                && record["session"] == "ses_write"
                && record["operation"] == "write"
                && record["detail"] == "create"
                && record["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("a/b/new.txt"))
        }));
    }

    #[test]
    fn creating_a_new_file_needs_no_prior_read() {
        let dir = temp_dir("noread");
        let output = run(&write_args("fresh.txt", "x"), &ctx_for(&dir));
        assert!(!output.is_error);
        assert_eq!(fs::read_to_string(dir.join("fresh.txt")).unwrap(), "x");
    }

    #[test]
    fn overwrite_after_read_replaces_full_content() {
        let dir = temp_dir("overwrite");
        fs::write(dir.join("cfg.txt"), "OLD").unwrap();
        let ctx = ctx_for(&dir);
        // Simulate a prior read of the file this session.
        ctx.record_read(dir.join("cfg.txt"));
        let output = run(&write_args("cfg.txt", "NEW"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert_eq!(fs::read_to_string(dir.join("cfg.txt")).unwrap(), "NEW");
    }

    #[test]
    fn overwrite_without_prior_read_is_refused_and_leaves_bytes_intact() {
        let dir = temp_dir("unread");
        fs::write(dir.join("cfg2.txt"), "ORIGINAL").unwrap();
        let output = run(&write_args("cfg2.txt", "NEW"), &ctx_for(&dir));
        assert!(output.is_error);
        let text = text_of(&output).to_lowercase();
        assert!(text.contains("read it first"), "got: {text}");
        // The original bytes are untouched.
        assert_eq!(
            fs::read_to_string(dir.join("cfg2.txt")).unwrap(),
            "ORIGINAL"
        );
    }

    #[test]
    fn successful_write_records_ledger_so_a_rewrite_succeeds() {
        let dir = temp_dir("refresh");
        let ctx = ctx_for(&dir);
        run(&write_args("loop.txt", "v1"), &ctx);
        let output = run(&write_args("loop.txt", "v2"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));
        assert_eq!(fs::read_to_string(dir.join("loop.txt")).unwrap(), "v2");
    }

    #[test]
    fn writing_to_a_directory_errors() {
        let dir = temp_dir("isdir");
        fs::create_dir(dir.join("sub")).unwrap();
        let output = run(&write_args("sub", "x"), &ctx_for(&dir));
        assert!(output.is_error);
        assert!(text_of(&output).contains("directory"));
    }

    #[test]
    fn missing_content_argument_errors() {
        let dir = temp_dir("nocontent");
        let output = run(&json!({ "path": "x.txt" }), &ctx_for(&dir));
        assert!(output.is_error);
        assert!(text_of(&output).contains("content"));
    }

    #[test]
    fn permission_deny_blocks_the_write_and_leaves_bytes_intact() {
        use permissions::{Action, PermissionEngine, Rule, Ruleset};

        let dir = temp_dir("perm");
        fs::write(dir.join("guard.txt"), "KEEP").unwrap();
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "write",
            "guard.txt",
            Action::Deny,
        )]));
        let ctx = ToolContext::new(dir.clone()).with_permissions(Arc::new(engine));
        let output = run(&write_args("guard.txt", "OVERWRITTEN"), &ctx);
        assert!(output.is_error);
        assert!(text_of(&output).contains("permission denied"));
        assert_eq!(fs::read_to_string(dir.join("guard.txt")).unwrap(), "KEEP");
    }

    /// A fake formatter hook that rewrites the just-written file (uppercasing it) so the test can
    /// assert the on-disk content reflects formatter output, not the raw written bytes.
    struct UppercasingFormatter;
    impl MutationHooks for UppercasingFormatter {
        fn format(&self, path: &Path) {
            if let Ok(content) = fs::read_to_string(path) {
                let _ = fs::write(path, content.to_uppercase());
            }
        }
    }

    #[test]
    fn formatter_hook_reformats_the_on_disk_file() {
        let dir = temp_dir("fmt");
        let ctx = ctx_for(&dir).with_hooks(Arc::new(UppercasingFormatter));
        let output = run(&write_args("m.rs", "fn main() {}"), &ctx);
        assert!(!output.is_error);
        // The on-disk content reflects the formatter, not the raw written text.
        assert_eq!(
            fs::read_to_string(dir.join("m.rs")).unwrap(),
            "FN MAIN() {}"
        );
    }

    #[test]
    fn no_formatter_writes_verbatim_with_no_error() {
        let dir = temp_dir("verbatim");
        let raw = "fn  main( ){let x=1;}";
        let output = run(&write_args("file.unknownext", raw), &ctx_for(&dir));
        assert!(!output.is_error);
        // With no formatter available the bytes are written exactly as provided.
        assert_eq!(
            fs::read_to_string(dir.join("file.unknownext")).unwrap(),
            raw
        );
    }

    /// A fake LSP hook that reports a diagnostic for a file containing the marker `BAD`, and none
    /// otherwise, so the test can assert diagnostics are re-injected only when present.
    struct MarkerLsp;
    impl MutationHooks for MarkerLsp {
        fn diagnostics(&self, path: &Path) -> Vec<Diagnostic> {
            let content = fs::read_to_string(path).unwrap_or_default();
            if let Some(index) = content.lines().position(|line| line.contains("BAD")) {
                vec![Diagnostic::new("error", index + 1, "mismatched types")]
            } else {
                Vec::new()
            }
        }
    }

    #[test]
    fn lsp_hook_reinjects_diagnostics_into_the_tool_result() {
        let dir = temp_dir("lsp");
        let ctx = ctx_for(&dir).with_hooks(Arc::new(MarkerLsp));
        let output = run(&write_args("t.rs", "ok\nBAD line\n"), &ctx);
        assert!(!output.is_error);
        let text = text_of(&output);
        assert!(text.contains("mismatched types"), "got: {text}");
        assert!(
            text.contains("line 2"),
            "diagnostic must carry the location: {text}"
        );
    }

    #[test]
    fn clean_file_produces_no_spurious_diagnostics() {
        let dir = temp_dir("clean");
        let ctx = ctx_for(&dir).with_hooks(Arc::new(MarkerLsp));
        let output = run(&write_args("ok.rs", "fn main() {}\n"), &ctx);
        assert!(!output.is_error);
        assert!(!text_of(&output).to_lowercase().contains("diagnostic"));
    }

    #[test]
    fn absence_of_hooks_is_not_an_error() {
        let dir = temp_dir("nohooks");
        let output = run(&write_args("plain.rs", "fn main() {}\n"), &ctx_for(&dir));
        assert!(!output.is_error);
        assert_eq!(
            fs::read_to_string(dir.join("plain.rs")).unwrap(),
            "fn main() {}\n"
        );
    }

    #[test]
    fn write_to_a_read_only_file_errors_and_leaves_bytes_intact() {
        let dir = temp_dir("ro");
        let ctx = ctx_for(&dir);
        let path = dir.join("ro.txt");
        fs::write(&path, "ORIGINAL").unwrap();
        // The file was read this session (so the read-guard passes), then made read-only.
        ctx.record_read(&path);
        set_readonly(&path, true);
        let output = run(&write_args("ro.txt", "CHANGED"), &ctx);
        assert!(output.is_error, "{}", text_of(&output));
        assert!(
            text_of(&output).to_lowercase().contains("permission"),
            "got: {}",
            text_of(&output)
        );
        // Restore perms to read the bytes back; the original content is intact (no truncation).
        set_readonly(&path, false);
        assert_eq!(fs::read_to_string(&path).unwrap(), "ORIGINAL");
    }

    #[test]
    fn write_under_a_read_only_directory_errors() {
        let dir = temp_dir("ro-dir");
        let sub = dir.join("locked");
        fs::create_dir(&sub).unwrap();
        set_readonly(&sub, true);
        let output = run(&write_args("locked/new.txt", "x"), &ctx_for(&dir));
        // Restore perms so the temp tree can be cleaned up regardless of the assertion outcome.
        set_readonly(&sub, false);
        assert!(output.is_error, "{}", text_of(&output));
        assert!(
            text_of(&output).to_lowercase().contains("permission"),
            "got: {}",
            text_of(&output)
        );
        assert!(!sub.join("new.txt").exists());
    }

    /// A checkpoint recorder spy that captures, for each call, the on-disk content of the path at
    /// the moment `record_pre_image` runs. Because it reads the file when called, the captured value
    /// proves whether the snapshot was taken BEFORE the tool's write (it sees the prior bytes) or
    /// after (it would see the new bytes).
    struct SnapshotSpy {
        captured: Mutex<Vec<(PathBuf, Option<String>)>>,
    }

    impl SnapshotSpy {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                captured: Mutex::new(Vec::new()),
            })
        }

        fn snapshots(&self) -> Vec<(PathBuf, Option<String>)> {
            self.captured.lock().unwrap().clone()
        }
    }

    impl CheckpointRecorder for SnapshotSpy {
        fn record_pre_image(&self, path: &Path) {
            let content = fs::read_to_string(path).ok();
            self.captured
                .lock()
                .unwrap()
                .push((path.to_path_buf(), content));
        }
    }

    #[test]
    fn checkpoint_captures_pre_image_before_overwrite() {
        let dir = temp_dir("ckpt-overwrite");
        fs::write(dir.join("x.txt"), "BEFORE").unwrap();
        let spy = SnapshotSpy::new();
        let ctx = ctx_for(&dir).with_checkpoints(spy.clone());
        ctx.record_read(dir.join("x.txt"));

        let output = run(&write_args("x.txt", "AFTER"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));

        let snapshots = spy.snapshots();
        assert_eq!(snapshots.len(), 1, "exactly one pre-mutation snapshot");
        assert_eq!(
            snapshots[0].1.as_deref(),
            Some("BEFORE"),
            "the snapshot captured the content immediately before the write"
        );
        assert_eq!(fs::read_to_string(dir.join("x.txt")).unwrap(), "AFTER");
    }

    #[test]
    fn checkpoint_captures_absent_pre_image_for_a_created_file() {
        let dir = temp_dir("ckpt-create");
        let spy = SnapshotSpy::new();
        let ctx = ctx_for(&dir).with_checkpoints(spy.clone());

        let output = run(&write_args("new.txt", "hello"), &ctx);
        assert!(!output.is_error, "{}", text_of(&output));

        let snapshots = spy.snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].1, None,
            "a created file has no prior content to snapshot"
        );
    }

    #[test]
    fn a_refused_write_records_no_checkpoint() {
        let dir = temp_dir("ckpt-refused");
        fs::write(dir.join("cfg.txt"), "ORIGINAL").unwrap();
        let spy = SnapshotSpy::new();
        // No prior read this session, so the overwrite is refused before any mutation.
        let ctx = ctx_for(&dir).with_checkpoints(spy.clone());

        let output = run(&write_args("cfg.txt", "NEW"), &ctx);
        assert!(output.is_error);
        assert!(
            spy.snapshots().is_empty(),
            "a refused write must not record a checkpoint"
        );
        assert_eq!(fs::read_to_string(dir.join("cfg.txt")).unwrap(), "ORIGINAL");
    }

    /// Toggle the read-only bit on `path` so tests can exercise the OS-unwritable guard.
    fn set_readonly(path: &Path, readonly: bool) {
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_readonly(readonly);
        fs::set_permissions(path, perms).unwrap();
    }
}

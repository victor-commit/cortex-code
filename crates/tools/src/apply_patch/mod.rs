//! The `apply_patch` tool: apply a multi-file patch envelope atomically.
//!
//! Behavior (architecture §3.4, validation contract VAL-TOOLS-021..026):
//!
//! - The model passes a single patch envelope (`*** Begin Patch` … `*** End Patch`) containing one
//!   or more file sections: `*** Add File:` (create a file from `+` lines), `*** Update File:`
//!   (context-anchored `@@` hunks, optionally with `*** Move to:`), and `*** Delete File:`.
//! - An Add section writes exactly the section's lines; an Update section changes only the targeted
//!   lines via context-anchored hunks (whitespace-tolerant matching); a Delete section removes the
//!   file.
//! - The whole envelope applies **atomically**: every section is parsed and validated against the
//!   filesystem (existence checks, hunk context resolution, permission gating) BEFORE any byte is
//!   written, so a single invalid section fails the operation with zero on-disk changes — never a
//!   partial mix.
//! - A malformed envelope (missing markers, unknown header, or a hunk referencing a non-existent
//!   file/region) returns a clear parse/apply error and makes no on-disk changes.
//!
//! This is the GPT-family file-editing tool: when a GPT-family model is active the runner
//! materializes `apply_patch` in place of `edit`/`write` (see [`crate::ToolRegistry::gpt`]).

mod apply;
mod parser;

use std::fs;
use std::path::PathBuf;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::tool::{Tool, ToolContext, ToolOutput};
use apply::apply_chunks;
use parser::{Hunk, parse_envelope};

const DESCRIPTION: &str = "Apply a patch to the filesystem using a file-oriented diff envelope. \
Wrap one or more file sections between '*** Begin Patch' and '*** End Patch'. Each section starts \
with a header: '*** Add File: <path>' (every following line is a '+' line giving the new file's \
content), '*** Update File: <path>' (followed by context-anchored '@@' hunks whose lines start \
with ' ' for context, '-' to remove, and '+' to add; optionally preceded by '*** Move to: \
<path>' to rename), or '*** Delete File: <path>' (nothing follows). Updating or deleting an \
existing file requires you to have read it earlier in this session; if you have not read it, read \
it first. Adding a new file needs no prior read. The entire envelope is applied atomically: if any \
section cannot be applied, no files are changed.";

/// The `apply_patch` tool.
pub struct ApplyPatchTool;

impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "The full patch envelope, beginning with '*** Begin Patch' and ending with '*** End Patch'."
                }
            },
            "required": ["patch"]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(&arguments, ctx) })
    }
}

/// A validated, resolved filesystem mutation ready to commit. Every action is computed and checked
/// before any is committed, so a logical failure in one section leaves the filesystem untouched.
enum Action {
    /// Create a new file with the given content.
    Create { path: PathBuf, contents: String },
    /// Overwrite an existing file in place with the given content.
    Overwrite { path: PathBuf, contents: String },
    /// Delete an existing file.
    Delete { path: PathBuf },
    /// Write the updated content to `to` and remove the original `from` (a rename).
    Rename {
        from: PathBuf,
        to: PathBuf,
        contents: String,
    },
}

/// A one-line, model-readable summary of a committed action.
fn action_summary(action: &Action, display: &str) -> String {
    match action {
        Action::Create { .. } => format!("added {display}"),
        Action::Overwrite { .. } => format!("updated {display}"),
        Action::Delete { .. } => format!("deleted {display}"),
        Action::Rename { .. } => format!("renamed {display}"),
    }
}

/// Execute an apply_patch call against the resolved arguments (synchronous filesystem work).
fn run(arguments: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(patch) = string_arg(arguments, "patch")
        .or_else(|| string_arg(arguments, "input"))
        .or_else(|| string_arg(arguments, "patch_text"))
    else {
        return ToolOutput::error("apply_patch: the 'patch' argument is required");
    };
    if patch.trim().is_empty() {
        return ToolOutput::error("apply_patch: the 'patch' argument must not be empty");
    }

    let hunks = match parse_envelope(&patch) {
        Ok(hunks) => hunks,
        Err(message) => {
            return ToolOutput::error(format!("apply_patch: {message} (no changes made)"));
        }
    };
    if hunks.is_empty() {
        return ToolOutput::error(
            "apply_patch: the patch contains no file sections (no changes made)",
        );
    }

    // Plan phase: resolve and validate every section against the filesystem WITHOUT writing. Any
    // failure here aborts with zero on-disk changes (atomicity, VAL-TOOLS-024/026). The original
    // path string is retained for permission gating and the result summary.
    let mut plan: Vec<(Action, String)> = Vec::new();
    for hunk in &hunks {
        let planned = match hunk {
            Hunk::Add { path, contents } => plan_add(ctx, path, contents),
            Hunk::Delete { path } => plan_delete(ctx, path),
            Hunk::Update {
                path,
                move_to,
                chunks,
            } => plan_update(ctx, path, move_to.as_deref(), chunks),
        };
        match planned {
            Ok(action) => plan.push((action, display_path(hunk))),
            Err(message) => return ToolOutput::error(message),
        }
    }

    // Permission gating for every affected path, still before any write, so a denied path fails the
    // whole operation with no partial application.
    for (_, display) in &plan {
        if !ctx.permits("apply_patch", display) {
            return ToolOutput::error(format!(
                "apply_patch: permission denied for {display} (no changes made)"
            ));
        }
    }

    // Commit phase: every action validated, so logical failures cannot leave a partial mix.
    let mut summaries = Vec::new();
    for (action, display) in &plan {
        if let Err(message) = commit(ctx, action) {
            return ToolOutput::error(message);
        }
        summaries.push(action_summary(action, display));
    }

    let count = summaries.len();
    let noun = if count == 1 { "change" } else { "changes" };
    ToolOutput::text(format!(
        "apply_patch: applied {count} {noun}: {}",
        summaries.join(", ")
    ))
}

/// Plan an Add section: the target must not already exist.
fn plan_add(ctx: &ToolContext, path: &str, contents: &str) -> Result<Action, String> {
    let resolved = ctx.resolve(path);
    if resolved.exists() {
        return Err(format!(
            "apply_patch: cannot add {path}: it already exists (no changes made)"
        ));
    }
    Ok(Action::Create {
        path: resolved,
        contents: contents.to_owned(),
    })
}

/// Plan a Delete section: the target must exist, be a file, and have been read this session
/// (consistent with the write/edit read-before-mutate guard).
fn plan_delete(ctx: &ToolContext, path: &str) -> Result<Action, String> {
    let resolved = ctx.resolve(path);
    match fs::metadata(&resolved) {
        Ok(metadata) if metadata.is_dir() => Err(format!(
            "apply_patch: cannot delete {path}: it is a directory (no changes made)"
        )),
        Ok(_) => {
            if !ctx.was_read(&resolved) {
                return Err(format!(
                    "apply_patch: cannot delete {path}: it has not been read this session; read it first (no changes made)"
                ));
            }
            Ok(Action::Delete { path: resolved })
        }
        Err(_) => Err(format!(
            "apply_patch: cannot delete {path}: it does not exist (no changes made)"
        )),
    }
}

/// Plan an Update section: read the file, apply its hunks in memory, and prepare the resulting write
/// (an in-place overwrite, or a rename when `move_to` is set).
fn plan_update(
    ctx: &ToolContext,
    path: &str,
    move_to: Option<&str>,
    chunks: &[parser::Chunk],
) -> Result<Action, String> {
    let resolved = ctx.resolve(path);
    let current = match fs::read_to_string(&resolved) {
        Ok(current) => current,
        Err(_) => {
            return Err(format!(
                "apply_patch: cannot update {path}: it does not exist or is not UTF-8 text (no changes made)"
            ));
        }
    };

    // Read-before-update guard, consistent with write/edit: an existing file must have been read
    // this session before an Update mutates it.
    if !ctx.was_read(&resolved) {
        return Err(format!(
            "apply_patch: cannot update {path}: it has not been read this session; read it first (no changes made)"
        ));
    }

    let updated = apply_chunks(&current, chunks)
        .map_err(|message| format!("apply_patch: {path}: {message} (no changes made)"))?;

    match move_to {
        Some(dest) => {
            let to = ctx.resolve(dest);
            if let Some(reason) = ctx.writability_error(&to) {
                return Err(format!(
                    "apply_patch: cannot update {path}: {reason} (no changes made)"
                ));
            }
            Ok(Action::Rename {
                from: resolved,
                to,
                contents: updated,
            })
        }
        None => {
            if let Some(reason) = ctx.writability_error(&resolved) {
                return Err(format!(
                    "apply_patch: cannot update {path}: {reason} (no changes made)"
                ));
            }
            Ok(Action::Overwrite {
                path: resolved,
                contents: updated,
            })
        }
    }
}

/// Commit one validated action to disk, recording any newly-written file as read so a later
/// edit/write in the same session is permitted.
fn commit(ctx: &ToolContext, action: &Action) -> Result<(), String> {
    match action {
        Action::Create { path, contents } | Action::Overwrite { path, contents } => {
            write_file(path, contents)?;
            ctx.record_read(path.clone());
        }
        Action::Delete { path } => {
            fs::remove_file(path).map_err(|err| {
                format!("apply_patch: could not delete {}: {err}", path.display())
            })?;
        }
        Action::Rename { from, to, contents } => {
            write_file(to, contents)?;
            ctx.record_read(to.clone());
            if from != to {
                fs::remove_file(from).map_err(|err| {
                    format!("apply_patch: could not remove {}: {err}", from.display())
                })?;
            }
        }
    }
    Ok(())
}

/// Write `contents` to `path`, creating any missing parent directories first.
fn write_file(path: &PathBuf, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "apply_patch: could not create parent of {}: {err}",
                path.display()
            )
        })?;
    }
    fs::write(path, contents.as_bytes())
        .map_err(|err| format!("apply_patch: could not write {}: {err}", path.display()))
}

/// The original (envelope) path a hunk targets, used for permission gating and the result summary.
fn display_path(hunk: &Hunk) -> String {
    match hunk {
        Hunk::Add { path, .. } | Hunk::Delete { path } => path.clone(),
        Hunk::Update {
            path,
            move_to: None,
            ..
        } => path.clone(),
        Hunk::Update {
            path,
            move_to: Some(dest),
            ..
        } => format!("{path} -> {dest}"),
    }
}

/// Extract a string argument verbatim (no trimming: patch text must be preserved exactly).
fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-apply-patch-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn ctx_for(dir: &Path) -> ToolContext {
        ToolContext::new(dir.to_path_buf())
    }

    fn apply(ctx: &ToolContext, patch: &str) -> ToolOutput {
        run(&json!({ "patch": patch }), ctx)
    }

    #[test]
    fn add_file_writes_exactly_the_added_lines() {
        let dir = temp_dir("add");
        let ctx = ctx_for(&dir);
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Add File: new.py\n+print(1)\n+print(2)\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert_eq!(
            fs::read_to_string(dir.join("new.py")).unwrap(),
            "print(1)\nprint(2)\n"
        );
    }

    #[test]
    fn update_file_changes_only_targeted_lines() {
        let dir = temp_dir("update");
        let ctx = ctx_for(&dir);
        fs::write(
            dir.join("app.py"),
            "import os\ndef greet():\n    print(\"Hi\")\n    return 0\n",
        )
        .unwrap();
        ctx.record_read(dir.join("app.py"));
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Update File: app.py\n@@ def greet():\n-    print(\"Hi\")\n+    print(\"Hello\")\n+    log()\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert_eq!(
            fs::read_to_string(dir.join("app.py")).unwrap(),
            "import os\ndef greet():\n    print(\"Hello\")\n    log()\n    return 0\n"
        );
    }

    #[test]
    fn delete_file_removes_only_the_target() {
        let dir = temp_dir("delete");
        let ctx = ctx_for(&dir);
        fs::write(dir.join("old.txt"), "gone\n").unwrap();
        fs::write(dir.join("sibling.txt"), "stays\n").unwrap();
        ctx.record_read(dir.join("old.txt"));
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Delete File: old.txt\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert!(!dir.join("old.txt").exists());
        assert_eq!(
            fs::read_to_string(dir.join("sibling.txt")).unwrap(),
            "stays\n"
        );
    }

    #[test]
    fn multi_file_envelope_applies_every_section() {
        let dir = temp_dir("multi");
        let ctx = ctx_for(&dir);
        fs::write(dir.join("update.txt"), "old line\n").unwrap();
        fs::write(dir.join("delete.txt"), "remove me\n").unwrap();
        ctx.record_read(dir.join("update.txt"));
        ctx.record_read(dir.join("delete.txt"));
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Add File: add.txt\n+created\n*** Update File: update.txt\n@@\n-old line\n+new line\n*** Delete File: delete.txt\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert_eq!(
            fs::read_to_string(dir.join("add.txt")).unwrap(),
            "created\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("update.txt")).unwrap(),
            "new line\n"
        );
        assert!(!dir.join("delete.txt").exists());
    }

    #[test]
    fn a_broken_section_leaves_all_targets_unchanged() {
        let dir = temp_dir("atomic");
        let ctx = ctx_for(&dir);
        fs::write(dir.join("update.txt"), "old line\n").unwrap();
        fs::write(dir.join("delete.txt"), "remove me\n").unwrap();
        ctx.record_read(dir.join("update.txt"));
        ctx.record_read(dir.join("delete.txt"));
        // The update section's context cannot be found, so the whole envelope must fail with no
        // file created, updated, or deleted.
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Add File: add.txt\n+created\n*** Update File: update.txt\n@@\n-NOT PRESENT\n+new line\n*** Delete File: delete.txt\n*** End Patch",
        );
        assert!(output.is_error, "{}", output.model_content());
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
    }

    #[test]
    fn fuzzy_context_with_drifted_indentation_applies() {
        let dir = temp_dir("fuzzy");
        let ctx = ctx_for(&dir);
        fs::write(dir.join("s.rs"), "fn main() {\n        let x = 1;\n}\n").unwrap();
        ctx.record_read(dir.join("s.rs"));
        // The hunk's context line omits the on-disk indentation; fuzzy matching still locates it.
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Update File: s.rs\n@@\n-let x = 1;\n+    let y = 2;\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert_eq!(
            fs::read_to_string(dir.join("s.rs")).unwrap(),
            "fn main() {\n    let y = 2;\n}\n"
        );
    }

    #[test]
    fn malformed_envelope_makes_no_changes() {
        let dir = temp_dir("malformed");
        let ctx = ctx_for(&dir);
        let before: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        // Missing the '*** End Patch' marker.
        let output = apply(&ctx, "*** Begin Patch\n*** Add File: x.txt\n+hi");
        assert!(output.is_error);
        assert!(output.model_content().contains("End Patch"));
        let after: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        assert_eq!(before.len(), after.len(), "the directory is untouched");
        assert!(!dir.join("x.txt").exists());
    }

    #[test]
    fn adding_an_existing_file_errors_with_no_change() {
        let dir = temp_dir("exists");
        let ctx = ctx_for(&dir);
        fs::write(dir.join("dup.txt"), "ORIGINAL").unwrap();
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Add File: dup.txt\n+new\n*** End Patch",
        );
        assert!(output.is_error);
        assert_eq!(fs::read_to_string(dir.join("dup.txt")).unwrap(), "ORIGINAL");
    }

    #[test]
    fn updating_a_missing_file_errors() {
        let dir = temp_dir("nofile");
        let ctx = ctx_for(&dir);
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Update File: ghost.txt\n@@\n-a\n+b\n*** End Patch",
        );
        assert!(output.is_error);
        assert!(output.model_content().contains("does not exist"));
    }

    #[test]
    fn rename_writes_destination_and_removes_source() {
        let dir = temp_dir("rename");
        let ctx = ctx_for(&dir);
        fs::write(dir.join("a.txt"), "one\n").unwrap();
        ctx.record_read(dir.join("a.txt"));
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Update File: a.txt\n*** Move to: b.txt\n@@\n-one\n+two\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert!(!dir.join("a.txt").exists());
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "two\n");
    }

    #[test]
    fn add_creates_missing_parent_directories() {
        let dir = temp_dir("nested");
        let ctx = ctx_for(&dir);
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Add File: nested/deep/file.txt\n+content\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert_eq!(
            fs::read_to_string(dir.join("nested/deep/file.txt")).unwrap(),
            "content\n"
        );
    }

    #[test]
    fn permission_deny_blocks_the_patch_and_leaves_files_intact() {
        use permissions::{Action as PermAction, PermissionEngine, Rule, Ruleset};

        let dir = temp_dir("perm");
        let engine = PermissionEngine::new(Ruleset::new(vec![Rule::new(
            "apply_patch",
            "guard.txt",
            PermAction::Deny,
        )]));
        let ctx = ToolContext::new(dir.clone()).with_permissions(Arc::new(engine));
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Add File: guard.txt\n+x\n*** End Patch",
        );
        assert!(output.is_error);
        assert!(output.model_content().contains("permission denied"));
        assert!(!dir.join("guard.txt").exists());
    }

    #[test]
    fn empty_patch_argument_errors() {
        let dir = temp_dir("empty");
        let ctx = ctx_for(&dir);
        let output = apply(&ctx, "   ");
        assert!(output.is_error);
        assert!(output.model_content().contains("must not be empty"));
    }

    #[test]
    fn update_of_an_unread_existing_file_is_refused_and_bytes_intact() {
        let dir = temp_dir("update-unread");
        let ctx = ctx_for(&dir);
        // Seed the file WITHOUT recording a read this session.
        fs::write(dir.join("cfg.txt"), "OLD\n").unwrap();
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Update File: cfg.txt\n@@\n-OLD\n+NEW\n*** End Patch",
        );
        assert!(output.is_error, "{}", output.model_content());
        assert!(
            output
                .model_content()
                .to_lowercase()
                .contains("read it first"),
            "got: {}",
            output.model_content()
        );
        // The original bytes are untouched.
        assert_eq!(fs::read_to_string(dir.join("cfg.txt")).unwrap(), "OLD\n");
    }

    #[test]
    fn update_after_a_prior_read_applies() {
        let dir = temp_dir("update-read");
        let ctx = ctx_for(&dir);
        fs::write(dir.join("cfg.txt"), "OLD\n").unwrap();
        // Simulate a prior `read` of the file this session.
        ctx.record_read(dir.join("cfg.txt"));
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Update File: cfg.txt\n@@\n-OLD\n+NEW\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert_eq!(fs::read_to_string(dir.join("cfg.txt")).unwrap(), "NEW\n");
    }

    #[test]
    fn delete_of_an_unread_existing_file_is_refused_and_file_survives() {
        let dir = temp_dir("delete-unread");
        let ctx = ctx_for(&dir);
        fs::write(dir.join("cfg.txt"), "KEEP\n").unwrap();
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Delete File: cfg.txt\n*** End Patch",
        );
        assert!(output.is_error, "{}", output.model_content());
        assert!(
            output
                .model_content()
                .to_lowercase()
                .contains("read it first"),
            "got: {}",
            output.model_content()
        );
        assert!(dir.join("cfg.txt").exists());
        assert_eq!(fs::read_to_string(dir.join("cfg.txt")).unwrap(), "KEEP\n");
    }

    #[test]
    fn add_file_needs_no_prior_read() {
        let dir = temp_dir("add-noread");
        let ctx = ctx_for(&dir);
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Add File: fresh.txt\n+content\n*** End Patch",
        );
        assert!(!output.is_error, "{}", output.model_content());
        assert_eq!(
            fs::read_to_string(dir.join("fresh.txt")).unwrap(),
            "content\n"
        );
    }

    #[test]
    fn update_of_a_read_only_file_errors_and_leaves_bytes_intact() {
        let dir = temp_dir("ro");
        let ctx = ctx_for(&dir);
        let path = dir.join("ro.txt");
        fs::write(&path, "ORIGINAL\n").unwrap();
        ctx.record_read(&path);
        set_readonly(&path, true);
        let output = apply(
            &ctx,
            "*** Begin Patch\n*** Update File: ro.txt\n@@\n-ORIGINAL\n+CHANGED\n*** End Patch",
        );
        assert!(output.is_error, "{}", output.model_content());
        assert!(
            output.model_content().to_lowercase().contains("permission"),
            "got: {}",
            output.model_content()
        );
        // Restore perms so the temp file content can be read back, then assert it is intact.
        set_readonly(&path, false);
        assert_eq!(fs::read_to_string(&path).unwrap(), "ORIGINAL\n");
    }

    /// Toggle the read-only bit on `path` so tests can exercise the OS-unwritable guard.
    fn set_readonly(path: &Path, readonly: bool) {
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_readonly(readonly);
        fs::set_permissions(path, perms).unwrap();
    }
}

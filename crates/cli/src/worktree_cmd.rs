//! The `cortex worktree` subcommands: real git worktree create/list/remove with machine events.
//!
//! Each create/remove runs a real `git worktree` command in the working directory's repository and
//! broadcasts a typed event on the [`EventBus`] (`worktree.ready`, `worktree.removed`, or
//! `worktree.failed`) so subscribers — the `-v` log sink today, the server SSE stream when the same
//! bus is shared — observe the operation without parsing stdout. The `list` subcommand parses
//! `git worktree list --porcelain`, so Cortex's output tracks git's own view. Diagnostics go to
//! stderr; machine output (`--format json`) is a single strict JSON document on stdout. Paths are
//! redacted at the output/event boundary (only secret-shaped substrings are masked, so ordinary
//! paths pass through verbatim and still match `git worktree list`).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use errors::Exit;
use events::{Event, EventBus};
use serde::Serialize;

use crate::cli::{
    OutputFormat, WorktreeArgs, WorktreeCommand, WorktreeCreateArgs, WorktreeListArgs,
    WorktreeRemoveArgs,
};

/// Dispatch a `cortex worktree` invocation. With no subcommand it behaves like `list`.
pub fn run(args: WorktreeArgs, bus: Arc<EventBus>) -> Exit {
    let cwd = match std::env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("cortex worktree: cannot determine the working directory: {err}");
            return Exit::Failure;
        }
    };
    if !is_git_repo(&cwd) {
        eprintln!("cortex worktree: the working directory is not a git repository");
        return Exit::Failure;
    }

    match args.command {
        None => list(&cwd, OutputFormat::Text),
        Some(WorktreeCommand::List(WorktreeListArgs { format })) => list(&cwd, format.format),
        Some(WorktreeCommand::Create(args)) => create(&cwd, args, &bus),
        Some(WorktreeCommand::Remove(args)) => remove(&cwd, args, &bus),
    }
}

fn list(cwd: &Path, format: OutputFormat) -> Exit {
    let worktrees = worktree_entries(cwd);
    match format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(&WorktreeList { worktrees }),
        OutputFormat::Text => emit_text(&render_worktree_list(&worktrees)),
    }
}

fn create(cwd: &Path, args: WorktreeCreateArgs, bus: &EventBus) -> Exit {
    let target = resolve_path(cwd, &args.path);
    let target_display = target.to_string_lossy().into_owned();

    let mut git_args: Vec<String> = vec!["worktree".to_owned(), "add".to_owned()];
    if args.force {
        git_args.push("--force".to_owned());
    }
    let branch = args
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty());
    let reference = args
        .reference
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let commitish = if args.new_branch {
        if let Some(branch) = branch {
            git_args.push("-b".to_owned());
            git_args.push(branch.to_owned());
        }
        reference
    } else {
        // No `-b`/`-B`: an existing branch is the commit-ish checked out after the path, so the
        // worktree attaches to it as-is. `-B` would reset that branch to HEAD, silently moving it.
        branch.or(reference)
    };
    git_args.push(target_display.clone());
    if let Some(commitish) = commitish {
        git_args.push(commitish.to_owned());
    }

    match run_git_mut(cwd, &git_args) {
        Ok(()) => {
            bus.publish(&Event::worktree_ready(redaction::redact(&target_display)));
            let view = worktree_entries(cwd)
                .into_iter()
                .find(|entry| entry.path == redaction::redact(&target_display))
                .unwrap_or(WorktreeView {
                    path: redaction::redact(&target_display),
                    head: None,
                    branch: args.branch.as_deref().map(redaction::redact),
                    bare: false,
                    detached: false,
                    locked: false,
                });
            emit_worktree(&view, args.format.format)
        }
        Err(message) => {
            bus.publish(&Event::worktree_failed(
                redaction::redact(&target_display),
                &message,
            ));
            eprintln!("cortex worktree: {message}");
            Exit::Failure
        }
    }
}

fn remove(cwd: &Path, args: WorktreeRemoveArgs, bus: &EventBus) -> Exit {
    let target = resolve_path(cwd, &args.path);
    let target_display = target.to_string_lossy().into_owned();

    let mut git_args: Vec<String> = vec!["worktree".to_owned(), "remove".to_owned()];
    if args.force {
        git_args.push("--force".to_owned());
    }
    git_args.push(target_display.clone());

    match run_git_mut(cwd, &git_args) {
        Ok(()) => {
            bus.publish(&Event::worktree_removed(redaction::redact(&target_display)));
            let removed = WorktreeRemoved {
                status: "removed",
                path: redaction::redact(&target_display),
            };
            match args.format.format {
                OutputFormat::Json | OutputFormat::Jsonl => emit_json(&removed),
                OutputFormat::Text => emit_text(&format!("removed {}\n", removed.path)),
            }
        }
        Err(message) => {
            bus.publish(&Event::worktree_failed(
                redaction::redact(&target_display),
                &message,
            ));
            eprintln!("cortex worktree: {message}");
            Exit::Failure
        }
    }
}

/// Whether `cwd` is inside a git working tree.
fn is_git_repo(cwd: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Parse `git worktree list --porcelain` into structured entries, redacting paths at the boundary.
fn worktree_entries(cwd: &Path) -> Vec<WorktreeView> {
    let Some(output) = run_git_read(cwd, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut worktrees = Vec::new();
    let mut current: Option<WorktreeView> = None;
    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(entry) = current.take() {
                worktrees.push(entry);
            }
            current = Some(WorktreeView {
                path: redaction::redact(path.trim()),
                head: None,
                branch: None,
                bare: false,
                detached: false,
                locked: false,
            });
        } else if let Some(entry) = current.as_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                entry.head = Some(head.trim().to_owned());
            } else if let Some(branch) = line.strip_prefix("branch ") {
                entry.branch = Some(redaction::redact(branch.trim()));
            } else if line == "bare" {
                entry.bare = true;
            } else if line == "detached" {
                entry.detached = true;
            } else if line.starts_with("locked") {
                entry.locked = true;
            }
        }
    }
    if let Some(entry) = current.take() {
        worktrees.push(entry);
    }
    worktrees
}

/// Resolve a worktree path argument relative to `cwd` without requiring it to exist.
fn resolve_path(cwd: &Path, input: &str) -> PathBuf {
    let candidate = Path::new(input);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    }
}

/// Run a read-only `git` command, returning trimmed stdout on success.
fn run_git_read(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Run a mutating `git` command, returning the scrubbed stderr (or a generic message) on failure.
fn run_git_mut(cwd: &Path, args: &[String]) -> Result<(), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|err| format!("could not run git: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        Err("git worktree command failed".to_owned())
    } else {
        Err(redaction::redact(detail))
    }
}

/// One git worktree for output, mirroring the server's `Worktree` shape.
#[derive(Debug, Serialize)]
pub(crate) struct WorktreeView {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    bare: bool,
    detached: bool,
    locked: bool,
}

#[derive(Debug, Serialize)]
struct WorktreeList {
    worktrees: Vec<WorktreeView>,
}

#[derive(Debug, Serialize)]
struct WorktreeRemoved {
    status: &'static str,
    path: String,
}

fn emit_worktree(view: &WorktreeView, format: OutputFormat) -> Exit {
    match format {
        OutputFormat::Json | OutputFormat::Jsonl => emit_json(view),
        OutputFormat::Text => emit_text(&render_worktree(view)),
    }
}

fn render_worktree(view: &WorktreeView) -> String {
    let mut out = format!("worktree {}\n", view.path);
    if let Some(branch) = view.branch.as_deref() {
        out.push_str(&format!("branch: {branch}\n"));
    }
    if let Some(head) = view.head.as_deref() {
        out.push_str(&format!("head: {head}\n"));
    }
    out
}

fn render_worktree_list(worktrees: &[WorktreeView]) -> String {
    let mut out = String::new();
    for view in worktrees {
        out.push_str(&view.path);
        if let Some(branch) = view.branch.as_deref() {
            out.push('\t');
            out.push_str(branch);
        }
        out.push('\n');
    }
    out
}

/// Serialize a value to a single-line strict JSON document terminated with a newline.
fn emit_json<T: Serialize>(value: &T) -> Exit {
    match serde_json::to_string(value) {
        Ok(mut body) => {
            body.push('\n');
            emit_text(&body)
        }
        Err(err) => {
            eprintln!("cortex worktree: failed to serialize output: {err}");
            Exit::Failure
        }
    }
}

/// Write a body to stdout, mapping a broken pipe to a generic failure exit.
fn emit_text(body: &str) -> Exit {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    if handle
        .write_all(body.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return Exit::Failure;
    }
    Exit::Success
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Collector {
        kinds: Mutex<Vec<String>>,
    }

    impl Collector {
        fn kinds(&self) -> Vec<String> {
            self.kinds.lock().expect("collector lock").clone()
        }
    }

    impl events::Subscriber for Collector {
        fn on_event(&self, event: &Event) {
            self.kinds
                .lock()
                .expect("collector lock")
                .push(event.kind().to_owned());
        }
    }

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn rev_parse(cwd: &Path, rev: &str) -> String {
        let output = Command::new("git")
            .args(["rev-parse", rev])
            .current_dir(cwd)
            .output()
            .expect("run git rev-parse");
        assert!(
            output.status.success(),
            "git rev-parse {rev} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn scratch_repo(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cortex-worktree-cmd-{label}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.name", "Test"]);
        git(&dir, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(dir.join("f.txt"), "x\n").expect("write");
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-qm", "init"]);
        dir
    }

    fn create_args(path: &str) -> WorktreeCreateArgs {
        WorktreeCreateArgs {
            path: path.to_owned(),
            branch: Some("feature".to_owned()),
            new_branch: true,
            reference: None,
            force: false,
            format: crate::cli::FormatArgs {
                format: OutputFormat::Json,
            },
        }
    }

    #[test]
    fn create_list_remove_runs_git_and_emits_events() {
        let repo = scratch_repo("lifecycle");
        let bus = EventBus::new();
        let collector = Arc::new(Collector::default());
        bus.subscribe(collector.clone());

        // Create a worktree on a new branch.
        let exit = create(&repo, create_args("wt-feature"), &bus);
        assert_eq!(exit, Exit::Success);
        assert!(
            collector.kinds().contains(&"worktree.ready".to_owned()),
            "worktree.ready emitted: {:?}",
            collector.kinds()
        );

        // Cortex's parsed list matches `git worktree list --porcelain`.
        let cortex_paths: Vec<String> = worktree_entries(&repo)
            .into_iter()
            .map(|view| view.path)
            .collect();
        let git_paths: Vec<String> = run_git_read(&repo, &["worktree", "list", "--porcelain"])
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(|path| path.trim().to_owned())
            .collect();
        assert_eq!(
            cortex_paths, git_paths,
            "cortex worktree list matches git worktree list"
        );
        assert!(cortex_paths.iter().any(|p| p.contains("wt-feature")));

        // Remove it.
        let exit = remove(
            &repo,
            WorktreeRemoveArgs {
                path: "wt-feature".to_owned(),
                force: true,
                format: crate::cli::FormatArgs {
                    format: OutputFormat::Json,
                },
            },
            &bus,
        );
        assert_eq!(exit, Exit::Success);
        assert!(
            collector.kinds().contains(&"worktree.removed".to_owned()),
            "worktree.removed emitted: {:?}",
            collector.kinds()
        );
        assert!(
            !worktree_entries(&repo)
                .iter()
                .any(|view| view.path.contains("wt-feature")),
            "worktree removed from the list"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn failed_create_emits_failed_event() {
        let repo = scratch_repo("failure");
        let bus = EventBus::new();
        let collector = Arc::new(Collector::default());
        bus.subscribe(collector.clone());

        assert_eq!(create(&repo, create_args("wt-dup"), &bus), Exit::Success);
        // Re-creating at the same non-empty path fails.
        let mut dup = create_args("wt-dup");
        dup.branch = Some("dup2".to_owned());
        assert_eq!(create(&repo, dup, &bus), Exit::Failure);
        assert!(
            collector.kinds().contains(&"worktree.failed".to_owned()),
            "worktree.failed emitted on git failure: {:?}",
            collector.kinds()
        );

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn create_attaches_existing_branch_without_resetting_it() {
        let repo = scratch_repo("existing-branch");
        // A branch pinned at the first commit, then HEAD advances past it.
        let original = rev_parse(&repo, "HEAD");
        git(&repo, &["branch", "existing"]);
        std::fs::write(repo.join("f.txt"), "y\n").expect("write");
        git(&repo, &["commit", "-aqm", "second"]);
        let head = rev_parse(&repo, "HEAD");
        assert_ne!(original, head, "HEAD must advance past the existing branch");

        let bus = EventBus::new();
        let args = WorktreeCreateArgs {
            path: "wt-existing".to_owned(),
            branch: Some("existing".to_owned()),
            new_branch: false,
            reference: None,
            force: false,
            format: crate::cli::FormatArgs {
                format: OutputFormat::Json,
            },
        };
        assert_eq!(create(&repo, args, &bus), Exit::Success);

        // Attaching the worktree must check the existing branch out as-is, never reset it to HEAD.
        assert_eq!(
            rev_parse(&repo, "existing"),
            original,
            "attaching a worktree to an existing branch must not move it to HEAD"
        );
        assert!(
            worktree_entries(&repo)
                .iter()
                .any(|view| view.path.contains("wt-existing")),
            "the existing-branch worktree should be listed"
        );

        let _ = std::fs::remove_dir_all(&repo);
    }
}

//! The `bash` tool: run a shell command in the session working directory with real capture,
//! permission gating by the command's real structure, secret-stripped child environment, a bounded
//! foreground timeout, background jobs, and large-output truncation to disk.
//!
//! Behavior (architecture §3.4, validation contract VAL-TOOLS-032..037/041/042):
//!
//! - **Capture (032/033).** A foreground command runs through `bash -c` rooted at the session
//!   working directory; its stdout, stderr, and exit code are captured and returned. A non-zero
//!   exit and stderr are surfaced (the failure is reported, the turn is not aborted).
//! - **Working directory (042).** The command runs with its current directory set to the session
//!   working directory, so `pwd` and relative paths resolve there, consistently with the file tools.
//! - **Permission gating (035).** The command is tree-sitter-parsed so the real root/sub-commands
//!   are extracted (see [`parse`]); each is checked against the permission engine, so a dangerous
//!   verb obscured by quoting/compound structure is still gated by its real command, and a benign
//!   string that merely contains the verb is not. A denied segment refuses the whole command before
//!   anything runs (no side effect).
//! - **Secret env (036).** The child's environment is rebuilt with the agent's secrets removed
//!   (see [`redaction::is_secret_env_var`]/[`redaction::value_is_secret`]), so a command cannot read
//!   `OPENROUTER_API_KEY` (or any listed/key-shaped secret).
//! - **Timeout (041).** A foreground command is bounded by a configured timeout; on expiry the
//!   child is terminated and reaped and a timeout result is returned, never an indefinite hang.
//! - **Background jobs (034).** `background: true` returns control immediately with a tracked,
//!   inspectable job handle; the spawned PID is tracked in [`BackgroundJobs`] and reaped on shutdown
//!   (no orphan).
//! - **Truncation (037).** Large output is truncated to the configured limits with a notice while
//!   the complete output is spilled to an on-disk file referenced in the result (see
//!   [`crate::output`]).

mod jobs;
mod parse;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::process::Command;

pub use jobs::{
    BackgroundJob, BackgroundJobs, JobHandle, pid_alive_same_process, process_start_time,
};

use crate::output::{OutputLimits, bound_output};
use crate::tool::{Tool, ToolContext, ToolOutput};

/// The default foreground command timeout (milliseconds) when neither the call nor the environment
/// overrides it. A foreground command that runs longer is terminated rather than hanging the turn.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

/// The environment variable that overrides the default foreground timeout (milliseconds), so an
/// operator (or a validator) can bound long commands without editing the call.
const TIMEOUT_ENV: &str = "CORTEX_BASH_TIMEOUT_MS";
/// Compatibility cap for in-band bash output lines before truncation + spill.
const BASH_MAX_LINES: usize = 2000;
/// Compatibility cap for in-band bash output bytes before truncation + spill.
const BASH_MAX_BYTES: usize = 50 * 1024;

const DESCRIPTION: &str = "Run a shell command in the session working directory and return its \
stdout, stderr, and exit code. The command runs via `bash -c`, so pipes, redirection, and `&&`/`||` \
work. Relative paths and `pwd` resolve against the working directory. Use `workdir` to run a single \
command from a different directory without changing the session default cwd. Set `background: true` for a \
long-running command to return immediately with a job handle (its output is written to a file); the \
job is terminated when the session ends. A foreground command is terminated if it exceeds the \
configured timeout. Large output is truncated with a notice and the complete output is written to a \
spill file named in the result.";

/// The `bash` tool.
pub struct BashTool;

impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to run."
                },
                "background": {
                    "type": "boolean",
                    "description": "Run as a background job, returning control immediately with a job handle instead of waiting for the command to finish. Use for long-running processes."
                },
                "timeout": {
                    "type": "number",
                    "description": "Foreground timeout in milliseconds, overriding the default. Ignored for background jobs."
                },
                "workdir": {
                    "type": "string",
                    "description": "Optional working directory for this command (absolute, or relative to the session working directory)."
                },
                "work_dir": {
                    "type": "string",
                    "description": "Compatibility alias for `workdir`."
                },
                "description": {
                    "type": "string",
                    "description": "A short description of what the command does."
                }
            },
            "required": ["command"]
        })
    }

    fn execute<'a>(&'a self, arguments: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move { run(arguments, ctx).await })
    }
}

/// Execute the `bash` tool against the resolved arguments.
async fn run(arguments: Value, ctx: &ToolContext) -> ToolOutput {
    let Some(command) = arguments.get("command").and_then(Value::as_str) else {
        return ToolOutput::error("bash: the 'command' argument is required");
    };
    let command = command.trim();
    if command.is_empty() {
        return ToolOutput::error("bash: the 'command' argument must not be empty");
    }
    if let Err(message) = ctx.ensure_not_planning("bash") {
        return ToolOutput::error(message);
    }
    let has_explicit_workdir = [
        "workdir",
        "work_dir",
        "workingDirectory",
        "working_directory",
    ]
    .into_iter()
    .any(|key| arguments.get(key).and_then(Value::as_str).is_some());
    let working_dir = match resolve_working_dir(&arguments, ctx) {
        Ok(working_dir) => working_dir,
        Err(message) => return ToolOutput::error(message),
    };

    // Permission gating by the command's REAL structure: extract each command invocation with
    // tree-sitter and refuse the whole command if any segment is not permitted, before anything
    // runs (so a denied command produces no side effect).
    for segment in parse::extract_commands(command) {
        if let Err(message) = ctx.ensure_permitted("bash", &segment) {
            return ToolOutput::error(format!("{message} (command not run)"));
        }
    }

    let background = arguments
        .get("background")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let output = if background {
        spawn_background(command, &working_dir, ctx)
    } else {
        run_foreground(command, &working_dir, ctx, timeout_for(&arguments)).await
    };
    if has_explicit_workdir && !output.is_error {
        ctx.record_memory_scope(&working_dir);
    }
    output
}

/// Resolve the foreground timeout: the call's `timeout` (ms), else `CORTEX_BASH_TIMEOUT_MS`, else
/// the default. A zero or invalid value falls through to the next source.
fn timeout_for(arguments: &Value) -> Duration {
    let from_call = arguments
        .get("timeout")
        .and_then(Value::as_u64)
        .filter(|ms| *ms > 0);
    let from_env = std::env::var(TIMEOUT_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|ms| *ms > 0);
    Duration::from_millis(from_call.or(from_env).unwrap_or(DEFAULT_TIMEOUT_MS))
}

/// Extract a string argument from the assembled arguments object.
fn string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Resolve the command working directory from `workdir` aliases, validating it exists and is a
/// directory. When omitted, the session working directory is used.
fn resolve_working_dir(arguments: &Value, ctx: &ToolContext) -> Result<PathBuf, String> {
    let Some(workdir) = string_arg(arguments, "workdir")
        .or_else(|| string_arg(arguments, "work_dir"))
        .or_else(|| string_arg(arguments, "workingDirectory"))
        .or_else(|| string_arg(arguments, "working_directory"))
    else {
        return Ok(ctx.cwd().to_path_buf());
    };

    let trimmed = workdir.trim();
    if trimmed.is_empty() {
        return Err("bash: the 'workdir' argument must not be empty".to_owned());
    }
    let resolved = ctx.resolve(trimmed);
    let metadata = std::fs::metadata(&resolved).map_err(|err| {
        format!("bash: workdir {trimmed} does not exist or is inaccessible: {err}")
    })?;
    if !metadata.is_dir() {
        return Err(format!("bash: workdir {trimmed} is not a directory"));
    }
    ctx.ensure_workdir_permitted(&resolved)?;
    Ok(resolved)
}

/// Build a `bash -c <command>` command rooted at the working directory with a secret-stripped
/// environment, so the child cannot read the agent's credentials.
///
/// The child leads its own process group ([`process_group(0)`](Command::process_group)), so reaping
/// can terminate the whole tree (the `bash` wrapper *and* any commands it spawns) by process group,
/// leaving no orphaned grandchild — see [`kill_process_group`].
fn base_command(command: &str, working_dir: &Path, ctx: &ToolContext) -> Command {
    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(command)
        .current_dir(working_dir)
        .env_clear()
        .envs(child_env())
        .envs(ctx.shell_env().iter().cloned())
        .stdin(Stdio::null())
        .process_group(0)
        .kill_on_drop(true);
    cmd
}

/// Terminate the process group led by `pid` (set up via [`base_command`]'s `process_group(0)`), so
/// the spawned `bash` wrapper and every descendant it started are killed — not just the direct
/// child. A negative pid targets the whole group; failures (the group is already gone) are ignored.
pub(super) async fn kill_process_group(pid: u32) {
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg("--")
        .arg(format!("-{pid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

/// Best-effort process-group cleanup guard for cancellation paths.
///
/// If a foreground bash command future is dropped (for example due to turn abort), this guard's
/// drop path sends `SIGKILL` to the whole process group so forked grandchildren are not orphaned.
struct ProcessGroupGuard {
    pid: u32,
    armed: bool,
}

impl ProcessGroupGuard {
    fn new(pid: u32) -> Self {
        Self { pid, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let _ = std::process::Command::new("kill")
            .arg("-KILL")
            .arg("--")
            .arg(format!("-{}", self.pid))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// The environment passed to a spawned child: the process environment with the agent's secrets
/// stripped, so no credential reaches the command.
fn child_env() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    filter_secret_env(std::env::vars_os())
}

/// Strip the agent's secrets from an environment: drop any variable whose name is a known
/// credential variable or whose value is API-key-shaped, keeping every other variable (including
/// `PATH`) verbatim. Pure over its input so the filtering is testable without mutating the process
/// environment.
fn filter_secret_env<I>(vars: I) -> Vec<(std::ffi::OsString, std::ffi::OsString)>
where
    I: IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
{
    vars.into_iter()
        .filter(|(name, value)| {
            if redaction::is_secret_env_var(&name.to_string_lossy()) {
                return false;
            }
            !redaction::value_is_secret(&value.to_string_lossy())
        })
        .collect()
}

/// Run a foreground command bounded by `timeout`, capturing stdout/stderr/exit. On timeout the
/// child is terminated (and reaped via `kill_on_drop`) and a timeout result is returned.
async fn run_foreground(
    command: &str,
    working_dir: &Path,
    ctx: &ToolContext,
    timeout: Duration,
) -> ToolOutput {
    let mut cmd = base_command(command, working_dir, ctx);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return ToolOutput::error(format!("bash: failed to start command: {err}")),
    };
    let pid = child.id();
    let mut process_group_guard = pid.map(ProcessGroupGuard::new);

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            if let Some(guard) = process_group_guard.as_mut() {
                guard.disarm();
            }
            if let Some(audit) = ctx.audit_sink() {
                let outcome = if output.status.success() {
                    "success"
                } else {
                    "error"
                };
                audit.record_shell_execution(command, working_dir, output.status.code(), outcome);
            }
            render_completed(&output)
        }
        Ok(Err(err)) => {
            if let Some(guard) = process_group_guard.as_mut() {
                guard.disarm();
            }
            if let Some(audit) = ctx.audit_sink() {
                audit.record_shell_execution(command, working_dir, None, "error");
            }
            ToolOutput::error(format!("bash: command failed to run: {err}"))
        }
        Err(_elapsed) => {
            // The cancelled `wait_with_output` future dropped the child, which `kill_on_drop`
            // terminates and the runtime reaps. Also kill the whole process group so a forked
            // grandchild (e.g. from a compound command) is not orphaned.
            if let Some(pid) = pid {
                kill_process_group(pid).await;
            }
            if let Some(guard) = process_group_guard.as_mut() {
                guard.disarm();
            }
            if let Some(audit) = ctx.audit_sink() {
                audit.record_shell_execution(command, working_dir, None, "timeout");
            }
            let pid = pid.map_or_else(|| "unknown".to_owned(), |p| p.to_string());
            ToolOutput::error(format!(
                "bash: command timed out after {}ms and was terminated (pid {pid})",
                timeout.as_millis()
            ))
        }
    }
}

/// Render a completed foreground command's captured output and exit status into a tool result,
/// bounding large stdout/stderr to disk and marking a non-zero exit as an error.
fn render_completed(output: &std::process::Output) -> ToolOutput {
    let limits = OutputLimits {
        max_lines: BASH_MAX_LINES,
        max_bytes: BASH_MAX_BYTES,
    };
    let stdout = redaction::redact(&String::from_utf8_lossy(&output.stdout));
    let stderr = redaction::redact(&String::from_utf8_lossy(&output.stderr));

    let mut sections: Vec<String> = Vec::new();

    if !stdout.is_empty() {
        let bounded = bound_output(&stdout, limits, "bash-stdout");
        sections.push(bounded.text);
    }
    if !stderr.is_empty() {
        let bounded = bound_output(&stderr, limits, "bash-stderr");
        sections.push(format!("[stderr]\n{}", bounded.text));
    }

    let code = output.status.code();
    let success = output.status.success();
    sections.push(match code {
        Some(code) => format!("[exit code: {code}]"),
        None => "[exit code: terminated by signal]".to_owned(),
    });

    let text = sections.join("\n");
    if success {
        ToolOutput::text(text)
    } else {
        ToolOutput::error(text)
    }
}

/// Spawn a background job: return control immediately with a tracked, inspectable job handle while
/// the process keeps running, its combined output streamed to an on-disk log file.
fn spawn_background(command: &str, working_dir: &Path, ctx: &ToolContext) -> ToolOutput {
    let jobs = ctx.background_jobs();
    let id = format!("bash-{}-{}", std::process::id(), jobs.len() + 1);

    let log_path = std::env::temp_dir()
        .join("cortex-tool-output")
        .join(format!("{id}.log"));
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let log = match std::fs::File::create(&log_path) {
        Ok(file) => file,
        Err(err) => {
            return ToolOutput::error(format!(
                "bash: failed to create the background job log file: {err}"
            ));
        }
    };
    let log_err = match log.try_clone() {
        Ok(file) => file,
        Err(err) => {
            return ToolOutput::error(format!("bash: failed to set up background output: {err}"));
        }
    };

    let mut cmd = base_command(command, working_dir, ctx);
    cmd.stdout(Stdio::from(log)).stderr(Stdio::from(log_err));

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return ToolOutput::error(format!("bash: failed to start background command: {err}"));
        }
    };
    let pid = child.id().unwrap_or(0);
    if let Some(audit) = ctx.audit_sink() {
        audit.record_shell_execution(command, working_dir, None, "started");
    }

    jobs.register(BackgroundJob::new(
        id.clone(),
        pid,
        command,
        log_path.clone(),
        child,
    ));

    ToolOutput::text(format!(
        "[background job started]\n\
         job id: {id}\n\
         pid: {pid}\n\
         cwd: {}\n\
         output: {}\n\
         The command is running in the background; its output is written to the file above. \
         The job is terminated when the session ends.",
        working_dir.display(),
        log_path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use permissions::{Action, PermissionEngine, Rule, Ruleset};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("cortex-bash-{tag}-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn audit_for(dir: &Path, session: &str) -> Arc<audit::AuditSink> {
        Arc::new(audit::AuditSink::new(dir.join("audit.jsonl"), session))
    }

    fn pid_alive(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
    }

    // VAL-TOOLS-032: stdout + zero exit captured; a file-writing command leaves the file on disk.
    #[tokio::test]
    async fn captures_stdout_and_zero_exit() {
        let dir = temp_dir("ok");
        let audit = audit_for(&dir, "ses_bash");
        let ctx = ToolContext::new(dir.clone()).with_audit(audit.clone());
        let out = run(json!({ "command": "echo cortex-ok" }), &ctx).await;
        assert!(!out.is_error, "a successful command is not an error");
        let text = out.model_content();
        assert!(text.contains("cortex-ok"), "stdout captured: {text}");
        assert!(
            text.contains("[exit code: 0]"),
            "zero exit reported: {text}"
        );
        let records = audit::read_jsonl(audit.path()).expect("audit records");
        assert!(records.iter().any(|record| {
            record["type"] == "shell_execution"
                && record["session"] == "ses_bash"
                && record["command"] == "echo cortex-ok"
                && record["exit_status"] == 0
                && record["cwd"]
                    .as_str()
                    .is_some_and(|cwd| cwd.contains("cortex-bash-ok"))
        }));
    }

    #[tokio::test]
    async fn a_file_writing_command_leaves_the_file_on_disk() {
        let dir = temp_dir("touch");
        let ctx = ToolContext::new(dir.clone());
        let out = run(json!({ "command": "touch made.txt" }), &ctx).await;
        assert!(!out.is_error, "got: {}", out.model_content());
        assert!(dir.join("made.txt").exists(), "touch created the file");
    }

    // VAL-TOOLS-033: a failing command reports a non-zero exit and surfaces stderr.
    #[tokio::test]
    async fn captures_non_zero_exit_and_stderr() {
        let dir = temp_dir("fail");
        let ctx = ToolContext::new(dir.clone());
        let out = run(json!({ "command": "ls /no/such/dir" }), &ctx).await;
        assert!(out.is_error, "a failing command is marked an error");
        let text = out.model_content();
        assert!(!text.contains("[exit code: 0]"), "non-zero exit: {text}");
        assert!(text.contains("[stderr]"), "stderr surfaced: {text}");
        assert!(
            text.contains("/no/such/dir"),
            "stderr names the path: {text}"
        );
    }

    // VAL-TOOLS-042: the command runs in the session working directory.
    #[tokio::test]
    async fn runs_in_the_session_working_directory() {
        let dir = temp_dir("cwd");
        let ctx = ToolContext::new(dir.clone());

        let pwd = run(json!({ "command": "pwd" }), &ctx).await;
        let canonical = std::fs::canonicalize(&dir).unwrap();
        assert!(
            pwd.model_content()
                .contains(&canonical.display().to_string())
                || pwd.model_content().contains(&dir.display().to_string()),
            "pwd reports the working dir: {}",
            pwd.model_content()
        );

        let touch = run(json!({ "command": "touch rel.txt" }), &ctx).await;
        assert!(!touch.is_error);
        assert!(
            dir.join("rel.txt").exists(),
            "relative side effect lands in cwd"
        );
    }

    #[tokio::test]
    async fn workdir_alias_runs_in_requested_directory_only_for_that_call() {
        let dir = temp_dir("workdir");
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let ctx = ToolContext::new(dir.clone());

        let scoped = run(
            json!({
                "command": "pwd",
                "workdir": "nested"
            }),
            &ctx,
        )
        .await;
        assert!(!scoped.is_error, "{}", scoped.model_content());
        let nested_canonical = std::fs::canonicalize(&nested).unwrap();
        assert!(
            scoped
                .model_content()
                .contains(&nested_canonical.display().to_string())
                || scoped
                    .model_content()
                    .contains(&nested.display().to_string()),
            "pwd should resolve to nested workdir: {}",
            scoped.model_content()
        );

        let default_pwd = run(json!({ "command": "pwd" }), &ctx).await;
        let root_canonical = std::fs::canonicalize(&dir).unwrap();
        assert!(
            default_pwd
                .model_content()
                .contains(&root_canonical.display().to_string())
                || default_pwd
                    .model_content()
                    .contains(&dir.display().to_string()),
            "session cwd should remain unchanged: {}",
            default_pwd.model_content()
        );
        assert_eq!(
            ctx.memory_scopes(),
            vec![ctx.resolve("nested")],
            "only the successful explicit workdir activates a memory scope"
        );
    }

    #[tokio::test]
    async fn unscoped_or_unsuccessful_bash_does_not_activate_memory() {
        let dir = temp_dir("memory-scope-negative");
        let nested = dir.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let ctx = ToolContext::new(dir.clone());

        let unscoped = run(json!({ "command": "cd nested && pwd" }), &ctx).await;
        assert!(!unscoped.is_error, "{}", unscoped.model_content());

        let failed = run(json!({ "command": "exit 7", "workdir": "nested" }), &ctx).await;
        assert!(failed.is_error, "{}", failed.model_content());

        let missing = run(json!({ "command": "pwd", "workdir": "missing" }), &ctx).await;
        assert!(missing.is_error, "{}", missing.model_content());
        assert!(
            ctx.memory_scopes().is_empty(),
            "only a successful explicitly validated workdir may activate memory"
        );
    }

    #[tokio::test]
    async fn workdir_requires_external_directory_permission_for_outside_paths() {
        let dir = temp_dir("workdir-external-deny");
        let outside = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!(
                "cortex-bash-workdir-ext-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&outside).expect("outside dir");

        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("bash", "*", Action::Allow),
            Rule::new("external_directory", "*", Action::Deny),
        ]));
        let ctx = ToolContext::new(dir.clone()).with_permissions(Arc::new(engine));

        let out = run(
            json!({
                "command": "pwd",
                "workdir": outside.to_string_lossy(),
            }),
            &ctx,
        )
        .await;
        assert!(out.is_error, "outside workdir should be denied");
        assert!(
            out.model_content()
                .contains("permission denied for workdir"),
            "got: {}",
            out.model_content()
        );
        assert!(
            ctx.memory_scopes().is_empty(),
            "a denied workdir must not activate nested memory"
        );
        let _ = std::fs::remove_dir_all(outside);
    }

    // VAL-TOOLS-036: secret env is stripped from the child (pure filter — the process env is not
    // mutated, which would require `unsafe` the workspace forbids). The end-to-end proof that a live
    // `OPENROUTER_API_KEY` is invisible to a spawned command is the black-box `cortex run` test.
    #[test]
    fn filter_strips_secret_env_by_name_and_shape() {
        use std::ffi::OsString;
        let key = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8));
        let vars: Vec<(OsString, OsString)> = vec![
            ("OPENROUTER_API_KEY".into(), key.clone().into()),
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("LEAKED_COPY".into(), key.clone().into()),
            ("HOME".into(), "/root".into()),
        ];
        let filtered = filter_secret_env(vars);
        let names: Vec<String> = filtered
            .iter()
            .map(|(n, _)| n.to_string_lossy().into_owned())
            .collect();

        assert!(
            !names.contains(&"OPENROUTER_API_KEY".to_owned()),
            "the credential var is stripped by name"
        );
        assert!(
            !names.contains(&"LEAKED_COPY".to_owned()),
            "a key-shaped value under any name is stripped by shape"
        );
        assert!(names.contains(&"PATH".to_owned()), "PATH passes through");
        assert!(
            names.contains(&"HOME".to_owned()),
            "ordinary vars pass through"
        );
        for (_, value) in &filtered {
            assert!(
                !value.to_string_lossy().contains("deadbeef"),
                "no surviving variable exposes the key value"
            );
        }
    }

    #[tokio::test]
    async fn child_env_passes_through_path_for_command_resolution() {
        let dir = temp_dir("path");
        let ctx = ToolContext::new(dir.clone());
        let out = run(json!({ "command": "printenv PATH" }), &ctx).await;
        assert!(
            !out.is_error,
            "PATH is available to the child: {}",
            out.model_content()
        );
        assert!(
            out.model_content().contains('/'),
            "the child sees a real PATH: {}",
            out.model_content()
        );
    }

    // VAL-TOOLS-035: tree-sitter parsing gates by the real command.
    #[tokio::test]
    async fn deny_rule_blocks_the_real_command_in_a_compound() {
        let dir = temp_dir("deny");
        std::fs::write(dir.join("victim.txt"), "keep me").unwrap();

        // Allow everything except `rm *`.
        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("bash", "*", Action::Allow),
            Rule::new("bash", "rm *", Action::Deny),
        ]));
        let ctx = ToolContext::new(dir.clone()).with_permissions(Arc::new(engine));

        let out = run(json!({ "command": "echo hi && rm victim.txt" }), &ctx).await;
        assert!(out.is_error, "the rm segment must block the command");
        assert!(
            out.model_content().contains("permission denied"),
            "got: {}",
            out.model_content()
        );
        assert!(
            dir.join("victim.txt").exists(),
            "victim.txt must survive a blocked command"
        );
    }

    #[tokio::test]
    async fn benign_string_containing_the_verb_is_not_blocked() {
        let dir = temp_dir("benign");
        let engine = PermissionEngine::new(Ruleset::new(vec![
            Rule::new("bash", "*", Action::Allow),
            Rule::new("bash", "rm *", Action::Deny),
        ]));
        let ctx = ToolContext::new(dir.clone()).with_permissions(Arc::new(engine));

        let out = run(json!({ "command": "echo rm" }), &ctx).await;
        assert!(
            !out.is_error,
            "`echo rm` must not be blocked: {}",
            out.model_content()
        );
        assert!(out.model_content().contains("rm"), "echo printed rm");
    }

    // VAL-TOOLS-041: a foreground command is bounded by a timeout and the child is reaped.
    #[tokio::test]
    async fn foreground_timeout_terminates_and_reaps() {
        let dir = temp_dir("timeout");
        let ctx = ToolContext::new(dir.clone());

        let started = std::time::Instant::now();
        let out = run(json!({ "command": "sleep 600", "timeout": 800 }), &ctx).await;
        let elapsed = started.elapsed();

        assert!(out.is_error, "a timed-out command is an error");
        assert!(
            out.model_content().contains("timed out"),
            "got: {}",
            out.model_content()
        );
        assert!(
            elapsed < Duration::from_secs(20),
            "the call returned promptly (no indefinite hang), took {elapsed:?}"
        );
    }

    // VAL-TOOLS-034: a background job returns immediately and is tracked + reapable.
    #[tokio::test]
    async fn background_job_returns_immediately_and_is_tracked() {
        let dir = temp_dir("bg");
        let ctx = ToolContext::new(dir.clone());

        let started = std::time::Instant::now();
        let out = run(
            json!({ "command": "sleep 30 && echo done > out.txt", "background": true }),
            &ctx,
        )
        .await;
        let elapsed = started.elapsed();

        assert!(!out.is_error, "got: {}", out.model_content());
        assert!(
            elapsed < Duration::from_secs(5),
            "background returns promptly (did not wait ~30s), took {elapsed:?}"
        );
        assert!(
            out.model_content().contains("background job started"),
            "a job handle is returned: {}",
            out.model_content()
        );

        let handles = ctx.background_jobs().handles();
        assert_eq!(handles.len(), 1, "the job is tracked");
        let pid = handles[0].pid;
        assert!(pid_alive(pid), "the background job is running");

        // Reap and confirm no orphan.
        ctx.background_jobs().shutdown().await;
        for _ in 0..50 {
            if !pid_alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!pid_alive(pid), "pid {pid} reaped on shutdown (no orphan)");
    }

    // VAL-TOOLS-037: large output is truncated with a notice and spilled complete to disk.
    #[tokio::test]
    async fn large_output_is_truncated_and_spilled() {
        let dir = temp_dir("spill");
        let ctx = ToolContext::new(dir.clone());

        let out = run(json!({ "command": "seq 1 200000" }), &ctx).await;
        let text = out.model_content();
        assert!(
            text.contains("output truncated"),
            "truncation notice: bounded result"
        );
        assert!(
            text.lines().count() < 200000,
            "the in-band result is bounded, got {} lines",
            text.lines().count()
        );

        // Extract the spill path from the notice and verify it holds all lines.
        let marker = "full output written to ";
        let start = text.find(marker).expect("a spill pointer") + marker.len();
        let rest = &text[start..];
        let end = rest.find(']').expect("the notice closes with ]");
        let path = &rest[..end];
        let spilled = std::fs::read_to_string(path).expect("read spill file");
        assert_eq!(spilled.lines().count(), 200000, "spill holds every line");
        let _ = std::fs::remove_file(path);
    }
}

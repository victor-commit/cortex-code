//! Direct local shell execution for the TUI's `!` shell mode.
//!
//! Shell mode runs a typed command locally and shows its output in the transcript — it is not an
//! LLM turn and never reaches a provider. Execution mirrors the agent `bash` tool's hardening: the
//! command runs through `bash -c` rooted at the working directory with a secret-stripped child
//! environment (so a command cannot read `OPENROUTER_API_KEY` or any key-shaped value), and its
//! captured output is redaction-scrubbed before being shown. The TUI event loop is synchronous, so
//! this runs the command with blocking [`std::process::Command`] and returns once it completes.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

/// The result of running a shell-mode command: the command itself, its combined output, and whether
/// it exited successfully (a failed command is rendered in a distinct error style).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutcome {
    /// The command that was run (as typed, trimmed).
    pub command: String,
    /// The command's captured stdout/stderr plus an exit-code line, redaction-scrubbed.
    pub output: String,
    /// Whether the command exited with a success status.
    pub success: bool,
}

/// Run `command` locally via `bash -c`, rooted at `cwd` with a secret-stripped environment, and
/// capture its stdout, stderr, and exit code into a [`ShellOutcome`]. A command that fails to start
/// is reported as an unsuccessful outcome rather than panicking.
#[must_use]
pub fn run(command: &str, cwd: &Path) -> ShellOutcome {
    let command = command.trim().to_owned();
    let result = Command::new("bash")
        .arg("-c")
        .arg(&command)
        .current_dir(cwd)
        .env_clear()
        .envs(child_env())
        .output();

    match result {
        Ok(output) => ShellOutcome {
            command: redaction::redact(&command),
            output: render(&output),
            success: output.status.success(),
        },
        Err(err) => ShellOutcome {
            command: redaction::redact(&command),
            output: format!("failed to run command: {err}"),
            success: false,
        },
    }
}

/// Parse `[exit code: N]` from a rendered shell outcome when the process code is not carried
/// separately. Returns `None` for signal-killed children and start failures.
#[must_use]
pub fn exit_status_from_output(output: &str) -> Option<i32> {
    const MARKER: &str = "[exit code: ";
    let start = output.rfind(MARKER)? + MARKER.len();
    let rest = output.get(start..)?;
    let end = rest.find(']')?;
    rest[..end].parse().ok()
}

/// Render a completed command's captured output: redaction-scrubbed stdout, then a labeled stderr
/// section when present, then a trailing exit-code line so the result is legible even when empty.
fn render(output: &std::process::Output) -> String {
    let stdout = redaction::redact(&String::from_utf8_lossy(&output.stdout));
    let stderr = redaction::redact(&String::from_utf8_lossy(&output.stderr));

    let mut sections: Vec<String> = Vec::new();
    let stdout = stdout.trim_end_matches('\n');
    if !stdout.is_empty() {
        sections.push(stdout.to_owned());
    }
    let stderr = stderr.trim_end_matches('\n');
    if !stderr.is_empty() {
        sections.push(format!("[stderr]\n{stderr}"));
    }
    sections.push(match output.status.code() {
        Some(code) => format!("[exit code: {code}]"),
        None => "[exit code: terminated by signal]".to_owned(),
    });
    sections.join("\n")
}

/// The environment passed to the child: the process environment with the agent's secrets stripped,
/// so a shell-mode command cannot read any credential.
fn child_env() -> Vec<(OsString, OsString)> {
    filter_secret_env(std::env::vars_os())
}

/// Strip secrets from an environment: drop any variable whose name is a known credential variable or
/// whose value is API-key-shaped, keeping every other variable (including `PATH`) verbatim. Pure
/// over its input so it is testable without mutating the process environment.
fn filter_secret_env<I>(vars: I) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    vars.into_iter()
        .filter(|(name, value)| {
            !redaction::is_secret_env_var(&name.to_string_lossy())
                && !redaction::value_is_secret(&value.to_string_lossy())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cwd() -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    #[test]
    fn runs_a_command_and_captures_stdout() {
        // VAL-TUI-056: a shell-mode command executes and its stdout is captured.
        let outcome = run("echo cortex-shell-ok", &cwd());
        assert!(outcome.success, "echo exits successfully: {outcome:?}");
        assert!(
            outcome.output.contains("cortex-shell-ok"),
            "stdout captured: {outcome:?}"
        );
    }

    #[test]
    fn reports_a_nonzero_exit_as_failure() {
        let outcome = run("exit 3", &cwd());
        assert!(!outcome.success, "non-zero exit is a failure");
        assert!(
            outcome.output.contains("exit code: 3"),
            "exit code surfaced: {outcome:?}"
        );
    }

    #[test]
    fn captures_stderr_in_a_labeled_section() {
        let outcome = run("echo oops 1>&2", &cwd());
        assert!(
            outcome.output.contains("[stderr]") && outcome.output.contains("oops"),
            "stderr labeled and shown: {outcome:?}"
        );
    }

    #[test]
    fn runs_in_the_given_working_directory() {
        // VAL-TUI-056: relative paths and `pwd` resolve against the working directory.
        let dir = std::env::temp_dir();
        let outcome = run("pwd", &dir);
        let canonical = std::fs::canonicalize(&dir).unwrap_or(dir);
        assert!(
            outcome.output.contains(&*canonical.to_string_lossy()),
            "pwd reflects the cwd ({}): {outcome:?}",
            canonical.display()
        );
    }

    #[test]
    fn strips_secret_env_vars_from_the_child() {
        let vars = vec![
            (OsString::from("PATH"), OsString::from("/usr/bin")),
            (
                OsString::from("OPENROUTER_API_KEY"),
                OsString::from("super-secret"),
            ),
        ];
        let filtered = filter_secret_env(vars);
        let names: Vec<String> = filtered
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"PATH".to_owned()), "PATH kept: {names:?}");
        assert!(
            !names.contains(&"OPENROUTER_API_KEY".to_owned()),
            "the credential is stripped: {names:?}"
        );
    }
}

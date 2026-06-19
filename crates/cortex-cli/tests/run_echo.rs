//! Black-box checks for the headless `cortex run` echo/minimal path (pre-LLM):
//! input-source precedence, deterministic reflection, empty-input handling, and the
//! no-network/no-credential guarantee (VAL-SKEL-013..017, VAL-SKEL-067).

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Generous upper bound: the echo path is local and immediate, so any run that exceeds
/// this is treated as a hang and fails the test loudly instead of blocking the suite.
const RUN_TIMEOUT: Duration = Duration::from_secs(10);

fn cmd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c
}

/// Run the binary with a hard timeout, optionally feeding `stdin_data`.
///
/// stdout/stderr are drained on dedicated threads so a child that produces output can never
/// deadlock against an undrained pipe, and stdin is written on its own thread so a child that
/// ignores stdin (the precedence case) cannot deadlock the writer.
fn run(mut command: Command, stdin_data: Option<&str>) -> Output {
    command.stdin(if stdin_data.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");

    if let Some(data) = stdin_data {
        let mut handle = child.stdin.take().expect("stdin handle");
        let owned = data.to_owned();
        std::thread::spawn(move || {
            let _ = handle.write_all(owned.as_bytes());
        });
    }

    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {RUN_TIMEOUT:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

// VAL-SKEL-013: a positional prompt argument is echoed to stdout, exit 0.
#[test]
fn arg_prompt_is_echoed_to_stdout() {
    let out = run(cmd(&["run", "hello world"]), None);

    assert_eq!(out.status.code(), Some(0), "echo path must exit 0");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "hello world\n",
        "stdout must reflect the prompt argument"
    );
    assert!(
        out.stderr.is_empty(),
        "no diagnostics on a successful echo, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// VAL-SKEL-014: with no argument, the prompt is read from stdin and reflected, exit 0.
#[test]
fn stdin_prompt_is_echoed_to_stdout() {
    let out = run(cmd(&["run"]), Some("ping from stdin\n"));

    assert_eq!(out.status.code(), Some(0), "stdin echo must exit 0");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "ping from stdin\n",
        "stdout must reflect the stdin prompt"
    );
}

// VAL-SKEL-015: no argument and empty stdin terminates deterministically (no hang),
// with the diagnostic on stderr and stdout left clean.
#[test]
fn empty_input_is_deterministic_and_terminates() {
    let out = run(cmd(&["run"]), None);

    assert_eq!(
        out.status.code(),
        Some(3),
        "no input maps to the stable MissingInput exit code"
    );
    assert!(
        out.stdout.is_empty(),
        "stdout must stay clean when there is no input"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_lowercase().contains("no input"),
        "stderr must carry a clear `no input` diagnostic, got: {stderr}"
    );

    let again = run(cmd(&["run"]), None);
    assert_eq!(
        out.status.code(),
        again.status.code(),
        "the empty-input exit code must be identical across runs"
    );
}

// VAL-SKEL-016: the echo path needs no credential and opens no socket, even with the
// network kill-switch engaged.
#[test]
fn echo_path_needs_no_network_or_credential() {
    let mut command = cmd(&["run", "offline"]);
    command.env_remove("OPENROUTER_API_KEY");
    command.env("CORTEX_NO_NET", "1");

    let out = run(command, None);

    assert_eq!(out.status.code(), Some(0), "offline echo must exit 0");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "offline\n",
        "stdout must reflect the prompt with no network"
    );
    assert!(
        out.stderr.is_empty(),
        "no network error must appear, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// VAL-SKEL-017: identical input yields byte-identical stdout across runs.
#[test]
fn identical_input_is_byte_identical_across_runs() {
    let first = run(cmd(&["run", "same deterministic text"]), None);
    let second = run(cmd(&["run", "same deterministic text"]), None);

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(
        first.stdout, second.stdout,
        "the echo data line must be free of nondeterminism (timestamps, etc.)"
    );
}

// VAL-SKEL-067: with BOTH an argument and piped stdin, a single documented source wins
// (the argument), with no blend and no hang, identically across runs.
#[test]
fn arg_wins_over_stdin_deterministically() {
    let first = run(cmd(&["run", "FROM_ARG"]), Some("FROM_STDIN\n"));
    let second = run(cmd(&["run", "FROM_ARG"]), Some("FROM_STDIN\n"));

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        "FROM_ARG\n",
        "the positional argument is the documented winning source"
    );
    assert!(
        !String::from_utf8_lossy(&first.stdout).contains("FROM_STDIN"),
        "stdin must not be blended into the output when an argument is present"
    );
    assert_eq!(
        first.stdout, second.stdout,
        "the resolved source must be identical across repeated runs"
    );
}

//! Black-box checks for strict machine-output formatting and stdout/stderr hygiene of the
//! headless `cortex run` echo path (VAL-SKEL-057..064).
//!
//! These run with piped (non-TTY) stdout/stderr, so they cover the machine-output and
//! non-TTY-no-color contracts directly. The TTY-colour contracts (escapes present only on a
//! real terminal, `NO_COLOR` on a TTY) are exercised as pure-logic unit tests next to the
//! renderer; a PTY validator covers them end-to-end.

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(10);

fn cmd(args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.args(args);
    c
}

/// Run the binary with a hard timeout, draining stdout/stderr on dedicated threads so an
/// undrained pipe can never deadlock the child.
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

fn top_level_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("JSON document must be an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

// VAL-SKEL-057: `--format json` emits a single clean, parseable JSON document on stdout.
#[test]
fn json_format_emits_single_parseable_document() {
    let out = run(cmd(&["run", "--format", "json", "hello world"]), None);

    assert_eq!(out.status.code(), Some(0), "json echo must exit 0");

    let value: Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be a single parseable JSON value");
    assert!(value.is_object(), "JSON output must be an object");
    assert_eq!(
        value.get("output").and_then(Value::as_str),
        Some("hello world"),
        "the JSON payload must reflect the input"
    );

    // Exactly one document: a single line of JSON terminated by exactly one newline.
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    assert!(stdout.ends_with('\n'), "JSON document ends with a newline");
    assert_eq!(
        stdout.trim_end_matches('\n').lines().count(),
        1,
        "exactly one JSON document with no extra lines, got: {stdout:?}"
    );
}

// VAL-SKEL-058: human/log chatter goes to stderr, never stdout, in JSON mode.
#[test]
fn json_mode_keeps_stdout_pure() {
    let out = run(cmd(&["run", "--format", "json", "payload here"]), None);

    assert_eq!(out.status.code(), Some(0));
    // stdout parses 100% as JSON (no interleaved prose) ...
    let _: Value = serde_json::from_slice(&out.stdout).expect("stdout is exclusively JSON");
    // ... and on a clean success there is no chatter anywhere.
    assert!(
        out.stderr.is_empty(),
        "no chatter expected on a clean json run, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// VAL-SKEL-059: `--jsonl` emits one valid JSON object per line; no blank/partial lines.
#[test]
fn jsonl_emits_one_object_per_line() {
    let out = run(cmd(&["run", "--jsonl", "line payload"]), None);

    assert_eq!(out.status.code(), Some(0), "jsonl echo must exit 0");
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");

    let mut non_empty = 0usize;
    for line in stdout.lines() {
        assert!(
            !line.is_empty(),
            "no blank lines may appear in jsonl output"
        );
        let value: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("line is not JSON: {line:?}: {e}"));
        assert!(value.is_object(), "each jsonl line must be a JSON object");
        non_empty += 1;
    }
    assert!(non_empty >= 1, "at least one jsonl event must be emitted");

    // The text event must carry the echoed payload, and the stream must terminate with `done`.
    assert!(
        stdout.contains("\"type\":\"text\""),
        "jsonl must include a text event, got: {stdout}"
    );
    assert!(
        stdout.contains("\"type\":\"done\""),
        "jsonl must terminate with a done event, got: {stdout}"
    );
}

// VAL-SKEL-060: `--format json` top-level schema/keys are stable across repeated echo runs.
#[test]
fn json_keys_are_stable_across_runs() {
    let first = run(cmd(&["run", "--format", "json", "alpha"]), None);
    let second = run(cmd(&["run", "--format", "json", "beta"]), None);

    assert_eq!(first.status.code(), Some(0));
    assert_eq!(second.status.code(), Some(0));

    let a: Value = serde_json::from_slice(&first.stdout).expect("first json");
    let b: Value = serde_json::from_slice(&second.stdout).expect("second json");
    assert_eq!(
        top_level_keys(&a),
        top_level_keys(&b),
        "top-level JSON key set must be identical across echo runs"
    );
}

// VAL-SKEL-061/063: piped (non-TTY) output and all machine output carry zero ANSI escapes.
#[test]
fn piped_output_has_no_ansi_escapes() {
    for args in [
        &["run", "styled text"][..],
        &["run", "--format", "json", "styled text"][..],
        &["run", "--jsonl", "styled text"][..],
    ] {
        let out = run(cmd(args), None);
        assert_eq!(out.status.code(), Some(0), "{args:?} must exit 0");
        assert!(
            !out.stdout.contains(&0x1b),
            "piped output must contain no ESC/ANSI byte for {args:?}, got: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

// VAL-SKEL-063: machine JSON output is never colorized even when a TTY is forced via NO_COLOR
// toggling — the format gate, not just the TTY gate, must suppress colour.
#[test]
fn machine_formats_ignore_color_env() {
    let mut c = cmd(&["run", "--format", "json", "no color here"]);
    c.env_remove("NO_COLOR"); // colour would be permitted by env, but format must still suppress it
    let out = run(c, None);
    assert_eq!(out.status.code(), Some(0));
    assert!(
        !out.stdout.contains(&0x1b),
        "json output must never be colorized, got: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

// VAL-SKEL-064: a default (non-verbose) successful run leaves stdout free of log lines.
#[test]
fn default_run_has_clean_stdout() {
    let out = run(cmd(&["run", "just the payload"]), None);

    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "just the payload\n",
        "stdout must contain only the echoed payload"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    for prefix in ["INFO", "DEBUG", "WARN", "ERROR", "TRACE"] {
        assert!(
            !stdout.contains(prefix),
            "no `{prefix}` log prefix may leak onto stdout, got: {stdout:?}"
        );
    }
}

//! Black-box checks for the typed event bus observed through the CLI's debug/verbose log path
//! (VAL-SKEL-048..051): a published lifecycle event reaches a subscriber, carries its typed
//! named fields end-to-end, is delivered to every subscriber exactly once, and is a clean no-op
//! when no subscriber is attached.

use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The bus path is local and immediate; anything slower than this is treated as a hang.
const RUN_TIMEOUT: Duration = Duration::from_secs(10);

fn cmd(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    // Keep observability hermetic: tests opt in via the `-v` flag or an explicit RUST_LOG,
    // never via an inherited environment.
    command.env_remove("RUST_LOG");
    command.env_remove("CORTEX_VERBOSE");
    command
}

/// Run the binary with a hard timeout, draining stdout/stderr on dedicated threads so a child
/// can never deadlock against an undrained pipe.
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
            panic!("cortex did not terminate within {RUN_TIMEOUT:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

// VAL-SKEL-048: a published event is observable by a subscriber via the verbose log path.
#[test]
fn verbose_run_shows_published_event_received_by_a_subscriber() {
    let out = run(cmd(&["-v", "run", "hi"]), None);

    assert_eq!(out.status.code(), Some(0), "verbose run still exits 0");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "hi\n",
        "the event log must not pollute stdout"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("event delivered to subscriber"),
        "stderr must show the event reaching a subscriber, got: {stderr}"
    );
}

// VAL-SKEL-049: the observed event carries its expected typed named fields/values end-to-end,
// not an opaque/empty record.
#[test]
fn logged_event_carries_typed_named_fields() {
    let out = run(cmd(&["-v", "run", "hi"]), None);
    let stderr = String::from_utf8_lossy(&out.stderr);

    for needle in [
        "lifecycle", // event kind
        "phase",
        "startup",
        "surface",
        "run",
        "version",
        "0.1.0",
    ] {
        assert!(
            stderr.contains(needle),
            "logged event payload must contain `{needle}` (named typed fields), got: {stderr}"
        );
    }
}

// VAL-SKEL-050: with two subscribers attached (the log sink plus a counting sink), a single
// publish is observed exactly once by each, with no loss and no duplicate delivery.
#[test]
fn two_subscribers_each_observe_the_publish_exactly_once() {
    let out = run(cmd(&["-v", "run", "hi"]), None);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);

    // The log sink emits exactly one delivery line for the single published event.
    let deliveries = stderr.matches("event delivered to subscriber").count();
    assert_eq!(
        deliveries, 1,
        "log sink must see exactly one delivery, got {deliveries}:\n{stderr}"
    );

    // The second (counting) subscriber independently reports a tally of exactly one.
    assert!(
        stderr.contains("event-bus subscriber tally"),
        "the second subscriber's tally must be observable, got: {stderr}"
    );
    assert!(
        stderr.contains("observed=1"),
        "the second subscriber must have observed exactly one delivery, got: {stderr}"
    );
}

// VAL-SKEL-051: publishing with no subscribers attached is a clean no-op: no error, no hang,
// exit 0, and nothing leaks onto stderr.
#[test]
fn publish_with_no_subscribers_is_a_clean_noop() {
    let out = run(cmd(&["run", "hi"]), None);

    assert_eq!(
        out.status.code(),
        Some(0),
        "a no-subscriber publish path must complete normally"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "hi\n",
        "stdout carries only the intended payload"
    );
    assert!(
        out.stderr.is_empty(),
        "no subscribers means no event log and a clean stderr, got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// The RUST_LOG path also makes delivery observable (verbose flag is not the only trigger).
#[test]
fn rust_log_env_enables_event_observability() {
    let mut command = cmd(&["run", "hi"]);
    command.env("RUST_LOG", "cortex=debug");
    let out = run(command, None);

    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("event delivered to subscriber"),
        "RUST_LOG must enable the debug event sink, got: {stderr}"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "hi\n",
        "RUST_LOG must not affect stdout"
    );
}

//! VAL-CROSS-027 (live, end-to-end): a saved "always allow" permission decision is durable across a
//! full process restart and honored in a later headless run without re-asking.
//!
//! Two fully separate `cortex run` processes share ONE data dir and project:
//!
//! 1. **Run 1** drives a real OpenRouter turn that runs a `bash echo` command under
//!    `--permission-allow-save 'bash:echo *'`. The action is surfaced as an ask (a
//!    `permission.asked` event is emitted, observable on stderr under `CORTEX_VERBOSE`), auto-approved
//!    as "always allow", the side effect (a file written by the echo) occurs, and the decision is
//!    persisted to the per-project store under the data dir.
//! 2. **Run 2** is a brand-new process on the same data dir + project with NO save flag. It runs a
//!    matching `bash echo` command, which proceeds with NO `permission.asked` event (the saved
//!    decision is honored) and its side effect occurs.
//!
//! This proves the decision survives the process exit and is reused without a re-prompt — the
//! durable, cross-process guarantee. The deterministic mechanics (persist/reload round-trip, no
//! re-ask) are locked offline by the `permission_store` and `permission` unit suites; this file is
//! the live end-to-end confirmation.
//!
//! Gated on `OPENROUTER_API_KEY` (skips cleanly without it). Process hygiene: each child is killed
//! by exact PID if it exceeds the hang-guard timeout; every fixture is hermetic (isolated HOME,
//! checkpoint store, and data dir under a unique scratch dir), so a run never touches the real env.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The mission's default model, which is reliable at tool calls; used for the live turns.
const MODEL: &str = "google/gemini-2.5-flash";

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

/// A hermetic scratch tree shared by both runs: one project working dir, one isolated HOME, one
/// checkpoint store base, and one data dir (where the saved-permission store lives), so the two
/// runs genuinely share a store and project across separate processes.
struct Sandbox {
    proj: PathBuf,
    home: PathBuf,
    ckpt: PathBuf,
    data: PathBuf,
}

/// Build a unique sandbox under the system temp dir.
fn sandbox(tag: &str) -> Sandbox {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "cortex-cross027-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let sb = Sandbox {
        proj: root.join("proj"),
        home: root.join("home"),
        ckpt: root.join("ckpt"),
        data: root.join("data"),
    };
    for dir in [&sb.proj, &sb.home, &sb.ckpt, &sb.data] {
        fs::create_dir_all(dir).expect("create sandbox dir");
    }
    sb
}

/// A `cortex` command rooted at the sandbox project dir, fully isolated and with event observability
/// enabled (`CORTEX_VERBOSE=1`) so `permission.asked`/`permission.replied` events are logged to
/// stderr while stdout stays clean JSONL. HOME / checkpoint base / data dir point at the sandbox, so
/// the DB, shadow store, AND saved-permission store all live under the shared scratch dir.
fn cortex(sb: &Sandbox, key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.current_dir(&sb.proj);
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env("HOME", &sb.home);
    c.env("CORTEX_CHECKPOINT_DIR", &sb.ckpt);
    c.env("CORTEX_DATA_DIR", &sb.data);
    // Enable event observability on stderr (the LogSink), so permission events are inspectable;
    // tracing is written to stderr only, leaving stdout as clean JSONL.
    c.env("CORTEX_VERBOSE", "1");
    c.env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_PERMISSION_DIR")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("NO_COLOR");
    c
}

/// Spawn the binary and collect its output, enforcing `timeout` as a hang guard; the exact child
/// PID is killed if the timeout is exceeded so no orphan is left behind.
fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
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
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {timeout:?} (possible hang)");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout reader"),
        stderr: err_thread.join().expect("join stderr reader"),
    }
}

/// Parse a `--format jsonl` stream into one JSON value per non-empty line.
fn jsonl_values(bytes: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect()
}

/// Whether any event is a SUCCESSFUL `bash` tool result — the echo action really ran (not narrated).
fn successful_bash(events: &[serde_json::Value]) -> bool {
    events.iter().any(|event| {
        event["type"] == "tool_result"
            && event["name"] == "bash"
            && event["is_error"] == serde_json::Value::Bool(false)
    })
}

/// Whether the saved-permission store under the data dir holds at least one store file (proving a
/// decision was persisted).
fn saved_store_present(sb: &Sandbox) -> bool {
    let dir = sb.data.join("permissions");
    fs::read_dir(&dir)
        .map(|entries| entries.filter_map(Result::ok).any(|e| e.path().is_file()))
        .unwrap_or(false)
}

/// VAL-CROSS-027: run 1 saves an "always allow bash echo *" decision (with a `permission.asked`
/// event); run 2 — a fully separate process on the same store + project — performs a matching echo
/// with NO `permission.asked` event, and its side effect occurs. The decision is durable across the
/// restart and honored without re-prompting.
#[test]
fn saved_always_allow_persists_across_restart_and_is_honored_headless() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let sb = sandbox("persist");

    // --- Run 1: a separate process that asks for `bash echo *`, saves "always allow", runs it. ---
    let run1 = run(
        cortex(
            &sb,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--format",
                "jsonl",
                "--permission-allow-save",
                "bash:echo *",
                "Use the bash tool to run exactly this one shell command and nothing else: \
                 echo cross027-run1 > marker1.txt",
            ],
        ),
        Duration::from_secs(120),
    );
    assert!(
        run1.status.success(),
        "run 1 completes (stderr: {})",
        String::from_utf8_lossy(&run1.stderr)
    );

    let run1_events = jsonl_values(&run1.stdout);
    assert!(
        successful_bash(&run1_events),
        "run 1 must really run the echo via the bash tool; events: {}",
        String::from_utf8_lossy(&run1.stdout)
    );
    assert_eq!(
        fs::read_to_string(sb.proj.join("marker1.txt"))
            .expect("run 1 side effect: marker1.txt")
            .trim(),
        "cross027-run1",
        "the echo side effect must occur in run 1"
    );

    // Run 1 surfaced the ask (the decision was genuinely asked, not pre-seeded).
    let run1_err = String::from_utf8_lossy(&run1.stderr);
    assert!(
        run1_err.contains("permission.asked"),
        "run 1 must record a permission.asked event for the gated echo; stderr:\n{run1_err}"
    );

    // The "always allow" decision was persisted to the per-project store under the shared data dir.
    assert!(
        saved_store_present(&sb),
        "run 1 must persist the saved decision to the data dir's permission store"
    );

    // --- Run 2: a brand-new process on the same store/project, NO save flag. ---
    let run2 = run(
        cortex(
            &sb,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--format",
                "jsonl",
                "Use the bash tool to run exactly this one shell command and nothing else: \
                 echo cross027-run2 > marker2.txt",
            ],
        ),
        Duration::from_secs(120),
    );
    assert!(
        run2.status.success(),
        "run 2 completes (stderr: {})",
        String::from_utf8_lossy(&run2.stderr)
    );

    let run2_events = jsonl_values(&run2.stdout);
    assert!(
        successful_bash(&run2_events),
        "run 2 must run the matching echo (its side effect occurs); events: {}",
        String::from_utf8_lossy(&run2.stdout)
    );
    assert_eq!(
        fs::read_to_string(sb.proj.join("marker2.txt"))
            .expect("run 2 side effect: marker2.txt")
            .trim(),
        "cross027-run2",
        "the echo side effect must occur in run 2"
    );

    // The crux: run 2 honored the SAVED decision with NO new ask — the saved decision is durable
    // across the restart and not re-prompted.
    let run2_err = String::from_utf8_lossy(&run2.stderr);
    assert!(
        !run2_err.contains("permission.asked"),
        "run 2 must honor the saved decision WITHOUT re-asking (no permission.asked); stderr:\n{run2_err}"
    );
}

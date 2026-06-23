//! Live black-box proof that imported sessions are first-class and continuable.
//!
//! Covers VAL-FINISH-028 with a real OpenRouter follow-up turn on an imported session.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

const TURN_TIMEOUT: Duration = Duration::from_secs(120);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let unique = format!(
        "cortex-session-import-live-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let dir = std::env::temp_dir().join(unique);
    fs::create_dir_all(&dir).expect("create scratch directory");
    dir
}

fn cortex(key: &str, home: &PathBuf, db: &PathBuf, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("CORTEX_DB_PATH", db)
        .env("OPENROUTER_API_KEY", key)
        .env_remove("CORTEX_NO_NET")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_MODEL")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
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
        if start.elapsed() > TURN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command did not terminate within {TURN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn parse_json(output: Output, context: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{context} must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("strict JSON")
}

#[test]
fn imported_session_is_listable_readable_and_continuable_live() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };

    let source_home = scratch_dir("source");
    let source_db = source_home.join("cortex.db");
    let target_home = scratch_dir("target");
    let target_db = target_home.join("cortex.db");

    let session = parse_json(
        run(cortex(
            &key,
            &source_home,
            &source_db,
            &["session", "new", "--format", "json"],
        )),
        "source session new",
    );
    let session_id = session["id"].as_str().expect("session id").to_owned();

    let marker = "Remember the word: marmalade. Acknowledge.";
    let _ = parse_json(
        run(cortex(
            &key,
            &source_home,
            &source_db,
            &[
                "session",
                "message",
                &session_id,
                marker,
                "--role",
                "user",
                "--format",
                "json",
            ],
        )),
        "source session message",
    );

    let exported = run(cortex(
        &key,
        &source_home,
        &source_db,
        &["export", &session_id],
    ));
    assert_eq!(
        exported.status.code(),
        Some(0),
        "export must succeed; stderr: {}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let artifact = target_home.join("artifact.json");
    fs::write(&artifact, &exported.stdout).expect("write artifact");

    let imported = run(cortex(
        &key,
        &target_home,
        &target_db,
        &["import", artifact.to_str().expect("utf8 artifact path")],
    ));
    assert_eq!(
        imported.status.code(),
        Some(0),
        "import must succeed; stderr: {}",
        String::from_utf8_lossy(&imported.stderr)
    );

    let listed = parse_json(
        run(cortex(
            &key,
            &target_home,
            &target_db,
            &["session", "list", "--format", "json"],
        )),
        "target session list",
    );
    let listed_ids: Vec<&str> = listed["sessions"]
        .as_array()
        .expect("sessions array")
        .iter()
        .map(|session| session["id"].as_str().expect("session id"))
        .collect();
    assert!(
        listed_ids.contains(&session_id.as_str()),
        "imported session should be listed"
    );

    let shown = parse_json(
        run(cortex(
            &key,
            &target_home,
            &target_db,
            &["session", "show", &session_id, "--format", "json"],
        )),
        "target session show",
    );
    let has_marker = shown["messages"]
        .as_array()
        .expect("messages array")
        .iter()
        .any(|message| message["content"] == marker);
    assert!(has_marker, "imported marker content should be readable");

    let continued = parse_json(
        run(cortex(
            &key,
            &target_home,
            &target_db,
            &[
                "run",
                "--session",
                &session_id,
                "--format",
                "json",
                "What word did I ask you to remember? Reply with only the word.",
            ],
        )),
        "continue imported session",
    );
    let answer = continued["content"]
        .as_str()
        .expect("assistant content")
        .to_ascii_lowercase();
    assert!(
        answer.contains("marmalade"),
        "continued turn should recall imported context, got: {answer}"
    );
}

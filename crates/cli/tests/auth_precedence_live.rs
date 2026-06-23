//! Live black-box coverage for auth precedence and secret hygiene (M7 finishing).
//!
//! These tests are gated on `OPENROUTER_API_KEY` and skip cleanly when it is absent, so offline
//! `cargo test --workspace` runs stay green. Mission validation runs them with the key exported:
//! `set -a; . /projects/cortex-code/.env.local; set +a`.
//!
//! Coverage:
//! - VAL-FINISH-014: env key precedence over an invalid stored key.
//! - VAL-FINISH-015: `auth login/list/logout` never echo the raw key.
//! - VAL-FINISH-016: a valid stored credential alone authenticates a real run.
//! - VAL-FINISH-017: verbose/debug output never leaks the key and emits redaction placeholders.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const RUN_TIMEOUT: Duration = Duration::from_secs(90);
static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cortex-auth-precedence-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("project");
        let data_dir = root.join("data");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&project).expect("create project dir");
        std::fs::create_dir_all(root.join(".config")).expect("create config dir");
        std::fs::create_dir_all(root.join(".local/share")).expect("create xdg data dir");
        Self {
            home: root,
            project,
            data_dir,
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn invalid_key() -> String {
    format!("sk-{}-{}", "or-v1", "deadbeef".repeat(8))
}

fn nonce(tag: &str) -> String {
    format!(
        "auth-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    )
}

fn cortex(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("XDG_CONFIG_HOME", sandbox.home.join(".config"));
    command.env("XDG_DATA_HOME", sandbox.home.join(".local/share"));
    command.env("CORTEX_DATA_DIR", &sandbox.data_dir);
    command
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn run(mut command: Command, stdin_text: Option<&str>) -> Output {
    command.stdin(if stdin_text.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    if let Some(stdin_text) = stdin_text {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin
            .write_all(stdin_text.as_bytes())
            .expect("write stdin payload");
        stdin.flush().expect("flush stdin payload");
    }

    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command did not terminate within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_no_key(output: &Output, key: &str) {
    assert!(
        !combined_output(output).contains(key),
        "raw API key must never appear in output"
    );
}

fn assert_no_backtrace(output: &Output) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "must not panic or print a backtrace"
    );
}

#[test]
fn auth_subcommands_never_echo_key_and_verbose_login_masks_it() {
    let Some(key) = live_key() else {
        eprintln!("skipping live auth precedence test: OPENROUTER_API_KEY not set");
        return;
    };
    let sandbox = Sandbox::new("auth-subcommands");

    let login = run(
        cortex(
            &sandbox,
            &["-vv", "auth", "login", "openrouter", "--api-key", &key],
        ),
        None,
    );
    assert_eq!(login.status.code(), Some(0), "auth login must succeed");
    assert_no_key(&login, &key);
    assert!(
        String::from_utf8_lossy(&login.stderr).contains(redaction::PLACEHOLDER),
        "verbose auth login should show a redaction placeholder"
    );

    let list = run(
        cortex(&sandbox, &["-vv", "auth", "list", "--format", "json"]),
        None,
    );
    assert_eq!(list.status.code(), Some(0), "auth list must succeed");
    assert_no_key(&list, &key);

    let logout = run(
        cortex(
            &sandbox,
            &["-vv", "auth", "logout", "openrouter", "--format", "json"],
        ),
        None,
    );
    assert_eq!(logout.status.code(), Some(0), "auth logout must succeed");
    assert_no_key(&logout, &key);
}

#[test]
fn env_key_overrides_invalid_stored_credential_for_live_run() {
    let Some(real_key) = live_key() else {
        eprintln!("skipping live auth precedence test: OPENROUTER_API_KEY not set");
        return;
    };
    let sandbox = Sandbox::new("env-over-file");
    let bad_key = invalid_key();

    let login_bad = run(
        cortex(
            &sandbox,
            &[
                "auth",
                "login",
                "openrouter",
                "--api-key",
                &bad_key,
                "--format",
                "json",
            ],
        ),
        None,
    );
    assert_eq!(
        login_bad.status.code(),
        Some(0),
        "storing an invalid test key should still succeed"
    );

    let check_nonce = nonce("env-wins");
    let prompt = format!("Reply with exactly this token and nothing else: {check_nonce}");
    let mut with_env = cortex(&sandbox, &["run", &prompt]);
    with_env.env("OPENROUTER_API_KEY", &real_key);
    let success = run(with_env, None);
    assert_eq!(
        success.status.code(),
        Some(0),
        "a valid env key must override an invalid stored key"
    );
    assert!(
        String::from_utf8_lossy(&success.stdout).contains(&check_nonce),
        "successful real run should include the requested token"
    );
    assert_no_key(&success, &real_key);
    assert_no_key(&success, &bad_key);

    let without_env = run(
        cortex(&sandbox, &["run", "Say hi in one short sentence."]),
        None,
    );
    assert_ne!(
        without_env.status.code(),
        Some(0),
        "without env key, the invalid stored key should fail auth"
    );
    assert!(
        without_env.stdout.is_empty(),
        "auth failure should not emit an assistant answer on stdout"
    );
    let lower = String::from_utf8_lossy(&without_env.stderr).to_lowercase();
    assert!(
        lower.contains("upstream") || lower.contains("unauthorized") || lower.contains("auth"),
        "stderr should report an auth/upstream failure"
    );
    assert!(
        !lower.contains("missing credential"),
        "failure should come from the stored invalid key, not from missing credentials"
    );
    assert_no_key(&without_env, &real_key);
    assert_no_key(&without_env, &bad_key);
    assert_no_backtrace(&without_env);
}

#[test]
fn stored_valid_credential_alone_authenticates_real_run() {
    let Some(real_key) = live_key() else {
        eprintln!("skipping live auth precedence test: OPENROUTER_API_KEY not set");
        return;
    };
    let sandbox = Sandbox::new("stored-only");

    let login = run(
        cortex(
            &sandbox,
            &[
                "auth",
                "login",
                "openrouter",
                "--api-key",
                &real_key,
                "--format",
                "json",
            ],
        ),
        None,
    );
    assert_eq!(login.status.code(), Some(0), "auth login must succeed");
    assert_no_key(&login, &real_key);

    let check_nonce = nonce("stored-only");
    let prompt = format!("Reply with exactly this token and nothing else: {check_nonce}");
    let success = run(cortex(&sandbox, &["run", &prompt]), None);
    assert_eq!(
        success.status.code(),
        Some(0),
        "stored credential should authenticate a real run when env key is unset"
    );
    assert!(
        String::from_utf8_lossy(&success.stdout).contains(&check_nonce),
        "stored-key run should include the requested token"
    );
    assert_no_key(&success, &real_key);
}

#[test]
fn verbose_run_logs_never_leak_the_api_key() {
    let Some(real_key) = live_key() else {
        eprintln!("skipping live auth precedence test: OPENROUTER_API_KEY not set");
        return;
    };
    let sandbox = Sandbox::new("verbose-run");
    let mut command = cortex(&sandbox, &["-vv", "run", "Say hi in one short sentence."]);
    command.env("OPENROUTER_API_KEY", &real_key);

    let out = run(command, None);
    assert_eq!(out.status.code(), Some(0), "verbose run should succeed");
    assert_no_key(&out, &real_key);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("event delivered to subscriber"),
        "verbose/debug run should emit event log lines on stderr"
    );
}

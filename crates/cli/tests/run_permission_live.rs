//! Black-box permission enforcement over real `cortex run` turns (VAL-PERM-012/013/015/016) plus a
//! non-live fast-fail check for a malformed permission flag.
//!
//! The live tests drive a real OpenRouter turn (default model `google/gemini-2.5-flash`) that is
//! instructed to use a tool whose action is denied by `--permission-deny`, then assert the deny
//! produced ZERO side effects (the contract evidence): a denied write/edit leaves the target file
//! byte-for-byte unchanged, a denied bash command creates no file, a denied read never leaks the
//! protected bytes, and a compound bash command with a denied segment is refused atomically (the
//! allowed segment does not run either). These tests are gated on `OPENROUTER_API_KEY` and skip
//! cleanly when it is absent, so the offline `cargo test --workspace` gate stays green; mission
//! validation runs them with the key exported
//! (`set -a; . /projects/cortex-code/.env.local; set +a`).
//!
//! The deterministic mechanism (model attempts the tool → engine denies → no effect) is proven
//! offline by the `tooling` unit tests; these live tests confirm the CLI wiring end-to-end.
//!
//! Process hygiene: each `cortex` child is killed by exact PID if it exceeds the hang-guard timeout;
//! every fixture uses a unique scratch directory under the system temp dir.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The default model the mission targets (good at tool calls); used for the live turns.
const MODEL: &str = "google/gemini-2.5-flash";
const FIXTURE_PORT_START: u16 = 3240;
const FIXTURE_PORT_END: u16 = 3279;
const READ_RECOVERY_MAX_ATTEMPTS: usize = 3;
const COMPOUND_RECOVERY_MAX_ATTEMPTS: usize = 3;
const LIVE_RETRY_BACKOFF: Duration = Duration::from_millis(300);
static LIVE_TEST_LOCK: Mutex<()> = Mutex::new(());
static NEXT_FIXTURE_PORT: AtomicU16 = AtomicU16::new(FIXTURE_PORT_START);

/// Resolve the live OpenRouter key, or `None` to skip the test cleanly when it is unset/empty.
fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn live_key_and_lock() -> Option<(String, std::sync::MutexGuard<'static, ()>)> {
    let key = live_key()?;
    let guard = match LIVE_TEST_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    Some((key, guard))
}

/// A unique scratch working directory for a fixture (relative tool paths resolve against it).
fn scratch_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "cortex-perm-live-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A `cortex` command rooted at `dir`, carrying the live key and a clean, non-verbose environment so
/// stdout stays pure prose and relative tool paths resolve against `dir`.
fn cortex_in(dir: &Path, key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.current_dir(dir);
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

/// A scratch layered-config tree: an isolated `home` (holding the global config and the per-run DB)
/// and a `proj` working dir under it (holding the project config and the fixture files). `proj` is
/// a child of `home`, so the project-config walk discovers `proj/cortex.toml` and stops at `home`.
fn layered_tree(tag: &str) -> (PathBuf, PathBuf) {
    let home = scratch_dir(tag);
    let proj = home.join("proj");
    fs::create_dir_all(&proj).expect("create project working dir");
    (home, proj)
}

/// Write the global config (`$HOME/.config/cortex/config.toml`) for a layered fixture.
fn write_global_config(home: &Path, contents: &str) {
    let path = home.join(".config").join("cortex").join("config.toml");
    fs::create_dir_all(path.parent().expect("config parent")).expect("create global config dir");
    fs::write(path, contents).expect("write global config");
}

/// A `cortex` command rooted at the project working dir with `home` as its HOME, so the global
/// config under `home/.config/cortex/config.toml` is discovered and the per-run DB lives in the
/// isolated home (never touching the real environment). Other config/data env is cleared so the
/// fixture is hermetic.
fn cortex_layered(home: &Path, proj: &Path, key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.current_dir(proj);
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env("HOME", home);
    c.env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

/// A config-file `[[permission]]` rule for `bash rm *` with the given action.
fn rm_rule(action: &str) -> String {
    format!("[[permission]]\ntool = \"bash\"\npattern = \"rm *\"\naction = \"{action}\"\n")
}

/// The prompt that drives a single, direct `rm x` bash tool call (no exploration first, which under
/// a narrow ruleset would otherwise be denied and could derail the turn).
const RM_X_PROMPT: &str = "Immediately use the bash tool to run exactly this one command: rm x . Do not read, list, or \
     inspect anything first — just run `rm x` now.";

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

#[derive(Debug, Clone)]
struct ScriptedResponse {
    status_line: &'static str,
    headers: Vec<(String, String)>,
    body: String,
}

impl ScriptedResponse {
    fn success(frames: &[String]) -> Self {
        Self {
            status_line: "HTTP/1.1 200 OK",
            headers: vec![
                ("content-type".to_owned(), "text/event-stream".to_owned()),
                ("cache-control".to_owned(), "no-cache".to_owned()),
                ("connection".to_owned(), "close".to_owned()),
            ],
            body: sse_body(frames),
        }
    }
}

struct Fixture {
    base_url: String,
    handle: thread::JoinHandle<()>,
}

impl Fixture {
    fn start(responses: Vec<ScriptedResponse>) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();
        let responses = Arc::new(Mutex::new(responses));
        let handle = {
            let responses = Arc::clone(&responses);
            thread::spawn(move || {
                let idle_deadline = Instant::now() + Duration::from_secs(45);
                while Instant::now() < idle_deadline {
                    let remaining = responses.lock().expect("responses lock").len();
                    if remaining == 0 {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = read_http_request(&mut stream);
                            let response = responses.lock().expect("responses lock").remove(0);
                            let _ = write_http_response(&mut stream, &response);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            handle,
        }
    }

    fn finish(self) {
        let _ = self.handle.join();
    }
}

fn bind_loopback_listener() -> TcpListener {
    for _ in 0..80 {
        let port = NEXT_FIXTURE_PORT.fetch_add(1, Ordering::SeqCst);
        if port > FIXTURE_PORT_END {
            NEXT_FIXTURE_PORT.store(FIXTURE_PORT_START, Ordering::SeqCst);
            continue;
        }
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no free loopback port in reserved fixture range 3240-3279");
}

fn read_http_request(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let already = buffer.len().saturating_sub(header_end);
    let mut remaining = content_length.saturating_sub(already);
    while remaining > 0 {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        remaining = remaining.saturating_sub(read);
    }
    Ok(())
}

fn write_http_response(stream: &mut TcpStream, response: &ScriptedResponse) -> std::io::Result<()> {
    let mut rendered = String::new();
    rendered.push_str(response.status_line);
    rendered.push_str("\r\n");
    for (name, value) in &response.headers {
        rendered.push_str(name);
        rendered.push_str(": ");
        rendered.push_str(value);
        rendered.push_str("\r\n");
    }
    rendered.push_str(&format!("content-length: {}\r\n\r\n", response.body.len()));
    rendered.push_str(&response.body);
    stream.write_all(rendered.as_bytes())?;
    stream.flush()
}

fn sse_body(frames: &[String]) -> String {
    let mut body = String::new();
    for frame in frames {
        body.push_str("data: ");
        body.push_str(frame);
        body.push_str("\n\n");
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn tool_call_frame(call_id: &str, name: &str, arguments: serde_json::Value) -> String {
    serde_json::json!({
        "id": "permission-fixture",
        "object": "chat.completion.chunk",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments.to_string()
                    }
                }]
            },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn text_delta_frame(text: &str) -> String {
    serde_json::json!({
        "id": "permission-fixture",
        "object": "chat.completion.chunk",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "delta": { "content": text },
            "finish_reason": null
        }]
    })
    .to_string()
}

fn done_frame(reason: &str) -> String {
    serde_json::json!({
        "id": "permission-fixture",
        "object": "chat.completion.chunk",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": reason
        }]
    })
    .to_string()
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).expect("write executable script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable script");
}

fn write_formatter_script(dir: &Path) -> PathBuf {
    let script = dir.join("uppercase-format.sh");
    #[cfg(unix)]
    write_executable(
        &script,
        "#!/usr/bin/env sh\nfile=\"$1\"\ntr '[:lower:]' '[:upper:]' < \"$file\" > \"$file.tmp\"\nmv \"$file.tmp\" \"$file\"\n",
    );
    script
}

fn write_project_config_with_formatter_and_permissions(
    proj: &Path,
    formatter_script: &Path,
    permission_rules: &str,
) {
    let formatter_script = formatter_script
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let config = format!(
        "[formatter.custom]\ncommand = [\"{formatter_script}\", \"$FILE\"]\nextensions = [\".fmtperm\"]\n\n{permission_rules}\n"
    );
    fs::write(proj.join("cortex.toml"), config).expect("write project config");
}

/// VAL-SKEL/usage: a malformed permission flag fails fast with the stable usage exit code, before
/// any network or credential work, and names the expected `TOOL:GLOB` form on stderr only. Needs no
/// key (the semantic flag-value error precedes credential resolution).
#[test]
fn malformed_permission_flag_fails_fast_with_usage_code() {
    let dir = scratch_dir("malformed");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cortex"));
    cmd.current_dir(&dir);
    cmd.args(["run", "--permission-deny", "not-a-rule", "hello"]);
    cmd.env_remove("OPENROUTER_API_KEY");
    cmd.env_remove("RUST_LOG");
    let out = run(cmd, Duration::from_secs(30));

    assert_eq!(
        out.status.code(),
        Some(2),
        "a malformed permission rule is a usage error"
    );
    assert!(
        out.stdout.is_empty(),
        "stdout stays byte-empty on a usage error"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("TOOL:GLOB"),
        "stderr names the expected rule form, got: {stderr}"
    );
}

/// VAL-PERM-012: a denied `write`/`edit` leaves the target file byte-for-byte unchanged (no partial
/// write, no truncation, no formatter run).
#[test]
fn denied_write_leaves_the_file_byte_for_byte_unchanged() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let dir = scratch_dir("perm012");
    let cfg = dir.join("cfg.txt");
    fs::write(&cfg, "ORIGINAL CONFIG VALUE\nsecond line\n").unwrap();
    let before = fs::read(&cfg).unwrap();

    let out = run(
        cortex_in(
            &dir,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--permission-allow",
                "*:**",
                "--permission-deny",
                "bash:*",
                "--permission-deny",
                "write:cfg.txt",
                "--permission-deny",
                "edit:cfg.txt",
                "Overwrite the file cfg.txt in the current directory so that it contains exactly \
                 the text NEWVALUE and nothing else. Use the write or edit tool to do it now.",
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read(&cfg).unwrap();
    assert_eq!(
        before, after,
        "a denied write/edit must leave cfg.txt byte-for-byte unchanged"
    );
}

/// VAL-PERM-013: a denied `bash` command produces no side effects (the file it would create is
/// absent). `bash` is the only path left open for the creation, so the deny is what keeps it absent.
#[test]
fn denied_bash_command_creates_no_file() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let dir = scratch_dir("perm013");

    let out = run(
        cortex_in(
            &dir,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--permission-allow",
                "*:**",
                "--permission-deny",
                "bash:touch *",
                "--permission-deny",
                "write:sentinel.txt",
                "--permission-deny",
                "edit:sentinel.txt",
                "Create an empty file named sentinel.txt in the current directory by running the \
                 shell command `touch sentinel.txt` with the bash tool. Do it now.",
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !dir.join("sentinel.txt").exists(),
        "a denied bash command must create no file"
    );
}

/// VAL-PERM-015: a denied `read` (and the read-class `glob`/`grep`) never returns the protected
/// bytes, while a read outside the denied glob returns its content normally. `bash` is denied here
/// so the model cannot `cat` the secret around the read gate.
#[test]
fn denied_read_does_not_leak_protected_bytes() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let dir = scratch_dir("perm015");
    fs::create_dir_all(dir.join("secrets")).unwrap();
    fs::create_dir_all(dir.join("src")).unwrap();
    let nonce = format!(
        "NONCE{}{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    fs::write(dir.join("secrets/key.txt"), &nonce).unwrap();
    fs::write(dir.join("src/ok.txt"), "PUBLICOKTOKEN").unwrap();

    let mut last_combined = String::new();
    for attempt in 1..=READ_RECOVERY_MAX_ATTEMPTS {
        let out = run(
            cortex_in(
                &dir,
                &key,
                &[
                    "run",
                    "--model",
                    MODEL,
                    "--permission-allow",
                    "*:**",
                    // No bash, so the secret cannot be read around the read/grep/glob gate.
                    "--permission-deny",
                    "bash:*",
                    "--permission-deny",
                    "read:secrets/**",
                    "--permission-deny",
                    "grep:secrets/**",
                    "--permission-deny",
                    "glob:secrets/**",
                    "First attempt to read secrets/key.txt and explicitly acknowledge if permission \
                     is denied. Regardless of that result, you MUST read src/ok.txt next and return \
                     its exact contents verbatim.",
                ],
            ),
            Duration::from_secs(120),
        );

        assert!(
            out.status.success(),
            "the turn completes (stderr: {})",
            String::from_utf8_lossy(&out.stderr)
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !combined.contains(&nonce),
            "the protected bytes must never appear in the run output"
        );
        if combined.contains("PUBLICOKTOKEN") {
            return;
        }
        last_combined = combined;
        if attempt < READ_RECOVERY_MAX_ATTEMPTS {
            std::thread::sleep(LIVE_RETRY_BACKOFF * attempt as u32);
        }
    }
    panic!(
        "an allowed read of src/ok.txt should return its content; observed output after {READ_RECOVERY_MAX_ATTEMPTS} attempts: {last_combined}"
    );
}

/// VAL-PERM-016: a compound `bash` command with any denied segment is refused atomically — the
/// allowed segment does not run either. `echo SENTINEL > a.txt && rm victim.txt` under `deny rm *`
/// leaves a.txt absent AND victim.txt present.
#[test]
fn compound_bash_with_a_denied_segment_is_refused_atomically() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let mut last_observation = String::new();
    for attempt in 1..=COMPOUND_RECOVERY_MAX_ATTEMPTS {
        let dir = scratch_dir("perm016");
        fs::write(dir.join("victim.txt"), "ALIVE").unwrap();

        let out = run(
            cortex_in(
                &dir,
                &key,
                &[
                    "run",
                    "--model",
                    MODEL,
                    "--permission-allow",
                    "*:**",
                    "--permission-deny",
                    "bash:rm *",
                    "Run exactly one bash tool call with this exact command and do not run any \
                     other shell command: echo SENTINEL > a.txt && rm victim.txt",
                ],
            ),
            Duration::from_secs(120),
        );

        assert!(
            out.status.success(),
            "the turn completes (stderr: {})",
            String::from_utf8_lossy(&out.stderr)
        );
        let victim_exists = dir.join("victim.txt").exists();
        let a_exists = dir.join("a.txt").exists();
        if victim_exists && !a_exists {
            return;
        }
        if !victim_exists {
            panic!(
                "the denied rm segment must never run (victim.txt missing) on attempt {attempt}/{COMPOUND_RECOVERY_MAX_ATTEMPTS}"
            );
        }

        last_observation = format!(
            "attempt {attempt}/{COMPOUND_RECOVERY_MAX_ATTEMPTS}: victim_exists={victim_exists}, a_exists={a_exists}, stdout={}, stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < COMPOUND_RECOVERY_MAX_ATTEMPTS {
            std::thread::sleep(LIVE_RETRY_BACKOFF * attempt as u32);
        }
    }

    panic!(
        "compound bash atomic refusal did not converge after {COMPOUND_RECOVERY_MAX_ATTEMPTS} attempts: {last_observation}"
    );
}

/// VAL-PERM-017: config-file permission layers gate a real `cortex run`. A global config `allow
/// bash rm *` with a project config (in the run's working dir) `deny bash rm *` blocks `rm x`, so
/// the project layer wins over the global layer and `x` is preserved on disk.
#[test]
fn config_project_deny_overrides_global_allow_blocking_rm() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = layered_tree("perm017-deny");
    write_global_config(&home, &rm_rule("allow"));
    fs::write(proj.join("cortex.toml"), rm_rule("deny")).unwrap();
    fs::write(proj.join("x"), "PRESERVE ME\n").unwrap();

    let out = run(
        cortex_layered(&home, &proj, &key, &["run", "--model", MODEL, RM_X_PROMPT]),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        proj.join("x").exists(),
        "the project `deny bash rm *` must win over the global allow, preserving x on disk"
    );
}

/// VAL-PERM-017: removing the project `deny` rule (leaving only the global `allow bash rm *`) lets
/// the same `cortex run` turn delete `x` — demonstrating deterministic cross-layer combination
/// (the global allow governs once nothing higher overrides it).
#[test]
fn config_global_allow_alone_permits_rm() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = layered_tree("perm017-allow");
    write_global_config(&home, &rm_rule("allow"));
    fs::write(proj.join("x"), "DELETE ME\n").unwrap();

    let out = run(
        cortex_layered(&home, &proj, &key, &["run", "--model", MODEL, RM_X_PROMPT]),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !proj.join("x").exists(),
        "with only the global allow (no project override) the rm runs and x is removed"
    );
}

/// VAL-PERM-017: a CLI `--permission-deny 'bash:rm *'` is appended ON TOP of the config layers, so
/// it overrides a config-file `allow bash rm *` (the CLI layer wins last). `x` is preserved.
#[test]
fn cli_deny_overrides_a_config_allow() {
    let Some((key, _guard)) = live_key_and_lock() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = layered_tree("perm017-cli");
    write_global_config(&home, &rm_rule("allow"));
    fs::write(proj.join("x"), "PRESERVE ME\n").unwrap();

    let out = run(
        cortex_layered(
            &home,
            &proj,
            &key,
            &[
                "run",
                "--model",
                MODEL,
                "--permission-deny",
                "bash:rm *",
                RM_X_PROMPT,
            ],
        ),
        Duration::from_secs(120),
    );

    assert!(
        out.status.success(),
        "the turn completes (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        proj.join("x").exists(),
        "the CLI deny is appended last and overrides the config-file allow, preserving x"
    );
}

/// VAL-GAP-PERM-003 deterministic formatter branch:
/// a custom formatter is configured for `.fmtperm`, but an `edit` capability deny with a
/// write-specific allow keeps the initial write successful while blocking the formatter-induced
/// mutation. This exercises the real post-mutation hook path without depending on live model
/// tool-call behavior or rustfmt availability.
#[cfg(unix)]
#[test]
fn formatter_induced_mutation_is_blocked_when_edit_capability_is_denied() {
    let (home, proj) = layered_tree("perm003-format-deny");
    let formatter = write_formatter_script(&home);
    write_project_config_with_formatter_and_permissions(
        &proj,
        &formatter,
        "[[permission]]\n\
tool = \"edit\"\n\
pattern = \"fixture.fmtperm\"\n\
action = \"deny\"\n\
\n\
[[permission]]\n\
tool = \"write\"\n\
pattern = \"fixture.fmtperm\"\n\
action = \"allow\"",
    );

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_write",
                "write",
                serde_json::json!({
                    "path": "fixture.fmtperm",
                    "content": "lowercase content\n"
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("done"), done_frame("stop")]),
    ]);

    let mut command = cortex_layered(
        &home,
        &proj,
        "test-openrouter-key",
        &["run", "--model", MODEL, "run"],
    );
    command.env("CORTEX_OPENROUTER_BASE_URL", &fixture.base_url);
    let out = run(command, Duration::from_secs(120));
    fixture.finish();

    assert!(
        out.status.success(),
        "run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(proj.join("fixture.fmtperm")).expect("read fixture file"),
        "lowercase content\n",
        "write should be allowed but formatter-induced mutation should be blocked by edit deny"
    );
}

/// VAL-GAP-PERM-003 deterministic formatter branch:
/// with `edit` capability allowed, the same custom formatter mutates the file through the real
/// post-mutation hook path.
#[cfg(unix)]
#[test]
fn formatter_induced_mutation_runs_when_edit_capability_is_allowed() {
    let (home, proj) = layered_tree("perm003-format-allow");
    let formatter = write_formatter_script(&home);
    write_project_config_with_formatter_and_permissions(
        &proj,
        &formatter,
        "[[permission]]\n\
tool = \"edit\"\n\
pattern = \"fixture.fmtperm\"\n\
action = \"allow\"",
    );

    let fixture = Fixture::start(vec![
        ScriptedResponse::success(&[
            tool_call_frame(
                "call_write",
                "write",
                serde_json::json!({
                    "path": "fixture.fmtperm",
                    "content": "lowercase content\n"
                }),
            ),
            done_frame("tool_calls"),
        ]),
        ScriptedResponse::success(&[text_delta_frame("done"), done_frame("stop")]),
    ]);

    let mut command = cortex_layered(
        &home,
        &proj,
        "test-openrouter-key",
        &["run", "--model", MODEL, "run"],
    );
    command.env("CORTEX_OPENROUTER_BASE_URL", &fixture.base_url);
    let out = run(command, Duration::from_secs(120));
    fixture.finish();

    assert!(
        out.status.success(),
        "run should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(proj.join("fixture.fmtperm")).expect("read fixture file"),
        "LOWERCASE CONTENT\n",
        "formatter-induced mutation should apply when edit capability is allowed"
    );
}

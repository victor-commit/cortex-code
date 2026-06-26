use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
const TIMEOUT: Duration = Duration::from_secs(35);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-gap-lsp-crash-status-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn cortex_in(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(cwd);
    command.env("CORTEX_DATA_DIR", cwd.join(".data"));
    command
}

fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            let mut stdout = Vec::new();
            child
                .stdout
                .take()
                .expect("stdout")
                .read_to_end(&mut stdout)
                .expect("read stdout");
            let mut stderr = Vec::new();
            child
                .stderr
                .take()
                .expect("stderr")
                .read_to_end(&mut stderr)
                .expect("read stderr");
            return Output {
                status,
                stdout,
                stderr,
            };
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

fn has_command(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn status_state_for(rows: &serde_json::Value, language: &str) -> String {
    rows.as_array()
        .expect("status array")
        .iter()
        .find(|row| row["language"] == language)
        .and_then(|row| row["state"].as_str())
        .unwrap_or_else(|| panic!("missing language {language} in {rows}"))
        .to_owned()
}

#[test]
fn status_persists_crash_after_managed_failure_and_clears_after_recovery() {
    if !has_command("python3") {
        eprintln!("skipping durable crash status test: python3 is not on PATH");
        return;
    }

    let dir = temp_dir("durable-crash");
    let fixture = dir.join("fixture.fx");
    std::fs::write(&fixture, "hello\n").expect("write fixture");

    let server_script = dir.join("fixture_lsp.py");
    std::fs::write(
        &server_script,
        r#"#!/usr/bin/env python3
import sys
sys.exit(1)
"#,
    )
    .expect("write failing server script");
    make_executable(&server_script);

    std::fs::write(
        dir.join("cortex.toml"),
        format!(
            r#"
[lsp.servers.fixture]
program = "{}"
extensions = ["fx"]
"#,
            server_script.display()
        ),
    )
    .expect("write config");

    let mut initial_status = cortex_in(&dir);
    initial_status.args(["lsp", "status", "--format", "json"]);
    initial_status
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let initial_output = run(initial_status, TIMEOUT);
    assert_eq!(
        initial_output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&initial_output.stderr)
    );
    let initial_rows: serde_json::Value =
        serde_json::from_slice(&initial_output.stdout).expect("initial status json");
    assert_eq!(status_state_for(&initial_rows, "fixture"), "available");

    let mut failing_diagnostics = cortex_in(&dir);
    failing_diagnostics.args(["lsp", "diagnostics", "fixture.fx", "--format", "json"]);
    failing_diagnostics
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let failing_output = run(failing_diagnostics, TIMEOUT);
    assert_eq!(
        failing_output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&failing_output.stderr)
    );
    let failing_doc: serde_json::Value =
        serde_json::from_slice(&failing_output.stdout).expect("failing diagnostics json");
    assert_eq!(
        failing_doc["status"], "unavailable",
        "failing managed startup should report unavailable diagnostics: {failing_doc}"
    );

    let mut crashed_status = cortex_in(&dir);
    crashed_status.args(["lsp", "status", "--format", "json"]);
    crashed_status
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let crashed_output = run(crashed_status, TIMEOUT);
    assert_eq!(
        crashed_output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&crashed_output.stderr)
    );
    let crashed_rows: serde_json::Value =
        serde_json::from_slice(&crashed_output.stdout).expect("crashed status json");
    assert_eq!(
        status_state_for(&crashed_rows, "fixture"),
        "crashed",
        "managed startup failure should persist a durable crashed state: {crashed_rows}"
    );

    std::fs::write(
        &server_script,
        r#"#!/usr/bin/env python3
import json
import sys

def read_message():
    headers = {}
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    while line not in (b"\r\n", b"\n", b""):
        name, value = line.decode("ascii", "replace").split(":", 1)
        headers[name.lower()] = value.strip()
        line = sys.stdin.buffer.readline()
    length = int(headers.get("content-length", "0"))
    if length <= 0:
        return None
    body = sys.stdin.buffer.read(length)
    if not body:
        return None
    return json.loads(body)

def send(payload):
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("ascii"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

while True:
    message = read_message()
    if message is None:
        break
    method = message.get("method")
    if method == "initialize" and "id" in message:
        send({"jsonrpc":"2.0","id":message["id"],"result":{"capabilities":{"textDocumentSync":1}}})
    elif "id" in message:
        send({"jsonrpc":"2.0","id":message["id"],"result":None})
"#,
    )
    .expect("write healthy server script");
    make_executable(&server_script);

    let mut recovering_diagnostics = cortex_in(&dir);
    recovering_diagnostics.args(["lsp", "diagnostics", "fixture.fx", "--format", "json"]);
    recovering_diagnostics
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let recovering_output = run(recovering_diagnostics, TIMEOUT);
    assert_eq!(
        recovering_output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&recovering_output.stderr)
    );
    let recovering_doc: serde_json::Value =
        serde_json::from_slice(&recovering_output.stdout).expect("recovery diagnostics json");
    assert_eq!(
        recovering_doc["status"], "clean",
        "successful managed restart should recover diagnostics collection: {recovering_doc}"
    );

    let mut recovered_status = cortex_in(&dir);
    recovered_status.args(["lsp", "status", "--format", "json"]);
    recovered_status
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let recovered_output = run(recovered_status, TIMEOUT);
    assert_eq!(
        recovered_output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&recovered_output.stderr)
    );
    let recovered_rows: serde_json::Value =
        serde_json::from_slice(&recovered_output.stdout).expect("recovered status json");
    assert_eq!(
        status_state_for(&recovered_rows, "fixture"),
        "available",
        "successful restart should clear stale crashed state for an idle installed executable: {recovered_rows}"
    );
}

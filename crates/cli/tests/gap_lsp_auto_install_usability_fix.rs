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
        "cortex-gap-lsp-usability-{tag}-{}-{unique}",
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

fn command_path(program: &str) -> Option<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program}"))
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[test]
fn status_reports_unavailable_when_go_installer_prerequisite_is_missing() {
    let dir = temp_dir("missing-go-prereq");
    std::fs::write(
        dir.join("cortex.toml"),
        r#"
[lsp.servers.go]
program = "gopls-usability-missing-prereq"
extensions = ["go"]
"#,
    )
    .expect("write config");

    let fake_bin = dir.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("create fake bin");

    let mut command = cortex_in(&dir);
    command.args(["lsp", "status", "--format", "json"]);
    command
        .env("PATH", fake_bin.to_string_lossy().to_string())
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let output = run(command, TIMEOUT);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status json");
    let go = rows
        .as_array()
        .expect("status array")
        .iter()
        .find(|row| row["language"] == "go")
        .expect("go row");
    assert_eq!(
        go["state"], "unavailable",
        "missing go installer prerequisite must not be reported as installing: {rows}"
    );
    let install = go["install"].as_str().expect("install hint");
    assert!(
        install.contains("go install")
            && install.contains("requires `go` on PATH")
            && install.contains("golang.org/x/tools/gopls"),
        "install hint must include actionable prerequisite guidance: {install}"
    );
}

#[test]
fn auto_install_go_server_via_gobin_converges_to_diagnostics_and_available_status() {
    if !has_command("python3") {
        eprintln!("skipping gopls GOBIN regression: python3 is not on PATH");
        return;
    }
    let Some(python3_path) = command_path("python3") else {
        eprintln!("skipping gopls GOBIN regression: unable to resolve python3 path");
        return;
    };

    let dir = temp_dir("go-gobin-discovery");
    std::fs::write(
        dir.join("cortex.toml"),
        r#"
[lsp.servers.go]
program = "gopls-gobin-fixture"
extensions = ["go"]
"#,
    )
    .expect("write config");
    std::fs::write(
        dir.join("bad.go"),
        "package main\nfunc main() {\n    BROKEN\n}\n",
    )
    .expect("write go fixture");

    let fake_bin = dir.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("create fake bin");
    let fake_gobin = dir.join("fake-gobin");
    std::fs::create_dir_all(&fake_gobin).expect("create fake gobin");

    let lsp_script = dir.join("fake_gopls.py");
    std::fs::write(
        &lsp_script,
        r#"#!/usr/bin/env python3
import json
import sys

def read_message():
    headers = {}
    line = sys.stdin.buffer.readline()
    if not line:
        return None
    while line not in (b"\r\n", b"\n", b""):
        name, value = line.decode("ascii").split(":", 1)
        headers[name.lower()] = value.strip()
        line = sys.stdin.buffer.readline()
    length = int(headers.get("content-length", "0"))
    if length <= 0:
        return None
    return json.loads(sys.stdin.buffer.read(length))

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
    elif method in ("textDocument/didOpen", "textDocument/didChange"):
        params = message.get("params", {})
        doc = params.get("textDocument", {})
        if method == "textDocument/didOpen":
            text = doc.get("text", "")
        else:
            changes = params.get("contentChanges", [])
            text = changes[0].get("text", "") if changes else ""
        diagnostics = []
        if "BROKEN" in text:
            diagnostics = [{
                "range": {"start": {"line": 2, "character": 4}, "end": {"line": 2, "character": 10}},
                "severity": 1,
                "source": "gopls-fixture",
                "code": "GO_FIXTURE",
                "message": "fake gopls parse error"
            }]
        send({
            "jsonrpc":"2.0",
            "method":"textDocument/publishDiagnostics",
            "params":{"uri": doc.get("uri"), "version": doc.get("version", 1), "diagnostics": diagnostics}
        })
    elif "id" in message:
        send({"jsonrpc":"2.0","id":message["id"],"result":None})
"#,
    )
    .expect("write fake gopls script");
    make_executable(&lsp_script);

    let fake_go = fake_bin.join("go");
    std::fs::write(
        &fake_go,
        r#"#!/bin/sh
if [ "${1:-}" = "env" ] && [ "${2:-}" = "GOBIN" ]; then
  printf '%s\n' "$FAKE_GOBIN"
  exit 0
fi
if [ "${1:-}" = "env" ] && [ "${2:-}" = "GOPATH" ]; then
  printf '%s\n' "$FAKE_GOPATH"
  exit 0
fi
if [ "${1:-}" = "install" ] && [ "${2:-}" = "golang.org/x/tools/gopls@latest" ]; then
  printf '%s\n' '#!/bin/sh' "exec \"$PYTHON3_PATH\" \"$FAKE_GOPLS_SCRIPT\" \"\$@\"" > "$FAKE_GOBIN/$TARGET_PROGRAM"
  /bin/chmod +x "$FAKE_GOBIN/$TARGET_PROGRAM"
  printf 'server=%s source=go target=golang.org/x/tools/gopls version=latest\n' "$TARGET_PROGRAM" > "$INSTALL_MARKER"
  exit 0
fi
exit 1
"#,
    )
    .expect("write fake go");
    make_executable(&fake_go);

    let install_marker = dir.join("install-marker.txt");
    let mut diagnostics = cortex_in(&dir);
    diagnostics.args(["lsp", "diagnostics", "bad.go", "--format", "json"]);
    diagnostics
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", fake_bin.to_string_lossy()),
        )
        .env("FAKE_GOBIN", &fake_gobin)
        .env("FAKE_GOPATH", dir.join("fake-gopath"))
        .env("FAKE_GOPLS_SCRIPT", &lsp_script)
        .env("TARGET_PROGRAM", "gopls-gobin-fixture")
        .env("INSTALL_MARKER", &install_marker)
        .env("PYTHON3_PATH", &python3_path)
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let diagnostics_output = run(diagnostics, TIMEOUT);
    assert_eq!(
        diagnostics_output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&diagnostics_output.stderr)
    );
    let stderr = String::from_utf8_lossy(&diagnostics_output.stderr);
    assert!(
        stderr.contains("lsp auto-install: server=gopls-gobin-fixture")
            && stderr.contains("source=go")
            && stderr.contains("target=golang.org/x/tools/gopls")
            && stderr.contains("version=latest"),
        "auto-install attempt should report server/source/target/version: {stderr}"
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&diagnostics_output.stdout).expect("diagnostics json");
    assert_eq!(doc["status"], "diagnostics");
    assert!(
        serde_json::to_string(&doc["diagnostics"])
            .expect("diagnostics text")
            .contains("fake gopls parse error"),
        "gopls diagnostics must be available after auto-install: {doc}"
    );
    assert!(
        fake_gobin.join("gopls-gobin-fixture").exists(),
        "auto-install must place the server binary in GOBIN"
    );
    assert!(
        install_marker.exists(),
        "fake go installer should run and record install metadata"
    );

    let mut status = cortex_in(&dir);
    status.args(["lsp", "status", "--format", "json"]);
    status
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", fake_bin.to_string_lossy()),
        )
        .env("FAKE_GOBIN", &fake_gobin)
        .env("FAKE_GOPATH", dir.join("fake-gopath"))
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let status_output = run(status, TIMEOUT);
    assert_eq!(
        status_output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&status_output.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&status_output.stdout).expect("status");
    let go = rows
        .as_array()
        .expect("status array")
        .iter()
        .find(|row| row["language"] == "go")
        .expect("go row");
    assert_eq!(
        go["state"], "available",
        "post-install status should converge away from installing/no_server once binary is discoverable: {rows}"
    );
}

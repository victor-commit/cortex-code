use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
// The first `lsp status`/`diagnostics` invocation runs a cold language-server enumeration that can
// take ~33-38s in slow sandboxes; this is a max-wait bound only, not a promptness assertion.
const TIMEOUT: Duration = Duration::from_secs(60);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-gap-lsp-registry-{tag}-{}-{unique}",
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

fn toml_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[test]
fn status_registry_covers_reference_language_matrix_with_install_guidance() {
    let dir = temp_dir("registry-matrix");
    let mut command = cortex_in(&dir);
    command.args(["lsp", "status", "--format", "json"]);
    command
        .env("CORTEX_NO_NET", "1")
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
    let list = rows.as_array().expect("status array");
    let lookup = |language: &str| {
        list.iter()
            .find(|row| row["language"] == language)
            .unwrap_or_else(|| panic!("missing language {language} in {rows}"))
    };

    for language in [
        "typescript",
        "python",
        "rust",
        "go",
        "cpp",
        "java",
        "zig",
        "nix",
        "terraform",
        "elixir",
        "dart",
        "kotlin",
        "tinymist",
        "csharp",
        "razor",
        "vue",
        "svelte",
        "astro",
    ] {
        let state = lookup(language)["state"].as_str().unwrap_or("");
        assert!(
            matches!(
                state,
                "disabled" | "available" | "installing" | "running" | "unavailable" | "crashed"
            ),
            "unexpected state {state:?} for {language}: {rows}"
        );
    }

    for language in ["go", "terraform", "tinymist", "csharp", "razor"] {
        let install = lookup(language)["install"]
            .as_str()
            .unwrap_or_else(|| panic!("missing install hint for {language} in {rows}"));
        assert!(
            !install.trim().is_empty(),
            "install hint should be non-empty for {language}"
        );
    }
}

#[test]
fn no_net_missing_tinymist_never_runs_auto_installer() {
    let dir = temp_dir("tinymist-no-net");
    std::fs::write(dir.join("bad.typ"), "#let x =\n").expect("write typ fixture");

    let fake_bin = dir.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("create fake bin");
    let marker = dir.join("cargo-invoked");
    let fake_cargo = fake_bin.join("cargo");
    std::fs::write(
        &fake_cargo,
        format!(
            "#!/bin/sh\nprintf invoked > '{}'\nexit 0\n",
            marker.display()
        ),
    )
    .expect("write fake cargo");
    make_executable(&fake_cargo);

    let mut command = cortex_in(&dir);
    command.args(["lsp", "diagnostics", "bad.typ", "--format", "json"]);
    command
        .env("CORTEX_NO_NET", "1")
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
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
    assert!(
        output.stderr.is_empty(),
        "no-net no-server path should keep stderr clean: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("diagnostics json");
    assert_eq!(doc["status"], "no_server");
    assert_eq!(doc["language"], "tinymist");
    assert_eq!(doc["program"], "tinymist");
    let note = doc["note"].as_str().expect("no-server note");
    assert!(note.contains("CORTEX_NO_NET=1"), "{note}");
    assert!(note.contains("tinymist"), "{note}");
    assert!(
        !marker.exists(),
        "CORTEX_NO_NET=1 must refuse auto-install before running cargo"
    );
}

#[test]
fn auto_install_tinymist_with_fake_cargo_makes_server_usable() {
    if !has_command("python3") {
        eprintln!("skipping fake tinymist install proof: python3 is not on PATH");
        return;
    }
    let Some(python3_path) = command_path("python3") else {
        eprintln!("skipping fake tinymist install proof: unable to resolve python3 path");
        return;
    };

    let dir = temp_dir("tinymist-install");
    std::fs::write(dir.join("bad.typ"), "#let x =\n").expect("write typ fixture");

    let fake_bin = dir.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("create fake bin");
    let lsp_script = dir.join("fake_tinymist_lsp.py");
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
        if "x =" in text:
            diagnostics = [{
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 5}},
                "severity": 1,
                "source": "tinymist",
                "code": "E_FAKE_TINYMIST",
                "message": "fake tinymist parse error"
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
    .expect("write fake lsp script");
    make_executable(&lsp_script);

    let install_marker = dir.join("install-log.txt");
    let fake_cargo = fake_bin.join("cargo");
    std::fs::write(
        &fake_cargo,
        r#"#!/bin/sh
if [ "${1:-}" = "install" ] && [ "${2:-}" = "--locked" ] && [ "${3:-}" = "tinymist" ]; then
  printf '%s\n' '#!/bin/sh' "exec \"$PYTHON3_PATH\" \"$FAKE_TINYMIST_SCRIPT\" \"\$@\"" > "$FAKE_BIN/tinymist"
  /bin/chmod +x "$FAKE_BIN/tinymist"
  printf 'server=tinymist source=cargo version=latest target=tinymist\n' > "$INSTALL_MARKER"
  exit 0
fi
exit 1
"#,
    )
    .expect("write fake cargo");
    make_executable(&fake_cargo);

    let mut command = cortex_in(&dir);
    command.args(["lsp", "diagnostics", "bad.typ", "--format", "json"]);
    command
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", fake_bin.to_string_lossy()),
        )
        .env("FAKE_BIN", &fake_bin)
        .env("FAKE_TINYMIST_SCRIPT", &lsp_script)
        .env("INSTALL_MARKER", &install_marker)
        .env("PYTHON3_PATH", &python3_path)
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

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("lsp auto-install: server=tinymist")
            && stderr.contains("source=cargo")
            && stderr.contains("target=tinymist")
            && stderr.contains("version=latest"),
        "install attempt should report server/source/target/version: {stderr}"
    );

    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("diagnostics json");
    assert_eq!(doc["status"], "diagnostics");
    assert_eq!(doc["language"], "tinymist");
    assert_eq!(doc["program"], "tinymist");
    let diagnostics = serde_json::to_string(&doc["diagnostics"]).expect("diagnostics json");
    assert!(
        diagnostics.contains("fake tinymist parse error"),
        "expected diagnostics from the fake installed server: {diagnostics}"
    );
    let marker = std::fs::read_to_string(&install_marker).expect("install marker");
    assert!(
        marker.contains("server=tinymist")
            && marker.contains("source=cargo")
            && marker.contains("version=latest"),
        "fake installer should have run and recorded install metadata: {marker}"
    );
}

#[test]
fn status_distinguishes_disabled_available_installing_running_unavailable_and_crashed() {
    let dir = temp_dir("status-lifecycle");
    let running_bin = PathBuf::from("/bin/sleep");
    let crashed_bin = PathBuf::from("/bin/true");
    let mut running_child = Command::new(&running_bin)
        .arg("30")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn running lsp fixture");

    let unavailable_program = format!("missing-no-plan-{}", std::process::id());
    let installable_program = format!("missing-installable-{}", std::process::id());
    std::fs::write(
        dir.join("cortex.toml"),
        format!(
            r#"
[lsp.servers.python]
disabled = true

[lsp.servers.unavailablefixture]
program = "{unavailable}"
extensions = ["unv"]

[lsp.servers.runningfixture]
program = "{running}"
extensions = ["run"]

[lsp.servers.crashedfixture]
program = "{crashed}"
extensions = ["crs"]

[lsp.servers.tinymist]
program = "{installable}"
extensions = ["typ", "typc"]
"#,
            unavailable = unavailable_program,
            running = toml_string(&running_bin),
            crashed = toml_string(&crashed_bin),
            installable = installable_program
        ),
    )
    .expect("write config");

    let mut command = cortex_in(&dir);
    command.args(["lsp", "status", "--format", "json"]);
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let output = run(command, TIMEOUT);
    let _ = running_child.kill();
    let _ = running_child.wait();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status json");
    let list = rows.as_array().expect("status array");
    let state_for = |language: &str| {
        list.iter()
            .find(|row| row["language"] == language)
            .and_then(|row| row["state"].as_str())
            .unwrap_or_else(|| panic!("missing language {language} in {rows}"))
            .to_owned()
    };
    assert_eq!(state_for("python"), "disabled");
    assert_eq!(state_for("unavailablefixture"), "unavailable");
    assert_eq!(state_for("runningfixture"), "running");
    assert_eq!(state_for("crashedfixture"), "available");
    assert_eq!(state_for("tinymist"), "installing");
}

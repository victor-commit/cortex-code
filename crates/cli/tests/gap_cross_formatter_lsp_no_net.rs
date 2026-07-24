//! Cross-flow hardening proof for VAL-GAP-CROSS-006.
//!
//! Locks the composition of the ecosystem-tooling subsystems for ONE real edit flow:
//!
//! 1. With tools installed, a real `cortex run` write turn (against a local scripted provider)
//!    reformats the file on disk (formatter), re-injects real LSP diagnostics with precise
//!    positions into the tool result, and records the mutation in the audit sink — and the
//!    standalone `cortex lsp diagnostics`, `cortex lsp status`, and `cortex formatter status`
//!    surfaces agree with what the edit flow observed.
//! 2. With `CORTEX_NO_NET=1` and a missing language server, the diagnostics surface refuses to
//!    auto-install (zero installer invocations, zero `connect()` syscalls), reports a clear no-net
//!    refusal, and the CLI `cortex lsp status` and server `GET /lsp` report the SAME status row.
//! 3. A real `cortex run` model turn under `CORTEX_NO_NET=1` fails closed and the refusal is audited
//!    as a single blocked network attempt with no successful network call.
//!
//! Every mechanism here is already implemented by completed precondition features; these tests are
//! durable regression locks proving the subsystems still compose and agree.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const RUN_TIMEOUT: Duration = Duration::from_secs(60);
static NEXT_PROVIDER_PORT: AtomicU16 = AtomicU16::new(18765);
static NEXT_SERVE_PORT: AtomicU16 = AtomicU16::new(18805);

/// The misformatted, error-bearing Python fixture the edit flow writes. `ruff format` collapses the
/// extra spacing (changing the bytes on disk), and the `ruff` language server reports a real unused
/// import (F401) and an undefined name (F821) against the formatted file.
const BAD_PY: &str = "import os\nx    =    1\nprint(undefined_name)\n";

// ----------------------------------------------------------------------------------------------
// Scratch sandbox
// ----------------------------------------------------------------------------------------------

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
    audit: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cortex-cross-fmt-lsp-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create home");
        std::fs::create_dir_all(&project).expect("create project");
        Self {
            db: root.join("data/cortex.db"),
            audit: root.join("audit.jsonl"),
            root,
            home,
            project,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ----------------------------------------------------------------------------------------------
// Local scripted OpenAI-compatible provider (no real network)
// ----------------------------------------------------------------------------------------------

/// A loopback SSE provider that drives a faithful edit flow: it first asks the agent to `read` the
/// pre-existing Python fixture (satisfying the write tool's read-before-overwrite guard), then to
/// `write` the misformatted, error-bearing content, and finally ends the turn with a short message.
struct ScriptedProvider {
    base_url: String,
    thread: Option<thread::JoinHandle<()>>,
}

impl ScriptedProvider {
    fn spawn_python_edit() -> Self {
        let (port, listener) = bind_in_range(&NEXT_PROVIDER_PORT);
        listener
            .set_nonblocking(true)
            .expect("set nonblocking listener");
        let thread = thread::spawn(move || {
            let mut tool_turns_sent = 0_u8;
            let mut sent_final_turn = false;
            for _ in 0..10 {
                let Some(mut stream) = accept_with_idle_timeout(&listener, Duration::from_secs(4))
                else {
                    break;
                };
                let request = read_http_body(&mut stream);
                let request_has_tools = String::from_utf8_lossy(&request).contains("\"tools\"");
                let body = match (request_has_tools, tool_turns_sent) {
                    (true, 0) => {
                        tool_turns_sent = 1;
                        python_read_tool_call_body()
                    }
                    (true, 1) => {
                        tool_turns_sent = 2;
                        python_write_tool_call_body()
                    }
                    _ => {
                        sent_final_turn = true;
                        final_body()
                    }
                };
                write_sse_response(&mut stream, &body).expect("write provider response");
                if tool_turns_sent >= 2 && sent_final_turn {
                    break;
                }
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            thread: Some(thread),
        }
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------------------------

/// Expected behavior #1: a real edit flow formats the file, reports LSP diagnostics with precise
/// positions, audits the mutation, and the standalone status/diagnostics surfaces agree.
#[test]
fn real_edit_flow_formats_reports_diagnostics_and_audits_consistently() {
    if !has_command("ruff") {
        eprintln!("skipping: ruff (formatter + python language server) is not on PATH");
        return;
    }

    let scratch = Scratch::new("edit-flow");
    // The `[tool.ruff]` marker enables the project-scoped ruff formatter; the file is the fixture
    // the model will (re)write through the edit flow.
    std::fs::write(scratch.project.join("pyproject.toml"), "[tool.ruff]\n").expect("write marker");
    std::fs::write(scratch.project.join("bad.py"), "placeholder = 0\n").expect("seed fixture");

    let provider = ScriptedProvider::spawn_python_edit();
    let out = run(cortex_with_provider(
        &scratch,
        &provider,
        &[
            "run",
            "--format",
            "json",
            "--permission-allow",
            "read:*",
            "--permission-allow",
            "write:*",
            "edit the python fixture",
        ],
    ));
    assert_success(&out);

    // (a) Formatted disk bytes: `ruff format` collapsed the spacing the model wrote.
    let on_disk = std::fs::read_to_string(scratch.project.join("bad.py")).expect("read fixture");
    assert!(
        on_disk.contains("x = 1\n") && !on_disk.contains("x    =    1"),
        "formatter must have reformatted the written bytes on disk: {on_disk:?}"
    );

    // (b) LSP diagnostics with positions were re-injected into the write tool result.
    let doc: Value = serde_json::from_slice(&out.stdout).expect("run JSON");
    let session = doc["session"].as_str().expect("session id");
    let write_result = tool_result_text(&doc, "write");
    assert!(
        write_result.contains("Diagnostics:"),
        "the write tool result must carry a diagnostics section: {write_result}"
    );

    // The standalone diagnostics surface, run over the file the edit flow already formatted, is the
    // structured source of truth for the positions; the edit-flow tool result must AGREE with it.
    let diagnostics = lsp_diagnostics_json(&scratch, "bad.py");
    assert_eq!(diagnostics["status"], "diagnostics", "{diagnostics}");
    assert_eq!(diagnostics["program"], "ruff", "{diagnostics}");
    let rows = diagnostics["diagnostics"]
        .as_array()
        .expect("diagnostics array");
    let f821 = find_diag(rows, "F821").expect("F821 undefined name diagnostic");
    let f401 = find_diag(rows, "F401").expect("F401 unused import diagnostic");
    assert!(
        f821["message"]
            .as_str()
            .is_some_and(|m| m.contains("undefined_name")),
        "{f821}"
    );

    for diag in [f821, f401] {
        let expected = rendered_position(diag);
        assert!(
            write_result.contains(&expected),
            "edit-flow tool result must report the same position `{expected}` as `cortex lsp diagnostics`: {write_result}"
        );
        let code = diag["code"].as_str().expect("code");
        assert!(
            write_result.contains(&format!("[{code}]")),
            "edit-flow tool result must name diagnostic code {code}: {write_result}"
        );
    }

    // (c) Audit records agree: exactly one successful file mutation for the edited file.
    let records = read_audit(&scratch.audit);
    assert_eq!(
        count(&records, |r| {
            r["type"] == "file_mutation"
                && r["operation"] == "write"
                && r["outcome"] == "success"
                && r["session"] == session
                && r["path"].as_str().is_some_and(|p| p.ends_with("bad.py"))
        }),
        1,
        "the edit flow should audit exactly one successful write of bad.py: {records:#?}"
    );

    // (d) Status surfaces agree: the formatter is enabled with its marker found, and the python
    // language server that produced the diagnostics is available.
    let formatter_rows = formatter_status_json(&scratch);
    let ruff_fmt = formatter_rows
        .iter()
        .find(|r| r["formatter"] == "ruff")
        .expect("ruff formatter row");
    assert_eq!(ruff_fmt["state"], "enabled", "{ruff_fmt}");
    assert_eq!(ruff_fmt["marker_found"], json!(true), "{ruff_fmt}");

    let lsp_rows = lsp_status_json(&scratch);
    let python = lsp_rows
        .iter()
        .find(|r| r["language"] == "python")
        .expect("python lsp row");
    assert_eq!(python["program"], "ruff", "{python}");
    assert!(
        matches!(python["state"].as_str(), Some("available" | "running")),
        "python language server should be available/running where ruff is installed: {python}"
    );
}

/// Expected behavior #2: under `CORTEX_NO_NET=1` with a missing server, the diagnostics surface
/// refuses to auto-install (no installer call, zero `connect()` syscalls), reports a clear no-net
/// refusal, and the CLI `cortex lsp status` and server `GET /lsp` report the SAME status row.
#[test]
fn no_net_missing_server_refuses_install_with_consistent_cli_and_server_status() {
    let scratch = Scratch::new("no-net-missing");
    std::fs::write(
        scratch.project.join("bad.ts"),
        "const x: number = 'nope';\n",
    )
    .expect("write ts fixture");

    // A fake `npm` that records its own invocation. The auto-installer for the typescript server is
    // `npm install -g ...`; under the kill-switch it must never be invoked.
    let fake_bin = scratch.project.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("create fake bin");
    let installer_marker = scratch.project.join("npm-invoked");
    write_executable(
        &fake_bin.join("npm"),
        &format!(
            "#!/bin/sh\nprintf invoked > '{}'\nexit 0\n",
            installer_marker.display()
        ),
    );
    // Restricted PATH so the real typescript-language-server is unreachable and the only `npm` is
    // the recording fake.
    let restricted_path = format!("{}:/usr/bin:/bin", fake_bin.display());

    // CLI diagnostics: a clear no-net refusal that claims no diagnostics and ran no installer.
    let diagnostics_out = run(cortex_no_net(
        &scratch,
        &restricted_path,
        &["lsp", "diagnostics", "bad.ts", "--format", "json"],
    ));
    assert_eq!(
        diagnostics_out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&diagnostics_out.stderr)
    );
    let diag: Value = serde_json::from_slice(&diagnostics_out.stdout).expect("diagnostics JSON");
    assert_eq!(diag["status"], "no_server", "{diag}");
    assert_eq!(diag["program"], "typescript-language-server", "{diag}");
    assert!(
        diag["diagnostics"].as_array().is_some_and(Vec::is_empty),
        "no diagnostics may be claimed under a no-net refusal: {diag}"
    );
    let note = diag["note"].as_str().expect("no-server note");
    assert!(
        note.contains("CORTEX_NO_NET=1") && note.contains("auto-installing"),
        "the refusal must clearly cite the kill-switch: {note}"
    );
    assert!(
        !installer_marker.exists(),
        "the kill-switch must not invoke the npm auto-installer"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.project.join("bad.ts")).expect("read ts"),
        "const x: number = 'nope';\n",
        "diagnostics must not mutate the fixture"
    );

    // Zero remote connect attempts: strace proves no `connect()` syscall is issued.
    if let Some(connects) = strace_connect_count(
        &scratch,
        &restricted_path,
        &["lsp", "diagnostics", "bad.ts", "--format", "json"],
    ) {
        assert_eq!(
            connects, 0,
            "under CORTEX_NO_NET=1 the diagnostics command must issue zero connect() syscalls"
        );
    } else {
        eprintln!("note: strace unavailable/ptrace-restricted; skipped the connect()-count proof");
    }

    // Consistent CLI/server status: the typescript row from `cortex lsp status` must equal the row
    // from the server `GET /lsp`, both run under the same no-net + restricted-PATH environment.
    let cli_status = lsp_status_json_with(&scratch, Some(&restricted_path));
    let cli_ts = cli_status
        .iter()
        .find(|r| r["language"] == "typescript")
        .expect("cli typescript row")
        .clone();
    assert_eq!(cli_ts["state"], "unavailable", "{cli_ts}");
    assert!(
        cli_ts["install"]
            .as_str()
            .is_some_and(|hint| hint.contains("blocked: CORTEX_NO_NET=1")),
        "the install hint must reflect the kill-switch: {cli_ts}"
    );

    let server_status = serve_lsp_status(&scratch, &restricted_path);
    let server_ts = server_status
        .iter()
        .find(|r| r["language"] == "typescript")
        .expect("server typescript row")
        .clone();
    assert_eq!(
        cli_ts, server_ts,
        "CLI `cortex lsp status` and server `GET /lsp` must report the SAME typescript status row"
    );
}

/// Expected behavior #2 (refusal half): a real model turn under `CORTEX_NO_NET=1` fails closed and
/// the refusal is audited as one blocked network attempt with no successful network call.
#[test]
fn no_net_real_run_refuses_with_audited_blocked_network_call() {
    let scratch = Scratch::new("no-net-run");
    let mut command = cortex_base(&scratch);
    command.args(["run", "--format", "json", "exercise no-net edit refusal"]);
    command.env("OPENROUTER_API_KEY", fake_openrouter_key());
    command.env("CORTEX_NO_NET", "1");
    command.env_remove("CORTEX_OPENROUTER_BASE_URL");

    let out = run(command);
    assert_eq!(
        out.status.code(),
        Some(4),
        "no-net run must fail closed with the net-disabled exit code; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty(), "a no-net refusal keeps stdout clean");

    let records = read_audit(&scratch.audit);
    assert_eq!(
        count(&records, |r| {
            r["type"] == "network_call" && r["actor"] == "llm" && r["outcome"] == "net-disabled"
        }),
        1,
        "the no-net refusal should be one blocked network record: {records:#?}"
    );
    assert_eq!(
        count(&records, |r| {
            r["type"] == "network_call" && r["outcome"] == "success"
        }),
        0,
        "a no-net refusal must not claim any successful network call: {records:#?}"
    );
}

// ----------------------------------------------------------------------------------------------
// CLI command builders
// ----------------------------------------------------------------------------------------------

fn cortex_base(scratch: &Scratch) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&scratch.project);
    command.env("HOME", &scratch.home);
    command.env("CORTEX_DB_PATH", &scratch.db);
    command.env("CORTEX_AUDIT_LOG", &scratch.audit);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn cortex_with_provider(scratch: &Scratch, provider: &ScriptedProvider, args: &[&str]) -> Command {
    let mut command = cortex_base(scratch);
    command.args(args);
    command.env("OPENROUTER_API_KEY", fake_openrouter_key());
    command.env("CORTEX_OPENROUTER_BASE_URL", &provider.base_url);
    command
}

fn cortex_no_net(scratch: &Scratch, path: &str, args: &[&str]) -> Command {
    let mut command = cortex_base(scratch);
    command.args(args);
    command.env("CORTEX_NO_NET", "1");
    command.env("PATH", path);
    command.env_remove("OPENROUTER_API_KEY");
    command
}

// ----------------------------------------------------------------------------------------------
// CLI status / diagnostics helpers
// ----------------------------------------------------------------------------------------------

fn lsp_diagnostics_json(scratch: &Scratch, file: &str) -> Value {
    let mut command = cortex_base(scratch);
    command.args(["lsp", "diagnostics", file, "--format", "json"]);
    command.env("CORTEX_NO_NET", "1");
    let out = run(command);
    assert_eq!(
        out.status.code(),
        Some(0),
        "lsp diagnostics stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("lsp diagnostics JSON")
}

fn lsp_status_json(scratch: &Scratch) -> Vec<Value> {
    lsp_status_json_with(scratch, None)
}

fn lsp_status_json_with(scratch: &Scratch, path: Option<&str>) -> Vec<Value> {
    let mut command = cortex_base(scratch);
    command.args(["lsp", "status", "--format", "json"]);
    command.env("CORTEX_NO_NET", "1");
    if let Some(path) = path {
        command.env("PATH", path);
    }
    let out = run(command);
    assert_eq!(out.status.code(), Some(0));
    serde_json::from_slice(&out.stdout).expect("lsp status JSON")
}

fn formatter_status_json(scratch: &Scratch) -> Vec<Value> {
    let mut command = cortex_base(scratch);
    command.args(["formatter", "status", "--format", "json"]);
    command.env("CORTEX_NO_NET", "1");
    let out = run(command);
    assert_eq!(out.status.code(), Some(0));
    serde_json::from_slice(&out.stdout).expect("formatter status JSON")
}

// ----------------------------------------------------------------------------------------------
// Server (cortex serve) helper
// ----------------------------------------------------------------------------------------------

/// Start `cortex serve` under no-net + the given PATH, fetch `GET /lsp`, and stop it by its
/// lsof-resolved listener PID (never a name-based kill).
fn serve_lsp_status(scratch: &Scratch, path: &str) -> Vec<Value> {
    let port = reserve_serve_port();
    let mut command = cortex_base(scratch);
    command.args([
        "serve",
        "--hostname",
        "127.0.0.1",
        "--port",
        &port.to_string(),
    ]);
    command.env("CORTEX_NO_NET", "1");
    command.env("CORTEX_DATA_DIR", scratch.root.join("serve-data"));
    command.env("PATH", path);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    let mut child = command.spawn().expect("spawn cortex serve");

    let health = format!("http://127.0.0.1:{port}/health");
    let mut ready = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if curl_get(&health).is_some() {
            ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(150));
    }
    assert!(ready, "cortex serve did not become healthy on port {port}");

    let body = curl_get(&format!("http://127.0.0.1:{port}/lsp")).expect("GET /lsp body");
    let rows: Vec<Value> = serde_json::from_str(&body).expect("GET /lsp JSON");

    // Stop the listener by its exact PID, then reap the launched child.
    kill_listener(port);
    let _ = child.wait();
    rows
}

fn kill_listener(port: u16) {
    if let Some(out) = Command::new("lsof")
        .args([
            "-tiTCP:".to_owned() + &port.to_string(),
            "-sTCP:LISTEN".to_owned(),
        ])
        .output()
        .ok()
        .filter(|out| out.status.success())
    {
        for pid in String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .filter_map(|pid| pid.parse::<i32>().ok())
        {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }
}

fn curl_get(url: &str) -> Option<String> {
    let out = Command::new("curl")
        .args(["-sf", url])
        .output()
        .expect("spawn curl");
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// ----------------------------------------------------------------------------------------------
// strace connect-count proof
// ----------------------------------------------------------------------------------------------

/// Run a `cortex` command under `strace -f -e trace=connect` and return the number of `connect()`
/// syscalls. Returns `None` when strace is unavailable or ptrace is restricted.
fn strace_connect_count(scratch: &Scratch, path: &str, args: &[&str]) -> Option<usize> {
    let probe = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "--version",
        ])
        .output()
        .ok()?;
    let probe_stderr = String::from_utf8_lossy(&probe.stderr).to_lowercase();
    if !probe.status.success() || probe_stderr.contains("ptrace") {
        return None;
    }

    let mut command = Command::new("strace");
    command
        .args(["-f", "-e", "trace=connect", env!("CARGO_BIN_EXE_cortex")])
        .args(args)
        .current_dir(&scratch.project)
        .env("HOME", &scratch.home)
        .env("CORTEX_DB_PATH", &scratch.db)
        .env("CORTEX_AUDIT_LOG", &scratch.audit)
        .env("CORTEX_NO_NET", "1")
        .env("PATH", path)
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_OPENROUTER_BASE_URL");
    let out = command.output().expect("spawn strace");
    Some(
        String::from_utf8_lossy(&out.stderr)
            .matches("connect(")
            .count(),
    )
}

// ----------------------------------------------------------------------------------------------
// SSE provider bodies
// ----------------------------------------------------------------------------------------------

fn python_read_tool_call_body() -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_read_py",
                        "type": "function",
                        "function": {
                            "name": "read",
                            "arguments": json!({ "path": "bad.py" }).to_string()
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
        })
        .to_string(),
    ])
}

fn python_write_tool_call_body() -> String {
    sse_body(&[
        json!({
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_write_py",
                        "type": "function",
                        "function": {
                            "name": "write",
                            "arguments": json!({ "path": "bad.py", "content": BAD_PY }).to_string()
                        }
                    }]
                },
                "finish_reason": null
            }]
        })
        .to_string(),
        json!({
            "choices": [{ "delta": {}, "finish_reason": "tool_calls" }]
        })
        .to_string(),
    ])
}

fn final_body() -> String {
    sse_body(&[
        json!({
            "choices": [{ "delta": { "content": "done" }, "finish_reason": null }]
        })
        .to_string(),
        json!({
            "choices": [{ "delta": {}, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4 }
        })
        .to_string(),
    ])
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

fn write_sse_response(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn read_http_body(stream: &mut TcpStream) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let read = match stream.read(&mut chunk) {
            Ok(0) => return buffer,
            Ok(read) => read,
            Err(_) => return buffer,
        };
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = find_header_end(&buffer) {
            break end;
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
        let read = match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) => break,
        };
        buffer.extend_from_slice(&chunk[..read]);
        remaining = remaining.saturating_sub(read);
    }
    buffer
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
}

fn accept_with_idle_timeout(listener: &TcpListener, idle_timeout: Duration) -> Option<TcpStream> {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > idle_timeout {
                    return None;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => panic!("accept provider request: {err}"),
        }
    }
}

// ----------------------------------------------------------------------------------------------
// Process + parsing helpers
// ----------------------------------------------------------------------------------------------

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex binary");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command timed out");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: stdout_thread.join().expect("stdout join"),
        stderr: stderr_thread.join().expect("stderr join"),
    }
}

fn assert_success(out: &Output) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn read_audit(path: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).expect("audit file");
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("audit JSONL record"))
        .collect()
}

fn count<F>(records: &[Value], mut predicate: F) -> usize
where
    F: FnMut(&Value) -> bool,
{
    records.iter().filter(|record| predicate(record)).count()
}

fn tool_result_text(doc: &Value, tool_name: &str) -> String {
    doc["tool_results"]
        .as_array()
        .expect("tool results")
        .iter()
        .find(|result| result["name"] == tool_name)
        .expect("target tool result")
        .get("parts")
        .and_then(Value::as_array)
        .expect("parts")
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

fn find_diag<'a>(rows: &'a [Value], code: &str) -> Option<&'a Value> {
    rows.iter().find(|row| row["code"] == code)
}

/// Reconstruct the exact same-line position string `cortex run` renders for a diagnostic, so the
/// two surfaces can be checked for agreement.
fn rendered_position(diag: &Value) -> String {
    let line = diag["line"].as_u64().expect("line");
    let character = diag["character"].as_u64().expect("character");
    let end_character = diag["end_character"].as_u64().expect("end_character");
    format!("at line {line}, character {character}-{end_character}")
}

fn has_command(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, body).expect("write executable");
    let mut perms = std::fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod executable");
}

#[cfg(not(unix))]
fn write_executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write executable");
}

fn fake_openrouter_key() -> String {
    format!("sk-{}-{}", "or-v1", "abcdef01".repeat(8))
}

fn reserve_serve_port() -> u16 {
    for _ in 0..60 {
        let port = NEXT_SERVE_PORT.fetch_add(1, Ordering::SeqCst);
        // Product reserved serve/provider loopback band is 18765-18864.
        if !(18765..=18864).contains(&port) {
            NEXT_SERVE_PORT.store(18805, Ordering::SeqCst);
            continue;
        }
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        if TcpListener::bind(addr).is_ok() {
            return port;
        }
    }
    panic!("no free serve port in 18765-18864");
}

fn bind_in_range(counter: &AtomicU16) -> (u16, TcpListener) {
    for _ in 0..60 {
        let port = counter.fetch_add(1, Ordering::SeqCst);
        // Product reserved serve/provider loopback band is 18765-18864.
        if !(18765..=18864).contains(&port) {
            counter.store(18765, Ordering::SeqCst);
            continue;
        }
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        if let Ok(listener) = TcpListener::bind(addr) {
            return (port, listener);
        }
    }
    panic!("no free provider port in 18765-18864");
}

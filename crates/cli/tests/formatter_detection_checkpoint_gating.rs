//! Black-box regression coverage for formatter detection, safety, checkpoints, permissions, and
//! post-format diagnostics.
//!
//! Each case drives the real `cortex` binary through a local scripted OpenAI-compatible provider.
//! The provider deterministically requests real built-in tools, while formatters, the shadow-git
//! store, permissions, and the language server remain the shipped implementations. This keeps the
//! assertions deterministic without replacing any product capability with a mock.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const TURN_TIMEOUT: Duration = Duration::from_secs(120);
static SCRATCH_COUNTER: AtomicUsize = AtomicUsize::new(0);
static NEXT_PROVIDER_PORT: AtomicU16 = AtomicU16::new(18775);

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    checkpoints: PathBuf,
    data: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let unique = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "cortex-formatter-gating-{tag}-{}-{unique}",
            std::process::id()
        ));
        let home = root.join("home");
        let project = root.join("project");
        let checkpoints = root.join("checkpoints");
        let data = root.join("data");
        for path in [&home, &project, &checkpoints, &data] {
            fs::create_dir_all(path).expect("create sandbox directory");
        }
        Self {
            root,
            home,
            project,
            checkpoints,
            data,
        }
    }

    fn cortex(&self, provider: &ScriptedProvider, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .current_dir(&self.project)
            .args(args)
            .env("HOME", &self.home)
            .env("CORTEX_DATA_DIR", &self.data)
            .env("CORTEX_CHECKPOINT_DIR", &self.checkpoints)
            .env("OPENROUTER_API_KEY", fake_api_key())
            .env("CORTEX_OPENROUTER_BASE_URL", &provider.base_url)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("CORTEX_DB_PATH")
            .env_remove("CORTEX_NO_NET")
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("NO_COLOR");
        command
    }

    fn plain_cortex(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .current_dir(&self.project)
            .args(args)
            .env("HOME", &self.home)
            .env("CORTEX_DATA_DIR", &self.data)
            .env("CORTEX_CHECKPOINT_DIR", &self.checkpoints)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("CORTEX_DB_PATH")
            .env_remove("CORTEX_NO_NET")
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("NO_COLOR");
        command
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

enum ProviderStep {
    Tool {
        name: &'static str,
        arguments: Value,
    },
    Final,
}

/// Minimal loopback SSE provider. It only scripts model tool calls, never formatter, checkpoint,
/// permission, or LSP behavior.
struct ScriptedProvider {
    base_url: String,
    worker: Option<thread::JoinHandle<()>>,
}

impl ScriptedProvider {
    fn start(steps: Vec<ProviderStep>) -> Self {
        let (port, listener) = bind_provider_listener();
        listener
            .set_nonblocking(true)
            .expect("set provider listener nonblocking");
        let worker = thread::spawn(move || {
            for (index, step) in steps.into_iter().enumerate() {
                let Some(mut stream) = accept_with_idle_timeout(&listener, Duration::from_secs(30))
                else {
                    panic!("provider did not receive request {index}");
                };
                let _ = read_http_body(&mut stream);
                let body = match step {
                    ProviderStep::Tool { name, arguments } => {
                        tool_call_body(&format!("formatter_call_{index}"), name, arguments)
                    }
                    ProviderStep::Final => final_body(),
                };
                write_sse_response(&mut stream, &body).expect("write scripted provider response");
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            worker: Some(worker),
        }
    }

    fn finish(mut self) {
        self.worker
            .take()
            .expect("provider worker")
            .join()
            .expect("provider worker must not panic");
    }
}

fn bind_provider_listener() -> (u16, TcpListener) {
    for _ in 0..100 {
        let port = NEXT_PROVIDER_PORT.fetch_add(1, Ordering::SeqCst);
        if port > 18864 {
            NEXT_PROVIDER_PORT.store(18775, Ordering::SeqCst);
            continue;
        }
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return (port, listener);
        }
    }
    panic!("no free loopback provider port in 18765-18864");
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn write_turn(path: &str, content: &str) -> Vec<ProviderStep> {
    vec![
        ProviderStep::Tool {
            name: "write",
            arguments: json!({ "path": path, "content": content }),
        },
        ProviderStep::Final,
    ]
}

fn overwrite_turn(path: &str, content: &str) -> Vec<ProviderStep> {
    vec![
        ProviderStep::Tool {
            name: "read",
            arguments: json!({ "path": path }),
        },
        ProviderStep::Tool {
            name: "write",
            arguments: json!({ "path": path, "content": content }),
        },
        ProviderStep::Final,
    ]
}

fn run_turn(
    sandbox: &Sandbox,
    args: &[&str],
    steps: Vec<ProviderStep>,
    configure: impl FnOnce(&mut Command),
) -> Output {
    let provider = ScriptedProvider::start(steps);
    let mut command = sandbox.cortex(&provider, args);
    configure(&mut command);
    let output = run_command(command);
    provider.finish();
    output
}

fn run_command(mut command: Command) -> Output {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("cortex stdout");
    let mut stderr = child.stderr.take().expect("cortex stderr");
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll cortex") {
            break status;
        }
        if started.elapsed() > TURN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex did not exit within {TURN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_reader.join().expect("join stdout reader"),
        stderr: stderr_reader.join().expect("join stderr reader"),
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "cortex command failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_json_turn(
    sandbox: &Sandbox,
    steps: Vec<ProviderStep>,
    configure: impl FnOnce(&mut Command),
) -> Value {
    let output = run_turn(
        sandbox,
        &[
            "run",
            "--format",
            "json",
            "perform the requested file operation",
        ],
        steps,
        configure,
    );
    assert_success(&output);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "cortex --format json must emit one document: {err}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn tool_result_text(document: &Value, name: &str) -> String {
    document["tool_results"]
        .as_array()
        .expect("tool results array")
        .iter()
        .find(|result| result["name"] == name)
        .unwrap_or_else(|| panic!("missing {name} tool result: {document}"))["parts"]
        .as_array()
        .expect("tool result parts")
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_command(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run_formatter(program: &str, args: &[&str], cwd: &Path) {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn reference formatter");
    assert!(
        output.status.success(),
        "reference {program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn rustfmt_expected(raw: &str) -> String {
    let reference = Sandbox::new("rustfmt-reference");
    let path = reference.project.join("expected.rs");
    fs::write(&path, raw).expect("write rustfmt reference input");
    run_formatter(
        "rustfmt",
        &["--edition", "2024", path.to_str().expect("utf8 path")],
        &reference.project,
    );
    fs::read_to_string(path).expect("read rustfmt reference output")
}

fn prettier_expected(raw: &str) -> String {
    let reference = Sandbox::new("prettier-reference");
    fs::write(reference.project.join(".prettierrc"), "{}\n").expect("write prettier marker");
    let path = reference.project.join("expected.ts");
    fs::write(&path, raw).expect("write prettier reference input");
    run_formatter(
        "prettier",
        &["--write", path.to_str().expect("utf8 path")],
        &reference.project,
    );
    fs::read_to_string(path).expect("read prettier reference output")
}

fn sha256(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .expect("run sha256sum");
    assert!(
        output.status.success(),
        "sha256sum failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum digest")
        .to_owned()
}

fn toml_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).expect("write executable script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable script");
}

/// VAL-FMT-006 and VAL-FMT-009: extension, binary, and config-marker gates select only the
/// applicable formatter. Unsupported or missing-binary files remain byte-identical and successful.
#[test]
fn formatter_selection_requires_extension_binary_and_marker_and_cleanly_skips_absent_formatters() {
    if !(has_command("rustfmt") && has_command("prettier")) {
        eprintln!("skipping real formatter selection proof: rustfmt or prettier is unavailable");
        return;
    }

    let rust = Sandbox::new("rust-selection");
    fs::write(rust.project.join("cortex.toml"), "lsp = false\n").expect("disable lsp");
    let raw_rust = "fn  main( ){let x=1;println!(\"{}\",x);}\n";
    let rust_doc = run_json_turn(&rust, write_turn("main.rs", raw_rust), |_| {});
    assert!(
        !rust_doc["tool_results"][0]["is_error"]
            .as_bool()
            .expect("write result error flag")
    );
    assert_eq!(
        fs::read_to_string(rust.project.join("main.rs")).expect("read formatted Rust"),
        rustfmt_expected(raw_rust),
        "a Rust file must be formatted by rustfmt, not a JavaScript or Python formatter"
    );

    let ts = Sandbox::new("prettier-marker");
    fs::write(ts.project.join("cortex.toml"), "lsp = false\n").expect("disable lsp");
    let raw_ts = "const value={foo:\"bar\",answer:42};\n";
    let no_marker_doc = run_json_turn(&ts, write_turn("without-config.ts", raw_ts), |_| {});
    assert!(
        !no_marker_doc["tool_results"][0]["is_error"]
            .as_bool()
            .expect("write result error flag")
    );
    assert_eq!(
        fs::read_to_string(ts.project.join("without-config.ts")).expect("read raw TypeScript"),
        raw_ts,
        "a TypeScript file without a prettier marker must remain unformatted"
    );

    fs::write(ts.project.join(".prettierrc"), "{}\n").expect("write prettier marker");
    let marker_doc = run_json_turn(&ts, write_turn("with-config.ts", raw_ts), |_| {});
    assert!(
        !marker_doc["tool_results"][0]["is_error"]
            .as_bool()
            .expect("write result error flag")
    );
    assert_eq!(
        fs::read_to_string(ts.project.join("with-config.ts")).expect("read prettier TypeScript"),
        prettier_expected(raw_ts),
        "a TypeScript file with only a prettier marker must use prettier"
    );

    let unknown = Sandbox::new("unknown-extension");
    fs::write(unknown.project.join("cortex.toml"), "lsp = false\n").expect("disable lsp");
    let raw_unknown = "verbatim   spacing\n";
    let unknown_doc = run_json_turn(&unknown, write_turn("note.unknownext", raw_unknown), |_| {});
    assert!(
        !unknown_doc["tool_results"][0]["is_error"]
            .as_bool()
            .expect("write result error flag")
    );
    assert_eq!(
        fs::read_to_string(unknown.project.join("note.unknownext"))
            .expect("read unknown extension"),
        raw_unknown,
        "no formatter for an extension is a clean no-op"
    );

    let absent = Sandbox::new("missing-binary");
    fs::write(absent.project.join("cortex.toml"), "lsp = false\n").expect("disable lsp");
    let raw_missing = "fn  main( ){let x=1;}\n";
    let missing_doc = run_json_turn(&absent, write_turn("missing.rs", raw_missing), |command| {
        command.env("PATH", "/usr/bin:/bin");
    });
    assert!(
        !missing_doc["tool_results"][0]["is_error"]
            .as_bool()
            .expect("write result error flag")
    );
    assert_eq!(
        fs::read_to_string(absent.project.join("missing.rs")).expect("read missing formatter file"),
        raw_missing,
        "a language whose formatter binary is absent must be left unchanged"
    );
}

/// VAL-FMT-007: the checkpoint belongs to the raw pre-write state, before the real formatter turns
/// the write into canonical Rust. A later CLI undo must restore the exact original SHA-256.
#[test]
fn pre_format_checkpoint_restores_original_sha256_after_real_rustfmt_write() {
    if !has_command("rustfmt") {
        eprintln!("skipping checkpoint formatter proof: rustfmt is unavailable");
        return;
    }

    let sandbox = Sandbox::new("checkpoint-before-format");
    fs::write(sandbox.project.join("cortex.toml"), "lsp = false\n").expect("disable lsp");
    let path = sandbox.project.join("main.rs");
    let original = "fn original() {\n    println!(\"before\");\n}\n";
    fs::write(&path, original).expect("seed original file");
    let original_hash = sha256(&path);
    let raw_write = "fn  changed( ){let value=1;println!(\"{}\",value);}\n";

    let document = run_json_turn(&sandbox, overwrite_turn("main.rs", raw_write), |_| {});
    assert!(
        !document["tool_results"]
            .as_array()
            .expect("tool results array")
            .iter()
            .find(|result| result["name"] == "write")
            .expect("write result")["is_error"]
            .as_bool()
            .expect("write result error flag")
    );
    assert_eq!(
        fs::read_to_string(&path).expect("read formatted write"),
        rustfmt_expected(raw_write),
        "the post-mutation formatter must have changed the raw write"
    );

    let undo = run_command(sandbox.plain_cortex(&["undo", "--format", "json"]));
    assert_success(&undo);
    let report: Value = serde_json::from_slice(&undo.stdout).expect("undo JSON");
    assert_eq!(report["status"], "restored", "undo report: {report}");
    assert_eq!(
        fs::read_to_string(&path).expect("read restored file"),
        original,
        "undo must recover pre-write bytes rather than formatted bytes"
    );
    assert_eq!(
        sha256(&path),
        original_hash,
        "the restored file must match the exact pre-format SHA-256"
    );
}

/// VAL-FMT-008 and VAL-FMT-011: format permission is independent of write permission, and a
/// configured custom formatter is the executable that runs when formatting is allowed.
#[cfg(unix)]
#[test]
fn formatter_permission_denial_preserves_raw_write_and_configured_custom_override_runs_when_allowed()
 {
    let sandbox = Sandbox::new("permission-custom");
    let formatter = sandbox.project.join("uppercase.sh");
    let marker = sandbox.project.join("formatter-runs.log");
    write_executable(
        &formatter,
        &format!(
            "#!/bin/sh\nfile=\"$1\"\nprintf ran >> '{}'\ntr '[:lower:]' '[:upper:]' < \"$file\" > \"$file.tmp\"\nmv \"$file.tmp\" \"$file\"\n",
            marker.display()
        ),
    );
    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            "lsp = false\n\n[formatter.custom]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".fmtperm\"]\n",
            toml_path(&formatter)
        ),
    )
    .expect("write custom formatter config");

    let raw_denied = "lowercase denied\n";
    let denied = run_turn(
        &sandbox,
        &[
            "run",
            "--format",
            "json",
            "--permission-allow",
            "write:*",
            "--permission-deny",
            "format:blocked.fmtperm",
            "write the configured file",
        ],
        write_turn("blocked.fmtperm", raw_denied),
        |_| {},
    );
    assert_success(&denied);
    assert_eq!(
        fs::read_to_string(sandbox.project.join("blocked.fmtperm")).expect("read denied file"),
        raw_denied,
        "denying format must not reject or alter the raw write"
    );
    assert!(
        !marker.exists(),
        "the custom formatter process must not run when format is denied"
    );

    let raw_allowed = "lowercase allowed\n";
    let allowed = run_turn(
        &sandbox,
        &[
            "run",
            "--format",
            "json",
            "--permission-allow",
            "write:*",
            "--permission-allow",
            "format:*",
            "write the configured file",
        ],
        write_turn("allowed.fmtperm", raw_allowed),
        |_| {},
    );
    assert_success(&allowed);
    assert_eq!(
        fs::read_to_string(sandbox.project.join("allowed.fmtperm")).expect("read allowed file"),
        "LOWERCASE ALLOWED\n",
        "allowing format must run the configured custom formatter after the write"
    );
    assert_eq!(
        fs::read_to_string(marker).expect("read formatter marker"),
        "ran",
        "the custom formatter override must be invoked exactly once"
    );
}

/// VAL-FMT-011: a custom `.rs` formatter wins over built-in rustfmt selection.
#[cfg(unix)]
#[test]
fn custom_formatter_override_replaces_builtin_rustfmt_for_matching_extension() {
    let sandbox = Sandbox::new("custom-over-rustfmt");
    let formatter = sandbox.project.join("custom-rust.sh");
    write_executable(
        &formatter,
        "#!/bin/sh\nprintf 'CUSTOM FORMATTER OUTPUT\\n' > \"$1\"\n",
    );
    fs::write(
        sandbox.project.join("cortex.toml"),
        format!(
            "lsp = false\n\n[formatter.custom]\ncommand = [\"{}\", \"$FILE\"]\nextensions = [\".rs\"]\n",
            toml_path(&formatter)
        ),
    )
    .expect("write custom rust formatter config");

    let document = run_json_turn(&sandbox, write_turn("custom.rs", "fn  main( ){}\n"), |_| {});
    assert!(
        !document["tool_results"][0]["is_error"]
            .as_bool()
            .expect("write result error flag")
    );
    assert_eq!(
        fs::read_to_string(sandbox.project.join("custom.rs")).expect("read custom output"),
        "CUSTOM FORMATTER OUTPUT\n",
        "the configured custom command must replace built-in rustfmt"
    );
}

/// VAL-FMT-010: a real rustfmt parse failure returns normal tool feedback and preserves the raw
/// write, rather than truncating/corrupting the file or crashing the headless run.
#[test]
fn real_formatter_parse_failure_preserves_written_bytes_and_reports_cleanly() {
    if !has_command("rustfmt") {
        eprintln!("skipping formatter failure proof: rustfmt is unavailable");
        return;
    }

    let sandbox = Sandbox::new("rustfmt-parse-failure");
    fs::write(sandbox.project.join("cortex.toml"), "lsp = false\n").expect("disable lsp");
    let raw = "fn broken( {\n";
    let document = run_json_turn(&sandbox, write_turn("broken.rs", raw), |_| {});
    let result = tool_result_text(&document, "write");
    assert!(
        result.contains("Formatter rustfmt failed"),
        "the formatter failure must be surfaced as feedback: {result}"
    );
    assert_eq!(
        fs::read_to_string(sandbox.project.join("broken.rs")).expect("read failed-format file"),
        raw,
        "a real formatter failure must leave the successful raw write intact"
    );
}

/// VAL-FMT-013: real rustfmt expands a one-line type error to line two, and real rust-analyzer
/// diagnostics injected into the write result must point at that post-format line. A separate clean
/// formatted write must report no phantom diagnostics.
#[test]
fn reinjected_rust_analyzer_diagnostics_use_post_format_positions_without_phantom_clean_results() {
    if !(has_command("rustfmt") && has_command("rust-analyzer")) {
        eprintln!("skipping formatter/LSP proof: rustfmt or rust-analyzer is unavailable");
        return;
    }

    let broken = Sandbox::new("post-format-diagnostics");
    fs::create_dir_all(broken.project.join("src")).expect("create Rust source directory");
    fs::write(
        broken.project.join("Cargo.toml"),
        "[package]\nname = \"formatter_position_probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Rust project manifest");
    let raw = "pub fn broken(){let value:u32=\"wrong\";}\n";
    let document = run_json_turn(&broken, write_turn("src/lib.rs", raw), |_| {});
    let formatted = fs::read_to_string(broken.project.join("src/lib.rs")).expect("read Rust file");
    assert_eq!(
        formatted,
        rustfmt_expected(raw),
        "the fixture must have been reformatted before diagnostics are collected"
    );
    assert!(
        formatted
            .lines()
            .nth(1)
            .is_some_and(|line| line.contains("\"wrong\"")),
        "the type-error token must be on formatted line two: {formatted}"
    );
    let diagnostics = tool_result_text(&document, "write");
    assert!(
        diagnostics.contains("Diagnostics:"),
        "a real Rust type mismatch must be reinjected: {diagnostics}"
    );
    assert!(
        diagnostics.contains("at line 2,"),
        "the diagnostic must point at the post-rustfmt line: {diagnostics}"
    );
    assert!(
        !diagnostics.contains("at line 1,"),
        "diagnostics must not retain the stale one-line position: {diagnostics}"
    );

    let clean = Sandbox::new("clean-post-format-diagnostics");
    fs::create_dir_all(clean.project.join("src")).expect("create clean Rust source directory");
    fs::write(
        clean.project.join("Cargo.toml"),
        "[package]\nname = \"formatter_clean_probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write clean Rust project manifest");
    let clean_document = run_json_turn(
        &clean,
        write_turn(
            "src/lib.rs",
            "pub fn clean(){let value=1;println!(\"{value}\");}\n",
        ),
        |_| {},
    );
    let clean_result = tool_result_text(&clean_document, "write");
    assert!(
        !clean_result.contains("Diagnostics:"),
        "formatting a clean file must not introduce phantom diagnostics: {clean_result}"
    );
}

fn tool_call_body(call_id: &str, name: &str, arguments: Value) -> String {
    sse_body(&[
        json!({
            "choices": [{
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
        .expect("set request read timeout");
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
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
        .map(|position| position + 4)
}

fn accept_with_idle_timeout(listener: &TcpListener, timeout: Duration) -> Option<TcpStream> {
    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() > timeout {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept provider connection: {error}"),
        }
    }
}

fn fake_api_key() -> String {
    format!("sk-{}-{}", "or-v1", "01234567".repeat(8))
}

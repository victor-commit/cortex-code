//! Custom markdown agents, generation, and selection.
//!
//! Offline tests prove discovery/error handling without a key. Live tests are gated on
//! `OPENROUTER_API_KEY` and exercise real OpenRouter turns for the user-facing behavior.

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(120);

fn scratch_tree(tag: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "cortex-custom-agent-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let proj = home.join("proj");
    fs::create_dir_all(proj.join(".cortex").join("agents")).expect("create agents dir");
    (home, proj)
}

fn write_agent(proj: &Path, filename: &str, body: &str) {
    fs::write(proj.join(".cortex").join("agents").join(filename), body).expect("write agent");
}

fn base_command(home: &Path, proj: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(proj);
    command.args(args);
    command.env("HOME", home);
    command
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn keyless(home: &Path, proj: &Path, args: &[&str]) -> Command {
    base_command(home, proj, args)
}

fn with_key(home: &Path, proj: &Path, key: &str, args: &[&str]) -> Command {
    let mut command = base_command(home, proj, args);
    command.env("OPENROUTER_API_KEY", key);
    command
}

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
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
            panic!("cortex command did not finish within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn has_denied_write_result(doc: &serde_json::Value) -> bool {
    doc["tool_results"].as_array().is_some_and(|results| {
        results.iter().any(|result| {
            if result["name"].as_str() != Some("write") {
                return false;
            }
            if !result["is_error"].as_bool().unwrap_or(false) {
                return false;
            }
            let rendered = serde_json::to_string(result)
                .unwrap_or_default()
                .to_lowercase();
            rendered.contains("permission denied")
                || rendered.contains("denied")
                || rendered.contains("not allowed")
                || rendered.contains("blocked")
        })
    })
}

fn content_indicates_denied_write(content: &str) -> bool {
    let lower = content.to_lowercase();
    (lower.contains("permission")
        && (lower.contains("denied") || lower.contains("blocked") || lower.contains("not allowed")))
        || lower.contains("unable to create")
        || lower.contains("cannot create")
        || lower.contains("can't create")
        || lower.contains("write access is blocked")
}

struct GenerationFixture {
    base_url: String,
    requests: Arc<AtomicUsize>,
    thread: Option<thread::JoinHandle<()>>,
}

impl GenerationFixture {
    fn start(expected_requests: usize, generated_name: &'static str) -> Self {
        let listener = bind_loopback_listener();
        listener
            .set_nonblocking(true)
            .expect("set generation fixture listener nonblocking");
        let port = listener.local_addr().expect("fixture address").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_thread = requests.clone();
        let thread = thread::spawn(move || {
            for request_index in 0..expected_requests {
                let mut stream = accept_with_timeout(&listener, Duration::from_secs(10))
                    .expect("generator should connect to the loopback provider");
                read_http_request(&mut stream).expect("read generation provider request");
                requests_thread.fetch_add(1, Ordering::Relaxed);
                let response = if request_index + 1 == expected_requests {
                    run_response()
                } else {
                    generation_response(generated_name, "Generated by loopback fixture.")
                };
                write_sse_response(&mut stream, &response)
                    .expect("write generation provider response");
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            requests,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> usize {
        self.thread
            .take()
            .expect("fixture thread")
            .join()
            .expect("join generation fixture");
        self.requests.load(Ordering::Relaxed)
    }
}

impl Drop for GenerationFixture {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn bind_loopback_listener() -> TcpListener {
    for port in 18765..18865 {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener;
        }
    }
    panic!("no permitted loopback fixture port is available in 18765-18864");
}

fn accept_with_timeout(listener: &TcpListener, timeout: Duration) -> Option<TcpStream> {
    let start = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() >= timeout {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("accept loopback provider: {err}"),
        }
    }
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
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
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
    let mut remaining = content_length.saturating_sub(buffer.len().saturating_sub(header_end));
    while remaining > 0 {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        remaining = remaining.saturating_sub(read);
    }
    Ok(())
}

fn generation_response(name: &str, description: &str) -> String {
    let generation = serde_json::json!({
        "name": name,
        "description": description,
        "system_prompt": "Answer exactly as the generated loopback agent."
    })
    .to_string();
    let first = serde_json::json!({
        "id": "generation-fixture",
        "object": "chat.completion.chunk",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": generation },
            "finish_reason": null
        }]
    })
    .to_string();
    let final_chunk = serde_json::json!({
        "id": "generation-fixture",
        "object": "chat.completion.chunk",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }]
    })
    .to_string();
    format!("data: {first}\n\ndata: {final_chunk}\n\ndata: [DONE]\n\n")
}

fn run_response() -> String {
    let first = serde_json::json!({
        "id": "generation-selection-fixture",
        "object": "chat.completion.chunk",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": "selected generated agent" },
            "finish_reason": null
        }]
    })
    .to_string();
    let final_chunk = serde_json::json!({
        "id": "generation-selection-fixture",
        "object": "chat.completion.chunk",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }]
    })
    .to_string();
    format!("data: {first}\n\ndata: {final_chunk}\n\ndata: [DONE]\n\n")
}

fn write_sse_response(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\n\
         content-type: text/event-stream\r\n\
         cache-control: no-cache\r\n\
         connection: close\r\n\
         content-length: {}\r\n\
         \r\n\
         {body}",
        body.len()
    )?;
    stream.flush()
}

fn fixture_generate(
    home: &Path,
    proj: &Path,
    base_url: &str,
    name: &str,
    agents_dir: Option<&str>,
) -> Command {
    let mut command = base_command(
        home,
        proj,
        &[
            "generate",
            "a deterministic generated agent",
            "--name",
            name,
            "--format",
            "json",
        ],
    );
    if let Some(agents_dir) = agents_dir {
        command.arg("--agents-dir").arg(agents_dir);
    }
    command
        .env("OPENROUTER_API_KEY", "fixture-key")
        .env("CORTEX_OPENROUTER_BASE_URL", base_url)
        .env_remove("CORTEX_NO_NET");
    command
}

fn valid_pirate_agent() -> &'static str {
    r#"---
name: pirate
description: Pirate responder
mode: primary
---
Always answer with pirate phrasing.
"#
}

#[test]
fn agent_list_discovers_custom_markdown_and_skips_malformed() {
    let (home, proj) = scratch_tree("list");
    write_agent(&proj, "pirate.md", valid_pirate_agent());
    write_agent(&proj, "broken.md", "---\nname: [\n---\nBody\n");
    let out = run(
        keyless(&home, &proj, &["agent", "--format", "json"]),
        Duration::from_secs(10),
    );
    assert!(
        out.status.success(),
        "agent list should succeed, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let doc: serde_json::Value = serde_json::from_str(stdout.trim_end()).expect("strict JSON");
    let names: Vec<&str> = doc["agents"]
        .as_array()
        .expect("agents")
        .iter()
        .filter_map(|agent| agent["name"].as_str())
        .collect();
    assert!(
        names.contains(&"build"),
        "built-ins remain listed: {names:?}"
    );
    assert!(
        names.contains(&"pirate"),
        "custom agent is listed: {names:?}"
    );
    assert!(
        !names.contains(&"broken"),
        "malformed custom agent is skipped: {names:?}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("broken.md"),
        "malformed agent should be flagged on stderr, got: {stderr}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "malformed agent must not panic: {stderr}"
    );
}

#[test]
fn unknown_agent_error_includes_custom_agent_availability_hint() {
    let (home, proj) = scratch_tree("unknown");
    write_agent(&proj, "pirate.md", valid_pirate_agent());

    let out = run(
        keyless(&home, &proj, &["run", "--agent", "ghost", "hi"]),
        Duration::from_secs(10),
    );
    assert_ne!(out.status.code(), Some(0));
    assert!(
        out.stdout.is_empty(),
        "unknown agent must not write an assistant answer: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("ghost"), "names requested agent: {stderr}");
    assert!(stderr.contains("pirate"), "hints custom agent: {stderr}");
    assert!(
        stderr.contains("build") && stderr.contains("plan"),
        "hints built-ins: {stderr}"
    );
    assert!(
        !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
        "unknown agent must not panic: {stderr}"
    );
}

#[test]
fn custom_agent_frontmatter_model_prompt_and_permission_live() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_tree("live");
    write_agent(
        &proj,
        "sentinel.md",
        r#"---
name: sentinel
description: Custom sentinel agent
mode: primary
model: google/gemini-2.5-flash
permission:
  - tool: write
    pattern: "**"
    action: deny
---
If asked to create a file, call the write tool first. In your final answer, briefly explain that the write was blocked by permissions.
"#,
    );

    let out = run(
        with_key(
            &home,
            &proj,
            &key,
            &[
                "run",
                "--agent",
                "sentinel",
                "--format",
                "json",
                "Use the write tool to create blocked.txt containing NOPE. Then reply briefly.",
            ],
        ),
        RUN_TIMEOUT,
    );
    assert!(
        out.status.success(),
        "custom agent turn should finish, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !proj.join("blocked.txt").exists(),
        "frontmatter permission deny write ** must block the write"
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let doc: serde_json::Value = serde_json::from_str(stdout.trim_end()).expect("strict JSON");
    assert_eq!(doc["agent"], "sentinel");
    assert_eq!(doc["model"], MODEL);
    let content = doc["content"].as_str().unwrap_or_default();
    assert!(
        !content.trim().is_empty(),
        "custom agent should produce a non-empty answer: {stdout}"
    );
    assert!(
        has_denied_write_result(&doc) || content_indicates_denied_write(content),
        "frontmatter deny should make blocked-write behavior observable in tool results or assistant content: {stdout}"
    );
}

#[test]
fn generate_authors_discoverable_and_runnable_agent_live() {
    let Some(key) = live_key() else {
        eprintln!("skipping: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_tree("generate");

    let out = run(
        with_key(
            &home,
            &proj,
            &key,
            &[
                "generate",
                "an agent that gives concise unit testing advice",
                "--name",
                "generated-test-agent",
                "--format",
                "json",
            ],
        ),
        RUN_TIMEOUT,
    );
    assert!(
        out.status.success(),
        "generate should succeed via a real model turn, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let doc: serde_json::Value = serde_json::from_str(stdout.trim_end()).expect("strict JSON");
    assert_eq!(doc["name"], "generated-test-agent");
    let path = PathBuf::from(doc["path"].as_str().expect("path"));
    assert!(path.exists(), "generated agent file exists at {path:?}");
    let contents = fs::read_to_string(&path).expect("read generated agent");
    assert!(contents.contains("name: \"generated-test-agent\""));
    assert!(contents.contains("description:"));
    assert!(contents.contains("---"));

    let listed = run(
        keyless(&home, &proj, &["agent", "--format", "json"]),
        Duration::from_secs(10),
    );
    assert!(listed.status.success());
    let list_stdout = String::from_utf8(listed.stdout).expect("utf8 stdout");
    assert!(
        list_stdout.contains("generated-test-agent"),
        "generated agent is discoverable: {list_stdout}"
    );

    let run_out = run(
        with_key(
            &home,
            &proj,
            &key,
            &[
                "run",
                "--agent",
                "generated-test-agent",
                "--format",
                "json",
                "Say one short sentence about unit tests.",
            ],
        ),
        RUN_TIMEOUT,
    );
    assert!(
        run_out.status.success(),
        "generated agent should be runnable, stderr: {}",
        String::from_utf8_lossy(&run_out.stderr)
    );
    let run_stdout = String::from_utf8(run_out.stdout).expect("utf8 stdout");
    let run_doc: serde_json::Value = serde_json::from_str(run_stdout.trim_end()).expect("json");
    assert_eq!(run_doc["agent"], "generated-test-agent");
    assert!(
        run_doc["content"]
            .as_str()
            .is_some_and(|content| !content.trim().is_empty()),
        "generated agent produced an assistant answer: {run_stdout}"
    );
}

#[test]
fn loopback_generation_rejects_collisions_and_only_reports_discoverable_success() {
    let (home, proj) = scratch_tree("generate-loopback-safety");
    fs::create_dir_all(proj.join(".cortex/alternate-agents")).expect("create configured agent dir");
    fs::write(
        proj.join("cortex.toml"),
        "agent_dirs = [\".cortex/alternate-agents\"]\n",
    )
    .expect("configure discoverable alternate agent dir");
    write_agent(
        &proj,
        "existing.md",
        "---\nname: existing-agent\ndescription: Existing custom agent\nmode: primary\n---\nExisting.\n",
    );

    let fixture = GenerationFixture::start(6, "loopback-agent");
    let builtin = run(
        fixture_generate(&home, &proj, &fixture.base_url, "PLAN", None),
        Duration::from_secs(15),
    );
    assert!(!builtin.status.success(), "built-in collision must fail");
    assert!(
        builtin.stdout.is_empty(),
        "failed generation must not report success: {}",
        String::from_utf8_lossy(&builtin.stdout)
    );
    assert!(
        String::from_utf8_lossy(&builtin.stderr).contains("collides with built-in agent 'plan'"),
        "built-in collision should be clear: {}",
        String::from_utf8_lossy(&builtin.stderr)
    );
    assert!(
        !proj.join(".cortex/agents/plan.md").exists(),
        "built-in collision must not create a file"
    );

    let existing = run(
        fixture_generate(&home, &proj, &fixture.base_url, " Existing-Agent ", None),
        Duration::from_secs(15),
    );
    assert!(
        !existing.status.success(),
        "normalized custom collision must fail"
    );
    assert!(existing.stdout.is_empty(), "no success report on collision");
    assert!(
        String::from_utf8_lossy(&existing.stderr).contains("collides with custom agent"),
        "custom collision should be clear: {}",
        String::from_utf8_lossy(&existing.stderr)
    );
    assert!(
        !proj.join(".cortex/agents/existing-agent.md").exists(),
        "custom collision must not create a file"
    );

    let undiscoverable = run(
        fixture_generate(
            &home,
            &proj,
            &fixture.base_url,
            "elsewhere-agent",
            Some("unlisted-agents"),
        ),
        Duration::from_secs(15),
    );
    assert!(
        !undiscoverable.status.success(),
        "an unlisted target directory must fail"
    );
    assert!(
        undiscoverable.stdout.is_empty(),
        "no success report on failure"
    );
    assert!(
        String::from_utf8_lossy(&undiscoverable.stderr)
            .contains("not a discovered custom-agent directory"),
        "undiscoverable target should be clear: {}",
        String::from_utf8_lossy(&undiscoverable.stderr)
    );
    assert!(
        !proj.join("unlisted-agents").exists(),
        "undiscoverable target must not be created"
    );

    let occupied = proj.join(".cortex/agents/filename-only.md");
    fs::create_dir_all(occupied.parent().expect("parent")).expect("create default agent directory");
    fs::write(&occupied, "not a valid custom agent\n").expect("create filename-only collision");
    let filename_collision = run(
        fixture_generate(&home, &proj, &fixture.base_url, "filename-only", None),
        Duration::from_secs(15),
    );
    assert!(
        !filename_collision.status.success(),
        "an occupied filename must not be renamed and reported as success"
    );
    assert!(
        filename_collision.stdout.is_empty(),
        "filename collision must not report a generated agent"
    );
    assert!(
        String::from_utf8_lossy(&filename_collision.stderr).contains("failed to write agent file"),
        "filename collision must be clear: {}",
        String::from_utf8_lossy(&filename_collision.stderr)
    );
    assert_eq!(
        fs::read_to_string(&occupied).expect("read original file"),
        "not a valid custom agent\n",
        "generation must never overwrite an existing path"
    );
    assert!(
        !proj.join(".cortex/agents/filename-only-2.md").exists(),
        "filename collision must not silently select a suffixed path"
    );

    let generated = run(
        fixture_generate(
            &home,
            &proj,
            &fixture.base_url,
            "loopback-agent",
            Some(".cortex/alternate-agents"),
        ),
        Duration::from_secs(15),
    );
    assert!(
        generated.status.success(),
        "configured discoverable generation should succeed: {}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&generated.stdout).expect("strict JSON success report");
    let path = PathBuf::from(report["path"].as_str().expect("generated path"));
    assert_eq!(
        path,
        proj.join(".cortex/alternate-agents/loopback-agent.md")
    );
    assert!(path.is_file(), "success must leave the reported agent file");

    let listed = run(
        keyless(&home, &proj, &["agent", "--format", "json"]),
        Duration::from_secs(10),
    );
    assert!(listed.status.success(), "agent listing must succeed");
    let list: serde_json::Value =
        serde_json::from_slice(&listed.stdout).expect("strict JSON agent list");
    assert!(
        list["agents"].as_array().is_some_and(|agents| {
            agents.iter().any(|agent| {
                agent["name"] == "loopback-agent" && agent["source"] == path.display().to_string()
            })
        }),
        "the reported generated agent must be immediately listable: {list}"
    );

    let mut selected_command = with_key(
        &home,
        &proj,
        "fixture-key",
        &[
            "run",
            "--agent",
            "loopback-agent",
            "--format",
            "json",
            "Confirm the selected custom agent.",
        ],
    );
    selected_command.env("CORTEX_OPENROUTER_BASE_URL", &fixture.base_url);
    let selected = run(selected_command, Duration::from_secs(15));
    assert!(
        selected.status.success(),
        "generated agent must be selectable: {}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let selected_json: serde_json::Value =
        serde_json::from_slice(&selected.stdout).expect("strict JSON selected agent result");
    assert_eq!(selected_json["agent"], "loopback-agent");
    assert_eq!(
        fixture.finish(),
        6,
        "each rejected or successful generation must use the deterministic loopback provider"
    );
}

#[test]
fn concurrent_loopback_generation_allows_only_one_discoverable_success() {
    let (home, proj) = scratch_tree("generate-loopback-concurrent");
    let fixture = GenerationFixture::start(2, "race-agent");

    let first = fixture_generate(&home, &proj, &fixture.base_url, "race-agent", None);
    let second = fixture_generate(&home, &proj, &fixture.base_url, "race-agent", None);
    let first_thread = thread::spawn(move || run(first, Duration::from_secs(15)));
    let second_thread = thread::spawn(move || run(second, Duration::from_secs(15)));
    let first = first_thread.join().expect("join first generator");
    let second = second_thread.join().expect("join second generator");

    let outcomes = [&first, &second];
    assert_eq!(
        outcomes.iter().filter(|out| out.status.success()).count(),
        1,
        "concurrent normalized identities must have exactly one success"
    );
    let failed = outcomes
        .iter()
        .find(|out| !out.status.success())
        .expect("one generator must fail");
    assert!(
        failed.stdout.is_empty(),
        "the failed concurrent generator must not report success: {}",
        String::from_utf8_lossy(&failed.stdout)
    );
    let failed_stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(
        failed_stderr.contains("collides with custom agent")
            || failed_stderr.contains("already in progress"),
        "concurrent collision must fail clearly: {failed_stderr}"
    );
    let generated = proj.join(".cortex/agents/race-agent.md");
    assert!(
        generated.is_file(),
        "one generated agent must remain discoverable"
    );
    assert_eq!(
        fixture.finish(),
        2,
        "both concurrent generators must use the deterministic loopback provider"
    );
}

#[test]
fn concurrent_generation_across_configured_dirs_allows_one_success() {
    let (home, proj) = scratch_tree("generate-loopback-cross-dir-concurrent");
    fs::create_dir_all(proj.join(".cortex/one")).expect("create first configured agent dir");
    fs::create_dir_all(proj.join(".cortex/two")).expect("create second configured agent dir");
    fs::write(
        proj.join("cortex.toml"),
        "agent_dirs = [\".cortex/one\", \".cortex/two\"]\n",
    )
    .expect("configure both generated agent directories");

    let fixture = GenerationFixture::start(2, "cross-dir-race");
    let first = fixture_generate(
        &home,
        &proj,
        &fixture.base_url,
        "cross-dir-race",
        Some(".cortex/one"),
    );
    let second = fixture_generate(
        &home,
        &proj,
        &fixture.base_url,
        "cross-dir-race",
        Some(".cortex/two"),
    );
    let first_thread = thread::spawn(move || run(first, Duration::from_secs(15)));
    let second_thread = thread::spawn(move || run(second, Duration::from_secs(15)));
    let first = first_thread.join().expect("join first generator");
    let second = second_thread.join().expect("join second generator");

    let outcomes = [&first, &second];
    assert_eq!(
        outcomes.iter().filter(|out| out.status.success()).count(),
        1,
        "configured directory races must have exactly one success"
    );
    let failed = outcomes
        .iter()
        .find(|out| !out.status.success())
        .expect("one generator must fail");
    assert!(failed.stdout.is_empty(), "failure must not report success");
    let failed_stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(
        failed_stderr.contains("collides with custom agent")
            || failed_stderr.contains("already in progress"),
        "cross-directory collision must fail clearly: {failed_stderr}"
    );
    assert_eq!(
        usize::from(proj.join(".cortex/one/cross-dir-race.md").is_file())
            + usize::from(proj.join(".cortex/two/cross-dir-race.md").is_file()),
        1,
        "the name must be persisted in exactly one discoverable directory"
    );
    assert_eq!(
        fixture.finish(),
        2,
        "both generators must reach the fixture"
    );
}

#[test]
fn concurrent_generation_outside_home_uses_workspace_state_lease() {
    let (home, _) = scratch_tree("generate-loopback-outside-home");
    let project = std::env::temp_dir().join(format!(
        "cortex-generated-agent-outside-home-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let state_dir = home.join("writable-cortex-state");
    fs::create_dir_all(project.join(".cortex/one")).expect("create first configured agent dir");
    fs::create_dir_all(project.join(".cortex/two")).expect("create second configured agent dir");
    fs::write(
        project.join("cortex.toml"),
        "agent_dirs = [\".cortex/one\", \".cortex/two\"]\n",
    )
    .expect("configure both generated agent directories");

    let fixture = GenerationFixture::start(2, "outside-home-race");
    let mut first = fixture_generate(
        &home,
        &project,
        &fixture.base_url,
        "outside-home-race",
        Some(".cortex/one"),
    );
    let mut second = fixture_generate(
        &home,
        &project,
        &fixture.base_url,
        "outside-home-race",
        Some(".cortex/two"),
    );
    first.env("CORTEX_DATA_DIR", &state_dir);
    second.env("CORTEX_DATA_DIR", &state_dir);
    let first_thread = thread::spawn(move || run(first, Duration::from_secs(15)));
    let second_thread = thread::spawn(move || run(second, Duration::from_secs(15)));
    let first = first_thread.join().expect("join first generator");
    let second = second_thread.join().expect("join second generator");

    let outcomes = [&first, &second];
    assert_eq!(
        outcomes.iter().filter(|out| out.status.success()).count(),
        1,
        "one workspace-keyed normalized-name lease must exclude cross-directory races"
    );
    assert_eq!(
        usize::from(project.join(".cortex/one/outside-home-race.md").is_file())
            + usize::from(project.join(".cortex/two/outside-home-race.md").is_file()),
        1,
        "the successful generation must remain discoverable in one configured target"
    );
    assert!(
        !project.join(".cortex/agents").exists(),
        "generation must not create a project-local .cortex/agents directory solely for locking"
    );

    let lease_dir = state_dir
        .join("generated-agent-leases")
        .join(checkpoints::workspace_key(&project));
    assert!(
        lease_dir.is_dir(),
        "leases must use the writable Cortex state directory: {}",
        lease_dir.display()
    );
    let lease_entries = fs::read_dir(&lease_dir)
        .expect("read released lease directory")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        !lease_entries
            .iter()
            .any(|entry| entry.starts_with(".cortex-generate-") && entry.ends_with(".lock")),
        "leases must be removed after generation: {lease_entries:?}"
    );
    assert_eq!(
        fixture.finish(),
        2,
        "both generators must reach the deterministic loopback provider"
    );

    let _ = fs::remove_dir_all(&home);
    let _ = fs::remove_dir_all(&project);
}

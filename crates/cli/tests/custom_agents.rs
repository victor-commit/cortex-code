//! Custom markdown agents, generation, and selection.
//!
//! Offline tests prove discovery/error handling without a key. Live tests are gated on
//! `OPENROUTER_API_KEY` and exercise real OpenRouter turns for the user-facing behavior.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
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
Every final answer must include the exact token CUSTOM-AGENT-OK. If asked to create a file, call the write tool first, then include CUSTOM-AGENT-OK in the final answer.
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
    assert!(
        doc["content"]
            .as_str()
            .is_some_and(|content| content.contains("CUSTOM-AGENT-OK")),
        "frontmatter body/system prompt should govern content: {stdout}"
    );
    assert!(
        stdout.contains("permission denied"),
        "denied write result should be visible in machine output: {stdout}"
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

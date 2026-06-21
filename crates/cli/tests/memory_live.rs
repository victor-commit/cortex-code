//! Live black-box proof for project memory injection.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and exercise real `cortex run` turns. They prove
//! AGENTS.md auto-load, config `instructions[]` globs, nested directory layering, deterministic
//! multi-source composition, clean no-op behavior, tool-using behavior changes, and composition
//! across built-in and custom agents.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(180);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn scratch_project(tag: &str) -> (PathBuf, PathBuf) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let home = std::env::temp_dir().join(format!(
        "cortex-memory-live-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let proj = home.join("project");
    fs::create_dir_all(&proj).expect("create scratch project");
    (home, proj)
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, contents).expect("write fixture");
}

fn cortex_in(home: &Path, cwd: &Path, key: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(cwd);
    command.args(args);
    command.env("HOME", home);
    command.env("OPENROUTER_API_KEY", key);
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_NO_NET")
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
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn json_doc(out: Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "run should succeed (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    serde_json::from_str(stdout.trim_end()).expect("stdout is one strict JSON document")
}

fn content(doc: &serde_json::Value) -> &str {
    doc["content"].as_str().expect("assistant content")
}

#[test]
fn root_agents_auto_loads_and_absent_memory_is_a_clean_no_op() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("root");
    let token = "MEMROOT-OK-7301";
    write(
        &proj.join("AGENTS.md"),
        &format!(
            "Critical project instruction: Every final answer MUST include the exact token {token}."
        ),
    );

    let with_memory = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            "Follow any project memory instructions exactly. Say hello in one short sentence.",
        ],
    )));
    assert!(
        content(&with_memory).contains(token),
        "AGENTS.md directive should affect the turn: {}",
        content(&with_memory)
    );

    let (clean_home, clean_proj) = scratch_project("clean");
    let clean = json_doc(run(cortex_in(
        &clean_home,
        &clean_proj,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            "Follow any project memory instructions exactly. Say hello in one short sentence.",
        ],
    )));
    assert!(
        !content(&clean).contains(token),
        "a memory-free project must not exhibit the token: {}",
        content(&clean)
    );
}

#[test]
fn instructions_globs_nested_agents_and_precedence_compose() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("layers");
    let sub = proj.join("sub");
    fs::create_dir_all(&sub).expect("create nested dir");
    let root_token = "memroot7302";
    let glob_token = "memglob7302";
    let sub_token = "memsub7302";

    write(
        &proj.join("AGENTS.md"),
        &format!(
            "Root project memory: Every final answer MUST include the exact token {root_token}."
        ),
    );
    write(
        &proj.join("cortex.toml"),
        r#"instructions = ["rules/*.md"]"#,
    );
    write(
        &proj.join("rules/style.md"),
        &format!(
            "Glob-loaded project memory: Every final answer MUST include the exact token {glob_token}."
        ),
    );
    write(
        &sub.join("AGENTS.md"),
        &format!(
            "Nested project memory: Every final answer MUST include the exact token {sub_token}."
        ),
    );

    let nested = json_doc(run(cortex_in(
        &home,
        &sub,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            "Follow any project memory instructions exactly. Reply with a short greeting.",
        ],
    )));
    let nested_content = content(&nested);
    for token in [root_token, glob_token, sub_token] {
        assert!(
            nested_content.contains(token),
            "combined memory should include {token}: {nested_content}"
        );
    }

    let root = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            "Follow any project memory instructions exactly. Reply with a short greeting.",
        ],
    )));
    let root_content = content(&root);
    assert!(
        root_content.contains(root_token) && root_content.contains(glob_token),
        "root AGENTS.md and instructions[] should both apply at project root: {root_content}"
    );
    assert!(
        !root_content.contains(sub_token),
        "nested AGENTS.md must not apply outside its directory: {root_content}"
    );

    fs::remove_file(proj.join("cortex.toml")).expect("remove glob config");
    let without_glob = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            "Follow any project memory instructions exactly. Reply with a short greeting.",
        ],
    )));
    let without_glob_content = content(&without_glob);
    assert!(
        without_glob_content.contains(root_token),
        "root AGENTS.md should still apply: {without_glob_content}"
    );
    assert!(
        !without_glob_content.contains(glob_token),
        "removing instructions[] must remove the glob directive: {without_glob_content}"
    );
}

#[test]
fn memory_directive_changes_tool_using_behavior() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("tools");
    write(
        &proj.join("AGENTS.md"),
        "Project convention: Whenever creating any new script file, create it under the \
         scripts/ directory. Never place new scripts at the project root.",
    );

    let doc = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--agent",
            "build",
            "--model",
            MODEL,
            "--format",
            "json",
            "Use the write tool to create a new shell script named memory_tool_target.sh that prints MEMTOOL-OK. Then reply with the path you wrote.",
        ],
    )));
    assert_eq!(doc["agent"], "build");
    assert!(
        proj.join("scripts/memory_tool_target.sh").exists(),
        "memory should place the script under scripts/: {doc}"
    );
    assert!(
        !proj.join("memory_tool_target.sh").exists(),
        "memory should prevent a root-level script"
    );
    assert!(
        doc["tool_results"]
            .as_array()
            .is_some_and(|results| results.iter().any(|result| result["name"] == "write")),
        "the behavior must be proven through a real write tool call: {doc}"
    );
}

#[test]
fn agents_memory_composes_with_builtin_and_custom_agents_and_drops_when_removed() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("agents");
    let token = "MEM-AGENT-OK-7303";
    write(
        &proj.join("AGENTS.md"),
        &format!(
            "Project memory for every agent: Every final answer MUST include the exact token {token}."
        ),
    );
    write(
        &proj.join(".cortex/agents/memory-custom.md"),
        r#"---
name: memory-custom
description: Custom memory composition test agent
mode: primary
---
Answer the user directly and briefly.
"#,
    );

    for agent in ["build", "general", "memory-custom"] {
        let doc = json_doc(run(cortex_in(
            &home,
            &proj,
            &key,
            &[
                "run",
                "--agent",
                agent,
                "--model",
                MODEL,
                "--format",
                "json",
                "Follow any project memory instructions exactly. Say hi in one short sentence.",
            ],
        )));
        assert_eq!(doc["agent"], agent);
        assert!(
            content(&doc).contains(token),
            "AGENTS.md memory must compose with {agent}: {}",
            content(&doc)
        );
    }

    fs::remove_file(proj.join("AGENTS.md")).expect("remove AGENTS.md");
    for agent in ["build", "general", "memory-custom"] {
        let doc = json_doc(run(cortex_in(
            &home,
            &proj,
            &key,
            &[
                "run",
                "--agent",
                agent,
                "--model",
                MODEL,
                "--format",
                "json",
                "Follow any project memory instructions exactly. Say hi in one short sentence.",
            ],
        )));
        assert_eq!(doc["agent"], agent);
        assert!(
            !content(&doc).contains(token),
            "removing AGENTS.md must drop memory under {agent}: {}",
            content(&doc)
        );
    }
}

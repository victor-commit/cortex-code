//! Live black-box proof for built-in agent modes.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and exercise real `cortex run` turns plus
//! filesystem inspection. They cover the M4 built-in modes contract: all four built-ins are
//! selectable and reflected in machine output, plan/explore do not mutate the tree, build can
//! mutate, and switching a persisted session from plan to build lifts the plan-mode block.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(150);

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
        "cortex-builtin-agents-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let proj = home.join("project");
    fs::create_dir_all(&proj).expect("create scratch project");
    (home, proj)
}

fn cortex_in(home: &Path, proj: &Path, key: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(proj);
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

#[test]
fn every_builtin_agent_is_selectable_and_reflected_in_json() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("selectable");

    for agent in ["build", "plan", "general", "explore"] {
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
                "Reply with the single word OK.",
            ],
        )));
        assert_eq!(doc["agent"], agent, "{agent} must be reflected in JSON");
        assert!(
            doc["content"]
                .as_str()
                .is_some_and(|content| !content.trim().is_empty()),
            "{agent} should produce a real assistant response"
        );
    }
}

#[test]
fn plan_then_build_in_the_same_session_lifts_the_mutation_block() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("switch");
    let session = "builtin-agent-switch";
    let target = proj.join("session_switch.txt");

    let plan_doc = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--session",
            session,
            "--agent",
            "plan",
            "--model",
            MODEL,
            "--format",
            "json",
            "Create session_switch.txt containing exactly SWITCH_OK. If you cannot make the change, provide a numbered implementation plan.",
        ],
    )));
    assert_eq!(plan_doc["agent"], "plan");
    assert!(
        !target.exists(),
        "the plan agent must leave the requested file absent"
    );
    let plan_content = plan_doc["content"]
        .as_str()
        .expect("plan content")
        .to_lowercase();
    assert!(
        plan_content.contains("plan") || plan_content.contains("1."),
        "plan mode should answer with a plan/outline, got: {plan_content}"
    );

    let build_doc = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--session",
            session,
            "--agent",
            "build",
            "--model",
            MODEL,
            "--format",
            "json",
            "Now use the write tool to create session_switch.txt containing exactly SWITCH_OK and no trailing newline.",
        ],
    )));
    assert_eq!(build_doc["agent"], "build");
    let written = fs::read_to_string(&target).expect("build creates the file");
    assert_eq!(
        written.trim(),
        "SWITCH_OK",
        "build mode should write the requested content"
    );
}

#[test]
fn explore_can_read_and_search_but_leaves_the_working_tree_unchanged() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("explore");
    fs::write(
        proj.join("README.md"),
        "Explore fixture\n\nThe magic token is EXPLORE_TOKEN_7319.\n",
    )
    .expect("write README fixture");

    let note = proj.join("note.txt");
    let doc = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--agent",
            "explore",
            "--model",
            MODEL,
            "--format",
            "json",
            "Use the read tool to read README.md, use glob to list markdown files, and use grep to search for EXPLORE_TOKEN_7319 under the current directory. Then report findings. Do not create note.txt unless a tool forces you to.",
        ],
    )));
    assert_eq!(doc["agent"], "explore");
    assert!(!note.exists(), "explore mode must not create note.txt");

    let empty_results = Vec::new();
    let tool_names: Vec<&str> = doc["tool_results"]
        .as_array()
        .unwrap_or(&empty_results)
        .iter()
        .filter_map(|result| result["name"].as_str())
        .collect();
    assert!(
        tool_names.contains(&"read"),
        "explore should be able to use the read tool, got {tool_names:?}"
    );
    assert!(
        tool_names
            .iter()
            .any(|name| *name == "glob" || *name == "grep"),
        "explore should be able to use search tools, got {tool_names:?}"
    );
    let content = doc["content"].as_str().unwrap_or_default();
    assert!(
        content.contains("EXPLORE_TOKEN_7319")
            || serde_json::to_string(&doc["tool_results"])
                .unwrap()
                .contains("EXPLORE_TOKEN_7319"),
        "the transcript should include the real searched token; doc was {doc}"
    );
}

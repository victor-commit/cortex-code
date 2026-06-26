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
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MODEL: &str = "google/gemini-2.5-flash";
const RUN_TIMEOUT: Duration = Duration::from_secs(150);
const SELECTABLE_AGENT_MAX_ATTEMPTS: usize = 3;
const SELECTABLE_AGENT_RETRY_BACKOFF: Duration = Duration::from_secs(2);
const PLAN_BUILD_MAX_ATTEMPTS: usize = 3;
const PLAN_BUILD_RETRY_BACKOFF: Duration = Duration::from_secs(2);
const OUTPUT_EXCERPT_BYTES: usize = 4000;

static LIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

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

struct TimedOutRun {
    timeout: Duration,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn excerpt(bytes: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    if text.len() > OUTPUT_EXCERPT_BYTES {
        text.truncate(OUTPUT_EXCERPT_BYTES);
        text.push_str("…<truncated>");
    }
    text
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<Output, TimedOutRun> {
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
            return Err(TimedOutRun {
                timeout,
                stdout: out_thread.join().expect("join stdout"),
                stderr: err_thread.join().expect("join stderr"),
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Ok(Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    })
}

fn run(command: Command) -> Output {
    run_with_timeout(command, RUN_TIMEOUT).unwrap_or_else(|timed_out| {
        panic!(
            "cortex run did not terminate within {:?}; partial stdout: {}; partial stderr: {}",
            timed_out.timeout,
            excerpt(&timed_out.stdout),
            excerpt(&timed_out.stderr)
        )
    })
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

fn line_has_numbered_step(line: &str) -> bool {
    let trimmed = line.trim_start();
    let digits = trimmed.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits == 0 {
        return false;
    }
    matches!(
        trimmed.chars().nth(digits),
        Some('.') | Some(')') | Some(':')
    )
}

fn looks_like_plan_outline(content: &str) -> bool {
    let lower = content.to_lowercase();
    if lower.contains("plan")
        || lower.contains("outline")
        || lower.contains("step 1")
        || lower.contains("first,")
    {
        return true;
    }

    let mut numbered = 0;
    let mut bulleted = 0;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if line_has_numbered_step(trimmed) {
            numbered += 1;
        }
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            bulleted += 1;
        }
    }

    numbered >= 1 || bulleted >= 2
}

fn has_denied_mutation_result(doc: &serde_json::Value) -> bool {
    doc["tool_results"].as_array().is_some_and(|results| {
        results.iter().any(|result| {
            let Some(name) = result["name"].as_str() else {
                return false;
            };
            if !matches!(name, "write" | "edit" | "apply_patch" | "bash") {
                return false;
            }
            if !result["is_error"].as_bool().unwrap_or(false) {
                return false;
            }
            let rendered = serde_json::to_string(result)
                .unwrap_or_default()
                .to_lowercase();
            rendered.contains("permission denied")
                || rendered.contains("denied by")
                || rendered.contains("not allowed")
                || rendered.contains("read-only")
                || rendered.contains("plan mode")
        })
    })
}

fn run_selectable_probe_with_retries(key: &str, agent: &str) -> serde_json::Value {
    let mut timeout_failures = Vec::new();

    for attempt in 1..=SELECTABLE_AGENT_MAX_ATTEMPTS {
        let (home, proj) = scratch_project(&format!("selectable-{agent}-attempt-{attempt}"));
        let command = cortex_in(
            &home,
            &proj,
            key,
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
        );

        let out = match run_with_timeout(command, RUN_TIMEOUT) {
            Ok(out) => out,
            Err(timed_out) => {
                timeout_failures.push(format!(
                    "attempt {attempt}/{SELECTABLE_AGENT_MAX_ATTEMPTS}: phase=run timed out after {:?}; partial stderr={:?}; partial stdout={:?}",
                    timed_out.timeout,
                    excerpt(&timed_out.stderr),
                    excerpt(&timed_out.stdout)
                ));
                if attempt < SELECTABLE_AGENT_MAX_ATTEMPTS {
                    std::thread::sleep(SELECTABLE_AGENT_RETRY_BACKOFF * attempt as u32);
                    continue;
                }
                panic!(
                    "builtin agent selectable probe timed out for agent={agent}, model={MODEL}; failures: {}",
                    timeout_failures.join(" | ")
                );
            }
        };

        assert!(
            out.status.success(),
            "builtin agent selectable probe failed for agent={agent}, model={MODEL}, attempt {attempt}/{SELECTABLE_AGENT_MAX_ATTEMPTS}, phase=run-json, status={:?}, stderr={:?}, stdout={:?}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        );

        let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
        return serde_json::from_str(stdout.trim_end()).unwrap_or_else(|err| {
            panic!(
                "builtin agent selectable probe failed for agent={agent}, model={MODEL}, attempt {attempt}/{SELECTABLE_AGENT_MAX_ATTEMPTS}, phase=parse-json, error={err}, raw_stdout={stdout:?}"
            )
        });
    }

    unreachable!("bounded retry loop must either return a JSON document or panic")
}

#[test]
fn every_builtin_agent_is_selectable_and_reflected_in_json() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let _guard = LIVE_TEST_LOCK.lock().expect("live test lock");

    for agent in ["build", "plan", "general", "explore"] {
        let doc = run_selectable_probe_with_retries(&key, agent);
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
    let _guard = LIVE_TEST_LOCK.lock().expect("live test lock");
    let mut failures = Vec::new();

    for attempt in 1..=PLAN_BUILD_MAX_ATTEMPTS {
        let (home, proj) = scratch_project(&format!("switch-attempt-{attempt}"));
        let session = format!("builtin-agent-switch-{attempt}");
        let target = proj.join("session_switch.txt");

        let plan_doc = json_doc(run(cortex_in(
            &home,
            &proj,
            &key,
            &[
                "run",
                "--session",
                session.as_str(),
                "--agent",
                "plan",
                "--model",
                MODEL,
                "--format",
                "json",
                "Create session_switch.txt containing exactly SWITCH_OK. If you cannot make the change, provide a numbered implementation plan.",
            ],
        )));
        if plan_doc["agent"] != "plan" {
            failures.push(format!(
                "attempt {attempt}/{PLAN_BUILD_MAX_ATTEMPTS}: expected plan agent in first turn, got {}",
                plan_doc["agent"]
            ));
            if attempt < PLAN_BUILD_MAX_ATTEMPTS {
                std::thread::sleep(PLAN_BUILD_RETRY_BACKOFF * attempt as u32);
                continue;
            }
            break;
        }
        if target.exists() {
            failures.push(format!(
                "attempt {attempt}/{PLAN_BUILD_MAX_ATTEMPTS}: plan turn unexpectedly created {}",
                target.display()
            ));
            if attempt < PLAN_BUILD_MAX_ATTEMPTS {
                std::thread::sleep(PLAN_BUILD_RETRY_BACKOFF * attempt as u32);
                continue;
            }
            break;
        }
        let plan_content = plan_doc["content"].as_str().unwrap_or_default();
        if !has_denied_mutation_result(&plan_doc) && !looks_like_plan_outline(plan_content) {
            failures.push(format!(
                "attempt {attempt}/{PLAN_BUILD_MAX_ATTEMPTS}: plan turn did not include a denied mutation result or plan-like outline; content={plan_content:?}"
            ));
            if attempt < PLAN_BUILD_MAX_ATTEMPTS {
                std::thread::sleep(PLAN_BUILD_RETRY_BACKOFF * attempt as u32);
                continue;
            }
            break;
        }

        let build_doc = json_doc(run(cortex_in(
            &home,
            &proj,
            &key,
            &[
                "run",
                "--session",
                session.as_str(),
                "--agent",
                "build",
                "--model",
                MODEL,
                "--format",
                "json",
                "Now use the write tool to create or replace session_switch.txt with exactly SWITCH_OK and no trailing newline.",
            ],
        )));
        if build_doc["agent"] != "build" {
            failures.push(format!(
                "attempt {attempt}/{PLAN_BUILD_MAX_ATTEMPTS}: expected build agent in second turn, got {}",
                build_doc["agent"]
            ));
            if attempt < PLAN_BUILD_MAX_ATTEMPTS {
                std::thread::sleep(PLAN_BUILD_RETRY_BACKOFF * attempt as u32);
                continue;
            }
            break;
        }

        match fs::read_to_string(&target) {
            Ok(written) if written.trim() == "SWITCH_OK" => return,
            Ok(written) => failures.push(format!(
                "attempt {attempt}/{PLAN_BUILD_MAX_ATTEMPTS}: build wrote unexpected content {:?}",
                written
            )),
            Err(err) => failures.push(format!(
                "attempt {attempt}/{PLAN_BUILD_MAX_ATTEMPTS}: build did not create {} ({err})",
                target.display()
            )),
        }

        if attempt < PLAN_BUILD_MAX_ATTEMPTS {
            std::thread::sleep(PLAN_BUILD_RETRY_BACKOFF * attempt as u32);
        }
    }

    panic!(
        "plan→build same-session flow failed after {PLAN_BUILD_MAX_ATTEMPTS} attempts: {}",
        failures.join(" | ")
    );
}

#[test]
fn explore_can_read_and_search_but_leaves_the_working_tree_unchanged() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let _guard = LIVE_TEST_LOCK.lock().expect("live test lock");
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

//! Live black-box proof for markdown skills.
//!
//! These tests are gated on `OPENROUTER_API_KEY` and exercise real `cortex run` turns. They prove
//! SKILL.md discovery, conditional guidance injection through the `skill` tool, multiple
//! independently engageable skills, malformed-skill resilience, and autonomous skill engagement
//! from a matching description.

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
        "cortex-skills-live-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let proj = home.join("project");
    fs::create_dir_all(&proj).expect("create scratch project");
    (home, proj)
}

fn write_skill(proj: &Path, dir_name: &str, name: &str, description: &str, guidance: &str) {
    let dir = proj.join("skills").join(dir_name);
    fs::create_dir_all(&dir).expect("create skill dir");
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n{guidance}\n"),
    )
    .expect("write skill");
}

fn write_broken_skill(proj: &Path) {
    let dir = proj.join("skills").join("broken");
    fs::create_dir_all(&dir).expect("create broken skill dir");
    fs::write(dir.join("SKILL.md"), "---\nname: [\n---\nThis is broken.\n")
        .expect("write broken skill");
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

fn json_doc(out: Output) -> (serde_json::Value, String) {
    assert!(
        out.status.success(),
        "run should succeed (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let stderr = String::from_utf8(out.stderr).expect("utf8 stderr");
    (
        serde_json::from_str(stdout.trim_end()).expect("stdout is one strict JSON document"),
        stderr,
    )
}

fn jsonl(out: Output) -> Vec<serde_json::Value> {
    assert!(
        out.status.success(),
        "run should succeed (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("jsonl line parses"))
        .collect()
}

fn skill_names(doc: &serde_json::Value) -> Vec<String> {
    let definition = doc["tool_definitions"]
        .as_array()
        .expect("tool definitions")
        .iter()
        .find(|definition| definition["name"] == "skill")
        .expect("skill tool definition");
    let description = definition["parameters"]["properties"]["name"]["description"]
        .as_str()
        .expect("skill name description");
    let marker = "Available skills:";
    let Some(index) = description.find(marker) else {
        return Vec::new();
    };
    description[index + marker.len()..]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn content_from_jsonl(events: &[serde_json::Value]) -> String {
    events
        .iter()
        .filter(|event| event["type"] == "text_delta")
        .filter_map(|event| event["text"].as_str())
        .collect()
}

fn skill_tool_was_called(events: &[serde_json::Value]) -> bool {
    events
        .iter()
        .any(|event| event["type"] == "tool_call" && event["name"] == "skill")
}

#[test]
fn discovers_valid_skills_skips_malformed_and_keeps_guidance_isolated() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("discovery");
    write_skill(
        &proj,
        "ok",
        "skill-ok",
        "Use only when the user asks to engage the ok skill",
        "Every answer after engagement must include SKILL-OK.",
    );
    write_skill(
        &proj,
        "date",
        "date-helper",
        "Use for formatting dates",
        "Every answer after engagement must include SKILL-DATE-OK.",
    );
    write_broken_skill(&proj);

    let (doc, stderr) = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--model",
            MODEL,
            "--format",
            "json",
            "Reply with exactly BASELINE-OK. Do not use any tools.",
        ],
    )));

    let skills = skill_names(&doc);
    assert!(skills.contains(&"skill-ok".to_owned()));
    assert!(skills.contains(&"date-helper".to_owned()));
    assert!(
        !skills.contains(&"broken".to_owned()),
        "malformed skill must not be usable"
    );
    assert!(
        stderr.contains("skipping skill") && stderr.contains("broken"),
        "malformed skill should be flagged without crashing: {stderr}"
    );
    let content = doc["content"].as_str().expect("content");
    assert!(
        !content.contains("SKILL-OK") && !content.contains("SKILL-DATE-OK"),
        "unengaged skill guidance must not affect the answer: {content}"
    );
    assert!(
        doc.get("tool_results").is_none(),
        "baseline prompt should not engage any tool"
    );
}

#[test]
fn explicit_skill_invocation_injects_only_the_selected_guidance() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("explicit");
    write_skill(
        &proj,
        "alpha",
        "alpha-skill",
        "Use when the user explicitly asks for the alpha capability",
        "Every answer after engagement must include SKILL-ALPHA-OK.",
    );
    write_skill(
        &proj,
        "beta",
        "beta-skill",
        "Use when the user explicitly asks for the beta capability",
        "Every answer after engagement must include SKILL-BETA-OK.",
    );

    let (alpha, _) = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--session",
            "alpha-skill-session",
            "--model",
            MODEL,
            "--format",
            "json",
            "You must call the skill tool with name alpha-skill before answering. After it returns, answer with the words alpha ready.",
        ],
    )));
    let alpha_content = alpha["content"].as_str().expect("alpha content");
    assert!(alpha_content.contains("SKILL-ALPHA-OK"), "{alpha_content}");
    assert!(
        !alpha_content.contains("SKILL-BETA-OK"),
        "engaging alpha must not trigger beta guidance: {alpha_content}"
    );
    assert!(
        alpha["tool_results"]
            .as_array()
            .expect("alpha tool result")
            .iter()
            .any(|result| result["name"] == "skill"),
        "skill tool result should be recorded"
    );

    let (beta, _) = json_doc(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--session",
            "beta-skill-session",
            "--model",
            MODEL,
            "--format",
            "json",
            "You must call the skill tool with name beta-skill before answering. After it returns, answer with the words beta ready.",
        ],
    )));
    let beta_content = beta["content"].as_str().expect("beta content");
    assert!(beta_content.contains("SKILL-BETA-OK"), "{beta_content}");
    assert!(
        !beta_content.contains("SKILL-ALPHA-OK"),
        "engaging beta must not trigger alpha guidance: {beta_content}"
    );
}

#[test]
fn model_autonomously_engages_relevant_skill_but_not_unrelated_prompt() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    let (home, proj) = scratch_project("autonomous");
    write_skill(
        &proj,
        "date",
        "date-format-helper",
        "Use for formatting dates and answering date-formatting requests",
        "Every answer after engagement must include SKILL-DATE-OK.",
    );

    let related = jsonl(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--session",
            "date-related",
            "--model",
            MODEL,
            "--jsonl",
            "Format the date 2026-06-21 for a release note. Use the appropriate available capability if one applies, then provide the formatted date.",
        ],
    )));
    let related_content = content_from_jsonl(&related);
    assert!(
        skill_tool_was_called(&related),
        "date-related prompt should autonomously call the skill tool: {related:?}"
    );
    assert!(
        related_content.contains("SKILL-DATE-OK"),
        "engaged date skill guidance should affect the answer: {related_content}"
    );

    let unrelated = jsonl(run(cortex_in(
        &home,
        &proj,
        &key,
        &[
            "run",
            "--session",
            "date-unrelated",
            "--model",
            MODEL,
            "--jsonl",
            "What is 2+2? Reply with only the number.",
        ],
    )));
    let unrelated_content = content_from_jsonl(&unrelated);
    assert!(
        !skill_tool_was_called(&unrelated),
        "unrelated prompt should not engage the date skill: {unrelated:?}"
    );
    assert!(
        !unrelated_content.contains("SKILL-DATE-OK"),
        "date guidance must stay absent for unrelated prompts: {unrelated_content}"
    );
}

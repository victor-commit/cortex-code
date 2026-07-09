//! Live end-to-end LSP diagnostics proof through `cortex run` tool results.
//!
//! These tests use real OpenRouter turns plus the real Ruff language server. They skip cleanly when
//! the API key is absent or Ruff is not installed.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn has_ruff() -> bool {
    Command::new("ruff")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cli-lsp-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn cortex_in(cwd: &Path, key: &str, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex"));
    c.current_dir(cwd);
    c.args(args);
    c.env("OPENROUTER_API_KEY", key);
    c.env("CORTEX_DATA_DIR", cwd.join(".data"));
    c.env_remove("CORTEX_NO_NET")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    c
}

fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
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
            panic!("cortex run timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn tool_text(doc: &serde_json::Value, tool: &str) -> String {
    doc["tool_results"]
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter(|result| result["name"] == tool)
        .flat_map(|result| result["parts"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn write_tool_reinjects_real_ruff_diagnostics_and_clean_file_has_none() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    if !has_ruff() {
        eprintln!("skipping live test: ruff is not on PATH");
        return;
    }

    let dir = temp_dir("write");
    let prompt = "Use the write tool exactly once to create bad.py with this exact content: import os\\nprint(undefined_name)\\n. Then stop.";
    let out = run(
        cortex_in(&dir, &key, &["run", "--format", "json", prompt]),
        Duration::from_secs(120),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json output");
    let text = tool_text(&doc, "write");
    assert!(
        text.contains("Diagnostics:"),
        "tool result should include diagnostics: {text}"
    );
    assert!(
        text.contains("Undefined name `undefined_name`"),
        "real Ruff diagnostic missing: {text}"
    );
    assert!(
        text.contains("error [F821] (Ruff) at line 2"),
        "severity/source/code/range missing: {text}"
    );
    assert!(
        text.contains("warning [F401] (Ruff) at line 1"),
        "warning diagnostic missing: {text}"
    );

    let clean_prompt = "Use the write tool exactly once to create clean.py with this exact content: print('ok')\\n. Then stop.";
    let clean = run(
        cortex_in(&dir, &key, &["run", "--format", "json", clean_prompt]),
        Duration::from_secs(120),
    );
    assert_eq!(
        clean.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let clean_doc: serde_json::Value = serde_json::from_slice(&clean.stdout).expect("json output");
    let clean_text = tool_text(&clean_doc, "write");
    assert!(
        !clean_text.contains("Diagnostics:"),
        "clean file must not report diagnostics: {clean_text}"
    );
}

#[test]
fn real_turn_edit_fix_loop_clears_reinjected_ruff_diagnostics() {
    let Some(key) = live_key() else {
        eprintln!("skipping live test: OPENROUTER_API_KEY not set");
        return;
    };
    if !has_ruff() {
        eprintln!("skipping live test: ruff is not on PATH");
        return;
    }

    let dir = temp_dir("edit-fix-loop");
    let session = format!(
        "lsp-edit-fix-loop-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let introduce = run(
        cortex_in(
            &dir,
            &key,
            &[
                "run",
                "--format",
                "json",
                "--session",
                &session,
                "Use the write tool exactly once to create loop.py with this exact content: import os\\nprint(undefined_name)\\n. Then stop.",
            ],
        ),
        Duration::from_secs(120),
    );
    assert_eq!(
        introduce.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&introduce.stderr)
    );
    let introduce_doc: serde_json::Value =
        serde_json::from_slice(&introduce.stdout).expect("introduce json output");
    let introduce_text = tool_text(&introduce_doc, "write");
    assert!(
        introduce_text.contains("Undefined name `undefined_name`"),
        "the real Ruff diagnostic must reach the model through write: {introduce_text}"
    );

    let fix = run(
        cortex_in(
            &dir,
            &key,
            &[
                "run",
                "--format",
                "json",
                "--session",
                &session,
                "First use read on loop.py because edit requires a current read. Then use edit to replace the exact full content import os\\nprint(undefined_name)\\n with print('ok')\\n. Do not use write or bash. Then stop.",
            ],
        ),
        Duration::from_secs(120),
    );
    assert_eq!(
        fix.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&fix.stderr)
    );
    let fix_doc: serde_json::Value = serde_json::from_slice(&fix.stdout).expect("fix json output");
    let fix_text = tool_text(&fix_doc, "edit");
    assert!(
        !fix_text.is_empty(),
        "the follow-up real turn must use edit after seeing the prior diagnostic: {fix_doc}"
    );
    assert!(
        !fix_text.contains("Diagnostics:"),
        "the corrected edit must clear diagnostics in the model-visible result: {fix_text}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("loop.py")).expect("read corrected file"),
        "print('ok')\n",
        "the follow-up edit must persist the diagnostic-driven correction"
    );

    let mut clean_command = cortex_in(
        &dir,
        &key,
        &["lsp", "diagnostics", "loop.py", "--format", "json"],
    );
    clean_command.env("CORTEX_NO_NET", "1");
    let clean = run(clean_command, Duration::from_secs(30));
    assert_eq!(
        clean.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let clean_doc: serde_json::Value =
        serde_json::from_slice(&clean.stdout).expect("clean lsp json output");
    assert_eq!(
        clean_doc["status"], "clean",
        "a fresh pre-installed Ruff server must confirm the corrected file is clean: {clean_doc}"
    );
    assert!(
        clean_doc["diagnostics"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "the corrected file must have no stale diagnostics: {clean_doc}"
    );
}

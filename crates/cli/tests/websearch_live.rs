//! Live black-box proof for the config-enabled `websearch` tool through `cortex run`.
//!
//! The test is gated on `OPENROUTER_API_KEY`: without a key it skips cleanly, while mission
//! validation runs it with the real key. The enabled path uses the real DuckDuckGo provider over
//! the network and records a `websearch` tool result. The disabled path uses a fresh project with
//! no websearch config and proves the tool is absent from the advertised tool set.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn live_key() -> Option<String> {
    match std::env::var("OPENROUTER_API_KEY") {
        Ok(key) if !key.trim().is_empty() => Some(key),
        _ => None,
    }
}

fn isolated_home(label: &str) -> PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "cortex-websearch-live-{label}-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".config")).expect("create config dir");
    std::fs::create_dir_all(root.join(".local/share")).expect("create data dir");
    std::fs::create_dir_all(root.join("project")).expect("create project dir");
    root
}

fn cortex(home: &std::path::Path, key: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(home.join("project"));
    command.args(args);
    command.env("OPENROUTER_API_KEY", key);
    command.env("HOME", home);
    command.env("XDG_CONFIG_HOME", home.join(".config"));
    command.env("XDG_DATA_HOME", home.join(".local/share"));
    command.env("CORTEX_DATA_DIR", home.join(".local/share/cortex"));
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("CORTEX_WEBSEARCH_ENABLED")
        .env_remove("CORTEX_WEBSEARCH_PROVIDER")
        .env_remove("CORTEX_WEBSEARCH_API_KEY")
        .env_remove("CORTEX_WEBSEARCH_ENDPOINT")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    command
}

fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut bytes);
        bytes
    });
    let stderr_thread = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut bytes);
        bytes
    });

    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex run did not terminate within {timeout:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

#[test]
fn real_openrouter_turn_uses_config_enabled_websearch() {
    let Some(key) = live_key() else {
        eprintln!("skipping websearch live proof: OPENROUTER_API_KEY is not set");
        return;
    };

    let enabled_home = isolated_home("enabled");
    std::fs::write(
        enabled_home.join("project/cortex.toml"),
        "[websearch]\nenabled = true\nprovider = \"duckduckgo\"\nmax_results = 5\n",
    )
    .expect("write websearch config");
    let prompt = "You must call the websearch tool exactly once with the query \
                  \"Rust programming language\". After reading the results, reply with exactly \
                  one result URL about Rust and nothing else.";
    let output = run(
        cortex(&enabled_home, &key, &["run", "--format", "json", prompt]),
        Duration::from_secs(180),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("run JSON should parse");
    let tools = doc["tools"].as_array().expect("tools array");
    assert!(
        tools.iter().any(|name| name == "websearch"),
        "enabled run must advertise websearch: {doc}"
    );
    let tool_results = doc["tool_results"].as_array().expect("tool results array");
    let websearch_result = tool_results
        .iter()
        .find(|result| result["name"] == "websearch")
        .expect("websearch tool call recorded");
    assert_eq!(websearch_result["is_error"], false, "{doc}");
    let tool_text = serde_json::to_string(websearch_result).expect("tool JSON");
    assert!(tool_text.contains("Rust"), "{tool_text}");
    assert!(tool_text.contains("http"), "{tool_text}");
    let answer = doc["content"].as_str().expect("content string");
    assert!(
        answer.contains("http") || answer.to_ascii_lowercase().contains("rust"),
        "assistant answer should be grounded in the real search result: {doc}"
    );

    let disabled_home = isolated_home("disabled");
    let disabled = run(
        cortex(
            &disabled_home,
            &key,
            &["run", "--format", "json", "Reply with the single word OK."],
        ),
        Duration::from_secs(120),
    );
    assert_eq!(
        disabled.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&disabled.stdout),
        String::from_utf8_lossy(&disabled.stderr)
    );
    let disabled_doc: serde_json::Value =
        serde_json::from_slice(&disabled.stdout).expect("disabled JSON should parse");
    assert!(
        !disabled_doc["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|name| name == "websearch"),
        "default config must omit websearch: {disabled_doc}"
    );
    assert!(
        disabled_doc
            .get("tool_results")
            .and_then(serde_json::Value::as_array)
            .map(|results| results.iter().all(|result| result["name"] != "websearch"))
            .unwrap_or(true),
        "disabled run must not call websearch: {disabled_doc}"
    );
}

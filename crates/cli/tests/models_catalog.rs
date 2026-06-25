//! Black-box proof for `cortex models`: the offline catalogue listing drawn from the embedded
//! models.dev snapshot. Covers VAL-FINISH-001..004 and VAL-FINISH-008 — a fully offline listing of
//! >=10 recognizable models that surfaces per-model pricing, context window, and the default model,
//! plus a strict structured JSON form with no human chatter interleaved on stdout.

use std::io::{BufRead, Read};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "cortex-cli-models-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = home.join("project");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&project).expect("create project");
        Self { home, project }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn cache_file(sandbox: &Sandbox) -> PathBuf {
    sandbox.home.join(".local/share/cortex/models.json")
}

fn write_large_cached_catalogue(sandbox: &Sandbox, models: usize) {
    let path = cache_file(sandbox);
    let parent = path.parent().expect("cache parent");
    std::fs::create_dir_all(parent).expect("create cache directory");

    let mut body = String::new();
    body.push_str("{\"openrouter\":{\"id\":\"openrouter\",\"name\":\"OpenRouter\",\"models\":{");
    for idx in 0..models {
        if idx > 0 {
            body.push(',');
        }
        let id = format!("google/epipe-stress-{idx}");
        body.push_str(&format!(
            "\"{id}\":{{\"id\":\"{id}\",\"name\":\"Epipe Stress {idx}\",\"cost\":{{\"input\":0.1,\"output\":0.2}},\"limit\":{{\"context\":8192,\"output\":1024}}}}"
        ));
    }
    body.push_str("}}}");
    std::fs::write(path, body).expect("write cache file");
}

/// Build a `cortex` invocation in an isolated HOME/CWD with no ambient credential or config.
fn cortex(sandbox: &Sandbox, no_net: bool, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("XDG_CONFIG_HOME", sandbox.home.join(".config"));
    command.env("CORTEX_DATA_DIR", sandbox.home.join(".local/share/cortex"));
    command.env_remove("OPENROUTER_API_KEY");
    command.env_remove("CORTEX_OPENROUTER_BASE_URL");
    if no_net {
        command.env("CORTEX_NO_NET", "1");
    } else {
        command.env_remove("CORTEX_NO_NET");
    }
    command
}

fn run(mut command: Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut stdout = child.stdout.take().expect("stdout");
    let mut stderr = child.stderr.take().expect("stderr");
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command did not terminate within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_thread.join().expect("join stdout"),
        stderr: stderr_thread.join().expect("join stderr"),
    }
}

fn run_with_early_stdout_close(mut command: Command, timeout: Duration) -> (Output, String) {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = std::io::BufReader::new(stdout);
    let mut first_line = String::new();
    let _ = reader.read_line(&mut first_line);
    drop(reader);

    let mut stderr = child.stderr.take().expect("stderr");
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("try wait") {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex command did not terminate within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    (
        Output {
            status,
            stdout: Vec::new(),
            stderr: stderr_thread.join().expect("join stderr"),
        },
        first_line,
    )
}

#[test]
fn models_lists_ten_plus_recognizable_ids_offline() {
    let sandbox = Sandbox::new("offline-list");
    let output = run(cortex(&sandbox, true, &["models"]), Duration::from_secs(20));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");

    let model_ids: Vec<&str> = stdout
        .lines()
        .flat_map(|line| line.split_whitespace())
        .filter(|token| {
            token.contains('/')
                && (token.starts_with("google/")
                    || token.starts_with("anthropic/")
                    || token.starts_with("openai/")
                    || token.starts_with("meta-llama/")
                    || token.starts_with("deepseek/"))
        })
        .collect();
    assert!(
        model_ids.len() >= 10,
        "expected >=10 recognizable model ids offline, found {}: {stdout}",
        model_ids.len()
    );
    for vendor in ["google/", "anthropic/", "openai/"] {
        assert!(
            model_ids.iter().any(|id| id.starts_with(vendor)),
            "listing must include a {vendor} model: {stdout}"
        );
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.to_lowercase().contains("network"),
        "offline listing must not emit a network error: {stderr}"
    );
}

#[test]
fn models_text_form_surfaces_default_pricing_and_context() {
    let sandbox = Sandbox::new("text-fields");
    let output = run(cortex(&sandbox, true, &["models"]), Duration::from_secs(20));
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");

    let default_line = stdout
        .lines()
        .find(|line| line.contains(DEFAULT_MODEL))
        .unwrap_or_else(|| panic!("default model must be listed: {stdout}"));
    assert!(
        default_line.to_lowercase().contains("default"),
        "default model line should be marked as the default: {default_line}"
    );
    assert!(
        default_line.contains("ctx"),
        "model line should surface a context window: {default_line}"
    );
    assert!(
        default_line.contains('$'),
        "model line should surface pricing: {default_line}"
    );
}

#[test]
fn models_json_is_strict_structured_and_offline() {
    let sandbox = Sandbox::new("json");
    let output = run(
        cortex(&sandbox, true, &["models", "--format", "json"]),
        Duration::from_secs(20),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The ENTIRE stdout must parse as a single strict JSON document (no chatter interleaved).
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is a single strict JSON document");

    assert_eq!(
        doc["default_model"], DEFAULT_MODEL,
        "top-level default_model must name the run default: {doc}"
    );

    let models: Vec<&serde_json::Value> = doc["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .flat_map(|provider| provider["models"].as_array().expect("models array"))
        .collect();
    assert!(
        models.len() >= 10,
        "json must list >=10 models, found {}",
        models.len()
    );

    // Default model entry exposes id, provider, pricing, and context length.
    let default = models
        .iter()
        .find(|model| model["id"] == DEFAULT_MODEL)
        .expect("default model present in json");
    assert_eq!(default["provider"], "openrouter");
    assert!(
        default["cost"]["input"]
            .as_f64()
            .expect("input price is a number")
            >= 0.0,
        "default input price must be a number >= 0"
    );
    assert!(
        default["cost"]["output"]
            .as_f64()
            .expect("output price is a number")
            >= 0.0,
        "default output price must be a number >= 0"
    );
    let ctx = default["context_length"]
        .as_u64()
        .expect("context_length is an integer");
    assert!(
        ctx >= 8000,
        "default context length must be a real window: {ctx}"
    );
    assert_eq!(default["default"], true, "default flag must be set");

    // At least one paid model with a strictly positive price (real pricing, not placeholder).
    assert!(
        models.iter().any(|model| {
            model["cost"]["input"].as_f64().unwrap_or(0.0) > 0.0
                && model["cost"]["output"].as_f64().unwrap_or(0.0) > 0.0
        }),
        "at least one model must show >0 pricing"
    );

    // Context windows must vary across models, not be a single hard-coded constant.
    let distinct: std::collections::BTreeSet<u64> = models
        .iter()
        .filter_map(|model| model["context_length"].as_u64())
        .collect();
    assert!(
        distinct.len() >= 2,
        "context windows must vary across models: {distinct:?}"
    );
}

#[test]
fn models_text_early_stdout_close_exits_cleanly() {
    let sandbox = Sandbox::new("early-close");
    write_large_cached_catalogue(&sandbox, 4_000);
    let (output, first_line) =
        run_with_early_stdout_close(cortex(&sandbox, true, &["models"]), Duration::from_secs(30));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !first_line.trim().is_empty(),
        "expected to read at least the first line before closing stdout"
    );
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        !stderr.contains("panicked at") && !stderr.contains("rust_backtrace"),
        "must not panic on early stdout close: {stderr}"
    );
    assert!(
        !stderr.contains("broken pipe"),
        "must not print broken-pipe diagnostics: {stderr}"
    );
}

#[test]
fn models_listing_excludes_provider_denied_by_experimental_policy() {
    let sandbox = Sandbox::new("policy-deny");
    std::fs::write(
        sandbox.project.join("cortex.toml"),
        r#"
[experimental]
policies = [{ effect = "deny", action = "provider.use", resource = "openrouter" }]
"#,
    )
    .expect("write policy config");

    let output = run(
        cortex(&sandbox, true, &["models", "--format", "json"]),
        Duration::from_secs(20),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is valid json");
    let providers = doc["providers"].as_array().expect("providers array");
    assert!(
        providers.is_empty(),
        "denied provider should be excluded from model listings: {doc}"
    );
}

//! Live audit redaction proof through the shipped `cortex run` binary.
//!
//! This test uses a real OpenRouter turn and a real JavaScript `shell.env` plugin. It proves the
//! durable audit JSONL remains informative while never persisting the live provider credential,
//! a prompt secret, or an injected shell secret.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(90);
const MODEL: &str = "google/gemini-2.5-flash";

struct Scratch {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
    db: PathBuf,
    audit: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let nonce = unique_suffix();
        let root = std::env::temp_dir().join(format!(
            "cortex-audit-redaction-live-{}-{nonce}",
            std::process::id()
        ));
        let home = root.join("home");
        let project = root.join("project");
        std::fs::create_dir_all(&home).expect("create isolated home");
        std::fs::create_dir_all(&project).expect("create isolated project");
        Self {
            db: root.join("data/cortex.db"),
            audit: root.join("audit.jsonl"),
            root,
            home,
            project,
        }
    }

    fn configure_shell_env_plugin(&self, secret: &str) {
        std::fs::write(
            self.project.join("audit-plugin.mjs"),
            r#"
export default async function plugin(_input, options) {
  return {
    "shell.env": async () => ({
      SECRET: options.secret,
      VISIBLE_AUDIT_FLAG: "present"
    })
  };
}
"#,
        )
        .expect("write real shell.env plugin");
        std::fs::write(
            self.project.join("cortex.toml"),
            format!("plugin = [[\"./audit-plugin.mjs\", {{ secret = \"{secret}\" }}]]\n"),
        )
        .expect("write plugin config");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Resolve the live provider credential, or skip cleanly when a live environment is unavailable.
fn live_key() -> Option<String> {
    std::env::var("OPENROUTER_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
}

fn skip_without_runtime() -> bool {
    if plugins::detect_runtime().is_none() {
        eprintln!("skipping live audit redaction proof: no Bun, Node, or Deno on PATH");
        true
    } else {
        false
    }
}

/// Run a child under a bounded timeout, reaping only this exact child PID if it stalls.
fn run(mut command: Command) -> Output {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cortex binary");
    let mut stdout = child.stdout.take().expect("stdout handle");
    let mut stderr = child.stderr.take().expect("stderr handle");
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll cortex child") {
            break status;
        }
        if started.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex live audit proof exceeded {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: stdout_reader.join().expect("join stdout reader"),
        stderr: stderr_reader.join().expect("join stderr reader"),
    }
}

fn cortex(scratch: &Scratch, key: &str, prompt: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command
        .current_dir(&scratch.project)
        .args([
            "run",
            "--format",
            "json",
            "--model",
            MODEL,
            "--permission-allow",
            "bash:*",
            prompt,
        ])
        .env("HOME", &scratch.home)
        .env("CORTEX_DB_PATH", &scratch.db)
        .env("CORTEX_AUDIT_LOG", &scratch.audit)
        .env("OPENROUTER_API_KEY", key);
    command
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn read_audit(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("read durable audit sink")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("audit record is JSONL"))
        .collect()
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    format!("{nanos:x}-{}", std::process::id())
}

// VAL-AUDIT-008, VAL-AUDIT-013, VAL-AUDIT-014: one real OpenRouter turn plus a real JS
// shell.env hook. The command's success proves the injected secret was available to bash, while
// its output stays non-secret. All assertions inspect the raw durable sink, not a mock or a
// re-redacted display view.
#[test]
fn real_openrouter_and_shell_env_audit_records_redact_all_secrets() {
    let Some(key) = live_key() else {
        eprintln!("skipping live audit redaction proof: OPENROUTER_API_KEY not set");
        return;
    };
    if skip_without_runtime() {
        return;
    }

    let scratch = Scratch::new();
    let shell_secret = format!("topsecret-{}", unique_suffix());
    let prompt_secret = format!("prompt-secret-{}", unique_suffix());
    scratch.configure_shell_env_plugin(&shell_secret);
    let prompt = format!(
        "This is an audit test. Do not repeat this confidential marker: {prompt_secret}. \
Use the bash tool exactly once and run exactly this command: \
test \"$SECRET\" = \"{shell_secret}\" && printf 'audit-shell-marker\\n'. \
Do not use any other tool and never print the SECRET value. \
After the command, reply with exactly AUDIT-REDACTION-DONE."
    );

    let output = run(cortex(&scratch, &key, &prompt));
    assert_eq!(
        output.status.code(),
        Some(0),
        "real audit redaction turn failed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value = serde_json::from_slice(&output.stdout).expect("strict run JSON");
    assert_eq!(document["role"], "assistant");
    assert_eq!(document["model"], MODEL);
    assert!(
        document["content"]
            .as_str()
            .is_some_and(|content| content.contains("AUDIT-REDACTION-DONE")),
        "the real assistant must complete the requested audit flow"
    );

    let raw = std::fs::read_to_string(&scratch.audit).expect("read raw audit sink");
    assert!(
        !raw.contains(&key),
        "the live provider credential must not appear in the audit sink"
    );
    assert!(
        !raw.contains(&shell_secret),
        "the injected shell secret must not appear in the audit sink"
    );
    assert!(
        !raw.contains(&prompt_secret),
        "the provider request body must not persist the prompt secret"
    );
    assert!(
        raw.contains(redaction::PLACEHOLDER),
        "the audit sink must retain canonical redaction markers"
    );

    let records = read_audit(&scratch.audit);
    let provider = records
        .iter()
        .find(|record| {
            record["type"] == "network_call"
                && record["kind"] == "provider_request"
                && record["provider"] == "openrouter"
        })
        .expect("a real provider request audit record");
    assert_eq!(provider["model"], MODEL);
    assert_eq!(provider["method"], "POST");
    assert_eq!(provider["host"], "openrouter.ai");
    assert!(
        provider["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.contains("/chat/completions")),
        "provider record must retain the request endpoint"
    );
    assert!(provider["duration_ms"].as_u64().is_some());
    assert_eq!(provider["outcome"], "success");
    assert_eq!(provider["auth"], redaction::PLACEHOLDER);
    assert_eq!(provider["body"], redaction::PLACEHOLDER);
    assert_eq!(provider["headers"]["authorization"], redaction::PLACEHOLDER);

    let shell_env = records
        .iter()
        .find(|record| record["type"] == "plugin_hook" && record["hook"] == "shell.env")
        .expect("a real shell.env hook audit record");
    assert_eq!(shell_env["env"]["SECRET"], redaction::PLACEHOLDER);
    assert_eq!(shell_env["env"]["VISIBLE_AUDIT_FLAG"], "present");
    assert!(
        shell_env["env_keys"]
            .as_array()
            .is_some_and(|keys| keys.iter().any(|key| key == "SECRET")),
        "the masked secret field must remain observable as present"
    );

    let expected_command = format!(
        "test \"$SECRET\" = \"{}\" && printf 'audit-shell-marker\\n'",
        redaction::PLACEHOLDER
    );
    assert!(
        records.iter().any(|record| {
            record["type"] == "shell_execution"
                && record["outcome"] == "success"
                && record["command"] == expected_command
        }),
        "the successful bash action must prove shell.env injection while masking its literal secret"
    );
}

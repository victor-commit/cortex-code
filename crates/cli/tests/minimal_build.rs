//! Minimal `--no-default-features` build contract (VAL-FINISH-069..073).
//!
//! Run with both feature sets:
//! - `cargo test -p cli --test minimal_build` (default: network+server ON)
//! - `cargo test -p cli --no-default-features --test minimal_build` (minimal: gates OFF)

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    db_path: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-min-build-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let home = root.join("home");
        fs::create_dir_all(&home).expect("create HOME");
        let db_path = root.join("state/cortex.db");
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).expect("create db parent");
        }
        Self {
            root,
            home,
            db_path,
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(args)
            .env_clear()
            .env(
                "PATH",
                std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
            )
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.root.join(".config"))
            .env("CORTEX_DB_PATH", &self.db_path)
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_NO_NET");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("spawn cortex")
    }

    #[cfg(not(feature = "network"))]
    fn run_with_key(&self, args: &[&str]) -> Output {
        let key = std::env::var("OPENROUTER_API_KEY").unwrap_or_else(|_| {
            // Synthetic fixture for offline fail-closed checks (assembled so no key-shaped
            // literal lands in source).
            format!("sk-{}-{}", "or-v1", "not-a-real-fixture-key")
        });
        let mut command = self.command(args);
        command.env("OPENROUTER_API_KEY", key);
        command.output().expect("spawn cortex")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn binary_reports_version() {
    let sandbox = Sandbox::new("version");
    let out = sandbox.run(&["--version"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("cortex") && text.contains("0."),
        "version line: {text}"
    );
}

#[test]
fn binary_prints_help() {
    let sandbox = Sandbox::new("help");
    let out = sandbox.run(&["--help"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("run") || text.contains("Usage"),
        "help should list usage/run: {text}"
    );
}

#[test]
fn config_reads_effective_model() {
    let sandbox = Sandbox::new("config");
    let out = sandbox.run(&["config", "--format", "json"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("config json");
    assert_eq!(
        value.get("model").and_then(|m| m.as_str()),
        Some("google/gemini-2.5-flash")
    );
}

#[test]
fn completion_bash_is_valid_script() {
    let sandbox = Sandbox::new("completion");
    let out = sandbox.run(&["completion", "bash"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let script = stdout(&out);
    assert!(
        script.contains("complete") && script.contains("cortex"),
        "bash completion missing markers: {script}"
    );
}

#[test]
fn search_finds_local_token() {
    let sandbox = Sandbox::new("search");
    let proj = sandbox.root.join("proj");
    fs::create_dir_all(&proj).expect("proj");
    let token = format!(
        "token-min-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    );
    let mut file = fs::File::create(proj.join("note.txt")).expect("note");
    writeln!(file, "unique {token} line").expect("write");

    let out = sandbox
        .command(&["search", &token])
        .current_dir(&proj)
        .output()
        .expect("search");
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains(&token) && text.contains("note.txt"),
        "search match missing: {text}"
    );
}

#[test]
fn export_import_and_replay_round_trip_offline() {
    let sandbox = Sandbox::new("replay");
    let session_id = format!("sess-min-{}", std::process::id());
    let artifact = sandbox.root.join("artifact.json");
    let body = format!(
        r#"{{
  "version": 1,
  "session": {{
    "id": "{session_id}",
    "title": "minimal-build",
    "model": "google/gemini-2.5-flash"
  }},
  "messages": [
    {{"role": "user", "content": "hello offline"}},
    {{"role": "assistant", "content": "replay works"}}
  ]
}}"#
    );
    fs::write(&artifact, body).expect("write artifact");

    let import = sandbox.run(&["import", artifact.to_str().expect("utf8 path")]);
    assert!(import.status.success(), "import stderr={}", stderr(&import));

    let replay = sandbox.run(&["replay", &session_id]);
    assert!(replay.status.success(), "replay stderr={}", stderr(&replay));
    let text = stdout(&replay);
    assert!(
        text.contains("hello offline") && text.contains("replay works"),
        "replay transcript: {text}"
    );

    let export = sandbox.run(&["export", &session_id]);
    assert!(export.status.success(), "export stderr={}", stderr(&export));
}

#[cfg(not(feature = "network"))]
#[test]
fn minimal_run_fails_closed_with_network_diagnostic_even_with_key() {
    let sandbox = Sandbox::new("run-fail");
    let out = sandbox.run_with_key(&["run", "hi"]);
    assert!(
        !out.status.success(),
        "minimal run must exit non-zero; stdout={}",
        stdout(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("network/live features not compiled in"),
        "must name the network feature diagnostic: {err}"
    );
    assert!(
        !err.to_lowercase().contains("panic") && !err.contains("stack backtrace"),
        "must not panic: {err}"
    );
    assert!(
        out.stdout.is_empty(),
        "fail-closed run keeps stdout empty: {}",
        stdout(&out)
    );
}

#[cfg(not(feature = "server"))]
#[test]
fn minimal_serve_fails_closed_with_server_diagnostic() {
    let sandbox = Sandbox::new("serve-fail");
    let out = sandbox.run(&["serve", "--port", "3210"]);
    assert!(!out.status.success(), "serve must fail closed");
    let err = stderr(&out);
    assert!(
        err.contains("server features not compiled in"),
        "must name the server feature diagnostic: {err}"
    );
}

#[cfg(not(feature = "network"))]
#[test]
fn minimal_debug_info_reports_features_off() {
    let sandbox = Sandbox::new("debug-info");
    let out = sandbox.run(&["debug", "info", "--format", "json"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("debug info json");
    assert_eq!(
        value.get("network_feature"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(value.get("server_feature"), Some(&serde_json::json!(false)));
}

#[cfg(feature = "network")]
#[test]
fn default_debug_info_reports_network_on() {
    let sandbox = Sandbox::new("debug-default");
    let out = sandbox.run(&["debug", "info", "--format", "json"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("debug info json");
    assert_eq!(value.get("network_feature"), Some(&serde_json::json!(true)));
}

#[cfg(feature = "server")]
#[test]
fn default_debug_info_reports_server_on() {
    let sandbox = Sandbox::new("debug-server-default");
    let out = sandbox.run(&["debug", "info", "--format", "json"]);
    assert!(out.status.success(), "stderr={}", stderr(&out));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("debug info json");
    assert_eq!(value.get("server_feature"), Some(&serde_json::json!(true)));
}

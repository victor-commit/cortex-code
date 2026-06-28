//! Black-box tests for the CLI lifecycle and diagnostics commands:
//! `cortex upgrade` / `cortex uninstall` (VAL-GAP-CLI-006) and the `cortex debug` namespace
//! (VAL-GAP-CLI-007). All checks are offline, deterministic, and use isolated scratch dirs.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch_dir(label: &str) -> PathBuf {
    let id = SCRATCH_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("cortex-{label}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A base command with the ambient Cortex env scrubbed, so each test fully controls resolution.
fn cortex(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.args(args);
    command
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("CORTEX_AUDIT_LOG")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CONFIG_HOME");
    command
}

fn run(mut command: Command) -> Output {
    command.output().expect("run cortex command")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|err| panic!("stdout is not JSON ({err}): {}", stdout(output)))
}

fn new_session(db_path: &Path) -> String {
    let mut command = cortex(&["session", "new", "--format", "json"]);
    command.env("CORTEX_DB_PATH", db_path);
    let output = run(command);
    assert!(
        output.status.success(),
        "session new failed: {}",
        stderr(&output)
    );
    json(&output)["id"].as_str().expect("session id").to_owned()
}

fn add_message(db_path: &Path, session: &str, role: &str, content: &str) {
    let mut command = cortex(&["session", "message", session, "--role", role, content]);
    command.env("CORTEX_DB_PATH", db_path);
    let output = run(command);
    assert!(
        output.status.success(),
        "session message failed: {}",
        stderr(&output)
    );
}

// --------------------------------------------------------------------------
// VAL-GAP-CLI-006: upgrade and uninstall are safe and self-describing
// --------------------------------------------------------------------------

#[test]
fn upgrade_reports_unsupported_status_and_documents_safeguards() {
    let output = run(cortex(&["upgrade", "--format", "json"]));
    assert!(
        output.status.success(),
        "upgrade exited non-zero: {}",
        stderr(&output)
    );
    let doc = json(&output);
    assert_eq!(doc["type"], "upgrade");
    assert_eq!(doc["supported"], false);
    assert_eq!(doc["status"], "unsupported");
    assert!(
        doc["message"]
            .as_str()
            .is_some_and(|m| m.contains("No files were changed")),
        "upgrade message should document its no-op safeguard: {doc}"
    );

    // Help documents the safeguards.
    let help = run(cortex(&["upgrade", "--help"]));
    assert!(help.status.success());
    assert!(
        stdout(&help).to_lowercase().contains("no files"),
        "upgrade --help should document that no files are changed: {}",
        stdout(&help)
    );
}

#[test]
fn uninstall_dry_run_lists_targets_without_removing_anything() {
    let home = scratch_dir("uninstall-dry");
    let data_dir = home.join(".local/share/cortex");
    let config_dir = home.join(".config/cortex");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(data_dir.join("cortex.db"), b"db").unwrap();
    std::fs::write(config_dir.join("config.toml"), b"cfg").unwrap();

    let mut command = cortex(&["uninstall", "--format", "json"]);
    command.env("HOME", &home);
    let output = run(command);
    assert!(
        output.status.success(),
        "uninstall dry run failed: {}",
        stderr(&output)
    );
    let doc = json(&output);
    assert_eq!(doc["dry_run"], true);
    assert!(
        doc["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["removed"] == false),
        "dry run must not remove anything: {doc}"
    );
    assert!(data_dir.exists(), "dry run must not delete the data dir");
    assert!(
        config_dir.exists(),
        "dry run must not delete the config dir"
    );

    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn uninstall_yes_removes_only_cortex_owned_paths() {
    let home = scratch_dir("uninstall-yes");
    let data_dir = home.join(".local/share/cortex");
    let config_dir = home.join(".config/cortex");
    let other_data = home.join(".local/share/other");
    let other_config = home.join(".config/other");
    for dir in [&data_dir, &config_dir, &other_data, &other_config] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(data_dir.join("cortex.db"), b"db").unwrap();
    std::fs::write(config_dir.join("config.toml"), b"cfg").unwrap();
    let keep_data = other_data.join("keep.txt");
    let keep_config = other_config.join("keep.txt");
    std::fs::write(&keep_data, b"keep").unwrap();
    std::fs::write(&keep_config, b"keep").unwrap();

    let mut command = cortex(&["uninstall", "--yes", "--format", "json"]);
    command.env("HOME", &home);
    let output = run(command);
    assert!(
        output.status.success(),
        "uninstall --yes failed: {}",
        stderr(&output)
    );
    let doc = json(&output);
    assert_eq!(doc["dry_run"], false);

    assert!(!data_dir.exists(), "cortex data dir should be removed");
    assert!(!config_dir.exists(), "cortex config dir should be removed");
    assert!(keep_data.exists(), "unrelated data file must be preserved");
    assert!(
        keep_config.exists(),
        "unrelated config file must be preserved"
    );

    std::fs::remove_dir_all(&home).ok();
}

// --------------------------------------------------------------------------
// VAL-GAP-CLI-007: db/debug namespace exposes safe diagnostics
// --------------------------------------------------------------------------

#[test]
fn debug_db_reports_counts_and_is_read_only() {
    let dir = scratch_dir("debug-db");
    let db_path = dir.join("cortex.db");
    let session = new_session(&db_path);
    add_message(&db_path, &session, "user", "hello there");

    let bytes_before = std::fs::read(&db_path).expect("read db before");

    let mut command = cortex(&["debug", "db", "--integrity", "--format", "json"]);
    command.env("CORTEX_DB_PATH", &db_path);
    let output = run(command);
    assert!(
        output.status.success(),
        "debug db failed: {}",
        stderr(&output)
    );
    let doc = json(&output);
    assert_eq!(doc["type"], "debug-db");
    assert_eq!(doc["integrity"], "ok");
    let counts = doc["counts"].as_array().expect("counts array");
    let sessions = counts
        .iter()
        .find(|c| c["table"] == "sessions")
        .and_then(|c| c["rows"].as_i64());
    assert_eq!(sessions, Some(1), "one session should be counted: {doc}");

    // Running it again must not change the on-disk database (read-only invariant).
    let mut again = cortex(&["debug", "db", "--integrity", "--format", "json"]);
    again.env("CORTEX_DB_PATH", &db_path);
    assert!(run(again).status.success());
    let bytes_after = std::fs::read(&db_path).expect("read db after");
    assert_eq!(
        bytes_before, bytes_after,
        "read-only debug db must not change the database file"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn debug_info_emits_runtime_status_as_json() {
    let dir = scratch_dir("debug-info");
    let db_path = dir.join("cortex.db");

    let mut command = cortex(&["debug", "info", "--format", "json"]);
    command.env("CORTEX_DB_PATH", &db_path);
    let output = run(command);
    assert!(
        output.status.success(),
        "debug info failed: {}",
        stderr(&output)
    );
    let doc = json(&output);
    assert_eq!(doc["type"], "debug-info");
    assert!(doc["version"].as_str().is_some());
    assert_eq!(doc["no_net"], false, "kill-switch should be off by default");

    // With the kill-switch engaged the status reports it (no network is touched either way).
    let mut no_net = cortex(&["debug", "info", "--format", "json"]);
    no_net
        .env("CORTEX_DB_PATH", &db_path)
        .env("CORTEX_NO_NET", "1");
    let output = run(no_net);
    assert!(output.status.success());
    assert_eq!(json(&output)["no_net"], true);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn debug_audit_redacts_secrets_and_parses_as_json() {
    let dir = scratch_dir("debug-audit");
    let audit_log = dir.join("audit.jsonl");
    // Assembled from fragments so no contiguous key-shaped literal lives in source.
    let secret = format!("sk-{}-{}", "or-v1", "deadbeef".repeat(4));
    let line = format!(r#"{{"type":"tool.execute","tool":"bash","secret":"{secret}"}}"#);
    std::fs::write(&audit_log, format!("{line}\n")).unwrap();

    let mut command = cortex(&["debug", "audit", "--format", "json"]);
    command.env("CORTEX_AUDIT_LOG", &audit_log);
    let output = run(command);
    assert!(
        output.status.success(),
        "debug audit failed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        !out.contains(&secret),
        "audit secret must be redacted: {out}"
    );
    let doc: serde_json::Value = serde_json::from_str(&out).expect("audit output is JSON");
    assert!(
        doc.as_array().is_some_and(|a| a.len() == 1),
        "one record expected: {doc}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn debug_config_redacts_secret_config_values() {
    let dir = scratch_dir("debug-config");
    let db_path = dir.join("cortex.db");
    std::fs::write(
        dir.join("cortex.toml"),
        "model = \"{env:CORTEX_TEST_SECRET}\"\n",
    )
    .unwrap();
    // Assembled from fragments so no contiguous key-shaped literal lives in source.
    let secret = format!("sk-{}-{}", "or-v1", "feedface".repeat(4));

    let mut command = cortex(&["debug", "config", "--format", "json"]);
    command
        .current_dir(&dir)
        .env("HOME", &dir)
        .env("CORTEX_DB_PATH", &db_path)
        .env("CORTEX_TEST_SECRET", &secret);
    let output = run(command);
    assert!(
        output.status.success(),
        "debug config failed: {}",
        stderr(&output)
    );
    let out = stdout(&output);
    assert!(
        !out.contains(&secret),
        "secret resolved into config must be redacted in the debug dump: {out}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

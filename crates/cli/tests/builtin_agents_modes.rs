//! Offline black-box checks for the built-in agent catalogue.
//!
//! The live behavior of the mode permissions is covered in `builtin_agents_modes_live.rs`; this
//! file stays network-free and proves the local `cortex agent` discovery surface enumerates the
//! built-ins without needing a provider key.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

fn scratch_home() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "cortex-agent-list-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create scratch home");
    dir
}

fn cortex(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    let home = scratch_home();
    command.args(args);
    command.current_dir(&home);
    command.env("HOME", &home);
    command
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_OPENROUTER_BASE_URL")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("CORTEX_DATA_DIR")
        .env_remove("CORTEX_DB_PATH")
        .env_remove("CORTEX_AUTH_FILE")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE")
        .env_remove("NO_COLOR");
    command
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.output().expect("run cortex")
}

#[test]
fn agent_command_lists_all_builtin_agents_as_text() {
    let out = run(cortex(&["agent"]));
    assert!(
        out.status.success(),
        "cortex agent should succeed without a key (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "listing agents should not write diagnostics: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    for name in ["build", "plan", "general", "explore"] {
        assert!(
            stdout.lines().any(|line| line.contains(name)),
            "`cortex agent` must enumerate {name}; output was:\n{stdout}"
        );
    }
}

#[test]
fn agent_command_lists_all_builtin_agents_as_json() {
    let out = run(cortex(&["agent", "--format", "json"]));
    assert!(
        out.status.success(),
        "cortex agent --format json should succeed without a key (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("utf8 stdout");
    let doc: serde_json::Value =
        serde_json::from_str(stdout.trim_end()).expect("stdout is one strict JSON document");
    assert_eq!(doc["default"], "build");
    let names: Vec<&str> = doc["agents"]
        .as_array()
        .expect("agents array")
        .iter()
        .filter_map(|agent| agent["name"].as_str())
        .collect();
    for name in ["build", "plan", "general", "explore"] {
        assert!(
            names.contains(&name),
            "`cortex agent --format json` must enumerate {name}; got {names:?}"
        );
    }
}

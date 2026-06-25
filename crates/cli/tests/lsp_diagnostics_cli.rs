//! Black-box `cortex lsp diagnostics` tests.
//!
//! This is the local user-facing LSP proof surface used when `CORTEX_NO_NET=1` must keep normal
//! `cortex run` model turns fail-closed. The command exercises real language servers without a
//! model turn, so pre-installed servers can produce diagnostics offline and missing servers report
//! a clean no-LSP/net-disabled result without running installers.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-cli-lsp-diagnostics-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn has_command(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn toml_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn cortex_in(cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(cwd);
    command.env("CORTEX_DATA_DIR", cwd.join(".data"));
    command
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
            panic!("cortex lsp diagnostics timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = std::fs::metadata(path)
        .expect("fake command metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod fake command");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

#[test]
fn no_net_missing_server_reports_no_lsp_and_never_runs_installer() {
    let dir = temp_dir("missing-ts");
    let file = dir.join("bad.ts");
    std::fs::write(&file, "const x: number = 'nope';\n").expect("write ts fixture");

    let fake_bin = dir.join("fake-bin");
    std::fs::create_dir_all(&fake_bin).expect("create fake bin");
    let installer_marker = dir.join("npm-invoked");
    let fake_npm = fake_bin.join("npm");
    std::fs::write(
        &fake_npm,
        format!(
            "#!/bin/sh\nprintf invoked > '{}'\nexit 0\n",
            installer_marker.display()
        ),
    )
    .expect("write fake npm");
    make_executable(&fake_npm);

    let path = format!("{}:/usr/bin:/bin", fake_bin.display());
    let mut command = cortex_in(&dir);
    command.args(["lsp", "diagnostics", "bad.ts", "--format", "json"]);
    command
        .env("CORTEX_NO_NET", "1")
        .env("PATH", path)
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let output = run(command, Duration::from_secs(30));

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr should stay clean on no-server success: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(doc["status"], "no_server");
    assert_eq!(doc["language"], "typescript");
    assert_eq!(doc["program"], "typescript-language-server");
    assert!(
        doc["diagnostics"].as_array().is_some_and(Vec::is_empty),
        "no diagnostics should be claimed: {doc}"
    );
    let note = doc["note"].as_str().expect("note");
    assert!(note.contains("CORTEX_NO_NET=1"), "{note}");
    assert!(note.contains("prevents auto-installing"), "{note}");
    assert!(
        !installer_marker.exists(),
        "no-net diagnostics must not run npm auto-install"
    );
    assert_eq!(
        std::fs::read_to_string(&file).expect("read fixture"),
        "const x: number = 'nope';\n",
        "diagnostic proof must not corrupt local files"
    );
}

#[test]
fn no_net_preinstalled_ruff_returns_real_diagnostics_offline() {
    if !has_command("ruff") {
        eprintln!("skipping local LSP proof: ruff is not on PATH");
        return;
    }

    let dir = temp_dir("ruff");
    let file = dir.join("bad.py");
    std::fs::write(&file, "import os\nprint(undefined_name)\n").expect("write python fixture");

    let mut command = cortex_in(&dir);
    command.args(["lsp", "diagnostics", "bad.py", "--format", "json"]);
    command
        .env("CORTEX_NO_NET", "1")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let output = run(command, Duration::from_secs(30));

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(doc["status"], "diagnostics");
    assert_eq!(doc["language"], "python");
    assert_eq!(doc["program"], "ruff");
    let rendered = serde_json::to_string(&doc["diagnostics"]).expect("diagnostics json");
    assert!(
        rendered.contains("Undefined name `undefined_name`"),
        "expected real Ruff F821 diagnostic: {rendered}"
    );
    assert!(
        rendered.contains("\"F821\"") && rendered.contains("\"F401\""),
        "expected Ruff error and warning codes: {rendered}"
    );
}

#[test]
fn lsp_server_can_be_disabled_per_language_via_config() {
    if !has_command("ruff") {
        eprintln!("skipping local LSP proof: ruff is not on PATH");
        return;
    }

    let dir = temp_dir("python-disabled");
    let file = dir.join("bad.py");
    std::fs::write(&file, "print(undefined_name)\n").expect("write python fixture");
    std::fs::write(
        dir.join("cortex.toml"),
        r#"
[lsp.servers.python]
disabled = true
"#,
    )
    .expect("write config");

    let mut command = cortex_in(&dir);
    command.args(["lsp", "diagnostics", "bad.py", "--format", "json"]);
    command
        .env("CORTEX_NO_NET", "1")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let output = run(command, Duration::from_secs(30));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(doc["status"], "no_server");
    assert!(
        doc["language"].is_null() && doc["program"].is_null(),
        "disabled language server should not resolve a launcher: {doc}"
    );
    let note = doc["note"].as_str().expect("note");
    assert!(
        note.contains("no language server is configured"),
        "disabled server should report unconfigured extension: {note}"
    );
}

#[test]
fn lsp_custom_program_receives_configured_environment() {
    let dir = temp_dir("custom-env");
    let file = dir.join("bad.py");
    std::fs::write(&file, "print(undefined_name)\n").expect("write python fixture");

    let marker = dir.join("lsp-env-marker.txt");
    let script = dir.join("capture-lsp-env.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s' \"${CORTEX_LSP_CAPTURE:-missing}\" > \"${CORTEX_LSP_MARKER:-/tmp/missing}\"\nexit 1\n",
    )
    .expect("write fake lsp server");
    make_executable(&script);
    std::fs::write(
        dir.join("cortex.toml"),
        format!(
            r#"
[lsp.servers.python]
program = "{}"
args = []
extensions = ["py"]
[lsp.servers.python.env]
CORTEX_LSP_CAPTURE = "env-ok"
CORTEX_LSP_MARKER = "{}"
"#,
            toml_string(&script),
            toml_string(&marker)
        ),
    )
    .expect("write config");

    let mut command = cortex_in(&dir);
    command.args(["lsp", "diagnostics", "bad.py", "--format", "json"]);
    command
        .env("CORTEX_NO_NET", "1")
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("RUST_LOG")
        .env_remove("CORTEX_VERBOSE");
    let output = run(command, Duration::from_secs(30));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(doc["status"], "unavailable");
    assert_eq!(doc["program"], script.to_string_lossy().as_ref());
    assert_eq!(
        std::fs::read_to_string(&marker).expect("marker written"),
        "env-ok",
        "custom server process should receive configured env vars"
    );
}

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);
const TIMEOUT: Duration = Duration::from_secs(20);

fn temp_dir(tag: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "cortex-gap-format-config-{tag}-{}-{unique}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
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
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            let mut stdout = Vec::new();
            child
                .stdout
                .take()
                .expect("stdout")
                .read_to_end(&mut stdout)
                .expect("read stdout");
            let mut stderr = Vec::new();
            child
                .stderr
                .take()
                .expect("stderr")
                .read_to_end(&mut stderr)
                .expect("read stderr");
            return Output {
                status,
                stdout,
                stderr,
            };
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn formatter_status_reports_enabled_disabled_missing_and_marker_states() {
    let dir = temp_dir("status");
    std::fs::write(
        dir.join("cortex.toml"),
        r#"
[formatter.rustfmt]
disabled = true

[formatter.gofmt]
command = ["/definitely/missing/gofmt", "$FILE"]

[formatter.prettier]
command = ["/bin/sh", "$FILE"]
"#,
    )
    .expect("write formatter config");

    let mut command = cortex_in(&dir);
    command.args(["formatter", "status", "--format", "json"]);
    let output = run(command, TIMEOUT);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status json");
    let rows = rows.as_array().expect("status rows");

    let rustfmt = rows
        .iter()
        .find(|row| row["name"] == "rustfmt")
        .unwrap_or_else(|| panic!("missing rustfmt in formatter status rows: {rows:?}"));
    assert_eq!(rustfmt["state"], "disabled");
    assert_eq!(rustfmt["source"], "config");

    let gofmt = rows
        .iter()
        .find(|row| row["name"] == "gofmt")
        .unwrap_or_else(|| panic!("missing gofmt in formatter status rows: {rows:?}"));
    assert_eq!(gofmt["state"], "missing");
    assert_eq!(gofmt["source"], "config");

    let prettier = rows
        .iter()
        .find(|row| row["name"] == "prettier")
        .unwrap_or_else(|| panic!("missing prettier in formatter status rows: {rows:?}"));
    assert_eq!(prettier["state"], "missing_config_marker");
    assert_eq!(prettier["source"], "config");
}

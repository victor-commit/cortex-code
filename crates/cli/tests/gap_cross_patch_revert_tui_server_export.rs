//! VAL-GAP-CROSS-012 (server/export half): after a tool creates multiple hunks in a file and the
//! TUI `/diff` viewer reverts ONE hunk, the on-disk bytes, the server diff surfaces
//! (`/session/{id}/diff` net change and `/vcs/diff` working-tree diff), and the CLI export
//! (`cortex snapshot diff --full`) all AGREE that only the selected hunk reverted — the unrelated
//! hunk survives everywhere and the reverted hunk is absent everywhere.
//!
//! The TUI hunk revert records the revert as a fresh shadow-store turn under the session id (locked
//! by the `tui`-crate test `hunk_revert_is_consistent_across_disk_and_checkpoint_diff_surfaces`).
//! Here we reproduce that exact store shape directly through the `checkpoints` API in a scratch git
//! repo — the original turn (baseline -> two hunks) plus the revert turn (two hunks -> one hunk) —
//! then read every surface back through fresh `cortex` processes (snapshot CLI + loopback serve).
//! Recording through the durable store and re-reading via separate processes is the cross-surface
//! proof; no provider turn (and no network) is needed for the deterministic composition. The live
//! end-to-end flow (revert in the TUI, then curl/serve + export) is proven black-box with tuistory.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use checkpoints::ShadowStore;
use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(45);

const BASELINE: &str = "line 1\nalpha\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nomega\nline 12\n";
const TWO_HUNKS: &str = "line 1\nALPHA\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nOMEGA\nline 12\n";
// The OMEGA hunk reverted, the ALPHA hunk kept — the state the TUI hunk revert leaves on disk.
const ONE_HUNK_REVERTED: &str = "line 1\nALPHA\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\nline 9\nline 10\nomega\nline 12\n";
const TARGET_FILE: &str = "src/lib.rs";

struct Scratch {
    home: PathBuf,
    project: PathBuf,
    db_path: PathBuf,
    checkpoint_base: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "cortex-cross-patch-revert-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = root.join("project");
        std::fs::create_dir_all(project.join("src")).expect("create project dir");
        Self {
            db_path: root.join("cortex.db"),
            checkpoint_base: root.join("checkpoints"),
            home: root,
            project,
        }
    }

    fn cortex(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .current_dir(&self.project)
            .args(args)
            .env("HOME", &self.home)
            .env("CORTEX_DB_PATH", &self.db_path)
            .env("CORTEX_CHECKPOINT_DIR", &self.checkpoint_base)
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("CORTEX_NO_NET")
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CORTEX_DATA_DIR")
            .env_remove("RUST_LOG")
            .env_remove("CORTEX_VERBOSE")
            .env_remove("NO_COLOR");
        command
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn git(project: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(project)
        .args(args)
        .env("GIT_AUTHOR_NAME", "cortex test")
        .env("GIT_AUTHOR_EMAIL", "test@cortex.invalid")
        .env("GIT_COMMITTER_NAME", "cortex test")
        .env("GIT_COMMITTER_EMAIL", "test@cortex.invalid")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

/// A small dependency-free content checksum so "disk checksum" agreement is asserted on the bytes,
/// not merely on equality of two literals.
fn checksum(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn run(mut command: Command) -> Output {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn cortex");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    let out_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let err_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {RUN_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

fn run_json(scratch: &Scratch, args: &[&str]) -> Value {
    let out = run(scratch.cortex(args));
    assert_eq!(
        out.status.code(),
        Some(0),
        "`cortex {}` failed:\nstdout={}\nstderr={}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "strict json from `cortex {}` failed: {err}\nstdout={}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn free_server_port() -> u16 {
    for port in 18765..=18864 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no free server port in 18765-18864");
}

fn wait_for_health(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if curl_get(port, "/health").is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(150));
    }
    panic!("server on port {port} did not become healthy");
}

/// Minimal loopback HTTP GET returning the 200 response body, or `None` on failure.
fn curl_get(port: u16, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") {
        return None;
    }
    Some(body.to_owned())
}

fn listener_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-tiTCP", &port.to_string(), "-sTCP:LISTEN"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .and_then(|line| line.trim().parse::<u32>().ok())
}

fn stop_server(mut child: Child, port: u16) {
    let pid = listener_pid(port).unwrap_or_else(|| child.id());
    let _ = Command::new("kill").arg(pid.to_string()).status();
    let _ = child.wait();
}

/// Assert a unified-diff body shows the remaining (ALPHA) hunk and NOT the reverted (OMEGA) hunk.
fn assert_only_remaining_hunk(body: &str, surface: &str) {
    assert!(
        body.contains("ALPHA"),
        "{surface}: the remaining hunk must be present: {body}"
    );
    assert!(
        body.contains("alpha"),
        "{surface}: the remaining hunk's removed line must be present: {body}"
    );
    assert!(
        !body.contains("OMEGA"),
        "{surface}: the reverted hunk must be ABSENT: {body}"
    );
}

#[test]
fn tui_hunk_revert_is_consistent_across_disk_server_diff_and_cli_export() {
    let scratch = Scratch::new("xflow");
    let file_abs = scratch.project.join(TARGET_FILE);

    // A scratch git repo whose committed HEAD is the baseline, so the working-tree diff has a stable
    // reference point.
    git(&scratch.project, &["init", "--quiet"]);
    std::fs::write(&file_abs, BASELINE).expect("write baseline");
    git(&scratch.project, &["add", "-A"]);
    git(&scratch.project, &["commit", "--quiet", "-m", "baseline"]);

    // A durable session so the server's session-scoped diff route (`require_session`) resolves.
    let session_id = {
        let db = db::Database::open(&scratch.db_path).expect("open seed store");
        let id = format!("ses_xflow_{}", std::process::id());
        db.sessions()
            .ensure(&id, Some("patch revert cross-flow"))
            .expect("ensure session");
        id
    };

    // Reproduce the exact shadow-store shape the TUI hunk revert produces, at the SAME location the
    // `cortex` binary resolves (CORTEX_CHECKPOINT_DIR base + workspace key).
    let git_dir = scratch
        .checkpoint_base
        .join(checkpoints::workspace_key(&scratch.project));
    let store = ShadowStore::open(&scratch.project, &git_dir).expect("open shadow store");

    // Original turn: the tool changes two well-separated regions (two hunks).
    let original = store.begin_turn(session_id.clone());
    original.record_pre_image(&file_abs);
    std::fs::write(&file_abs, TWO_HUNKS).expect("apply two hunks");
    original
        .finalize()
        .expect("finalize original")
        .expect("original recorded a snapshot");

    // Revert turn: the TUI `/diff` viewer reverts ONLY the second (OMEGA) hunk, recorded as a fresh
    // turn under the same session id (this is exactly what `revert_diff_selection` now does).
    let revert = store.begin_turn(session_id.clone());
    revert.record_pre_image(&file_abs);
    std::fs::write(&file_abs, ONE_HUNK_REVERTED).expect("revert one hunk");
    revert
        .finalize()
        .expect("finalize revert")
        .expect("revert recorded a snapshot");

    // --- Disk checksum ---
    let disk = std::fs::read(&file_abs).expect("read reverted file");
    assert_eq!(
        checksum(&disk),
        checksum(ONE_HUNK_REVERTED.as_bytes()),
        "disk bytes must equal the single-hunk-reverted content"
    );

    // --- CLI export: `cortex snapshot diff` (session-scoped + global both agree) ---
    let cli_scoped = run_json(
        &scratch,
        &[
            "snapshot",
            "diff",
            "--session",
            &session_id,
            "--full",
            "--format",
            "json",
        ],
    );
    let cli_files = cli_scoped["files"].as_array().expect("files array");
    assert_eq!(
        cli_files.len(),
        1,
        "exactly one net-changed file: {cli_scoped}"
    );
    assert_eq!(cli_files[0]["path"], TARGET_FILE);
    assert_eq!(cli_files[0]["additions"], 1, "one remaining addition");
    assert_eq!(cli_files[0]["deletions"], 1, "one remaining deletion");
    assert_only_remaining_hunk(
        cli_files[0]["diff"].as_str().expect("full diff body"),
        "cortex snapshot diff",
    );

    let cli_global = run_json(
        &scratch,
        &["snapshot", "diff", "--full", "--format", "json"],
    );
    let global_files = cli_global["files"].as_array().expect("files array");
    assert_eq!(global_files.len(), 1, "global snapshot also one file");
    assert_only_remaining_hunk(
        global_files[0]["diff"].as_str().expect("full diff body"),
        "cortex snapshot diff (global)",
    );

    // --- Server diff surfaces ---
    let port = free_server_port();
    let mut serve = scratch
        .cortex(&[
            "serve",
            "--hostname",
            "127.0.0.1",
            "--port",
            &port.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn serve");

    // If the server died immediately, surface it rather than hanging on health.
    if let Ok(Some(status)) = serve.try_wait() {
        panic!("cortex serve exited early with {status:?}");
    }
    wait_for_health(port);

    // /session/{id}/diff — session-scoped net change reflects only the remaining hunk.
    let session_body =
        curl_get(port, &format!("/session/{session_id}/diff")).expect("session diff 200");
    let session_diff: Value = serde_json::from_str(&session_body).expect("session diff json");
    let session_files = session_diff.as_array().expect("session diff array");
    assert_eq!(
        session_files.len(),
        1,
        "one net-changed file: {session_diff}"
    );
    assert_eq!(session_files[0]["path"], TARGET_FILE);
    assert_eq!(session_files[0]["status"], "modified");
    assert_only_remaining_hunk(
        session_files[0]["diff"]
            .as_str()
            .expect("session diff body"),
        "/session/{id}/diff",
    );

    // /vcs/diff?staged=true — working-tree diff vs committed HEAD also reflects only the remaining
    // hunk, proving the on-disk result and the git view agree.
    let vcs_body = curl_get(port, "/vcs/diff?staged=true").expect("vcs diff 200");
    let vcs: Value = serde_json::from_str(&vcs_body).expect("vcs diff json");
    assert_eq!(vcs["repository"], true, "scratch dir is a git repo");
    let vcs_files = vcs["files"].as_array().expect("vcs files array");
    assert_eq!(vcs_files.len(), 1, "git sees one changed file: {vcs}");
    assert_eq!(vcs_files[0]["added"], 1, "git: one remaining addition");
    assert_eq!(vcs_files[0]["removed"], 1, "git: one remaining deletion");
    assert_only_remaining_hunk(vcs["diff"].as_str().expect("vcs diff body"), "/vcs/diff");

    stop_server(serve, port);
    assert!(
        listener_pid(port).is_none(),
        "the server port must be released after shutdown"
    );
}

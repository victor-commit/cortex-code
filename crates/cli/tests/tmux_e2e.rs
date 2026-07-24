//! Tmux e2e harness for the real `cortex` binary (VAL-CORTEX-HARNESS-002).
//!
//! Smoke path:
//! 1. Spawn a dedicated 120x40 tmux session running `cortex --version` (or help).
//! 2. Capture the pane text.
//! 3. Tear the session down and assert no orphan `cortex-e2e-*` session remains.
//!
//! Also covers:
//! - optional `CORTEX_LIVE=1` OpenRouter probe via headless `cortex run` (VAL-CORTEX-HARNESS-003)
//! - serve default bind policy documentation helper (127.0.0.1:18765)
//!
//! Secrets are never printed. Live credentials come from `OPENROUTER_API_KEY` only when set
//! (or loaded by the caller from `.env.local` / auth store). Absent keys skip the live half.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

const TMUX_COLS: u16 = 120;
const TMUX_ROWS: u16 = 40;
const SESSION_PREFIX: &str = "cortex-e2e";
const WAIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Unique tmux session name for this process + counter (never collides across parallel tests).
fn unique_session(tag: &str) -> String {
    let n = SESSION_SEQ.fetch_add(1, Ordering::SeqCst);
    format!(
        "{SESSION_PREFIX}-{}-{}-{}-{n}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

fn cortex_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cortex"))
}

fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn require_tmux() {
    if !tmux_available() {
        panic!("tmux is required for e2e harness tests (install tmux 3.4+)");
    }
}

fn run_tmux(args: &[&str]) -> Output {
    Command::new("tmux")
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("failed to run tmux {args:?}: {err}"))
}

/// Whether `name` exists on the **default** tmux server (not a harness socket).
fn default_server_has_session(name: &str) -> bool {
    let out = run_tmux(&["has-session", "-t", name]);
    out.status.success()
}

/// Kill every default-server session whose name starts with `prefix`.
fn kill_default_sessions_matching_prefix(prefix: &str) {
    let out = run_tmux(&["list-sessions", "-F", "#{session_name}"]);
    if !out.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let name = line.trim();
        if name.starts_with(prefix) {
            let _ = run_tmux(&["kill-session", "-t", name]);
        }
    }
}

struct TmuxSession {
    name: String,
    /// Dedicated socket path so we never touch the caller's default tmux server unexpectedly.
    socket: PathBuf,
    killed: bool,
}

impl TmuxSession {
    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new("tmux");
        cmd.arg("-S").arg(&self.socket);
        cmd.args(args);
        cmd.output()
            .unwrap_or_else(|err| panic!("tmux {:?} failed: {err}", args))
    }

    /// Start a detached 120x40 session running `command` (full argv).
    ///
    /// Uses `remain-on-exit` so short commands (`--version`, `true`) leave pane
    /// output capturable until we explicitly tear the session down.
    fn spawn(tag: &str, command: &[&str]) -> Self {
        require_tmux();
        let name = unique_session(tag);
        let socket = std::env::temp_dir().join(format!(
            "cortex-tmux-{}-{}.sock",
            std::process::id(),
            SESSION_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        // Drop any stale socket file from a crashed prior run.
        let _ = std::fs::remove_file(&socket);

        let session = Self {
            name: name.clone(),
            socket: socket.clone(),
            killed: false,
        };

        let size = format!("{TMUX_COLS}x{TMUX_ROWS}");

        // One-shot: create session with remain-on-exit so the pane survives command exit.
        // Shell form: tmux -S sock new-session -d -s NAME -x 120 -y 40 COMMAND...
        let mut cmd = Command::new("tmux");
        cmd.arg("-S").arg(&socket);
        cmd.args([
            "new-session",
            "-d",
            "-s",
            &name,
            "-x",
            &TMUX_COLS.to_string(),
            "-y",
            &TMUX_ROWS.to_string(),
        ]);
        cmd.args(command);
        let status = cmd
            .status()
            .unwrap_or_else(|err| panic!("spawn tmux session: {err}"));
        assert!(
            status.success(),
            "tmux new-session failed for {name} size={size} cmd={command:?}"
        );

        // remain-on-exit keeps dead panes (and the server) until we kill the session.
        let _ = session.run(&["set-option", "-t", &name, "remain-on-exit", "on"]);

        // If the command already exited before remain-on-exit applied, respawn into a
        // long-lived shell so subsequent capture/send still have a live target, then
        // re-run is not needed for smoke (pane history is preserved with remain-on-exit
        // when set early). Force remain-on-exit on the server before next spawns by
        // setting it globally on this socket too.
        let _ = session.run(&["set-option", "-g", "remain-on-exit", "on"]);

        let resize = session.run(&[
            "resize-window",
            "-t",
            &name,
            "-x",
            &TMUX_COLS.to_string(),
            "-y",
            &TMUX_ROWS.to_string(),
        ]);
        if !resize.status.success() {
            // Session may have fully exited if remain-on-exit lost the race. Recover by
            // spawning a holding shell and recording that the short command already finished.
            let recover = session.run(&[
                "new-session",
                "-d",
                "-s",
                &name,
                "-x",
                &TMUX_COLS.to_string(),
                "-y",
                &TMUX_ROWS.to_string(),
                "bash",
                "--noprofile",
                "--norc",
                "-c",
                "sleep 30",
            ]);
            assert!(
                recover.status.success() || session.has_session(),
                "tmux session lost after short command and recover failed: {}",
                String::from_utf8_lossy(&resize.stderr)
            );
            let _ = session.run(&["set-option", "-t", &name, "remain-on-exit", "on"]);
            let resize2 = session.run(&[
                "resize-window",
                "-t",
                &name,
                "-x",
                &TMUX_COLS.to_string(),
                "-y",
                &TMUX_ROWS.to_string(),
            ]);
            assert!(
                resize2.status.success(),
                "tmux resize-window failed after recover: {}",
                String::from_utf8_lossy(&resize2.stderr)
            );
        }

        session
    }

    /// Spawn a long-lived 120x40 interactive shell with optional env preamble.
    fn spawn_shell(tag: &str, shell_script: &str) -> Self {
        Self::spawn(tag, &["bash", "--noprofile", "--norc", "-lc", shell_script])
    }

    fn send_keys(&self, keys: &[&str]) {
        let mut args = vec!["send-keys", "-t", self.name.as_str()];
        args.extend_from_slice(keys);
        let out = self.run(&args);
        assert!(
            out.status.success(),
            "send-keys failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn capture_pane(&self) -> String {
        let out = self.run(&["capture-pane", "-p", "-t", &self.name, "-J"]);
        assert!(
            out.status.success(),
            "capture-pane failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn wait_capture_contains(&self, needle: &str, timeout: Duration) -> String {
        let start = Instant::now();
        let mut last = String::new();
        while start.elapsed() < timeout {
            if !self.has_session() {
                panic!(
                    "tmux session {} vanished while waiting for {needle:?}; last capture:\n{last}",
                    self.name
                );
            }
            last = self.capture_pane();
            if last.contains(needle) {
                return last;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "tmux pane never contained {needle:?} within {timeout:?}; last capture:\n{last}"
        );
    }

    fn has_session(&self) -> bool {
        self.run(&["has-session", "-t", &self.name]).status.success()
    }

    fn kill(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if self.killed {
            return;
        }
        let _ = self.run(&["kill-session", "-t", &self.name]);
        let _ = self.run(&["kill-server"]);
        let _ = std::fs::remove_file(&self.socket);
        self.killed = true;
    }
}

impl Drop for TmuxSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Isolated HOME + data dirs so the binary never touches the operator's real cortex state.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "cortex-tmux-scratch-{}-{}-{}",
            tag,
            std::process::id(),
            SESSION_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).expect("home");
        std::fs::create_dir_all(root.join("config")).expect("config");
        std::fs::create_dir_all(root.join("data")).expect("data");
        std::fs::create_dir_all(root.join("work")).expect("work");
        Self { root }
    }

    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn config(&self) -> PathBuf {
        self.root.join("config")
    }
    fn data(&self) -> PathBuf {
        self.root.join("data")
    }
    fn work(&self) -> PathBuf {
        self.root.join("work")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn apply_isolated_env(cmd: &mut Command, scratch: &Scratch) {
    cmd.env("HOME", scratch.home());
    cmd.env("XDG_CONFIG_HOME", scratch.config());
    cmd.env("XDG_DATA_HOME", scratch.data());
    cmd.env("CORTEX_DATA_DIR", scratch.data().join("cortex"));
    cmd.env("CORTEX_DB_PATH", scratch.data().join("cortex.db"));
    cmd.env_remove("RUST_LOG");
    cmd.env_remove("CORTEX_VERBOSE");
    cmd.env_remove("NO_COLOR");
}

#[test]
fn cortex_version_and_help_exit_zero() {
    let scratch = Scratch::new("version-help");
    let bin = cortex_bin();

    for args in [["--version"].as_slice(), ["--help"].as_slice()] {
        let mut cmd = Command::new(&bin);
        cmd.args(args);
        apply_isolated_env(&mut cmd, &scratch);
        let out = cmd.output().expect("spawn cortex");
        assert!(
            out.status.success(),
            "cortex {args:?} failed: status={:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            !stdout.trim().is_empty(),
            "cortex {args:?} produced empty stdout"
        );
        if args == ["--version"] {
            assert!(
                stdout.contains("cortex") || stdout.contains("0."),
                "unexpected --version output: {stdout}"
            );
        }
        if args == ["--help"] {
            assert!(
                stdout.to_lowercase().contains("usage") || stdout.contains("Commands:"),
                "unexpected --help output: {stdout}"
            );
        }
    }
}

#[test]
fn tmux_smoke_launch_capture_teardown() {
    require_tmux();
    let scratch = Scratch::new("smoke");
    let bin = cortex_bin();
    let bin_str = bin.display().to_string();

    // Long-lived shell so the session stays up; run --version via the shell command line.
    let shell = format!(
        "export HOME={home:?} XDG_CONFIG_HOME={cfg:?} XDG_DATA_HOME={data:?} \
         CORTEX_DATA_DIR={cdata:?} CORTEX_DB_PATH={db:?} PS1='$ '; \
         cd {work:?}; {bin} --version; exec bash --noprofile --norc",
        home = scratch.home(),
        cfg = scratch.config(),
        data = scratch.data(),
        cdata = scratch.data().join("cortex"),
        db = scratch.data().join("cortex.db"),
        work = scratch.work(),
        bin = bin_str,
    );

    let session = TmuxSession::spawn_shell("smoke", &shell);
    let name = session.name.clone();
    let socket = session.socket.clone();

    let pane = session.wait_capture_contains("cortex", WAIT_TIMEOUT);
    assert!(
        pane.to_lowercase().contains("cortex"),
        "pane missing cortex banner:\n{pane}"
    );
    assert!(session.has_session(), "session should still be alive after capture");

    session.kill();

    // Dedicated socket is gone.
    assert!(
        !socket.exists()
            || Command::new("tmux")
                .args(["-S", &socket.display().to_string(), "list-sessions"])
                .status()
                .map(|s| !s.success())
                .unwrap_or(true),
        "dedicated tmux server still reachable after teardown"
    );

    // Global default-server must not have been dirtied with our session name.
    assert!(
        !default_server_has_session(&name),
        "orphan default-server session still present: {name}"
    );
    kill_default_sessions_matching_prefix(&format!("{SESSION_PREFIX}-smoke-"));
}

#[test]
fn tmux_send_keys_and_help_capture() {
    require_tmux();
    let scratch = Scratch::new("keys");
    let bin = cortex_bin();
    let bin_str = bin.display().to_string();

    // Keep an interactive shell in the pane so we can send keys after spawn.
    let shell = format!(
        "export HOME={home:?} XDG_CONFIG_HOME={cfg:?} XDG_DATA_HOME={data:?} \
         CORTEX_DATA_DIR={cdata:?} CORTEX_DB_PATH={db:?} PS1='$ '; \
         cd {work:?}; exec bash --noprofile --norc",
        home = scratch.home(),
        cfg = scratch.config(),
        data = scratch.data(),
        cdata = scratch.data().join("cortex"),
        db = scratch.data().join("cortex.db"),
        work = scratch.work(),
    );

    let session = TmuxSession::spawn_shell("keys", &shell);
    // Wait for shell readiness, then drive cortex via send-keys.
    thread::sleep(Duration::from_millis(150));
    session.send_keys(&[&format!("{bin_str} --help"), "Enter"]);
    let pane = session.wait_capture_contains("Usage:", WAIT_TIMEOUT);
    assert!(
        pane.contains("Usage:") || pane.contains("Commands:"),
        "help capture missing expected tokens:\n{pane}"
    );

    // Send another short command and re-capture.
    session.send_keys(&[&format!("{bin_str} --version"), "Enter"]);
    let pane2 = session.wait_capture_contains("cortex", WAIT_TIMEOUT);
    assert!(
        pane2.to_lowercase().contains("cortex"),
        "version capture missing cortex:\n{pane2}"
    );

    let name = session.name.clone();
    session.kill();
    assert!(
        !default_server_has_session(&name),
        "orphan session after keys test: {name}"
    );
}

#[test]
fn serve_default_port_policy_is_loopback_18765() {
    // Structural CLI default (no server start). Keeps port policy asserted without binding.
    let scratch = Scratch::new("serve-default");
    let mut cmd = Command::new(cortex_bin());
    cmd.args(["serve", "--help"]);
    apply_isolated_env(&mut cmd, &scratch);
    let out = cmd.output().expect("serve --help");
    assert!(out.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("18765"),
        "serve --help must document default port 18765, got:\n{text}"
    );
    assert!(
        text.contains("127.0.0.1"),
        "serve --help must document loopback host 127.0.0.1, got:\n{text}"
    );
}

/// Optional live probe. Gated on `CORTEX_LIVE=1` **and** a non-empty `OPENROUTER_API_KEY`.
/// Never prints the key or response body beyond a redacted length summary.
#[test]
fn optional_cortex_live_openrouter_probe() {
    let live = std::env::var("CORTEX_LIVE").unwrap_or_default();
    if live != "1" && live.to_lowercase() != "true" {
        eprintln!("skip optional_cortex_live_openrouter_probe: CORTEX_LIVE not enabled");
        return;
    }
    let key = match std::env::var("OPENROUTER_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => {
            eprintln!(
                "skip optional_cortex_live_openrouter_probe: OPENROUTER_API_KEY unset/empty"
            );
            return;
        }
    };
    // Refuse to proceed if someone exported a placeholder that looks committed.
    assert!(
        !key.contains("YOUR_") && !key.contains("changeme"),
        "refusing placeholder API key"
    );

    let scratch = Scratch::new("live-probe");
    let mut cmd = Command::new(cortex_bin());
    cmd.args([
        "run",
        "--format",
        "text",
        "Reply with exactly the four characters: pong",
    ]);
    apply_isolated_env(&mut cmd, &scratch);
    cmd.env("OPENROUTER_API_KEY", &key);
    cmd.env_remove("CORTEX_NO_NET");
    cmd.current_dir(scratch.work());
    cmd.stdin(Stdio::null());

    let start = Instant::now();
    let out = cmd
        .output()
        .expect("spawn cortex run live probe");
    let elapsed = start.elapsed();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Never echo key material if it accidentally appears.
    assert!(
        !stdout.contains(key.trim()) && !stderr.contains(key.trim()),
        "live probe leaked API key into output (redacted assertion)"
    );

    assert!(
        out.status.success(),
        "live probe failed status={:?} stderr_len={} stdout_len={} elapsed={elapsed:?}",
        out.status,
        stderr.len(),
        stdout.len()
    );
    assert!(
        !stdout.trim().is_empty(),
        "live probe returned empty assistant text"
    );
    // Soft content check: model should mention pong, but we accept any non-empty reply.
    eprintln!(
        "live probe ok chars={} elapsed_ms={} (body redacted)",
        stdout.trim().chars().count(),
        elapsed.as_millis()
    );
}

#[test]
fn no_orphan_cortex_e2e_sessions_after_harness_helpers() {
    require_tmux();
    // Create a long-lived holder, confirm it exists, then tear down cleanly.
    let session = TmuxSession::spawn_shell("orphan-check", "exec sleep 60");
    let name = session.name.clone();
    let socket = session.socket.clone();
    assert!(session.has_session(), "session should exist right after spawn");
    session.kill();
    assert!(
        !default_server_has_session(&name),
        "session {name} should be gone from default server"
    );
    // Dedicated socket server must not linger.
    let still = Command::new("tmux")
        .args(["-S", &socket.display().to_string(), "has-session", "-t", &name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(!still, "dedicated socket still hosts session {name}");
    kill_default_sessions_matching_prefix(SESSION_PREFIX);
}

/// Normalize pane text for golden compares (strip trailing spaces per line).
#[allow(dead_code)]
fn normalize_pane(text: &str) -> String {
    text.lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

#[test]
fn normalize_pane_trims_trailing_ws() {
    assert_eq!(normalize_pane("ab  \ncd\t\n"), "ab\ncd");
}

/// Public helper path check used by future golden drivers under `tests/e2e_tmux/`.
#[test]
fn workspace_e2e_tmux_dir_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/e2e_tmux");
    assert!(
        root.exists(),
        "expected tests/e2e_tmux directory at {}",
        root.display()
    );
}

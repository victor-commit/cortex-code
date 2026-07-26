//! Black-box proof for `cortex models --refresh`: a real models.dev fetch behind the
//! `CORTEX_NO_NET` kill-switch, with the plain listing staying fully offline and a failed refresh
//! degrading cleanly without corrupting the catalogue. Covers VAL-FINISH-005..007 and VAL-FINISH-009.
//!
//! Network determinism: the happy-path and forced-failure cases drive the binary against a local
//! fixture endpoint via the `CORTEX_MODELS_DEV_URL` seam (bound on an OS-assigned ephemeral loopback
//! port, 127.0.0.1:0), so they never depend on external connectivity. A separate, connectivity-gated
//! test exercises the real models.dev endpoint to prove the live fetch (VAL-FINISH-005).

use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// The dedicated exit code for a kill-switch refusal (`Exit::NetDisabled`).
const NET_DISABLED_CODE: i32 = 4;

const DEFAULT_MODEL: &str = "google/gemini-2.5-flash";

/// A models.dev-shaped fixture: the openrouter provider with several live ids (two absent from the
/// embedded snapshot, which dynamic curation must retain) and a second provider that mirrors the
/// real multi-provider response shape.
const FIXTURE: &str = r#"{
  "anthropic": { "id": "anthropic", "name": "Anthropic", "models": {} },
  "openrouter": {
    "id": "openrouter",
    "name": "OpenRouter",
    "api": "https://openrouter.ai/api/v1",
    "env": ["OPENROUTER_API_KEY"],
    "models": {
      "google/gemini-2.5-flash": { "id": "google/gemini-2.5-flash", "name": "Gemini 2.5 Flash", "tool_call": true, "cost": { "input": 0.3, "output": 2.5 }, "limit": { "context": 1048576, "output": 65535 } },
      "anthropic/claude-sonnet-4": { "id": "anthropic/claude-sonnet-4", "name": "Claude Sonnet 4", "tool_call": true, "cost": { "input": 3, "output": 15 }, "limit": { "context": 1000000, "output": 64000 } },
      "openai/gpt-4o": { "id": "openai/gpt-4o", "name": "GPT-4o", "tool_call": true, "cost": { "input": 2.5, "output": 10 }, "limit": { "context": 128000, "output": 16384 } },
      "anthropic/claude-3.5-sonnet": { "id": "anthropic/claude-3.5-sonnet", "name": "Claude 3.5 Sonnet", "cost": { "input": 3, "output": 15 }, "limit": { "context": 200000, "output": 8192 } },
      "some/unlisted-model-v9": { "id": "some/unlisted-model-v9", "name": "Unlisted", "cost": { "input": 1, "output": 1 }, "limit": { "context": 1000, "output": 100 } }
    }
  }
}"#;

struct Sandbox {
    home: PathBuf,
    project: PathBuf,
    data_dir: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "cortex-cli-refresh-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let project = home.join("project");
        let data_dir = home.join("data");
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&project).expect("create project");
        Self {
            home,
            project,
            data_dir,
        }
    }

    fn cache_file(&self) -> PathBuf {
        self.data_dir.join("models.json")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Build a `cortex` invocation in an isolated HOME/CWD/data-dir with no ambient credential.
fn cortex(sandbox: &Sandbox, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
    command.current_dir(&sandbox.project);
    command.args(args);
    command.env("HOME", &sandbox.home);
    command.env("XDG_CONFIG_HOME", sandbox.home.join(".config"));
    command.env("CORTEX_DATA_DIR", &sandbox.data_dir);
    command.env_remove("OPENROUTER_API_KEY");
    command.env_remove("CORTEX_NO_NET");
    command.env_remove("CORTEX_MODELS_DEV_URL");
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

/// Bind a fixture HTTP server on an OS-assigned ephemeral loopback port.
fn bind_fixture_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener");
    let port = listener.local_addr().expect("local addr").port();
    (listener, port)
}

/// Obtain a loopback port that refuses connections, for a deterministic fetch failure: bind an
/// OS-assigned ephemeral port, read it, then drop the listener so the port is closed.
fn closed_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral loopback listener");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

/// Serve a single HTTP/1.1 response carrying `body` and then close, bounded so the thread never
/// hangs if no client connects.
fn serve_once(listener: TcpListener, body: String) -> JoinHandle<()> {
    listener
        .set_nonblocking(true)
        .expect("listener nonblocking");
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).ok();
                    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
                    let mut buf = [0u8; 8192];
                    let _ = stream.read(&mut buf);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    return;
                }
                Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() > deadline {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        }
    })
}

// ---- strace helpers (connect-syscall tracing), gated on availability ---------------------------

/// Whether `strace` can trace a child here (it may be absent or `ptrace` may be restricted).
fn strace_usable() -> bool {
    let probe = Command::new("strace")
        .args([
            "-f",
            "-e",
            "trace=connect",
            env!("CARGO_BIN_EXE_cortex"),
            "--version",
        ])
        .output();
    match probe {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            out.status.success() && !stderr.to_lowercase().contains("ptrace")
        }
        Err(_) => false,
    }
}

/// Run `cortex` under `strace`, returning (connect_count, exit_code) for the given env tweaks.
fn strace_connect_count(sandbox: &Sandbox, args: &[&str], env: &[(&str, &str)]) -> (usize, i32) {
    let mut cmd = Command::new("strace");
    cmd.args(["-f", "-e", "trace=connect", env!("CARGO_BIN_EXE_cortex")])
        .args(args)
        .current_dir(&sandbox.project)
        .env("HOME", &sandbox.home)
        .env("CORTEX_DATA_DIR", &sandbox.data_dir)
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("CORTEX_NO_NET")
        .env_remove("CORTEX_MODELS_DEV_URL")
        .stdout(Stdio::null());
    for (key, value) in env {
        cmd.env(key, value);
    }
    let out = cmd.output().expect("spawn strace");
    let trace = String::from_utf8_lossy(&out.stderr);
    (
        trace.matches("connect(").count(),
        out.status.code().unwrap_or(-1),
    )
}

fn no_panic(stderr: &str) {
    assert!(
        !stderr.contains("panicked at") && !stderr.contains("RUST_BACKTRACE"),
        "must not panic: {stderr}"
    );
}

// ---- VAL-FINISH-006: `--refresh` honors CORTEX_NO_NET=1 (fail-closed before any socket) --------

#[test]
fn refresh_under_kill_switch_fails_closed() {
    let sandbox = Sandbox::new("no-net");
    let mut command = cortex(&sandbox, &["models", "--refresh"]);
    command.env("CORTEX_NO_NET", "1");
    let output = run(command, Duration::from_secs(20));

    assert_eq!(
        output.status.code(),
        Some(NET_DISABLED_CODE),
        "the kill-switch must exit with the dedicated net-disabled code"
    );
    assert!(
        output.stdout.is_empty(),
        "a refusal keeps stdout byte-empty (it must not claim a successful refresh)"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    no_panic(&stderr);
    assert!(
        stderr.to_lowercase().contains("network disabled"),
        "stderr must read as a net-disabled error: {stderr}"
    );
    assert!(
        stderr.contains("CORTEX_NO_NET"),
        "stderr must name the kill-switch: {stderr}"
    );
    assert!(
        !sandbox.cache_file().exists(),
        "a refused refresh must not write a cache"
    );
}

#[test]
fn refresh_under_kill_switch_makes_zero_connects() {
    if !strace_usable() {
        eprintln!("skipping strace assertion: strace unavailable or ptrace restricted");
        return;
    }
    let sandbox = Sandbox::new("no-net-strace");
    let (connects, code) = strace_connect_count(
        &sandbox,
        &["models", "--refresh"],
        &[("CORTEX_NO_NET", "1")],
    );
    assert_eq!(
        connects, 0,
        "under CORTEX_NO_NET=1 the refresh must issue zero connect() syscalls"
    );
    assert_eq!(code, NET_DISABLED_CODE, "and exit net-disabled");
}

// ---- VAL-FINISH-007: plain `cortex models` performs no network access --------------------------

#[test]
fn plain_listing_succeeds_offline() {
    let sandbox = Sandbox::new("offline");
    // Even with the kill-switch set, the plain listing succeeds (it never touches the network).
    let mut command = cortex(&sandbox, &["models"]);
    command.env("CORTEX_NO_NET", "1");
    let output = run(command, Duration::from_secs(20));
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(DEFAULT_MODEL),
        "offline listing must include the default model: {stdout}"
    );
}

#[test]
fn plain_listing_makes_zero_connects() {
    if !strace_usable() {
        eprintln!("skipping strace assertion: strace unavailable or ptrace restricted");
        return;
    }
    let sandbox = Sandbox::new("offline-strace");
    let (connects, code) = strace_connect_count(&sandbox, &["models"], &[]);
    assert_eq!(
        connects, 0,
        "plain `cortex models` must open no outbound socket"
    );
    assert_eq!(code, 0, "plain listing exits 0");
}

// ---- VAL-FINISH-005 (deterministic): refresh hits a real socket, lists, and caches -------------

#[test]
fn refresh_against_local_endpoint_lists_curated_models_and_caches() {
    let sandbox = Sandbox::new("local-refresh");
    let (listener, port) = bind_fixture_port();
    let server = serve_once(listener, FIXTURE.to_owned());
    let url = format!("http://127.0.0.1:{port}/api.json");

    let mut command = cortex(&sandbox, &["models", "--refresh", "--format", "json"]);
    command.env("CORTEX_MODELS_DEV_URL", &url);
    let output = run(command, Duration::from_secs(30));
    server.join().ok();

    assert_eq!(
        output.status.code(),
        Some(0),
        "a successful refresh exits 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("refresh --format json is a strict document");
    let ids: Vec<String> = doc["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .flat_map(|p| p["models"].as_array().expect("models array"))
        .map(|m| m["id"].as_str().unwrap_or_default().to_owned())
        .collect();

    // The primary provider is curated DYNAMICALLY from the fetched list: well-formed live ids are
    // listed, INCLUDING ids absent from the tiny embedded snapshot (they are no longer intersected
    // away). Only malformed entries are dropped.
    assert!(ids.iter().any(|id| id == DEFAULT_MODEL), "ids: {ids:?}");
    assert!(
        ids.iter().any(|id| id == "anthropic/claude-sonnet-4"),
        "ids: {ids:?}"
    );
    assert!(
        ids.iter().any(|id| id == "anthropic/claude-3.5-sonnet"),
        "a snapshot-absent live id must be retained by dynamic curation: {ids:?}"
    );
    assert!(
        ids.iter().any(|id| id == "some/unlisted-model-v9"),
        "a snapshot-absent live id must be retained by dynamic curation: {ids:?}"
    );

    // The refresh wrote a cache, and a subsequent offline listing reads it (source = "cached").
    assert!(
        sandbox.cache_file().exists(),
        "a successful refresh writes the on-disk cache"
    );
    let listing = run(
        cortex(&sandbox, &["models", "--format", "json"]),
        Duration::from_secs(20),
    );
    assert_eq!(listing.status.code(), Some(0));
    let listing_doc: serde_json::Value =
        serde_json::from_slice(&listing.stdout).expect("listing json");
    assert_eq!(
        listing_doc["providers"][0]["source"], "cached",
        "after a refresh the plain listing reads the cache: {listing_doc}"
    );
}

#[test]
fn refresh_against_local_endpoint_makes_at_least_one_connect() {
    if !strace_usable() {
        eprintln!("skipping strace assertion: strace unavailable or ptrace restricted");
        return;
    }
    let sandbox = Sandbox::new("local-refresh-strace");
    let (listener, port) = bind_fixture_port();
    let server = serve_once(listener, FIXTURE.to_owned());
    let url = format!("http://127.0.0.1:{port}/api.json");

    let (connects, code) = strace_connect_count(
        &sandbox,
        &["models", "--refresh"],
        &[("CORTEX_MODELS_DEV_URL", &url)],
    );
    server.join().ok();
    assert!(
        connects >= 1,
        "the refresh path must reach a real connect() (unlike the plain listing)"
    );
    assert_eq!(code, 0, "the local refresh succeeds");
}

#[test]
fn refresh_text_early_stdout_close_exits_cleanly() {
    let sandbox = Sandbox::new("early-close");
    let (listener, port) = bind_fixture_port();
    let server = serve_once(listener, FIXTURE.to_owned());
    let url = format!("http://127.0.0.1:{port}/api.json");

    let mut command = cortex(&sandbox, &["models", "--refresh"]);
    command.env("CORTEX_MODELS_DEV_URL", &url);
    let (output, first_line) = run_with_early_stdout_close(command, Duration::from_secs(30));
    server.join().ok();

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

// ---- VAL-FINISH-009: a failed `--refresh` degrades cleanly (no crash, no corruption) -----------

#[test]
fn refresh_forced_failure_degrades_cleanly() {
    let sandbox = Sandbox::new("forced-fail");
    let url = format!("http://127.0.0.1:{}/api.json", closed_port());

    let mut command = cortex(&sandbox, &["models", "--refresh"]);
    command.env("CORTEX_MODELS_DEV_URL", &url);
    // The connect_timeout (5s) bounds this well under the harness timeout.
    let output = run(command, Duration::from_secs(20));

    assert_ne!(
        output.status.code(),
        Some(0),
        "a forced network failure must not report a successful refresh"
    );
    assert_ne!(
        output.status.code(),
        Some(NET_DISABLED_CODE),
        "a real fetch failure is distinct from a kill-switch refusal"
    );
    assert!(output.stdout.is_empty(), "no listing on a failed refresh");
    let stderr = String::from_utf8_lossy(&output.stderr);
    no_panic(&stderr);
    assert!(
        stderr.to_lowercase().contains("transport") || stderr.to_lowercase().contains("refresh"),
        "stderr must carry a clear network/refresh diagnostic: {stderr}"
    );
    assert!(
        !sandbox.cache_file().exists(),
        "a failed refresh must not write (or corrupt) the cache"
    );

    // A subsequent plain listing still works (the catalogue is not corrupted).
    let listing = run(cortex(&sandbox, &["models"]), Duration::from_secs(20));
    assert_eq!(listing.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&listing.stdout);
    assert!(
        stdout.contains(DEFAULT_MODEL),
        "the offline catalogue still lists models after a failed refresh: {stdout}"
    );
}

#[test]
fn refresh_failure_preserves_an_existing_cache() {
    let sandbox = Sandbox::new("preserve-cache");

    // First, a successful refresh against the local fixture writes a cache.
    let (listener, port) = bind_fixture_port();
    let server = serve_once(listener, FIXTURE.to_owned());
    let url = format!("http://127.0.0.1:{port}/api.json");
    let mut ok = cortex(&sandbox, &["models", "--refresh"]);
    ok.env("CORTEX_MODELS_DEV_URL", &url);
    assert_eq!(run(ok, Duration::from_secs(30)).status.code(), Some(0));
    server.join().ok();
    let cached_before =
        std::fs::read_to_string(sandbox.cache_file()).expect("cache exists after success");

    // Then a failed refresh must leave that cache byte-for-byte intact.
    let bad_url = format!("http://127.0.0.1:{}/api.json", closed_port());
    let mut bad = cortex(&sandbox, &["models", "--refresh"]);
    bad.env("CORTEX_MODELS_DEV_URL", &bad_url);
    let failed = run(bad, Duration::from_secs(20));
    assert_ne!(failed.status.code(), Some(0));

    let cached_after =
        std::fs::read_to_string(sandbox.cache_file()).expect("cache still present after failure");
    assert_eq!(
        cached_before, cached_after,
        "a failed refresh must not modify the existing cache"
    );
}

// ---- VAL-FINISH-005 (live): a real models.dev refresh, gated on real connectivity --------------

/// Whether the real models.dev endpoint is reachable (skip the live proof cleanly otherwise).
fn models_dev_reachable() -> bool {
    let addrs = match std::net::ToSocketAddrs::to_socket_addrs(&("models.dev", 443)) {
        Ok(addrs) => addrs,
        Err(_) => return false,
    };
    for addr in addrs {
        if TcpStream::connect_timeout(&addr, Duration::from_secs(5)).is_ok() {
            return true;
        }
    }
    false
}

#[test]
fn live_refresh_fetches_models_dev() {
    if !models_dev_reachable() {
        eprintln!("skipping live models.dev refresh: endpoint unreachable");
        return;
    }
    let sandbox = Sandbox::new("live");
    // No CORTEX_MODELS_DEV_URL override: hit the real models.dev endpoint.
    let output = run(
        cortex(&sandbox, &["models", "--refresh"]),
        Duration::from_secs(40),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "a live refresh exits 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    no_panic(&String::from_utf8_lossy(&output.stderr));
    assert!(
        stdout.contains(DEFAULT_MODEL),
        "the live refresh lists the default model: {stdout}"
    );
    // The primary provider is curated dynamically from the CURRENT models.dev list (not pinned to
    // the embedded ids), so it surfaces current models across many vendors.
    assert!(
        stdout.contains("anthropic/"),
        "the live refresh surfaces current anthropic-vendor models: {stdout}"
    );
    assert!(
        sandbox.cache_file().exists(),
        "the live refresh caches the fetched snapshot"
    );
}

// ---- W2 first-class providers: refresh surfaces them; offline cache keeps them ------------

/// models.dev-shaped fixture covering the W2 first-class catalogue providers (anthropic,
/// openai, google, plus several OpenAI-Chat catalogue vendors). Each entry carries at least one
/// well-formed model so `curate_provider` retains the provider (empty model maps are dropped).
const W2_FIXTURE: &str = r#"{
  "openrouter": {
    "id": "openrouter",
    "name": "OpenRouter",
    "api": "https://openrouter.ai/api/v1",
    "env": ["OPENROUTER_API_KEY"],
    "models": {
      "google/gemini-2.5-flash": { "id": "google/gemini-2.5-flash", "name": "Gemini 2.5 Flash", "tool_call": true, "cost": { "input": 0.3, "output": 2.5 }, "limit": { "context": 1048576, "output": 65535 } }
    }
  },
  "anthropic": {
    "id": "anthropic",
    "name": "Anthropic",
    "api": "https://api.anthropic.com",
    "env": ["ANTHROPIC_API_KEY"],
    "models": {
      "claude-sonnet-4": { "id": "claude-sonnet-4", "name": "Claude Sonnet 4", "tool_call": true, "cost": { "input": 3, "output": 15 }, "limit": { "context": 200000, "output": 64000 } }
    }
  },
  "openai": {
    "id": "openai",
    "name": "OpenAI",
    "api": "https://api.openai.com/v1",
    "env": ["OPENAI_API_KEY"],
    "models": {
      "gpt-4o": { "id": "gpt-4o", "name": "GPT-4o", "tool_call": true, "cost": { "input": 2.5, "output": 10 }, "limit": { "context": 128000, "output": 16384 } }
    }
  },
  "google": {
    "id": "google",
    "name": "Google",
    "api": "https://generativelanguage.googleapis.com",
    "env": ["GEMINI_API_KEY"],
    "models": {
      "gemini-2.5-flash": { "id": "gemini-2.5-flash", "name": "Gemini 2.5 Flash", "tool_call": true, "cost": { "input": 0.3, "output": 2.5 }, "limit": { "context": 1048576, "output": 65535 } }
    }
  },
  "deepseek": {
    "id": "deepseek",
    "name": "DeepSeek",
    "api": "https://api.deepseek.com",
    "env": ["DEEPSEEK_API_KEY"],
    "models": {
      "deepseek-chat": { "id": "deepseek-chat", "name": "DeepSeek Chat", "tool_call": true, "cost": { "input": 0.14, "output": 0.28 }, "limit": { "context": 64000, "output": 8192 } }
    }
  },
  "moonshotai": {
    "id": "moonshotai",
    "name": "Moonshot AI",
    "api": "https://api.moonshot.cn/v1",
    "env": ["MOONSHOT_API_KEY"],
    "models": {
      "kimi-k2": { "id": "kimi-k2", "name": "Kimi K2", "tool_call": true, "cost": { "input": 0.6, "output": 2.5 }, "limit": { "context": 128000, "output": 8192 } }
    }
  },
  "zai": {
    "id": "zai",
    "name": "Z.AI",
    "api": "https://api.z.ai/api/paas/v4",
    "env": ["ZAI_API_KEY"],
    "models": {
      "glm-4.5": { "id": "glm-4.5", "name": "GLM 4.5", "tool_call": true, "cost": { "input": 0.6, "output": 2.2 }, "limit": { "context": 128000, "output": 8192 } }
    }
  },
  "fireworks-ai": {
    "id": "fireworks-ai",
    "name": "Fireworks AI",
    "api": "https://api.fireworks.ai/inference/v1",
    "env": ["FIREWORKS_API_KEY"],
    "models": {
      "llama-v3p1-70b": { "id": "llama-v3p1-70b", "name": "Llama 3.1 70B", "tool_call": true, "cost": { "input": 0.9, "output": 0.9 }, "limit": { "context": 131072, "output": 8192 } }
    }
  },
  "huggingface": {
    "id": "huggingface",
    "name": "Hugging Face",
    "api": "https://router.huggingface.co/v1",
    "env": ["HF_TOKEN"],
    "models": {
      "meta-llama/Llama-3.1-8B-Instruct": { "id": "meta-llama/Llama-3.1-8B-Instruct", "name": "Llama 3.1 8B Instruct", "tool_call": true, "cost": { "input": 0.1, "output": 0.1 }, "limit": { "context": 128000, "output": 4096 } }
    }
  },
  "nvidia": {
    "id": "nvidia",
    "name": "NVIDIA",
    "api": "https://integrate.api.nvidia.com/v1",
    "env": ["NVIDIA_API_KEY"],
    "models": {
      "meta/llama-3.1-70b-instruct": { "id": "meta/llama-3.1-70b-instruct", "name": "Llama 3.1 70B Instruct", "tool_call": true, "cost": { "input": 0.35, "output": 0.4 }, "limit": { "context": 128000, "output": 4096 } }
    }
  },
  "baseten": {
    "id": "baseten",
    "name": "Baseten",
    "api": "https://inference.baseten.co/v1",
    "env": ["BASETEN_API_KEY"],
    "models": {
      "deepseek-ai/DeepSeek-V3": { "id": "deepseek-ai/DeepSeek-V3", "name": "DeepSeek V3", "tool_call": true, "cost": { "input": 0.5, "output": 1.5 }, "limit": { "context": 64000, "output": 8192 } }
    }
  },
  "minimax": {
    "id": "minimax",
    "name": "MiniMax",
    "api": "https://api.minimax.io/v1",
    "env": ["MINIMAX_API_KEY"],
    "models": {
      "MiniMax-M1": { "id": "MiniMax-M1", "name": "MiniMax M1", "tool_call": true, "cost": { "input": 0.4, "output": 2.2 }, "limit": { "context": 1000000, "output": 40000 } }
    }
  },
  "nebius": {
    "id": "nebius",
    "name": "Nebius",
    "api": "https://api.studio.nebius.ai/v1",
    "env": ["NEBIUS_API_KEY"],
    "models": {
      "meta-llama/Meta-Llama-3.1-70B-Instruct": { "id": "meta-llama/Meta-Llama-3.1-70B-Instruct", "name": "Llama 3.1 70B Instruct", "tool_call": true, "cost": { "input": 0.13, "output": 0.4 }, "limit": { "context": 128000, "output": 4096 } }
    }
  },
  "venice": {
    "id": "venice",
    "name": "Venice",
    "api": "https://api.venice.ai/api/v1",
    "env": ["VENICE_API_KEY"],
    "models": {
      "llama-3.3-70b": { "id": "llama-3.3-70b", "name": "Llama 3.3 70B", "tool_call": true, "cost": { "input": 0.7, "output": 2.8 }, "limit": { "context": 128000, "output": 8192 } }
    }
  }
}"#;

/// Provider ids the W2 first-class profiles pin for models.dev-backed vendors. Local-only
/// profiles (`lmstudio`, `llamacpp`) have no `models_dev_id` and are not expected from a
/// models.dev refresh — they are asserted only when already present.
const W2_CATALOGUE_PROVIDER_IDS: &[&str] = &[
    "anthropic",
    "openai",
    "google",
    "deepseek",
    "moonshotai",
    "zai",
    "fireworks-ai",
    "huggingface",
    "nvidia",
    "baseten",
    "minimax",
    "nebius",
    "venice",
];

fn provider_ids_from_models_json(stdout: &[u8]) -> Vec<String> {
    let doc: serde_json::Value =
        serde_json::from_slice(stdout).expect("models --format json is a strict document");
    doc["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .map(|provider| {
            provider["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

#[test]
fn w2_providers_appear_in_models_and_survive_offline_cache() {
    let sandbox = Sandbox::new("w2-providers");
    let (listener, port) = bind_fixture_port();
    let server = serve_once(listener, W2_FIXTURE.to_owned());
    let url = format!("http://127.0.0.1:{port}/api.json");

    // Given: a models.dev fixture carrying the W2 first-class catalogue providers
    // When: cortex models --refresh --format json hits that fixture
    let mut refresh = cortex(&sandbox, &["models", "--refresh", "--format", "json"]);
    refresh.env("CORTEX_MODELS_DEV_URL", &url);
    let refresh_out = run(refresh, Duration::from_secs(30));
    server.join().ok();

    assert_eq!(
        refresh_out.status.code(),
        Some(0),
        "W2 refresh exits 0; stderr: {}",
        String::from_utf8_lossy(&refresh_out.stderr)
    );
    let refreshed_ids = provider_ids_from_models_json(&refresh_out.stdout);
    for id in W2_CATALOGUE_PROVIDER_IDS {
        assert!(
            refreshed_ids.iter().any(|listed| listed == id),
            "refresh must surface W2 provider {id}; got {refreshed_ids:?}"
        );
    }
    assert!(
        refreshed_ids.iter().any(|id| id == "openrouter"),
        "primary openrouter provider remains: {refreshed_ids:?}"
    );
    assert!(
        sandbox.cache_file().exists(),
        "successful W2 refresh writes the on-disk cache"
    );

    // When: plain listing under CORTEX_NO_NET=1 (no network, must read cache/embedded merge)
    // Then: the same W2 providers still appear, tagged as cached
    let mut offline = cortex(&sandbox, &["models", "--format", "json"]);
    offline.env("CORTEX_NO_NET", "1");
    let offline_out = run(offline, Duration::from_secs(20));
    assert_eq!(
        offline_out.status.code(),
        Some(0),
        "offline listing exits 0 under CORTEX_NO_NET; stderr: {}",
        String::from_utf8_lossy(&offline_out.stderr)
    );
    let offline_doc: serde_json::Value =
        serde_json::from_slice(&offline_out.stdout).expect("offline listing json");
    let offline_ids = provider_ids_from_models_json(&offline_out.stdout);
    for id in W2_CATALOGUE_PROVIDER_IDS {
        assert!(
            offline_ids.iter().any(|listed| listed == id),
            "CORTEX_NO_NET listing must keep W2 provider {id} from cache; got {offline_ids:?}"
        );
    }
    // At least one W2 provider (and openrouter) must report source=cached so we know the
    // offline path is the cache, not a silent re-fetch or empty embedded-only view.
    let sources: Vec<&str> = offline_doc["providers"]
        .as_array()
        .expect("providers array")
        .iter()
        .filter_map(|provider| provider["source"].as_str())
        .collect();
    assert!(
        sources.contains(&"cached"),
        "offline listing after refresh must read the cache (source=cached): {offline_doc}"
    );

    // Local-only profiles are not models.dev-backed; only assert if the listing already has them.
    for local_id in ["lmstudio", "llamacpp"] {
        if offline_ids.iter().any(|id| id == local_id) {
            assert!(
                refreshed_ids.iter().any(|id| id == local_id),
                "local provider {local_id} present offline must also have been in the refresh listing"
            );
        }
    }
}

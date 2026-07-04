//! Byte-stability baseline harness for the Wave 9 headless CLI surface.
//!
//! The mission's hard invariant is that `--format json` / `--format jsonl` machine bytes stay
//! byte-identical while the human/TTY text branch gains brand styling. This harness pins that
//! invariant so any later styling feature (`run`/`stats`/`search`/`models`/`replay`/`help`/`serve`)
//! that accidentally leaks styling into the machine path — or changes a machine format at all —
//! fails loudly.
//!
//! It proves three things for each machine command, deterministically and offline:
//! - **Zero ANSI when piped (VAL-CLI-001).** Piped/redirected stdout carries no `\x1b` byte.
//! - **TTY output equals piped output.** The same command captured over a REAL pseudo-terminal
//!   (so the binary sees `stdout().is_terminal() == true`) yields the identical machine bytes,
//!   proving the machine path never routes through the TTY-gated styler.
//! - **`NO_COLOR` on a TTY equals piped (VAL-CLI-002).** A TTY capture with `NO_COLOR=1` is also
//!   byte-identical, and likewise ANSI-free.
//!
//! It also pins the exact machine bytes (with the dynamic session id normalized) as a golden, so
//! a change to the serializers is caught even if all three captures move together.
//!
//! Reachability: fully offline. A throwaway SQLite DB is seeded via `session new` + `session
//! message`, and every probed command (`stats`, `replay`) is a local persistence read under
//! `CORTEX_NO_NET=1` — no network, no user data.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;

const RUN_TIMEOUT: Duration = Duration::from_secs(20);

/// A hermetic sandbox: a throwaway HOME + config dir + SQLite DB, seeded with one deterministic
/// session so machine output is reproducible byte-for-byte (only the session id is dynamic).
struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
    db_path: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "cortex-bytestable-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        let home = root.join("home");
        let config = root.join("config");
        fs::create_dir_all(&home).expect("create scratch HOME");
        fs::create_dir_all(&config).expect("create scratch config");
        Self {
            db_path: root.join("state/cortex.db"),
            root,
            home,
            config,
        }
    }

    /// The hermetic environment shared by every invocation (offline, no user data, no key).
    fn env(&self) -> Vec<(String, String)> {
        vec![
            ("HOME".to_owned(), self.home.display().to_string()),
            (
                "XDG_CONFIG_HOME".to_owned(),
                self.config.display().to_string(),
            ),
            (
                "CORTEX_DB_PATH".to_owned(),
                self.db_path.display().to_string(),
            ),
            ("CORTEX_NO_NET".to_owned(), "1".to_owned()),
        ]
    }

    /// A piped (non-TTY) invocation with the hermetic environment.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cortex"));
        command
            .args(args)
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("NO_COLOR");
        for (key, value) in self.env() {
            command.env(key, value);
        }
        command
    }

    /// Run piped and collect the raw stdout bytes (plus status/stderr).
    fn run_piped(&self, args: &[&str]) -> Output {
        collect(spawn(self.command(args)))
    }

    /// Run over a REAL pseudo-terminal so the binary observes `stdout().is_terminal() == true`,
    /// returning the child's stdout bytes with the terminal line-discipline CR injection removed
    /// (cooked-mode PTYs translate the program's `\n` into `\r\n`; that `\r` is a kernel artifact,
    /// not a byte the program emitted, so it is normalized away before comparison).
    fn run_in_pty(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Vec<u8> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 92,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");

        let mut builder = CommandBuilder::new(env!("CARGO_BIN_EXE_cortex"));
        for arg in args {
            builder.arg(arg);
        }
        builder.env_clear();
        builder.env("PATH", std::env::var("PATH").unwrap_or_default());
        builder.env("TERM", "xterm-256color");
        for (key, value) in self.env() {
            builder.env(key, value);
        }
        for (key, value) in extra_env {
            builder.env(key, value);
        }

        let mut child = pair.slave.spawn_command(builder).expect("spawn cortex in pty");
        // Drop the slave so the master read sees EOF once the child exits.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("clone pty reader");

        let output = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = reader.read_to_end(&mut buf);
            buf
        });

        let start = Instant::now();
        loop {
            if child.try_wait().expect("try_wait pty child").is_some() {
                break;
            }
            if start.elapsed() > RUN_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                panic!("cortex did not terminate within {RUN_TIMEOUT:?} in a pty");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Keep the master alive until the child has exited, then let the reader drain to EOF.
        let raw = output.join().expect("join pty reader");
        drop(pair.master);
        strip_cr(&raw)
    }

    fn new_session(&self) -> String {
        let out = self.run_piped(&["session", "new", "--format", "json"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "session new failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let doc: Value = serde_json::from_slice(&out.stdout).expect("session JSON");
        doc["id"].as_str().expect("session id").to_owned()
    }

    fn append_message(&self, session_id: &str, role: &str, content: &str) {
        let out = self.run_piped(&[
            "session", "message", session_id, content, "--role", role, "--format", "json",
        ]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "session message failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Seed the canonical deterministic transcript used by the golden assertions.
    fn seed(&self) -> String {
        let session_id = self.new_session();
        self.append_message(&session_id, "user", "hello world");
        self.append_message(&session_id, "assistant", "hi there");
        session_id
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn spawn(mut command: Command) -> Child {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.spawn().expect("spawn cortex binary")
}

fn collect(mut child: Child) -> Output {
    let mut out = child.stdout.take().expect("stdout handle");
    let mut err = child.stderr.take().expect("stderr handle");
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
        if start.elapsed() > RUN_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cortex did not terminate within {RUN_TIMEOUT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        status,
        stdout: out_thread.join().expect("join stdout"),
        stderr: err_thread.join().expect("join stderr"),
    }
}

/// Remove carriage returns injected by the PTY line discipline (`\n` → `\r\n`).
fn strip_cr(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().filter(|&b| b != b'\r').collect()
}

/// True if any ANSI/ESC (`0x1b`) byte is present.
fn contains_ansi(bytes: &[u8]) -> bool {
    bytes.contains(&0x1b)
}

/// Replace the concrete session id with a stable placeholder so machine bytes can be pinned.
fn normalize_session_id(bytes: &[u8], session_id: &str) -> String {
    String::from_utf8(bytes.to_vec())
        .expect("machine output is utf8")
        .replace(session_id, "<SID>")
}

/// The machine commands this harness pins. Each is offline and deterministic against the seeded DB.
/// Machine formats (`--format json`/`jsonl`) never route through the TTY styler, so they are
/// byte-identical on a TTY and when piped. A non-empty golden pins the exact bytes; an empty
/// golden means the content is dynamic (e.g. `models` reads the offline catalogue) so only the
/// zero-ANSI + TTY==pipe invariants are checked, not a fixed golden.
fn machine_commands(session_id: &str) -> Vec<(&'static str, Vec<&str>, &'static str)> {
    vec![
        (
            "stats_json",
            vec!["stats", "--format", "json"],
            r#"{"totals":{"input_tokens":0,"output_tokens":0,"total_tokens":0,"cost":0.0},"models":[],"sessions":[]}"#,
        ),
        (
            "stats_jsonl",
            vec!["stats", "--format", "jsonl"],
            r#"{"totals":{"input_tokens":0,"output_tokens":0,"total_tokens":0,"cost":0.0},"models":[],"sessions":[]}"#,
        ),
        (
            "replay_json",
            vec!["replay", session_id, "--format", "json"],
            r#"{"session_id":"<SID>","messages":[{"role":"user","text":"hello world"},{"role":"assistant","text":"hi there"}]}"#,
        ),
        (
            "replay_jsonl",
            vec!["replay", session_id, "--format", "jsonl"],
            r#"{"session_id":"<SID>","messages":[{"role":"user","text":"hello world"},{"role":"assistant","text":"hi there"}]}"#,
        ),
        // `cortex models` machine formats: dynamic catalogue content, so no fixed golden — but the
        // json/jsonl bytes must still be identical on a TTY and when piped (VAL-CLI-028).
        ("models_json", vec!["models", "--format", "json"], ""),
        ("models_jsonl", vec!["models", "--format", "jsonl"], ""),
    ]
}

/// The human/text commands whose TTY branch is intentionally styled. When piped they must emit the
/// historical plain bytes with zero ANSI (styling auto-disables off a TTY); on a TTY they gain
/// brand ANSI. `session_id` seeds the `replay` transcript.
fn human_text_commands(session_id: &str) -> Vec<(&'static str, Vec<&str>)> {
    vec![
        ("models_text", vec!["models"]),
        ("replay_text", vec!["replay", session_id]),
    ]
}

// VAL-CLI-001: every machine command emits zero ANSI escape bytes when piped/redirected.
#[test]
fn machine_output_has_zero_ansi_when_piped() {
    let sandbox = Sandbox::new("piped-ansi");
    let session_id = sandbox.seed();
    for (label, args, _golden) in machine_commands(&session_id) {
        let out = sandbox.run_piped(&args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{label} must exit 0: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !contains_ansi(&out.stdout),
            "{label} piped stdout must contain no ANSI/ESC byte, got: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

// Byte-stability baseline: piped machine bytes equal the pinned golden (session id normalized),
// and are exactly one JSON document terminated by a single `\n`.
#[test]
fn machine_output_matches_the_pinned_golden_bytes() {
    let sandbox = Sandbox::new("golden");
    let session_id = sandbox.seed();
    for (label, args, golden) in machine_commands(&session_id) {
        // Dynamic-content commands (empty golden) are covered by the ANSI + TTY==pipe invariants.
        if golden.is_empty() {
            continue;
        }
        let out = sandbox.run_piped(&args);
        assert_eq!(out.status.code(), Some(0), "{label} must exit 0");
        let normalized = normalize_session_id(&out.stdout, &session_id);
        let expected = format!("{golden}\n");
        assert_eq!(
            normalized, expected,
            "{label} machine bytes drifted from the pinned baseline"
        );
        // The document is exactly one line: strict JSON, single trailing newline.
        let body = normalized.strip_suffix('\n').expect("single trailing newline");
        assert!(
            !body.contains('\n'),
            "{label} must be a single compact line"
        );
        let _: Value = serde_json::from_str(body).expect("machine output parses as strict JSON");
    }
}

// VAL-CLI-001 (TTY axis): the machine path never routes through the styler, so a REAL-TTY capture
// is byte-identical to the piped capture and equally ANSI-free.
#[test]
fn machine_output_is_identical_on_a_tty_and_when_piped() {
    let sandbox = Sandbox::new("tty-eq-pipe");
    let session_id = sandbox.seed();
    for (label, args, _golden) in machine_commands(&session_id) {
        let piped = sandbox.run_piped(&args).stdout;
        let tty = sandbox.run_in_pty(&args, &[]);
        assert!(
            !contains_ansi(&tty),
            "{label} TTY stdout must contain no ANSI byte on the machine path, got: {:?}",
            String::from_utf8_lossy(&tty)
        );
        assert_eq!(
            normalize_session_id(&piped, &session_id),
            normalize_session_id(&tty, &session_id),
            "{label} machine bytes must be identical on a TTY and when piped"
        );
    }
}

// VAL-CLI-002: with NO_COLOR=1 on a TTY the machine bytes are still byte-identical to piped output
// and carry no ANSI (color is a human-text concern that never reaches the machine path anyway).
#[test]
fn no_color_on_a_tty_matches_piped_bytes() {
    let sandbox = Sandbox::new("no-color-tty");
    let session_id = sandbox.seed();
    for (label, args, _golden) in machine_commands(&session_id) {
        let piped = sandbox.run_piped(&args).stdout;
        let tty_no_color = sandbox.run_in_pty(&args, &[("NO_COLOR", "1")]);
        assert!(
            !contains_ansi(&tty_no_color),
            "{label} NO_COLOR TTY stdout must contain no ANSI byte, got: {:?}",
            String::from_utf8_lossy(&tty_no_color)
        );
        assert_eq!(
            normalize_session_id(&piped, &session_id),
            normalize_session_id(&tty_no_color, &session_id),
            "{label} NO_COLOR TTY bytes must equal piped bytes"
        );
    }
}

// Sanity: the PTY harness genuinely presents a terminal to the child. `cortex --version` is a
// stable, offline invocation; capturing it over the PTY must succeed and be ANSI-free (the version
// line is not styled), proving the harness's TTY plumbing works end to end.
#[test]
fn pty_harness_presents_a_terminal_and_captures_output() {
    let sandbox = Sandbox::new("pty-sanity");
    let captured = sandbox.run_in_pty(&["--version"], &[]);
    let text = String::from_utf8_lossy(&captured);
    assert!(
        text.contains("cortex") || text.contains("0.1.0"),
        "pty capture of --version should contain the version line, got: {text:?}"
    );
    assert!(
        !contains_ansi(&captured),
        "the --version line must not be styled"
    );
}

// VAL-CLI-028/032 (piped side): the styled human-text commands (`models`, `replay`) emit the
// historical plain bytes with ZERO ANSI when piped/redirected — the TTY styler auto-disables.
#[test]
fn styled_human_text_is_plain_when_piped() {
    let sandbox = Sandbox::new("human-piped");
    let session_id = sandbox.seed();
    for (label, args) in human_text_commands(&session_id) {
        let out = sandbox.run_piped(&args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{label} must exit 0: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            !contains_ansi(&out.stdout),
            "{label} piped stdout must contain no ANSI/ESC byte, got: {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
    // The piped `replay` text keeps the exact `[role] content\n` GOLD format.
    let replay = sandbox.run_piped(&["replay", &session_id]);
    assert_eq!(
        String::from_utf8_lossy(&replay.stdout),
        "[user] hello world\n[assistant] hi there\n",
        "piped replay text is the byte-stable GOLD format"
    );
}

// VAL-CLI-028/032 (TTY side): on a REAL terminal the styled human-text commands DO emit brand ANSI
// (the styler is enabled), proving the TTY branch is actually exercised.
#[test]
fn styled_human_text_is_ansi_on_a_tty() {
    let sandbox = Sandbox::new("human-tty");
    let session_id = sandbox.seed();
    for (label, args) in human_text_commands(&session_id) {
        let tty = sandbox.run_in_pty(&args, &[]);
        assert!(
            contains_ansi(&tty),
            "{label} TTY stdout must be styled with ANSI, got: {:?}",
            String::from_utf8_lossy(&tty)
        );
    }
}

// VAL-CLI-036: `cortex --help` piped/non-TTY carries NO ANSI and the derive-generated wording is
// unchanged (real subcommand names + option flags present).
#[test]
fn help_is_plain_and_unchanged_when_piped() {
    let sandbox = Sandbox::new("help-piped");
    let out = sandbox.run_piped(&["--help"]);
    assert_eq!(out.status.code(), Some(0), "--help exits 0");
    assert!(out.stderr.is_empty(), "--help writes nothing to stderr");
    assert!(
        !contains_ansi(&out.stdout),
        "piped --help must contain no ANSI byte, got: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    // Derive-generated metadata: usage, options, and real subcommand names are present.
    assert!(text.contains("Usage:"), "usage present: {text}");
    for name in ["run", "tui", "serve", "session", "models", "replay"] {
        assert!(text.contains(name), "piped help names {name}: {text}");
    }
}

// VAL-CLI-033/034: `cortex --help` on a REAL terminal is brand-styled (ANSI) and shows the curated
// shortlist plus the `…and N more` overflow hint.
#[test]
fn help_is_styled_with_shortlist_on_a_tty() {
    let sandbox = Sandbox::new("help-tty");
    let captured = sandbox.run_in_pty(&["--help"], &[]);
    assert!(
        contains_ansi(&captured),
        "TTY --help must be brand-styled with ANSI, got: {:?}",
        String::from_utf8_lossy(&captured)
    );
    let text = String::from_utf8_lossy(&captured);
    assert!(
        text.contains("more") && text.contains("cortex help <command>"),
        "TTY help shows the overflow hint: {text}"
    );
    assert!(text.contains("Commands:"), "TTY help shows the Commands section: {text}");
}

// VAL-CLI-035: `cortex --version` reports the authentic crate version on both a TTY and when piped.
#[test]
fn version_reports_authentic_value() {
    let sandbox = Sandbox::new("version");
    let piped = sandbox.run_piped(&["--version"]);
    assert_eq!(
        String::from_utf8_lossy(&piped.stdout).trim(),
        format!("cortex {}", env!("CARGO_PKG_VERSION")),
    );
    let tty = sandbox.run_in_pty(&["--version"], &[]);
    assert_eq!(
        String::from_utf8_lossy(&tty).trim(),
        format!("cortex {}", env!("CARGO_PKG_VERSION")),
        "the --version line is authentic and unstyled on a TTY too"
    );
}
